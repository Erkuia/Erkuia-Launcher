#![windows_subsystem = "windows"]

use anyhow::Context;

mod error;
mod paths;

use error::{ErrorCode, UserError};

slint::include_modules!();

fn main() -> anyhow::Result<()> {
    // Matches the installer so both windows render with the same widget style.
    std::env::set_var("SLINT_STYLE", "fluent-light");

    let paths = paths::Paths::resolve()
        .map_err(|error| report(ErrorCode::Config, &error))
        .context("failed to resolve launcher paths")?;

    let app = LauncherWindow::new().context("failed to create launcher window")?;

    app.set_launcher_version(format!("v{}", env!("CARGO_PKG_VERSION")).into());
    app.set_data_directory(paths.data_dir().display().to_string().into());
    app.set_program_directory(
        paths::install_dir()
            .map(|dir| dir.display().to_string())
            .unwrap_or_default()
            .into(),
    );

    app.run().context("launcher window failed")?;

    Ok(())
}

/// Log the narrowed failure and hand the original error back untouched, so the
/// full context chain still reaches the caller.
fn report(code: ErrorCode, error: &anyhow::Error) -> anyhow::Error {
    // Replaced by the rolling file log in L3-3.
    eprintln!("{}", UserError::from_error(code, error));

    anyhow::anyhow!("{error:#}")
}
