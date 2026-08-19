use anyhow::Context;

mod download;
mod install_files;
mod manifest;
mod progress;
mod state;

slint::include_modules!();

fn main() -> anyhow::Result<()> {
    let manifest = manifest::load_manifest().context("failed to load installer manifest")?;
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
        move || {
            if let Some(app) = app.upgrade() {
                let next = state::Step::from_index(app.get_current_step()).next();
                app.set_current_step(next.index());
            }
        }
    });

    app.run().context("installer window failed")?;
    Ok(())
}
