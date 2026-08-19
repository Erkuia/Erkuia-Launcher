use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{bail, Context};

use crate::{
    download, install_files,
    manifest::{ComponentStatus, Manifest},
    progress::{InstallEvent, InstallStage},
    shortcuts, uninstall,
};

type EventSink<'a> = &'a mut dyn FnMut(InstallEvent);

pub struct InstallOptions {
    pub install_dir: PathBuf,
    pub create_desktop_shortcut: bool,
    pub run_after_install: bool,
    pub launch_after_install: bool,
}

pub fn run_install(
    manifest: &Manifest,
    options: &InstallOptions,
    emit: EventSink<'_>,
) -> anyhow::Result<()> {
    emit(InstallEvent::Progress {
        stage: InstallStage::Prepare,
        local_percent: 0.0,
        message: "설치 준비 중...".to_string(),
    });

    validate_required_components(manifest)?;

    std::fs::create_dir_all(&options.install_dir).with_context(|| {
        format!(
            "failed to create install directory {}",
            options.install_dir.display()
        )
    })?;

    let cache_dir = installer_cache_dir()?;
    std::fs::create_dir_all(&cache_dir)
        .with_context(|| format!("failed to create installer cache {}", cache_dir.display()))?;

    emit(InstallEvent::Progress {
        stage: InstallStage::Prepare,
        local_percent: 100.0,
        message: "설치 준비 완료".to_string(),
    });

    let downloaded =
        download::download_ready_components(&manifest.install_plan.components, &cache_dir, emit)?;

    let installed_components =
        install_files::install_downloaded_components(&downloaded, &options.install_dir, emit)?;

    shortcuts::create_launcher_shortcuts(
        &options.install_dir,
        options.create_desktop_shortcut,
        emit,
    )
    .context("failed to create launcher shortcuts")?;

    uninstall::register_uninstaller(manifest, &options.install_dir, emit)
        .context("failed to register uninstaller")?;

    if options.launch_after_install {
        maybe_launch_after_install(&options.install_dir, options.run_after_install, emit)
            .context("failed to launch installed launcher")?;
    }

    emit(InstallEvent::Progress {
        stage: InstallStage::Finalize,
        local_percent: 100.0,
        message: "설치 완료".to_string(),
    });
    emit(InstallEvent::Completed {
        install_dir: options.install_dir.display().to_string(),
        installed_count: installed_components
            .iter()
            .filter(|component| component.target_path.exists())
            .count(),
    });

    Ok(())
}

fn validate_required_components(manifest: &Manifest) -> anyhow::Result<()> {
    let pending_required: Vec<&str> = manifest
        .install_plan
        .components
        .iter()
        .filter(|component| component.required && component.status == ComponentStatus::Pending)
        .map(|component| component.id.as_str())
        .collect();

    if pending_required.is_empty() || manifest.installer.allow_pending_required_components {
        return Ok(());
    }

    bail!(
        "required install components are pending: {}",
        pending_required.join(", ")
    )
}

fn installer_cache_dir() -> anyhow::Result<PathBuf> {
    Ok(std::env::temp_dir().join("rendog-launcher-installer"))
}

pub fn resolve_install_path(path: &str) -> anyhow::Result<PathBuf> {
    if let Some(rest) = path.strip_prefix("%ProgramFiles%") {
        let program_files = std::env::var("ProgramFiles")
            .context("ProgramFiles environment variable is missing")?;
        return Ok(Path::new(&program_files).join(trim_path_separator(rest)));
    }

    if path.contains('%') {
        bail!("unsupported environment variable in install path: {}", path);
    }

    Ok(PathBuf::from(path))
}

fn trim_path_separator(path: &str) -> &str {
    path.trim_start_matches(['\\', '/'])
}

fn maybe_launch_after_install(
    install_dir: &Path,
    run_after_install: bool,
    emit: EventSink<'_>,
) -> anyhow::Result<()> {
    if !run_after_install {
        return Ok(());
    }

    let launcher_path = install_dir.join("RendogLauncher.exe");
    if !launcher_path.exists() {
        emit(InstallEvent::Progress {
            stage: InstallStage::Finalize,
            local_percent: 50.0,
            message: "런처 파일이 아직 준비되지 않아 자동 실행을 건너뛰었어요.".to_string(),
        });
        return Ok(());
    }

    Command::new(&launcher_path)
        .current_dir(install_dir)
        .spawn()
        .with_context(|| format!("failed to launch {}", launcher_path.display()))?;

    Ok(())
}

pub fn launch_installed_launcher(install_dir: &Path) -> anyhow::Result<bool> {
    let launcher_path = install_dir.join("RendogLauncher.exe");
    if !launcher_path.exists() {
        return Ok(false);
    }

    Command::new(&launcher_path)
        .current_dir(install_dir)
        .spawn()
        .with_context(|| format!("failed to launch {}", launcher_path.display()))?;

    Ok(true)
}
