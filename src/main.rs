use std::{path::PathBuf, sync::Arc};

use anyhow::Context;

mod download;
mod install;
mod install_files;
mod manifest;
mod progress;
mod state;

slint::include_modules!();

fn main() -> anyhow::Result<()> {
    let manifest = Arc::new(manifest::load_manifest().context("failed to load installer manifest")?);
    let app = InstallerWindow::new().context("failed to create installer window")?;

    app.set_product_name(manifest.product.name.into());
    app.set_installer_name(manifest.installer.name.into());
    app.set_install_path(manifest.install_plan.default_install_dir.into());
    app.set_run_after_install(manifest.installer.default_run_after_install);
    app.set_create_desktop_shortcut(manifest.installer.default_create_desktop_shortcut);
    app.set_current_step(state::Step::Welcome.index());
    app.set_progress_percent(0);
    app.set_progress_message("설치 준비 중...".into());

    app.on_continue_clicked({
        let app = app.as_weak();
        let manifest = Arc::clone(&manifest);
        move || {
            if let Some(app) = app.upgrade() {
                let current = state::Step::from_index(app.get_current_step());

                if matches!(current, state::Step::InstallPath) {
                    app.set_current_step(state::Step::Installing.index());
                    app.set_progress_percent(0);
                    app.set_progress_message("설치 준비 중...".into());
                    start_install(app.as_weak(), Arc::clone(&manifest), app.get_install_path().into());
                    return;
                }

                app.set_current_step(current.next().index());
            }
        }
    });

    app.run().context("installer window failed")?;
    Ok(())
}

fn start_install(app: slint::Weak<InstallerWindow>, manifest: Arc<manifest::Manifest>, install_path: String) {
    std::thread::spawn(move || {
        let options = install::InstallOptions {
            install_dir: PathBuf::from(install_path),
        };

        let result = install::run_install(&manifest, &options, &mut |event| {
            dispatch_install_event(app.clone(), event);
        });

        if let Err(error) = result {
            dispatch_install_event(
                app,
                progress::InstallEvent::Failed {
                    code: "INSTALL_FAILED".to_string(),
                    message: error.to_string(),
                },
            );
        }
    });
}

fn dispatch_install_event(app: slint::Weak<InstallerWindow>, event: progress::InstallEvent) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(app) = app.upgrade() {
            match event {
                progress::InstallEvent::Progress {
                    stage,
                    local_percent,
                    message,
                } => {
                    app.set_progress_percent(progress::overall_percent(stage, local_percent));
                    app.set_progress_message(message.into());
                }
                progress::InstallEvent::Completed => {
                    app.set_progress_percent(100);
                    app.set_progress_message("설치 완료".into());
                    app.set_current_step(state::Step::Complete.index());
                }
                progress::InstallEvent::Failed { code, message } => {
                    app.set_progress_message(format!("{}: {}", code, message).into());
                }
            }
        }
    });
}
