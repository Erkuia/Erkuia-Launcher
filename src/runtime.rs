#![allow(dead_code)]

use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use serde::Deserialize;

use crate::{
    hash::Checksum,
    java::{self, JavaInstall},
    task::{Cancel, Reporter, Stage},
};

pub const API_BASE: &str = "https://api.adoptium.net/v3";
const IMAGE_TYPE: &str = "jre";
const JVM_IMPL: &str = "hotspot";
const VENDOR: &str = "eclipse";

#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub binary: Binary,
    #[serde(rename = "release_name", default)]
    pub release_name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Binary {
    pub package: Package,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Package {
    pub name: String,
    pub link: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub checksum: String,
}

pub fn architecture() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else if cfg!(target_arch = "x86_64") {
        "x64"
    } else {
        "x86"
    }
}

pub fn operating_system() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "mac"
    } else {
        "linux"
    }
}

pub fn assets_url(major: u32) -> String {
    format!(
        "{API_BASE}/assets/latest/{major}/{JVM_IMPL}?architecture={}&image_type={IMAGE_TYPE}&os={}&vendor={VENDOR}",
        architecture(),
        operating_system()
    )
}

pub fn pick_asset(assets: Vec<Asset>) -> Option<Asset> {
    assets
        .into_iter()
        .find(|asset| asset.binary.package.link.ends_with(".zip"))
}

fn fetch_asset(major: u32) -> anyhow::Result<Asset> {
    let assets: Vec<Asset> = crate::http::send(crate::http::client()?.get(assets_url(major)))
        .context("Java 런타임 목록을 받지 못했어요.")?
        .json()
        .context("Java 런타임 목록을 해석하지 못했어요.")?;

    let Some(asset) = pick_asset(assets) else {
        bail!("이 시스템에 맞는 Java {major} 런타임을 찾지 못했어요.");
    };

    Ok(asset)
}

fn download_package(
    asset: &Asset,
    cache_dir: &Path,
    reporter: &Reporter,
    cancel: &Cancel,
) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(cache_dir)
        .with_context(|| format!("{} 폴더를 만들지 못했어요.", cache_dir.display()))?;

    let destination = cache_dir.join(&asset.binary.package.name);
    let checksum = (!asset.binary.package.checksum.is_empty())
        .then(|| Checksum::Sha256(asset.binary.package.checksum.clone()));

    if destination.is_file()
        && checksum
            .as_ref()
            .is_none_or(|checksum| checksum.matches_file(&destination))
    {
        return Ok(destination);
    }

    let mut response = crate::http::send(crate::http::client()?.get(&asset.binary.package.link))
        .context("Java 런타임을 내려받지 못했어요.")?;

    let temp = destination.with_extension("part");
    let mut file = std::fs::File::create(&temp)
        .with_context(|| format!("{} 을(를) 만들지 못했어요.", temp.display()))?;

    let total = asset.binary.package.size.max(1);
    let mut written = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];

    loop {
        if cancel.is_cancelled() {
            drop(file);
            std::fs::remove_file(&temp).ok();
            bail!("작업을 취소했어요.");
        }

        let read = std::io::Read::read(&mut response, &mut buffer)
            .context("Java 런타임 응답을 읽지 못했어요.")?;
        if read == 0 {
            break;
        }

        std::io::Write::write_all(&mut file, &buffer[..read])
            .context("Java 런타임을 저장하지 못했어요.")?;
        written += read as u64;

        reporter.progress(
            Stage::Java,
            (written as f32 / total as f32).clamp(0.0, 0.7),
            "Java 런타임을 내려받는 중...",
        );
    }

    drop(file);

    if let Some(checksum) = &checksum {
        if !checksum.matches_file(&temp) {
            std::fs::remove_file(&temp).ok();
            bail!("Java 런타임 검증에 실패했어요.");
        }
    }

    std::fs::rename(&temp, &destination)
        .with_context(|| format!("{} 을(를) 배치하지 못했어요.", destination.display()))?;

    Ok(destination)
}

