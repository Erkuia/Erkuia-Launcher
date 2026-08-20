#![windows_subsystem = "windows"]

use anyhow::Context;

mod error;
mod paths;

use error::{ErrorCode, UserError};

slint::include_modules!();

fn main() -> anyhow::Result<()> {
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

fn report(code: ErrorCode, error: &anyhow::Error) -> anyhow::Error {
    eprintln!("{}", UserError::from_error(code, error));

    anyhow::anyhow!("{error:#}")
}
