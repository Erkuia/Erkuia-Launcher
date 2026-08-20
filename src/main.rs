#![windows_subsystem = "windows"]

use std::{cell::RefCell, rc::Rc};

use anyhow::Context;
use slint::PhysicalPosition;

mod error;
mod paths;

use error::{ErrorCode, UserError};
use paths::Paths;

slint::include_modules!();

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
    prepare_paths(&app, &paths);

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

    app.on_error_retry({
        let app = app.as_weak();
        let paths = Rc::clone(&paths);
        move || {
            if let Some(app) = app.upgrade() {
                prepare_paths(&app, &paths);
            }
        }
    });

    app.on_close_clicked({
        let app = app.as_weak();
        let title_drag_state = Rc::clone(&title_drag_state);
        move || {
            *title_drag_state.borrow_mut() = None;
            if let Some(app) = app.upgrade() {
                let _ = app.hide();
            }
            slint::quit_event_loop().ok();
        }
    });

    app.run().context("launcher window failed")?;

    Ok(())
}

fn prepare_paths(app: &LauncherWindow, slot: &Rc<RefCell<Option<Paths>>>) {
    match Paths::resolve().and_then(|paths| paths.bootstrap().map(|()| paths)) {
        Ok(paths) => {
            app.set_data_directory(paths.data_dir().display().to_string().into());
            app.set_error_open(false);
            *slot.borrow_mut() = Some(paths);
        }
        Err(error) => {
            *slot.borrow_mut() = None;
            show_error(app, ErrorCode::Config, &error);
        }
    }
}

fn show_error(app: &LauncherWindow, code: ErrorCode, error: &anyhow::Error) {
    let user_error = UserError::from_error(code, error);

    app.set_error_code(user_error.code.as_str().into());
    app.set_error_message(user_error.message.into());
    app.set_error_retryable(true);
    app.set_error_open(true);
}
