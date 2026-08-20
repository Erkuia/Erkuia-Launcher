#![windows_subsystem = "windows"]

use std::{cell::RefCell, rc::Rc, time::Duration};

use anyhow::Context;
use slint::{PhysicalPosition, TimerMode};

mod config;
mod error;
mod logger;
mod paths;

use config::Config;
use error::{ErrorCode, UserError};
use paths::Paths;

slint::include_modules!();

const SAVE_DEBOUNCE: Duration = Duration::from_millis(400);

#[derive(Clone, Copy)]
struct TitleDragState {
    pointer_x: f32,
    pointer_y: f32,
}

fn main() -> anyhow::Result<()> {
    std::env::set_var("SLINT_STYLE", "fluent-light");

    let app = LauncherWindow::new().context("failed to create launcher window")?;

    app.set_launcher_version(format!("v{}", env!("CARGO_PKG_VERSION")).into());
    app.set_program_directory(
        paths::install_dir()
            .map(|dir| dir.display().to_string())
            .unwrap_or_default()
            .into(),
    );

    let paths: Rc<RefCell<Option<Paths>>> = Rc::new(RefCell::new(None));
    let settings: Rc<RefCell<Config>> = Rc::new(RefCell::new(Config::default()));
    let save_timer = Rc::new(slint::Timer::default());

    start_up(&app, &paths, &settings);

    let schedule_save = {
        let timer = Rc::clone(&save_timer);
        let paths = Rc::clone(&paths);
        let settings = Rc::clone(&settings);
        let app_weak = app.as_weak();

        move || {
            let paths = Rc::clone(&paths);
            let settings = Rc::clone(&settings);
            let app_weak = app_weak.clone();

            timer.start(TimerMode::SingleShot, SAVE_DEBOUNCE, move || {
                save_settings(&paths, &settings, &app_weak);
            });
        }
    };

    let title_drag_state = Rc::new(RefCell::new(None::<TitleDragState>));

    app.on_title_drag_started({
        let app = app.as_weak();
        let title_drag_state = Rc::clone(&title_drag_state);
        move |pointer_x, pointer_y| {
            if app.upgrade().is_some() {
                *title_drag_state.borrow_mut() = Some(TitleDragState {
                    pointer_x,
                    pointer_y,
                });
            }
        }
    });

    app.on_title_drag_moved({
        let app = app.as_weak();
        let title_drag_state = Rc::clone(&title_drag_state);
        move |pointer_x, pointer_y| {
            if let (Some(app), Some(state)) = (app.upgrade(), *title_drag_state.borrow()) {
                let window = app.window();
                let scale_factor = window.scale_factor();
                let delta_x = ((pointer_x - state.pointer_x) * scale_factor).round() as i32;
                let delta_y = ((pointer_y - state.pointer_y) * scale_factor).round() as i32;

                if delta_x == 0 && delta_y == 0 {
                    return;
                }

                let current = window.position();
                window.set_position(PhysicalPosition::new(
                    current.x + delta_x,
                    current.y + delta_y,
                ));
            }
        }
    });

    app.on_title_drag_ended({
        let title_drag_state = Rc::clone(&title_drag_state);
        move || {
            *title_drag_state.borrow_mut() = None;
        }
    });

    app.on_minimize_clicked({
        let app = app.as_weak();
        let title_drag_state = Rc::clone(&title_drag_state);
        move || {
            *title_drag_state.borrow_mut() = None;
            if let Some(app) = app.upgrade() {
                app.window().set_minimized(true);
            }
        }
    });

    app.on_open_directory_clicked(move || {
        if let Ok(dir) = paths::install_dir() {
            let _ = std::process::Command::new("explorer.exe").arg(dir).spawn();
        }
    });

    app.on_fps_changed({
        let settings = Rc::clone(&settings);
        let schedule_save = schedule_save.clone();
        move |fps| {
            settings.borrow_mut().target_fps = fps;
            schedule_save();
        }
    });

    app.on_adaptive_changed({
        let settings = Rc::clone(&settings);
        move |enabled| {
            settings.borrow_mut().adaptive_rendering = enabled;
            schedule_save();
        }
    });

    app.on_error_retry({
        let app = app.as_weak();
        let paths = Rc::clone(&paths);
        let settings = Rc::clone(&settings);
        move || {
            if let Some(app) = app.upgrade() {
                start_up(&app, &paths, &settings);
            }
        }
    });

    app.on_close_clicked({
        let app = app.as_weak();
        let title_drag_state = Rc::clone(&title_drag_state);
        let paths = Rc::clone(&paths);
        let settings = Rc::clone(&settings);
        let save_timer = Rc::clone(&save_timer);
        move || {
            *title_drag_state.borrow_mut() = None;

            // A debounced save may still be pending; write it out now instead of
            // dropping the user's last change on the way out.
            save_timer.stop();
            save_settings(&paths, &settings, &app);

            if let Some(app) = app.upgrade() {
                let _ = app.hide();
            }
            slint::quit_event_loop().ok();
        }
    });

    app.run().context("launcher window failed")?;

    Ok(())
}

