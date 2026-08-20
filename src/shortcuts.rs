use std::path::{Path, PathBuf};

use anyhow::{bail, Context};

use crate::{
    powershell,
    progress::{InstallEvent, InstallStage},
};

type EventSink<'a> = &'a mut dyn FnMut(InstallEvent);

const SHORTCUT_FILE_NAME: &str = "Rendog Launcher.lnk";
const START_MENU_FOLDER_NAME: &str = "Rendog Launcher";
const FINALIZE_MESSAGE: &str = "설치 완료 중...";

pub fn create_launcher_shortcuts(
    install_dir: &Path,
    create_desktop_shortcut: bool,
    emit: EventSink<'_>,
) -> anyhow::Result<()> {
    emit(InstallEvent::Progress {
        stage: InstallStage::Shortcuts,
        local_percent: 0.0,
        message: FINALIZE_MESSAGE.to_string(),
    });

    let launcher_path = install_dir.join("RendogLauncher.exe");
    if !launcher_path.exists() {
        emit(InstallEvent::Progress {
            stage: InstallStage::Shortcuts,
            local_percent: 100.0,
            message: "런처 파일이 아직 준비되지 않아 바로가기 생성을 건너뛰었어요.".to_string(),
        });
        return Ok(());
    }

    apply_desktop_shortcut(install_dir, create_desktop_shortcut)
        .context("failed to apply desktop shortcut")?;

    emit(InstallEvent::Progress {
        stage: InstallStage::Shortcuts,
        local_percent: 50.0,
        message: FINALIZE_MESSAGE.to_string(),
    });

    let start_menu_dir = known_folder_path("Programs")?.join(START_MENU_FOLDER_NAME);
    std::fs::create_dir_all(&start_menu_dir).with_context(|| {
        format!(
            "failed to create start menu directory {}",
            start_menu_dir.display()
        )
    })?;
    create_shortcut(
        &start_menu_dir.join(SHORTCUT_FILE_NAME),
        &launcher_path,
        install_dir,
    )
    .context("failed to create start menu shortcut")?;

    emit(InstallEvent::Progress {
        stage: InstallStage::Shortcuts,
        local_percent: 100.0,
        message: FINALIZE_MESSAGE.to_string(),
    });

    Ok(())
}

/// Create or remove the desktop shortcut so it matches `enabled`. The complete
/// page keeps this option editable after the install finished, so this has to
/// stay idempotent in both directions.
pub fn apply_desktop_shortcut(install_dir: &Path, enabled: bool) -> anyhow::Result<()> {
    let desktop_shortcut = known_folder_path("Desktop")?.join(SHORTCUT_FILE_NAME);

    if !enabled {
        if desktop_shortcut.exists() {
            std::fs::remove_file(&desktop_shortcut).with_context(|| {
                format!("failed to remove {}", desktop_shortcut.display())
            })?;
        }
        return Ok(());
    }

    let launcher_path = install_dir.join("RendogLauncher.exe");
    if !launcher_path.exists() {
        return Ok(());
    }

    create_shortcut(&desktop_shortcut, &launcher_path, install_dir)
        .context("failed to create desktop shortcut")
}

fn known_folder_path(shell_folder_name: &str) -> anyhow::Result<PathBuf> {
    let script = format!(
        "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; $w = New-Object -ComObject WScript.Shell; Write-Output $w.SpecialFolders('{}')",
        powershell::escape_single_quoted(shell_folder_name)
    );
    let output = powershell::output(&["-NoProfile", "-NonInteractive", "-Command", &script])
        .context("failed to query Windows shell folder")?;

    if !output.status.success() {
        bail!("Windows shell folder query failed");
    }

    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        bail!("Windows shell folder query returned an empty path");
    }

    Ok(PathBuf::from(path))
}

fn create_shortcut(
    shortcut_path: &Path,
    target_path: &Path,
    working_dir: &Path,
) -> anyhow::Result<()> {
    let script = format!(
        "$w = New-Object -ComObject WScript.Shell; $s = $w.CreateShortcut('{}'); $s.TargetPath = '{}'; $s.WorkingDirectory = '{}'; $s.Save()",
        powershell::escape_single_quoted(&shortcut_path.display().to_string()),
        powershell::escape_single_quoted(&target_path.display().to_string()),
        powershell::escape_single_quoted(&working_dir.display().to_string())
    );
    let output = powershell::output(&["-NoProfile", "-NonInteractive", "-Command", &script])
        .context("failed to run shortcut creation script")?;

    if !output.status.success() {
        bail!("shortcut creation script failed");
    }

    Ok(())
}
