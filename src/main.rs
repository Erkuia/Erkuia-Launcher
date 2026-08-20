#![windows_subsystem = "windows"]

use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc, time::Duration};

use anyhow::Context;
use slint::PhysicalPosition;

mod dialogs;
mod download;
mod elevation;
mod install;
mod install_files;
mod manifest;
mod powershell;
mod progress;
mod shortcuts;
mod state;
mod storage;
mod uninstall;

slint::include_modules!();

#[derive(Clone, Copy)]
struct TitleDragState {
    window_position: PhysicalPosition,
    pointer_x: f32,
    pointer_y: f32,
    scale_factor: f32,
}

fn main() -> anyhow::Result<()> {
    std::env::set_var("SLINT_STYLE", "fluent-light");

    let manifest =
        Arc::new(manifest::load_manifest().context("failed to load installer manifest")?);

    if uninstall::is_uninstall_mode() {
        return uninstall::run_uninstall_from_args(&manifest);
    }

    let is_admin = elevation::is_running_as_admin().unwrap_or(false);
    let resuming_elevated = elevation::is_elevated_install_mode();

    let app = InstallerWindow::new().context("failed to create installer window")?;

    app.set_product_name(manifest.product.name.clone().into());
    app.set_installer_name(manifest.installer.name.clone().into());
    app.set_installer_version(format!("v{}", env!("CARGO_PKG_VERSION")).into());
    let default_install_dir =
        install::resolve_install_path(&manifest.install_plan.default_install_dir)
            .unwrap_or_else(|_| PathBuf::from(&manifest.install_plan.default_install_dir));
    app.set_install_path(default_install_dir.display().to_string().into());
    app.set_run_after_install(manifest.installer.default_run_after_install);
    app.set_create_desktop_shortcut(manifest.installer.default_create_desktop_shortcut);
    app.set_current_step(state::Step::Welcome.index());
    app.set_progress_percent(0);
    app.set_progress_message("설치 준비 중...".into());
    app.set_capacity_text(storage::capacity_text(storage::required_bytes(&manifest), None).into());
    app.set_error_code("".into());
    app.set_error_message("".into());

    let title_drag_state = Rc::new(RefCell::new(None::<TitleDragState>));

    app.on_title_drag_started({
        let app = app.as_weak();
        let title_drag_state = Rc::clone(&title_drag_state);
        move |pointer_x, pointer_y| {
            if let Some(app) = app.upgrade() {
                *title_drag_state.borrow_mut() = Some(TitleDragState {
                    window_position: app.window().position(),
                    pointer_x,
                    pointer_y,
                    scale_factor: app.window().scale_factor(),
                });
            }
        }
    });

    app.on_title_drag_moved({
        let app = app.as_weak();
        let title_drag_state = Rc::clone(&title_drag_state);
        move |pointer_x, pointer_y| {
            if let (Some(app), Some(state)) = (app.upgrade(), *title_drag_state.borrow()) {
                let delta_x = ((pointer_x - state.pointer_x) * state.scale_factor).round() as i32;
                let delta_y = ((pointer_y - state.pointer_y) * state.scale_factor).round() as i32;
                app.window().set_position(PhysicalPosition::new(
                    state.window_position.x + delta_x,
                    state.window_position.y + delta_y,
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

    app.on_start_clicked({
        let app = app.as_weak();
        let manifest = Arc::clone(&manifest);
        move || {
            if let Some(app) = app.upgrade() {
                app.set_current_step(state::Step::InstallPath.index());
                refresh_capacity_text(app.as_weak(), Arc::clone(&manifest));
            }
        }
    });

    app.on_retry_clicked({
        let app = app.as_weak();
        move || {
            if let Some(app) = app.upgrade() {
                app.set_error_code("".into());
                app.set_error_message("".into());
                app.set_current_step(state::Step::InstallPath.index());
            }
        }
    });

    app.on_install_clicked({
        let app = app.as_weak();
        let manifest = Arc::clone(&manifest);
        move || {
            let Some(app) = app.upgrade() else {
                return;
            };

            // The install writes into Program Files and HKLM, so it needs the
            // administrator token before any work starts. Instead of handing the
            // job to a hidden background process, relaunch this same UI elevated
            // and let it resume straight at the install step.
            if manifest.installer.requires_admin_on_install && !is_admin {
                match elevation::restart_as_admin_for_install(
                    &app.get_install_path(),
                    app.get_create_desktop_shortcut(),
                    app.get_run_after_install(),
                ) {
                    Ok(()) => {
                        let _ = app.hide();
                        slint::quit_event_loop().ok();
                    }
                    Err(error) => {
                        app.set_error_code("ADMIN_REQUIRED".into());
                        app.set_error_message(error.to_string().into());
                        app.set_current_step(state::Step::Error.index());
                    }
                }
                return;
            }

            begin_install(&app, Arc::clone(&manifest));
        }
    });

    app.on_launch_clicked({
        let app = app.as_weak();
        let title_drag_state = Rc::clone(&title_drag_state);
        move || {
            *title_drag_state.borrow_mut() = None;
            if let Some(app) = app.upgrade() {
                finish_installer(&app, true, is_admin);
            }
            slint::quit_event_loop().ok();
        }
    });

    app.on_close_clicked({
        let app = app.as_weak();
        let title_drag_state = Rc::clone(&title_drag_state);
        move || {
            *title_drag_state.borrow_mut() = None;
            if let Some(app) = app.upgrade() {
                let launch = state::Step::from_index(app.get_current_step())
                    == state::Step::Complete
                    && app.get_run_after_install();
                finish_installer(&app, launch, is_admin);
            }
            slint::quit_event_loop().ok();
        }
    });

    app.on_browse_clicked({
        let app = app.as_weak();
        let manifest = Arc::clone(&manifest);
        move || {
            if let Some(app) = app.upgrade() {
                let current_path = install::resolve_install_path(&app.get_install_path())
                    .unwrap_or_else(|_| PathBuf::from(app.get_install_path().to_string()));

                if let Some(path) = dialogs::pick_install_directory(&current_path) {
                    app.set_install_path(path.display().to_string().into());
                    refresh_capacity_text(app.as_weak(), Arc::clone(&manifest));
                }
            }
        }
    });

    if resuming_elevated {
        if let Some(install_dir) = elevation::install_dir_from_args() {
            app.set_install_path(install_dir.into());
        }
        app.set_create_desktop_shortcut(
            elevation::desktop_shortcut_from_args()
                .unwrap_or(manifest.installer.default_create_desktop_shortcut),
        );
        app.set_run_after_install(
            elevation::run_after_install_from_args()
                .unwrap_or(manifest.installer.default_run_after_install),
        );

        let app_weak = app.as_weak();
        let manifest = Arc::clone(&manifest);
        slint::Timer::single_shot(Duration::from_millis(80), move || {
            if let Some(app) = app_weak.upgrade() {
                begin_install(&app, Arc::clone(&manifest));
            }
        });
    }

    app.run().context("installer window failed")?;
    Ok(())
}

fn begin_install(app: &InstallerWindow, manifest: Arc<manifest::Manifest>) {
    app.set_current_step(state::Step::Installing.index());
    app.set_progress_percent(0);
    app.set_progress_message("설치 준비 중...".into());
    app.set_error_code("".into());
    app.set_error_message("".into());
    start_install(
        app.as_weak(),
        manifest,
        app.get_install_path().into(),
        app.get_create_desktop_shortcut(),
    );
}

/// Apply the choices that stay editable on the complete page, then optionally
/// hand control over to the freshly installed launcher.
fn finish_installer(app: &InstallerWindow, launch: bool, is_admin: bool) {
    if state::Step::from_index(app.get_current_step()) != state::Step::Complete {
        let _ = app.hide();
        return;
    }

    let install_dir = install::resolve_install_path(&app.get_install_path())
        .unwrap_or_else(|_| PathBuf::from(app.get_install_path().to_string()));

    let _ = shortcuts::apply_desktop_shortcut(&install_dir, app.get_create_desktop_shortcut());

    if launch {
        let _ = install::launch_installed_launcher(&install_dir, is_admin);
    }

    let _ = app.hide();
}

fn refresh_capacity_text(app: slint::Weak<InstallerWindow>, manifest: Arc<manifest::Manifest>) {
    let Some(strong_app) = app.upgrade() else {
        return;
    };
    let install_path = strong_app.get_install_path().to_string();

    std::thread::spawn(move || {
        let required = storage::required_bytes(&manifest);
        let free = storage::free_bytes_for_path(&install_path);
        let text = storage::capacity_text(required, free);

        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = app.upgrade() {
                app.set_capacity_text(text.into());
            }
        });
    });
}

fn start_install(
    app: slint::Weak<InstallerWindow>,
    manifest: Arc<manifest::Manifest>,
    install_path: String,
    create_desktop_shortcut: bool,
) {
    std::thread::spawn(move || {
        let options = install::InstallOptions {
            install_dir: install::resolve_install_path(&install_path)
                .unwrap_or_else(|_| PathBuf::from(install_path)),
            create_desktop_shortcut,
        };

        let result = install::run_install(&manifest, &options, &mut |event| {
            dispatch_install_event(app.clone(), Arc::clone(&manifest), event);
        });

        if let Err(error) = result {
            dispatch_install_event(
                app,
                Arc::clone(&manifest),
                progress::InstallEvent::Failed {
                    code: "INSTALL_FAILED".to_string(),
                    message: error.to_string(),
                },
            );
        }
    });
}

fn dispatch_install_event(
    app: slint::Weak<InstallerWindow>,
    manifest: Arc<manifest::Manifest>,
    event: progress::InstallEvent,
) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(app) = app.upgrade() {
            match event {
                progress::InstallEvent::Progress {
                    stage,
                    local_percent,
                    message,
                } => {
                    app.set_progress_percent(progress::overall_percent_with_weights(
                        &manifest.progress_weights,
                        stage,
                        local_percent,
                    ));
                    app.set_progress_message(message.into());
                }
                progress::InstallEvent::Completed {
                    install_dir,
                    installed_count,
                } => {
                    app.set_progress_percent(100);
                    app.set_progress_message(
                        format!(
                            "설치 완료: {}개 항목이 {}에 설치됐어요.",
                            installed_count, install_dir
                        )
                        .into(),
                    );
                    app.set_current_step(state::Step::Complete.index());
                }
                progress::InstallEvent::Failed { code, message } => {
                    app.set_error_code(code.into());
                    app.set_error_message(message.into());
                    app.set_current_step(state::Step::Error.index());
                }
            }
        }
    });
}
