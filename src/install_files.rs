use std::{
    fs::File,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

use anyhow::{bail, Context};

use crate::{
    download::DownloadedComponent,
    manifest::TargetRoot,
    progress::{InstallEvent, InstallStage},
};

type EventSink<'a> = &'a mut dyn FnMut(InstallEvent);

#[derive(Debug, Clone)]
pub struct InstallRoots {
    pub install_dir: PathBuf,
    pub data_dir: PathBuf,
}

impl InstallRoots {
    pub fn root_for(&self, target_root: TargetRoot) -> &Path {
        match target_root {
            TargetRoot::Install => &self.install_dir,
            TargetRoot::Data => &self.data_dir,
        }
    }
}

pub fn install_downloaded_components(
    downloaded: &[DownloadedComponent],
    roots: &InstallRoots,
    emit: EventSink<'_>,
) -> anyhow::Result<Vec<InstalledComponent>> {
    std::fs::create_dir_all(&roots.install_dir).context("failed to create install directory")?;
    std::fs::create_dir_all(&roots.data_dir).context("failed to create data directory")?;

    let total_size = total_input_size(downloaded)?;
    let mut installed_before = 0_u64;
    let mut installed = Vec::new();

    for component in downloaded {
        let target_path = safe_join(
            roots.root_for(component.target_root),
            &component.target_path,
        )?;

        if let Some(parent) = target_path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create target directory {}", parent.display())
            })?;
        }

        emit(InstallEvent::Progress {
            stage: InstallStage::InstallFiles,
            local_percent: percent(installed_before, total_size),
            message: "파일 배치 중...".to_string(),
        });

        let copied =
            copy_with_progress(component, &target_path, installed_before, total_size, emit)?;
        installed_before = installed_before.saturating_add(copied);

        installed.push(InstalledComponent { target_path });
    }

    emit(InstallEvent::Progress {
        stage: InstallStage::InstallFiles,
        local_percent: 100.0,
        message: "파일 배치 중...".to_string(),
    });

    Ok(installed)
}

fn copy_with_progress(
    component: &DownloadedComponent,
    target_path: &Path,
    installed_before: u64,
    total_size: u64,
    emit: EventSink<'_>,
) -> anyhow::Result<u64> {
    let mut source = File::open(&component.file_path)
        .with_context(|| format!("failed to open {}", component.file_path.display()))?;
    let mut target = File::create(target_path)
        .with_context(|| format!("failed to create {}", target_path.display()))?;
    let mut buffer = [0_u8; 64 * 1024];
    let mut copied = 0_u64;

    loop {
        let read = source
            .read(&mut buffer)
            .context("failed to read install source")?;
        if read == 0 {
            break;
        }

        target
            .write_all(&buffer[..read])
            .context("failed to write install target")?;
        copied += read as u64;

        emit(InstallEvent::Progress {
            stage: InstallStage::InstallFiles,
            local_percent: percent(installed_before + copied, total_size),
            message: "파일 배치 중...".to_string(),
        });
    }

    Ok(copied)
}

fn total_input_size(downloaded: &[DownloadedComponent]) -> anyhow::Result<u64> {
    let mut total = 0_u64;

    for component in downloaded {
        total = total.saturating_add(
            component
                .file_path
                .metadata()
                .with_context(|| format!("failed to read {}", component.file_path.display()))?
                .len(),
        );
    }

    Ok(total)
}

fn safe_join(root: &Path, relative: &Path) -> anyhow::Result<PathBuf> {
    if relative.is_absolute() {
        bail!("install target path must be relative");
    }

    for part in relative.components() {
        if matches!(
            part,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        ) {
            bail!("install target path escapes its root directory");
        }
    }

    Ok(root.join(relative))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots() -> InstallRoots {
        InstallRoots {
            install_dir: PathBuf::from(r"C:\Program Files\Rendog Launcher"),
            data_dir: PathBuf::from(r"C:\Users\test\AppData\Roaming\RendogLauncher"),
        }
    }

    #[test]
    fn routes_components_to_their_root() {
        let roots = roots();
        assert_eq!(
            roots.root_for(TargetRoot::Install),
            Path::new(r"C:\Program Files\Rendog Launcher")
        );
        assert_eq!(
            roots.root_for(TargetRoot::Data),
            Path::new(r"C:\Users\test\AppData\Roaming\RendogLauncher")
        );
    }

    #[test]
    fn rejects_paths_escaping_their_root() {
        assert!(safe_join(Path::new(r"C:\root"), Path::new(r"..\evil.exe")).is_err());
        assert!(safe_join(Path::new(r"C:\root"), Path::new(r"C:\evil.exe")).is_err());
    }
}

fn percent(done: u64, total: u64) -> f32 {
    if total == 0 {
        return 0.0;
    }

    (done as f32 / total as f32 * 100.0).clamp(0.0, 100.0)
}

#[derive(Debug)]
pub struct InstalledComponent {
    pub target_path: PathBuf,
}
