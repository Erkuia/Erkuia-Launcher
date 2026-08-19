use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{bail, Context};

use crate::{
    manifest::Manifest,
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
        message: "제거 프로그램 등록 중...".to_string(),
    });

    let installer_path =
        std::env::current_exe().context("failed to resolve current installer executable")?;
    let uninstall_command = format!(
        "\"{}\" --uninstall --install-dir \"{}\"",
        installer_path.display(),
        install_dir.display()
    );

    let script = format!(
        "New-Item -Path '{}' -Force | Out-Null; \
         New-ItemProperty -Path '{}' -Name DisplayName -Value '{}' -PropertyType String -Force | Out-Null; \
         New-ItemProperty -Path '{}' -Name DisplayVersion -Value '{}' -PropertyType String -Force | Out-Null; \
         New-ItemProperty -Path '{}' -Name Publisher -Value '{}' -PropertyType String -Force | Out-Null; \
         New-ItemProperty -Path '{}' -Name InstallLocation -Value '{}' -PropertyType String -Force | Out-Null; \
         New-ItemProperty -Path '{}' -Name UninstallString -Value '{}' -PropertyType String -Force | Out-Null; \
         New-ItemProperty -Path '{}' -Name DisplayIcon -Value '{}' -PropertyType String -Force | Out-Null",
        escape_powershell_single_quoted(UNINSTALL_KEY),
        escape_powershell_single_quoted(UNINSTALL_KEY),
        escape_powershell_single_quoted(&manifest.uninstall.display_name),
        escape_powershell_single_quoted(UNINSTALL_KEY),
        env!("CARGO_PKG_VERSION"),
        escape_powershell_single_quoted(UNINSTALL_KEY),
        "Rendog",
        escape_powershell_single_quoted(UNINSTALL_KEY),
        escape_powershell_single_quoted(&install_dir.display().to_string()),
        escape_powershell_single_quoted(UNINSTALL_KEY),
        escape_powershell_single_quoted(&uninstall_command),
        escape_powershell_single_quoted(UNINSTALL_KEY),
        escape_powershell_single_quoted(&installer_path.display().to_string())
    );

    run_powershell(&script).context("failed to write uninstall registry entry")?;

    emit(InstallEvent::Progress {
        stage: InstallStage::RegisterUninstaller,
        local_percent: 100.0,
        message: "제거 프로그램 등록 완료".to_string(),
    });

    Ok(())
}

pub fn is_uninstall_mode() -> bool {
    std::env::args().any(|arg| arg == "--uninstall")
}

pub fn run_uninstall_from_args(manifest: &Manifest) -> anyhow::Result<()> {
    let install_dir = uninstall_install_dir_from_args()
        .or_else(|| default_install_dir_from_manifest(manifest).ok())
        .context("missing uninstall install directory")?;

    run_uninstall(manifest, &install_dir)
}

fn run_uninstall(manifest: &Manifest, install_dir: &Path) -> anyhow::Result<()> {
    remove_shortcuts(manifest)?;

    if install_dir.exists() {
        std::fs::remove_dir_all(install_dir)
            .with_context(|| format!("failed to remove {}", install_dir.display()))?;
    }

    run_powershell(&format!(
        "Remove-Item -Path '{}' -Recurse -Force -ErrorAction SilentlyContinue",
        escape_powershell_single_quoted(UNINSTALL_KEY)
    ))
    .context("failed to remove uninstall registry entry")?;

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
        let program_files =
            std::env::var("ProgramFiles").context("ProgramFiles environment variable is missing")?;
        return Ok(Path::new(&program_files).join(rest.trim_start_matches(['\\', '/'])));
    }

    Ok(PathBuf::from(&manifest.install_plan.default_install_dir))
}

fn run_powershell(script: &str) -> anyhow::Result<()> {
    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .context("failed to run PowerShell")?;

    if !output.status.success() {
        bail!("PowerShell command failed");
    }

    Ok(())
}

fn escape_powershell_single_quoted(value: &str) -> String {
    value.replace('\'', "''")
}
