#![windows_subsystem = "windows"]

use std::{
    cell::RefCell,
    path::Path,
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{bail, Context};
use slint::{PhysicalPosition, TimerMode};

mod auth;
mod bundled;
mod config;
mod error;
mod flow;
mod hash;
mod http;
mod java;
mod launch;
mod logger;
mod manifest;
mod mc;
mod modconfig;
mod mods;
mod paths;
mod runtime;
mod shell;
mod task;
mod update;

use auth::{
    avatar,
    device::DeviceIdentity,
    minecraft::MinecraftProfile,
    msa,
    session::Session,
    store::SecretStore,
};
use config::{AccountRecord, Config};
use error::{ErrorCode, UserError};
use manifest::Manifest;
use mods::ModInfo;
use paths::Paths;
use task::{Cancel, Stage};

slint::include_modules!();

const SAVE_DEBOUNCE: Duration = Duration::from_millis(400);

#[derive(Clone, Copy)]
struct TitleDragState {
    pointer_x: f32,
    pointer_y: f32,
}

#[derive(Clone)]
struct AppState {
    paths: Arc<Mutex<Option<Paths>>>,
    settings: Arc<Mutex<Config>>,
    secrets: Arc<Mutex<SecretStore>>,
    manifest: Arc<Mutex<Manifest>>,
    cancel: Cancel,
}

impl AppState {
    fn new() -> Self {
        Self {
            paths: Arc::new(Mutex::new(None)),
            settings: Arc::new(Mutex::new(Config::default())),
            secrets: Arc::new(Mutex::new(SecretStore::new())),
            manifest: Arc::new(Mutex::new(manifest::builtin())),
            cancel: Cancel::new(),
        }
    }

    fn cache_dir(&self) -> Option<std::path::PathBuf> {
        self.paths.lock().ok()?.as_ref().map(Paths::cache_dir)
    }

    fn mod_dirs(&self) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
        let held = self.paths.lock().ok()?;
        let paths = held.as_ref()?;

        Some((paths.mods_dir(), paths.disabled_mods_dir()))
    }

    fn manifest(&self) -> Manifest {
        self.manifest
            .lock()
            .map(|manifest| manifest.clone())
            .unwrap_or_else(|_| manifest::builtin())
    }

    fn scan_mods(&self) -> Vec<ModInfo> {
        let Some((mods_dir, disabled_dir)) = self.mod_dirs() else {
            return Vec::new();
        };

        mods::scan(&mods_dir, &disabled_dir, Some(&self.manifest()))
    }

    fn identity(&self) -> anyhow::Result<DeviceIdentity> {
        self.secrets
            .lock()
            .map_err(|_| anyhow::anyhow!("계정 저장소 잠금이 손상됐어요."))?
            .identity()
    }

    fn snapshot(&self) -> Config {
        self.settings
            .lock()
            .map(|settings| settings.clone())
            .unwrap_or_default()
    }

    fn persist(&self) -> anyhow::Result<()> {
        let Some(paths) = self.paths.lock().ok().and_then(|paths| paths.clone()) else {
            return Ok(());
        };

        let settings = self.snapshot();
        settings.save(&paths.config_file())?;

        let secrets = self
            .secrets
            .lock()
            .map_err(|_| anyhow::anyhow!("계정 저장소 잠금이 손상됐어요."))?
            .clone();
        secrets.save(&paths.secrets_file())?;

        Ok(())
    }
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

    let state = AppState::new();
    let save_timer = Rc::new(slint::Timer::default());

    start_up(&app, &state);

    let schedule_save = {
        let timer = Rc::clone(&save_timer);
        let state = state.clone();
        let app_weak = app.as_weak();

        move || {
            let state = state.clone();
            let app_weak = app_weak.clone();

            timer.start(TimerMode::SingleShot, SAVE_DEBOUNCE, move || {
                persist_or_report(&state, &app_weak);
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
            if let (Some(app), Some(drag)) = (app.upgrade(), *title_drag_state.borrow()) {
                let window = app.window();
                let scale_factor = window.scale_factor();
                let delta_x = ((pointer_x - drag.pointer_x) * scale_factor).round() as i32;
                let delta_y = ((pointer_y - drag.pointer_y) * scale_factor).round() as i32;

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
            if let Err(error) = shell::open(&dir.display().to_string()) {
                log::warn!("폴더를 열지 못했습니다: {error:#}");
            }
        }
    });

    app.on_fps_changed({
        let state = state.clone();
        let schedule_save = schedule_save.clone();
        move |fps| {
            if let Ok(mut settings) = state.settings.lock() {
                settings.target_fps = fps;
            }
            schedule_save();
        }
    });

    app.on_adaptive_changed({
        let state = state.clone();
        move |enabled| {
            if let Ok(mut settings) = state.settings.lock() {
                settings.adaptive_rendering = enabled;
            }
            schedule_save();
        }
    });

    app.on_start_clicked({
        let app = app.as_weak();
        let state = state.clone();
        move || {
            if let Some(app) = app.upgrade() {
                start_game(&app, &state);
            }
        }
    });

    app.on_add_mod_clicked({
        let app = app.as_weak();
        let state = state.clone();
        move || {
            let Some(app) = app.upgrade() else {
                return;
            };

            let Some(source) = rfd::FileDialog::new()
                .set_title("모드 파일 선택")
                .add_filter("Minecraft 모드", &["jar"])
                .pick_file()
            else {
                return;
            };

            with_mod_dirs(&app, &state, |mods_dir, disabled_dir, _| {
                mods::add_local(mods_dir, disabled_dir, &source).map(|_| ())
            });
        }
    });

    app.on_toggle_mod({
        let app = app.as_weak();
        let state = state.clone();
        move |id, enabled| {
            let Some(app) = app.upgrade() else {
                return;
            };

            with_mod_dirs(&app, &state, |mods_dir, disabled_dir, entries| {
                mods::set_enabled_by_id(mods_dir, disabled_dir, entries, &id, enabled)
            });
        }
    });

    app.on_remove_mod({
        let app = app.as_weak();
        let state = state.clone();
        move |id| {
            let Some(app) = app.upgrade() else {
                return;
            };

            with_mod_dirs(&app, &state, |mods_dir, disabled_dir, entries| {
                mods::remove_by_id(mods_dir, disabled_dir, entries, &id)
            });
        }
    });

    app.on_login_clicked({
        let app = app.as_weak();
        let state = state.clone();
        move || {
            if let Some(app) = app.upgrade() {
                start_login(&app, &state);
            }
        }
    });

    app.on_add_account_clicked({
        let app = app.as_weak();
        let state = state.clone();
        move || {
            if let Some(app) = app.upgrade() {
                start_login(&app, &state);
            }
        }
    });

    app.on_logout_clicked({
        let app = app.as_weak();
        let state = state.clone();
        move || {
            let Some(app) = app.upgrade() else {
                return;
            };

            let removed = state.snapshot().selected_account;
            let Some(id) = removed else {
                return;
            };

            if let Ok(mut settings) = state.settings.lock() {
                settings.remove_account(&id);
            }
            if let Ok(mut secrets) = state.secrets.lock() {
                secrets.remove(&id);
            }

            log::info!("계정 로그아웃: {id}");
            apply_accounts(&app, &state);
            persist_or_report(&state, &app.as_weak());
        }
    });

    app.on_switch_account({
        let app = app.as_weak();
        let state = state.clone();
        move |id| {
            let Some(app) = app.upgrade() else {
                return;
            };

            if let Ok(mut settings) = state.settings.lock() {
                settings.selected_account = Some(id.to_string());
            }

            log::info!("계정 전환: {id}");
            apply_accounts(&app, &state);
            persist_or_report(&state, &app.as_weak());
        }
    });

    app.on_error_retry({
        let app = app.as_weak();
        let state = state.clone();
        move || {
            if let Some(app) = app.upgrade() {
                start_up(&app, &state);
            }
        }
    });

    app.on_close_clicked({
        let app = app.as_weak();
        let title_drag_state = Rc::clone(&title_drag_state);
        let state = state.clone();
        let save_timer = Rc::clone(&save_timer);
        move || {
            *title_drag_state.borrow_mut() = None;
            state.cancel.cancel();

            save_timer.stop();
            persist_or_report(&state, &app);

            if let Some(app) = app.upgrade() {
                let _ = app.hide();
            }
            slint::quit_event_loop().ok();
        }
    });

    app.show().context("launcher window failed to open")?;

    slint::run_event_loop_until_quit().context("launcher window failed")?;

    Ok(())
}

fn start_up(app: &LauncherWindow, state: &AppState) {
    let resolved = match Paths::resolve().and_then(|resolved| resolved.bootstrap().map(|()| resolved))
    {
        Ok(resolved) => resolved,
        Err(error) => {
            if let Ok(mut paths) = state.paths.lock() {
                *paths = None;
            }
            show_error(app, ErrorCode::Config, &error);
            return;
        }
    };

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
        Err(error) => {
            log::error!("설정을 불러오지 못해 기본값으로 시작합니다: {error:#}");
            show_error(app, ErrorCode::Config, &error);
            Config::default()
        }
    };

    let secrets = match SecretStore::load(&resolved.secrets_file()) {
        Ok(secrets) => secrets,
        Err(error) => {
            log::error!("계정 저장소를 불러오지 못했습니다: {error:#}");
            show_error(app, ErrorCode::Login, &error);
            SecretStore::new()
        }
    };

    app.set_target_fps(loaded.target_fps);
    app.set_adaptive_rendering(loaded.adaptive_rendering);

    if let Ok(mut settings) = state.settings.lock() {
        *settings = loaded;
    }
    if let Ok(mut stored) = state.secrets.lock() {
        *stored = secrets;
    }
    if let Ok(mut stored) = state.manifest.lock() {
        *stored = manifest::load_local(&resolved.cache_dir());
    }
    if let Ok(mut paths) = state.paths.lock() {
        *paths = Some(resolved);
    }

    apply_accounts(app, state);
    refresh_mods(app, state);
    check_for_update(app, state);
}

fn pending_update(state: &AppState) -> Option<update::Version> {
    state
        .manifest
        .lock()
        .ok()
        .and_then(|manifest| update::available(&manifest, env!("CARGO_PKG_VERSION")))
}

fn announce_update(app: &LauncherWindow, found: Option<update::Version>) {
    match found {
        Some(version) => {
            app.set_update_version(format!("v{version}").into());
            app.set_update_available(true);
        }
        None => app.set_update_available(false),
    }
}

fn check_for_update(app: &LauncherWindow, state: &AppState) {
    let Some(paths) = state.paths.lock().ok().and_then(|paths| paths.clone()) else {
        return;
    };

    // The cached manifest answers immediately and works offline; the fetch below
    // only corrects it.
    announce_update(app, pending_update(state));

    let owner = state.clone();
    let weak = app.as_weak();

    std::thread::spawn(move || {
        // A failed check gets a log line and nothing else. The launcher is fully
        // usable without it, and "GitHub was unreachable" is not something the
        // person can act on from a modal.
        let manifest = match manifest::fetch(manifest::DEFAULT_URL, &paths.cache_dir()) {
            Ok(manifest) => manifest,
            Err(error) => {
                log::info!("업데이트 확인을 건너뜁니다: {error:#}");
                return;
            }
        };

        if let Ok(mut stored) = owner.manifest.lock() {
            *stored = manifest;
        }

        let found = pending_update(&owner);

        if let Some(version) = found {
            log::info!("새 런처 버전 v{version} 이(가) 있습니다.");
        }

        let _ = weak.upgrade_in_event_loop(move |app| announce_update(&app, found));
    });
}

fn refresh_mods(app: &LauncherWindow, state: &AppState) {
    let entries: Vec<ModEntry> = mods::local(&state.scan_mods())
        .into_iter()
        .map(|info| ModEntry {
            id: info.id.clone().into(),
            name: info.name.clone().into(),
            description: info.description.clone().into(),
            enabled: info.enabled,
        })
        .collect();

    app.set_local_mods(slint::ModelRc::new(slint::VecModel::from(entries)));
}

fn with_mod_dirs<F>(app: &LauncherWindow, state: &AppState, action: F)
where
    F: FnOnce(&Path, &Path, &[ModInfo]) -> anyhow::Result<()>,
{
    let Some((mods_dir, disabled_dir)) = state.mod_dirs() else {
        return;
    };

    let entries = state.scan_mods();

    if let Err(error) = action(&mods_dir, &disabled_dir, &entries) {
        show_error(app, ErrorCode::Mod, &error);
        return;
    }

    refresh_mods(app, state);
}

fn start_game(app: &LauncherWindow, state: &AppState) {
    let Some(paths) = state.paths.lock().ok().and_then(|paths| paths.clone()) else {
        return;
    };

    state.cancel.reset();

    let cancel = state.cancel.clone();
    let settings = state.snapshot();
    let secrets = match state.secrets.lock() {
        Ok(secrets) => secrets.clone(),
        Err(_) => return,
    };
    let owner = state.clone();
    let weak = app.as_weak();
    let restore = app.as_weak();

    task::spawn(app, ErrorCode::Launch, move |reporter| {
        let outcome = flow::run(&paths, &settings, &secrets, reporter, &cancel)?;

        if let Ok(mut settings) = owner.settings.lock() {
            settings.managed_mods = outcome.managed_mods;
        }

        let _ = weak.upgrade_in_event_loop(move |app| {
            persist_or_report(&owner, &app.as_weak());

            log::info!("Minecraft 실행 완료 · 런처를 숨깁니다.");
            let _ = app.hide();
        });

        let mut child = outcome.child;
        let status = child.wait().context("Minecraft 종료를 기다리지 못했어요.")?;

        log::info!("Minecraft 종료 ({status}) · 런처를 다시 엽니다.");

        let _ = restore.upgrade_in_event_loop(|app| {
            let _ = app.show();
        });

        if !status.success() {
            bail!(
                "Minecraft 가 비정상 종료됐어요. ({status})\n자세한 기록: {}",
                paths.logs_dir().join(flow::GAME_LOG_FILE).display()
            );
        }

        Ok(())
    });
}

fn start_login(app: &LauncherWindow, state: &AppState) {
    let identity = match state.identity() {
        Ok(identity) => identity,
        Err(error) => {
            show_error(app, ErrorCode::Login, &error);
            return;
        }
    };

    state.cancel.reset();

    let cancel = state.cancel.clone();
    let cache_dir = state.cache_dir();
    let settings = Arc::clone(&state.settings);
    let secrets = Arc::clone(&state.secrets);
    let owner = state.clone();
    let weak = app.as_weak();

    task::spawn(app, ErrorCode::Login, move |reporter| {
        reporter.overall(0.05, "Microsoft 로그인 준비 중...");
        let code = msa::request_device_code()?;

        msa::open_in_browser(&code.direct_verification_uri())?;
        reporter.waiting("브라우저에서 로그인을 완료해 주세요.");

        let token = msa::poll_for_token(&code, &cancel)?;
        reporter.overall(0.45, "Xbox 인증 중...");

        let mut session = Session::from_msa_token(identity, token)?;
        session.verify_ownership()?;

        reporter.overall(0.8, "프로필을 불러오는 중...");
        let profile = session.profile()?;

        if let (Some(cache_dir), Some(skin_url)) = (cache_dir.as_deref(), profile.skin_url.as_ref())
        {
            reporter.overall(0.92, "스킨을 불러오는 중...");
            if let Err(error) = avatar::fetch_head(cache_dir, &profile.id, skin_url) {
                log::warn!("스킨을 불러오지 못했습니다: {error:#}");
            }
        }

        record_account(&settings, &secrets, &profile, session.refresh_token());
        log::info!("로그인 완료: {} ({})", profile.name, profile.id);

        let _ = weak.upgrade_in_event_loop(move |app| {
            apply_accounts(&app, &owner);
            persist_or_report(&owner, &app.as_weak());
        });

        Ok(())
    });
}

fn record_account(
    settings: &Arc<Mutex<Config>>,
    secrets: &Arc<Mutex<SecretStore>>,
    profile: &MinecraftProfile,
    refresh_token: &str,
) {
    if let Ok(mut settings) = settings.lock() {
        settings.upsert_account(AccountRecord {
            id: profile.id.clone(),
            name: profile.name.clone(),
        });
    }

    if let Ok(mut secrets) = secrets.lock() {
        secrets.upsert(profile.id.clone(), refresh_token);
    }
}

fn apply_accounts(app: &LauncherWindow, state: &AppState) {
    let settings = state.snapshot();
    let cache_dir = state.cache_dir();
    let selected = settings.selected();

    app.set_signed_in(selected.is_some());
    app.set_account_name(
        selected
            .map(|account| account.name.clone())
            .unwrap_or_default()
            .into(),
    );
    app.set_account_initial(selected.map(AccountRecord::initial).unwrap_or_default().into());
    app.set_account_avatar(load_avatar(cache_dir.as_deref(), selected.map(|a| a.id.as_str())));

    let others: Vec<Account> = settings
        .others()
        .into_iter()
        .map(|record| Account {
            id: record.id.clone().into(),
            name: record.name.clone().into(),
            initial: record.initial().into(),
            avatar: load_avatar(cache_dir.as_deref(), Some(&record.id)),
        })
        .collect();

    app.set_other_accounts(slint::ModelRc::new(slint::VecModel::from(others)));
    app.set_status_hint(if selected.is_some() {
        "".into()
    } else {
        "시작하려면 먼저 로그인해 주세요".into()
    });
}

fn load_avatar(cache_dir: Option<&Path>, id: Option<&str>) -> slint::Image {
    let head = cache_dir
        .zip(id)
        .and_then(|(dir, id)| avatar::load_cached(dir, id));

    match head {
        Some(head) => avatar::to_image(&head),
        None => slint::Image::default(),
    }
}

fn persist_or_report(state: &AppState, app: &slint::Weak<LauncherWindow>) {
    if let Err(error) = state.persist() {
        log::error!("저장 실패: {error:#}");

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
