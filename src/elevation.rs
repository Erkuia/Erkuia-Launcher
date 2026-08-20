use std::path::Path;

use anyhow::{bail, Context};

use crate::powershell;

/// Set on the elevated child process. The child keeps the full installer UI and
/// resumes at the install step, so the user never loses the window while the
/// UAC prompt is answered.
pub const ELEVATED_INSTALL_FLAG: &str = "--elevated-install";
pub const UNINSTALL_FLAG: &str = "--uninstall";

pub fn is_elevated_install_mode() -> bool {
    has_flag(ELEVATED_INSTALL_FLAG)
}

pub fn install_dir_from_args() -> Option<String> {
    value_after_arg("--install-dir")
}

pub fn data_dir_from_args() -> Option<String> {
    value_after_arg("--data-dir")
}

pub fn desktop_shortcut_from_args() -> Option<bool> {
    bool_after_arg("--desktop-shortcut")
}

pub fn run_after_install_from_args() -> Option<bool> {
    bool_after_arg("--run-after-install")
}

/// Physical screen position the pre-elevation window was sitting at, so the
/// elevated process can reopen exactly where the user left it instead of
/// jumping back to the default spot.
pub fn window_position_from_args() -> Option<(i32, i32)> {
    let x = value_after_arg("--window-x")?.parse().ok()?;
    let y = value_after_arg("--window-y")?.parse().ok()?;

    Some((x, y))
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
    data_dir: &Path,
    create_desktop_shortcut: bool,
    run_after_install: bool,
    window_position: (i32, i32),
) -> anyhow::Result<()> {
    let args = vec![
        ELEVATED_INSTALL_FLAG.to_string(),
        "--install-dir".to_string(),
        install_dir.to_string(),
        "--data-dir".to_string(),
        data_dir.display().to_string(),
        "--desktop-shortcut".to_string(),
        create_desktop_shortcut.to_string(),
        "--run-after-install".to_string(),
        run_after_install.to_string(),
        "--window-x".to_string(),
        window_position.0.to_string(),
        "--window-y".to_string(),
        window_position.1.to_string(),
    ];

    start_elevated(&args)
}

pub fn restart_as_admin_for_uninstall(install_dir: &Path, data_dir: &Path) -> anyhow::Result<()> {
    let args = vec![
        UNINSTALL_FLAG.to_string(),
        "--install-dir".to_string(),
        install_dir.display().to_string(),
        "--data-dir".to_string(),
        data_dir.display().to_string(),
    ];

    start_elevated(&args)
}

/// Re-launch this executable through `Start-Process -Verb RunAs`, which is what
/// raises the UAC prompt. `-WindowStyle Hidden` is deliberately not used: the
/// elevated process owns the installer window from here on.
fn start_elevated(args: &[String]) -> anyhow::Result<()> {
    let exe = std::env::current_exe().context("failed to resolve current executable")?;
    let argument_list = powershell::join_command_line_args(args);
    let script = format!(
        "$ErrorActionPreference = 'Stop'; try {{ Start-Process -FilePath '{}' -ArgumentList '{}' -Verb RunAs; exit 0 }} catch {{ exit 1 }}",
        powershell::escape_single_quoted(&exe.display().to_string()),
        powershell::escape_single_quoted(&argument_list)
    );

    let output = powershell::output(&["-NoProfile", "-NonInteractive", "-Command", &script])
        .context("failed to request administrator permission")?;

    if !output.status.success() {
        bail!("관리자 권한 요청이 취소되었거나 완료되지 않았어요. 다시 시도해 주세요.");
    }

    Ok(())
}

fn has_flag(name: &str) -> bool {
    std::env::args().skip(1).any(|arg| arg == name)
}

fn bool_after_arg(name: &str) -> Option<bool> {
    value_after_arg(name).and_then(|value| match value.as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    })
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
