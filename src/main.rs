#![windows_subsystem = "windows"]

use anyhow::Context;

slint::include_modules!();

fn main() -> anyhow::Result<()> {
    // Matches the installer so both windows render with the same widget style.
    std::env::set_var("SLINT_STYLE", "fluent-light");

    let app = LauncherWindow::new().context("failed to create launcher window")?;

    app.set_launcher_version(format!("v{}", env!("CARGO_PKG_VERSION")).into());

    app.run().context("launcher window failed")?;

    Ok(())
}
