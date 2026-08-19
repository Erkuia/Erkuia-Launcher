use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{bail, Context};

use crate::progress::{InstallEvent, InstallStage};

type EventSink<'a> = &'a mut dyn FnMut(InstallEvent);

pub fn create_launcher_shortcuts(
    install_dir: &Path,
    create_desktop_shortcut: bool,
    emit: EventSink<'_>,
) -> anyhow::Result<()> {
    emit(InstallEvent::Progress {
        stage: InstallStage::Shortcuts,
        local_percent: 0.0,
        message: "바로가기 생성 준비 중...".to_string(),
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

    if create_desktop_shortcut {
        let desktop_shortcut = known_folder_path("Desktop")?.join("Rendog Launcher.lnk");
        create_shortcut(&desktop_shortcut, &launcher_path, install_dir)
            .context("failed to create desktop shortcut")?;
    }

    emit(InstallEvent::Progress {
        stage: InstallStage::Shortcuts,
        local_percent: 50.0,
        message: "시작 메뉴 바로가기 생성 중...".to_string(),
    });

    let start_menu_dir = known_folder_path("Programs")?.join("Rendog Launcher");
    std::fs::create_dir_all(&start_menu_dir).with_context(|| {
        format!(
            "failed to create start menu directory {}",
            start_menu_dir.display()
        )
    })?;
    create_shortcut(
        &start_menu_dir.join("Rendog Launcher.lnk"),
        &launcher_path,
        install_dir,
    )
    .context("failed to create start menu shortcut")?;

    emit(InstallEvent::Progress {
        stage: InstallStage::Shortcuts,
        local_percent: 100.0,
        message: "바로가기 생성 완료".to_string(),
    });

    Ok(())
}

fn known_folder_path(shell_folder_name: &str) -> anyhow::Result<PathBuf> {
    let script = format!(
        "$w = New-Object -ComObject WScript.Shell; [Console]::OutputEncoding = [System.Text.Encoding]::UTF8; Write-Output $w.SpecialFolders('{}')",
        escape_powershell_single_quoted(shell_folder_name)
    );
    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
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

fn create_shortcut(shortcut_path: &Path, target_path: &Path, working_dir: &Path) -> anyhow::Result<()> {
    let script = format!(
        "$w = New-Object -ComObject WScript.Shell; $s = $w.CreateShortcut('{}'); $s.TargetPath = '{}'; $s.WorkingDirectory = '{}'; $s.Save()",
        escape_powershell_single_quoted(&shortcut_path.display().to_string()),
        escape_powershell_single_quoted(&target_path.display().to_string()),
        escape_powershell_single_quoted(&working_dir.display().to_string())
    );
    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .context("failed to run shortcut creation script")?;

    if !output.status.success() {
        bail!("shortcut creation script failed");
    }

    Ok(())
}

fn escape_powershell_single_quoted(value: &str) -> String {
    value.replace('\'', "''")
}
