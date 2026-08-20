use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{bail, Context};

use crate::{
    download,
    install_files::{self, InstallRoots},
    manifest::{ComponentStatus, Manifest},
    paths,
    progress::{InstallEvent, InstallStage},
    shortcuts, uninstall,
};

type EventSink<'a> = &'a mut dyn FnMut(InstallEvent);

pub struct InstallOptions {
    pub install_dir: PathBuf,
    /// Writable runtime directory. Resolved before UAC elevation so the
    /// elevated process writes into the *logged-on* Windows user's profile,
    /// not the profile of whichever Windows administrator answered the prompt.
    pub data_dir: PathBuf,
    pub create_desktop_shortcut: bool,
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

    std::fs::create_dir_all(&options.data_dir).with_context(|| {
        format!(
            "failed to create data directory {}",
            options.data_dir.display()
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

    let roots = InstallRoots {
        install_dir: options.install_dir.clone(),
        data_dir: options.data_dir.clone(),
    };

    let installed_components =
        install_files::install_downloaded_components(&downloaded, &roots, emit)?;

    shortcuts::create_launcher_shortcuts(
        &options.install_dir,
        options.create_desktop_shortcut,
        emit,
    )
    .context("failed to create launcher shortcuts")?;

    uninstall::register_uninstaller(manifest, &options.install_dir, &options.data_dir, emit)
        .context("failed to register uninstaller")?;

    emit(InstallEvent::Progress {
        stage: InstallStage::Finalize,
        local_percent: 100.0,
        message: "설치 완료 중...".to_string(),
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

/// Resolve the writable runtime directory for the current Windows user.
///
/// Must be called *before* UAC elevation: inside the elevated process
/// `%APPDATA%` can resolve to a different Windows user folder when a standard
/// user answers the prompt with administrator credentials.
pub fn resolve_data_dir(manifest: &Manifest) -> anyhow::Result<PathBuf> {
    paths::expand(&manifest.install_plan.data_dir)
}

/// Start the installed launcher.
///
/// When the installer itself is elevated, spawning the launcher directly would
/// hand it the administrator token as well. Going through `explorer.exe` makes
/// the shell start it at the logged-on user's normal integrity level instead.
pub fn launch_installed_launcher(install_dir: &Path, drop_elevation: bool) -> anyhow::Result<bool> {
    let launcher_path = install_dir.join("RendogLauncher.exe");
    if !launcher_path.exists() {
        return Ok(false);
    }

    if drop_elevation && launch_through_shell(&launcher_path).is_ok() {
        return Ok(true);
    }

    Command::new(&launcher_path)
        .current_dir(install_dir)
        .spawn()
        .with_context(|| format!("failed to launch {}", launcher_path.display()))?;

    Ok(true)
}

fn launch_through_shell(launcher_path: &Path) -> anyhow::Result<()> {
    Command::new("explorer.exe")
        .arg(launcher_path)
        .spawn()
        .with_context(|| format!("failed to shell-launch {}", launcher_path.display()))?;

    Ok(())
}
