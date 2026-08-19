use anyhow::{bail, Context};

use crate::powershell;

pub fn is_install_mode() -> bool {
    std::env::args().any(|arg| arg == "--install")
}

pub fn install_dir_from_args() -> Option<String> {
    value_after_arg("--install-dir")
}

pub fn desktop_shortcut_from_args() -> Option<bool> {
    value_after_arg("--desktop-shortcut").and_then(|value| match value.as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    })
}

pub fn run_after_install_from_args() -> Option<bool> {
    value_after_arg("--run-after-install").and_then(|value| match value.as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    })
}

pub fn is_running_as_admin() -> anyhow::Result<bool> {
    let output = powershell::output(&[
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)",
        ])
        .context("failed to check administrator status")?;

    if !output.status.success() {
        bail!("administrator status check failed");
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim() == "True")
}

pub fn restart_as_admin_for_install(
    install_dir: &str,
    create_desktop_shortcut: bool,
    run_after_install: bool,
) -> anyhow::Result<()> {
    let exe = std::env::current_exe().context("failed to resolve current executable")?;
    let args = format!(
        "--install --install-dir \"{}\" --desktop-shortcut {} --run-after-install {}",
        powershell::escape_double_quoted(install_dir),
        create_desktop_shortcut,
        run_after_install
    );
    let script = format!(
        "Start-Process -FilePath \"{}\" -ArgumentList '{}' -Verb RunAs -WindowStyle Hidden",
        powershell::escape_double_quoted(&exe.display().to_string()),
        powershell::escape_single_quoted(&args)
    );
    let output = powershell::output(&["-NoProfile", "-NonInteractive", "-Command", &script])
        .context("failed to request administrator permission")?;

    if !output.status.success() {
        bail!("administrator permission request was not completed");
    }

    Ok(())
}

fn value_after_arg(name: &str) -> Option<String> {
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        if arg == name {
            return args.next();
        }
    }

    None
}
