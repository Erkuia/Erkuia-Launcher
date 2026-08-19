use std::path::{Path, PathBuf};

use anyhow::{bail, Context};

use crate::{
    download,
    install_files::{self, InstalledComponent},
    manifest::Manifest,
    progress::{InstallEvent, InstallStage},
    shortcuts,
    uninstall,
};

type EventSink<'a> = &'a mut dyn FnMut(InstallEvent);

pub struct InstallOptions {
    pub install_dir: PathBuf,
    pub create_desktop_shortcut: bool,
}

pub struct InstallResult {
    pub install_dir: PathBuf,
    pub installed_components: Vec<InstalledComponent>,
}

pub fn default_install_options(manifest: &Manifest) -> anyhow::Result<InstallOptions> {
    Ok(InstallOptions {
        install_dir: expand_windows_path(&manifest.install_plan.default_install_dir)?,
        create_desktop_shortcut: manifest.installer.default_create_desktop_shortcut,
    })
}

pub fn run_install(
    manifest: &Manifest,
    options: &InstallOptions,
    emit: EventSink<'_>,
) -> anyhow::Result<InstallResult> {
    emit(InstallEvent::Progress {
        stage: InstallStage::Prepare,
        local_percent: 0.0,
        message: "설치 준비 중...".to_string(),
    });

    std::fs::create_dir_all(&options.install_dir).with_context(|| {
        format!(
            "failed to create install directory {}",
            options.install_dir.display()
        )
    })?;

    let cache_dir = installer_cache_dir()?;
    std::fs::create_dir_all(&cache_dir).with_context(|| {
        format!("failed to create installer cache {}", cache_dir.display())
    })?;

    emit(InstallEvent::Progress {
        stage: InstallStage::Prepare,
        local_percent: 100.0,
        message: "설치 준비 완료".to_string(),
    });

    let downloaded = download::download_ready_components(
        &manifest.install_plan.components,
        &cache_dir,
        emit,
    )?;

    let installed_components =
        install_files::install_downloaded_components(&downloaded, &options.install_dir, emit)?;

    shortcuts::create_launcher_shortcuts(&options.install_dir, options.create_desktop_shortcut, emit)
        .context("failed to create launcher shortcuts")?;

    uninstall::register_uninstaller(manifest, &options.install_dir, emit)
        .context("failed to register uninstaller")?;

    emit(InstallEvent::Progress {
        stage: InstallStage::Finalize,
        local_percent: 100.0,
        message: "설치 완료".to_string(),
    });
    emit(InstallEvent::Completed);

    Ok(InstallResult {
        install_dir: options.install_dir.clone(),
        installed_components,
    })
}

fn installer_cache_dir() -> anyhow::Result<PathBuf> {
    Ok(std::env::temp_dir().join("rendog-launcher-installer"))
}

fn expand_windows_path(path: &str) -> anyhow::Result<PathBuf> {
    if let Some(rest) = path.strip_prefix("%ProgramFiles%") {
        let program_files =
            std::env::var("ProgramFiles").context("ProgramFiles environment variable is missing")?;
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