fn start_up(
    app: &LauncherWindow,
    paths: &Rc<RefCell<Option<Paths>>>,
    settings: &Rc<RefCell<Config>>,
) {
    let resolved = match Paths::resolve().and_then(|resolved| resolved.bootstrap().map(|()| resolved))
    {
        Ok(resolved) => resolved,
        Err(error) => {
            *paths.borrow_mut() = None;
            show_error(app, ErrorCode::Config, &error);
            return;
        }
    };

    // Logging comes up before anything else can fail, so later errors leave a
    // trace even though the window has no console to print to.
    if let Err(error) = logger::init(&resolved.logs_dir()) {
        show_error(app, ErrorCode::Config, &error);
    }

    log::info!(
        "Rendog Launcher v{} 시작 · data={}",
        env!("CARGO_PKG_VERSION"),
        resolved.data_dir().display()
    );

    app.set_data_directory(resolved.data_dir().display().to_string().into());

    let loaded = match Config::load(&resolved.config_file()) {
        Ok(loaded) => {
            app.set_error_open(false);
            loaded
        }
        // A broken config must not block startup: fall back to defaults, but say
        // so rather than silently overwriting whatever the user had.
        Err(error) => {
            log::error!("설정을 불러오지 못해 기본값으로 시작합니다: {error:#}");
            show_error(app, ErrorCode::Config, &error);
            Config::default()
        }
    };

    app.set_target_fps(loaded.target_fps);
    app.set_adaptive_rendering(loaded.adaptive_rendering);
    apply_accounts(app, &loaded);

    *settings.borrow_mut() = loaded;
    *paths.borrow_mut() = Some(resolved);
}

fn apply_accounts(app: &LauncherWindow, settings: &Config) {
    let selected = settings.selected();

    app.set_signed_in(selected.is_some());
    app.set_account_name(selected.map(|a| a.name.clone()).unwrap_or_default().into());
    app.set_account_initial(selected.map(|a| a.initial()).unwrap_or_default().into());

    let others: Vec<Account> = settings
        .others()
        .into_iter()
        .map(|record| Account {
            id: record.id.clone().into(),
            name: record.name.clone().into(),
            initial: record.initial().into(),
            avatar: slint::Image::default(),
        })
        .collect();

    app.set_other_accounts(slint::ModelRc::new(slint::VecModel::from(others)));
}

fn save_settings(
    paths: &Rc<RefCell<Option<Paths>>>,
    settings: &Rc<RefCell<Config>>,
    app: &slint::Weak<LauncherWindow>,
) {
    let Some(path) = paths.borrow().as_ref().map(Paths::config_file) else {
        return;
    };
    let snapshot = settings.borrow().clone();

    if let Err(error) = snapshot.save(&path) {
        log::error!("설정 저장 실패: {error:#}");

        if let Some(app) = app.upgrade() {
            show_error(&app, ErrorCode::Config, &error);
        }
    }
}

fn show_error(app: &LauncherWindow, code: ErrorCode, error: &anyhow::Error) {
    let user_error = UserError::from_error(code, error);

    log::error!("[{}] {error:#}", user_error.code);

    app.set_error_code(user_error.code.as_str().into());
    app.set_error_message(user_error.message.into());
    app.set_error_retryable(true);
    app.set_error_open(true);
}