pub fn extract(
    archive_path: &Path,
    runtime_dir: &Path,
    reporter: &Reporter,
    cancel: &Cancel,
) -> anyhow::Result<()> {
    let file = std::fs::File::open(archive_path)
        .with_context(|| format!("{} 을(를) 열지 못했어요.", archive_path.display()))?;
    let mut archive =
        zip::ZipArchive::new(file).context("Java 런타임 압축을 열지 못했어요.")?;

    std::fs::create_dir_all(runtime_dir)
        .with_context(|| format!("{} 폴더를 만들지 못했어요.", runtime_dir.display()))?;

    let total = archive.len().max(1);

    for index in 0..archive.len() {
        if cancel.is_cancelled() {
            bail!("작업을 취소했어요.");
        }

        let mut entry = archive
            .by_index(index)
            .context("Java 런타임 항목을 읽지 못했어요.")?;

        let Some(relative) = entry.enclosed_name() else {
            bail!("압축 안에 안전하지 않은 경로가 있어요: {}", entry.name());
        };

        let destination = runtime_dir.join(relative);

        if entry.is_dir() {
            std::fs::create_dir_all(&destination)
                .with_context(|| format!("{} 폴더를 만들지 못했어요.", destination.display()))?;
        } else {
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("{} 폴더를 만들지 못했어요.", parent.display()))?;
            }

            let mut out = std::fs::File::create(&destination)
                .with_context(|| format!("{} 을(를) 만들지 못했어요.", destination.display()))?;
            std::io::copy(&mut entry, &mut out)
                .with_context(|| format!("{} 을(를) 풀지 못했어요.", destination.display()))?;
        }

        reporter.progress(
            Stage::Java,
            0.7 + 0.3 * (index as f32 + 1.0) / total as f32,
            "Java 런타임을 설치하는 중...",
        );
    }

    Ok(())
}

pub fn ensure(
    runtime_dir: &Path,
    cache_dir: &Path,
    required_major: u32,
    reporter: &Reporter,
    cancel: &Cancel,
) -> anyhow::Result<JavaInstall> {
    if let Some(install) = java::detect(runtime_dir, required_major) {
        return Ok(install);
    }

    reporter.progress(Stage::Java, 0.0, "Java 런타임을 준비하는 중...");
    log::info!("Java {required_major} 미탐지 · Adoptium 에서 내려받습니다.");

    let asset = fetch_asset(required_major)?;
    log::info!(
        "Adoptium {} · {} ({} 바이트)",
        asset.release_name,
        asset.binary.package.name,
        asset.binary.package.size
    );

    let archive = download_package(&asset, cache_dir, reporter, cancel)?;
    extract(&archive, runtime_dir, reporter, cancel)?;

    match java::detect(runtime_dir, required_major) {
        Some(install) => Ok(install),
        None => bail!("Java 런타임을 설치했지만 인식하지 못했어요."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESPONSE: &str = r#"[{
        "release_name": "jdk-21.0.3+9",
        "binary": {
            "image_type": "jre",
            "package": {
                "name": "OpenJDK21U-jre_x64_windows_hotspot_21.0.3_9.zip",
                "link": "https://example.invalid/OpenJDK21U-jre.zip",
                "size": 44000000,
                "checksum": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            }
        }
    }]"#;

    #[test]
    fn the_query_pins_platform_and_image_type() {
        let url = assets_url(21);

        assert!(url.starts_with("https://api.adoptium.net/v3/assets/latest/21/hotspot?"));
        assert!(url.contains("image_type=jre"));
        assert!(url.contains("vendor=eclipse"));
        assert!(url.contains(&format!("architecture={}", architecture())));
        assert!(url.contains(&format!("os={}", operating_system())));
    }

    #[test]
    fn parses_the_asset_response() {
        let assets: Vec<Asset> = serde_json::from_str(RESPONSE).unwrap();
        let asset = pick_asset(assets).unwrap();

        assert_eq!(asset.release_name, "jdk-21.0.3+9");
        assert_eq!(asset.binary.package.size, 44_000_000);
        assert!(asset.binary.package.link.ends_with(".zip"));
    }

    #[test]
    fn non_zip_packages_are_skipped() {
        let assets: Vec<Asset> = serde_json::from_str(
            r#"[{
                "release_name": "jdk-21",
                "binary": { "package": {
                    "name": "x.tar.gz",
                    "link": "https://example.invalid/x.tar.gz",
                    "size": 1,
                    "checksum": ""
                }}
            }]"#,
        )
        .unwrap();

        assert!(pick_asset(assets).is_none());
    }

    #[test]
    fn an_empty_asset_list_yields_nothing() {
        assert!(pick_asset(Vec::new()).is_none());
    }

    #[test]
    fn a_package_without_a_checksum_still_parses() {
        let assets: Vec<Asset> = serde_json::from_str(
            r#"[{ "binary": { "package": {
                "name": "a.zip", "link": "https://example.invalid/a.zip"
            }}}]"#,
        )
        .unwrap();
        let asset = pick_asset(assets).unwrap();

        assert!(asset.binary.package.checksum.is_empty());
        assert_eq!(asset.binary.package.size, 0);
    }

    #[test]
    fn the_architecture_matches_the_build_target() {
        let expected = if cfg!(target_arch = "aarch64") {
            "aarch64"
        } else if cfg!(target_arch = "x86_64") {
            "x64"
        } else {
            "x86"
        };

        assert_eq!(architecture(), expected);
    }
}
