use std::path::{Path, PathBuf};

use anyhow::{bail, Context};

use crate::{
    elevation,
    manifest::Manifest,
    powershell,
    progress::{InstallEvent, InstallStage},
};

type EventSink<'a> = &'a mut dyn FnMut(InstallEvent);

const UNINSTALL_KEY: &str =
    r"HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\RendogLauncher";

pub fn register_uninstaller(
    manifest: &Manifest,
    install_dir: &Path,
    emit: EventSink<'_>,
) -> anyhow::Result<()> {
    emit(InstallEvent::Progress {
        stage: InstallStage::RegisterUninstaller,
        local_percent: 0.0,
        message: "설치 완료 중...".to_string(),
    });

    let installer_path = copy_uninstaller_to_install_dir(install_dir)?;
    let uninstall_command = format!(
        "{} {} --install-dir {}",
        powershell::quote_command_line_arg(&installer_path.display().to_string()),
        elevation::UNINSTALL_FLAG,
        powershell::quote_command_line_arg(&install_dir.display().to_string())
    );

    let script = format!(
        "New-Item -Path '{}' -Force | Out-Null; \
         New-ItemProperty -Path '{}' -Name DisplayName -Value '{}' -PropertyType String -Force | Out-Null; \
         New-ItemProperty -Path '{}' -Name DisplayVersion -Value '{}' -PropertyType String -Force | Out-Null; \
         New-ItemProperty -Path '{}' -Name Publisher -Value '{}' -PropertyType String -Force | Out-Null; \
         New-ItemProperty -Path '{}' -Name InstallLocation -Value '{}' -PropertyType String -Force | Out-Null; \
         New-ItemProperty -Path '{}' -Name UninstallString -Value '{}' -PropertyType String -Force | Out-Null; \
         New-ItemProperty -Path '{}' -Name DisplayIcon -Value '{}' -PropertyType String -Force | Out-Null",
        powershell::escape_single_quoted(UNINSTALL_KEY),
        powershell::escape_single_quoted(UNINSTALL_KEY),
        powershell::escape_single_quoted(&manifest.uninstall.display_name),
        powershell::escape_single_quoted(UNINSTALL_KEY),
        env!("CARGO_PKG_VERSION"),
        powershell::escape_single_quoted(UNINSTALL_KEY),
        "Rendog",
        powershell::escape_single_quoted(UNINSTALL_KEY),
        powershell::escape_single_quoted(&install_dir.display().to_string()),
        powershell::escape_single_quoted(UNINSTALL_KEY),
        powershell::escape_single_quoted(&uninstall_command),
        powershell::escape_single_quoted(UNINSTALL_KEY),
        powershell::escape_single_quoted(&installer_path.display().to_string())
    );

    run_powershell(&script).context("failed to write uninstall registry entry")?;

    emit(InstallEvent::Progress {
        stage: InstallStage::RegisterUninstaller,
        local_percent: 100.0,
        message: "설치 완료 중...".to_string(),
    });

    Ok(())
}

pub fn is_uninstall_mode() -> bool {
    std::env::args()
        .skip(1)
        .any(|arg| arg == elevation::UNINSTALL_FLAG)
}

pub fn run_uninstall_from_args(manifest: &Manifest) -> anyhow::Result<()> {
    let install_dir = uninstall_install_dir_from_args()
        .or_else(|| default_install_dir_from_manifest(manifest).ok())
        .context("missing uninstall install directory")?;

    if !elevation::is_running_as_admin().unwrap_or(false) {
        elevation::restart_as_admin_for_uninstall(&install_dir)?;
        return Ok(());
    }

    run_uninstall(manifest, &install_dir)
}

fn run_uninstall(manifest: &Manifest, install_dir: &Path) -> anyhow::Result<()> {
    remove_shortcuts(manifest)?;

    if manifest.uninstall.preserve_user_data_by_default {
        preserve_user_data(install_dir)?;
    }

    run_powershell(&format!(
        "Remove-Item -Path '{}' -Recurse -Force -ErrorAction SilentlyContinue",
        powershell::escape_single_quoted(UNINSTALL_KEY)
    ))
    .context("failed to remove uninstall registry entry")?;

    schedule_install_dir_removal(install_dir)?;

    Ok(())
}

