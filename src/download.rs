use std::{
    fs::File,
    io::{Read, Write},
    path::{Component as PathComponent, Path, PathBuf},
    time::Duration,
};

use anyhow::{bail, Context};
use sha2::{Digest, Sha256};

use crate::{
    manifest::{Component, ComponentStatus},
    progress::{InstallEvent, InstallStage},
};

type EventSink<'a> = &'a mut dyn FnMut(InstallEvent);

pub fn download_ready_components(
    components: &[Component],
    cache_dir: &Path,
    emit: EventSink<'_>,
) -> anyhow::Result<Vec<DownloadedComponent>> {
    let ready_components: Vec<&Component> = components
        .iter()
        .filter(|component| component.status == ComponentStatus::Ready)
        .collect();

    if ready_components.is_empty() {
        bail!("no ready install components found");
    }

    std::fs::create_dir_all(cache_dir).context("failed to create installer cache directory")?;

    let total_size: u64 = ready_components
        .iter()
        .filter_map(|component| component.size)
        .sum();
    let mut downloaded_before = 0_u64;
    let mut downloaded = Vec::new();

    for component in ready_components {
        emit(InstallEvent::Progress {
            stage: InstallStage::Download,
            local_percent: percent(downloaded_before, total_size),
            message: format!("{} 다운로드 중...", component.name),
        });

        let file_path = cache_file_path(cache_dir, &component.file_name)?;
        let bytes = download_component(component, &file_path, downloaded_before, total_size, emit)
            .with_context(|| format!("failed to download {}", component.id))?;

        downloaded_before = downloaded_before.saturating_add(bytes);

        emit(InstallEvent::Progress {
            stage: InstallStage::Verify,
            local_percent: 0.0,
            message: format!("{} 무결성 검사 중...", component.name),
        });

        verify_component(component, &file_path)
            .with_context(|| format!("failed to verify {}", component.id))?;

        emit(InstallEvent::Progress {
            stage: InstallStage::Verify,
            local_percent: 100.0,
            message: format!("{} 무결성 검사 완료", component.name),
        });

        downloaded.push(DownloadedComponent {
            file_path,
            target_path: PathBuf::from(&component.target_path),
        });
    }

    Ok(downloaded)
}

fn download_component(
    component: &Component,
    file_path: &Path,
    downloaded_before: u64,
    total_size: u64,
    emit: EventSink<'_>,
) -> anyhow::Result<u64> {
    let url = component
        .url
        .as_ref()
        .context("component is ready but has no download url")?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .context("failed to prepare download client")?;
    let mut response = client.get(url).send().with_context(|| {
        format!(
            "{} 다운로드 요청에 실패했어요. 네트워크 연결을 확인한 뒤 다시 시도해 주세요.",
            component.name
        )
    })?;

    if !response.status().is_success() {
        bail!(
            "{} 다운로드 실패: 서버가 HTTP {}를 반환했어요.",
            component.name,
            response.status()
        );
    }

    let mut file = File::create(file_path).context("failed to create download file")?;
    let mut buffer = [0_u8; 64 * 1024];
    let mut component_downloaded = 0_u64;

    loop {
        let read = response
            .read(&mut buffer)
            .context("failed to read download response")?;
        if read == 0 {
            break;
        }

        file.write_all(&buffer[..read])
            .context("failed to write download file")?;
        component_downloaded += read as u64;

        emit(InstallEvent::Progress {
            stage: InstallStage::Download,
            local_percent: percent(downloaded_before + component_downloaded, total_size),
            message: format!("{} 다운로드 중...", component.name),
        });
    }

    Ok(component_downloaded)
}

fn verify_component(component: &Component, file_path: &Path) -> anyhow::Result<()> {
    if let Some(expected_size) = component.size {
        let actual_size = file_path
            .metadata()
            .context("failed to read downloaded file metadata")?
            .len();
        if actual_size != expected_size {
            bail!(
                "download size mismatch: expected {}, got {}",
                expected_size,
                actual_size
            );
        }
    }

    if let Some(expected_hash) = &component.sha256 {
        let actual_hash = sha256_file(file_path)?;
        if !actual_hash.eq_ignore_ascii_case(expected_hash) {
            bail!("sha256 mismatch");
        }
    }

    Ok(())
}

fn sha256_file(file_path: &Path) -> anyhow::Result<String> {
    let mut file = File::open(file_path).context("failed to open file for hashing")?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];

    loop {
        let read = file
            .read(&mut buffer)
            .context("failed to read file for hashing")?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

fn percent(done: u64, total: u64) -> f32 {
    if total == 0 {
        return 0.0;
    }

    (done as f32 / total as f32 * 100.0).clamp(0.0, 100.0)
}

fn cache_file_path(cache_dir: &Path, file_name: &str) -> anyhow::Result<PathBuf> {
    let path = Path::new(file_name);

    if path.is_absolute() {
        bail!("component file name must be relative");
    }

    if path
        .components()
        .any(|part| !matches!(part, PathComponent::Normal(_)))
    {
        bail!("component file name must not contain path separators");
    }

    Ok(cache_dir.join(path))
}

#[derive(Debug)]
pub struct DownloadedComponent {
    pub file_path: PathBuf,
    pub target_path: PathBuf,
}