fn preserve_user_data(install_dir: &Path) -> anyhow::Result<()> {
    let user_data_dir = install_dir.join("user-data");

    if !user_data_dir.exists() {
        return Ok(());
    }

    let backup_dir = install_dir.with_file_name("Rendog Launcher User Data");
    if backup_dir.exists() {
        std::fs::remove_dir_all(&backup_dir)
            .with_context(|| format!("failed to replace {}", backup_dir.display()))?;
    }

    std::fs::rename(&user_data_dir, &backup_dir).with_context(|| {
        format!(
            "failed to preserve user data from {} to {}",
            user_data_dir.display(),
            backup_dir.display()
        )
    })?;

    Ok(())
}

fn copy_uninstaller_to_install_dir(install_dir: &Path) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(install_dir)
        .with_context(|| format!("failed to create {}", install_dir.display()))?;

    let current_exe =
        std::env::current_exe().context("failed to resolve current installer executable")?;
    let installed_uninstaller = install_dir.join("RendogLauncherInstaller.exe");

    if current_exe != installed_uninstaller {
        std::fs::copy(&current_exe, &installed_uninstaller).with_context(|| {
            format!(
                "failed to copy uninstaller from {} to {}",
                current_exe.display(),
                installed_uninstaller.display()
            )
        })?;
    }

    Ok(installed_uninstaller)
}

fn schedule_install_dir_removal(install_dir: &Path) -> anyhow::Result<()> {
    if !install_dir.exists() {
        return Ok(());
    }

    let script = format!(
        "Start-Sleep -Seconds 1; Remove-Item -LiteralPath '{}' -Recurse -Force -ErrorAction SilentlyContinue",
        powershell::escape_single_quoted(&install_dir.display().to_string())
    );

    powershell::spawn_hidden(&script).context("failed to schedule install directory removal")?;

    Ok(())
}

fn remove_shortcuts(manifest: &Manifest) -> anyhow::Result<()> {
    let mut script = String::new();

    if manifest.uninstall.remove_desktop_shortcut {
        script.push_str(
            "$w = New-Object -ComObject WScript.Shell; \
             $desktop = $w.SpecialFolders('Desktop'); \
             Remove-Item -LiteralPath (Join-Path $desktop 'Rendog Launcher.lnk') -Force -ErrorAction SilentlyContinue; ",
        );
    }

    if manifest.uninstall.remove_start_menu_shortcut {
        script.push_str(
            "$w = New-Object -ComObject WScript.Shell; \
             $programs = $w.SpecialFolders('Programs'); \
             Remove-Item -LiteralPath (Join-Path $programs 'Rendog Launcher') -Recurse -Force -ErrorAction SilentlyContinue; ",
        );
    }

    if !script.is_empty() {
        run_powershell(&script).context("failed to remove shortcuts")?;
    }

    Ok(())
}

fn uninstall_install_dir_from_args() -> Option<PathBuf> {
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        if arg == "--install-dir" {
            return args.next().map(PathBuf::from);
        }
    }

    None
}

fn default_install_dir_from_manifest(manifest: &Manifest) -> anyhow::Result<PathBuf> {
    if let Some(rest) = manifest
        .install_plan
        .default_install_dir
        .strip_prefix("%ProgramFiles%")
    {
        let program_files = std::env::var("ProgramFiles")
            .context("ProgramFiles environment variable is missing")?;
        return Ok(Path::new(&program_files).join(rest.trim_start_matches(['\\', '/'])));
    }

    Ok(PathBuf::from(&manifest.install_plan.default_install_dir))
}

fn run_powershell(script: &str) -> anyhow::Result<()> {
    let output = powershell::output(&["-NoProfile", "-NonInteractive", "-Command", script])?;

    if !output.status.success() {
        bail!("PowerShell command failed");
    }

    Ok(())
}
