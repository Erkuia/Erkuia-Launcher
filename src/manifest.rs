use std::path::Path;

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};

pub const DEFAULT_URL: &str =
    "https://github.com/foliq/Rendog-Launcher/releases/latest/download/launcher-manifest.json";

pub const SUPPORTED_SCHEMA: u32 = 1;
const CACHE_FILE: &str = "launcher-manifest.json";
const SHA256_HEX_LEN: usize = 64;
const SHA512_HEX_LEN: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    pub launcher: LauncherRelease,
    pub minecraft: MinecraftTarget,
    pub server: ServerTarget,
    #[serde(default)]
    pub mods: Vec<ModArtifact>,
    #[serde(default)]
    pub files: Vec<FileArtifact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LauncherRelease {
    pub version: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MinecraftTarget {
    pub version: String,
    #[serde(rename = "fabricLoader")]
    pub fabric_loader: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerTarget {
    pub address: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModArtifact {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub required: bool,
    pub url: String,
    #[serde(rename = "fileName")]
    pub file_name: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sha256: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sha512: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileArtifact {
    #[serde(rename = "targetPath")]
    pub target_path: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

fn is_hex(value: &str, length: usize) -> bool {
    value.len() == length && value.chars().all(|c| c.is_ascii_hexdigit())
}

fn is_sha256(value: &str) -> bool {
    is_hex(value, SHA256_HEX_LEN)
}

fn is_sha512(value: &str) -> bool {
    is_hex(value, SHA512_HEX_LEN)
}

impl ModArtifact {
    pub fn checksum(&self) -> Option<crate::hash::Checksum> {
        if is_sha256(&self.sha256) {
            Some(crate::hash::Checksum::Sha256(self.sha256.clone()))
        } else if is_sha512(&self.sha512) {
            Some(crate::hash::Checksum::Sha512(self.sha512.clone()))
        } else {
            None
        }
    }
}

impl Manifest {
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let manifest: Self =
            serde_json::from_str(text).context("런처 매니페스트 형식이 올바르지 않아요.")?;

        manifest.validate()?;

        Ok(manifest)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.schema_version != SUPPORTED_SCHEMA {
            bail!(
                "지원하지 않는 매니페스트 버전이에요: {} (지원 {})",
                self.schema_version,
                SUPPORTED_SCHEMA
            );
        }

        if !is_sha256(&self.launcher.sha256) {
            bail!("런처 SHA-256 값이 올바르지 않아요.");
        }

        for artifact in &self.mods {
            if artifact.checksum().is_none() {
                bail!("{} 에 쓸 수 있는 체크섬이 없어요.", artifact.id);
            }
            if artifact.size == 0 {
                bail!("{} 의 크기가 0이에요.", artifact.id);
            }
        }

        for artifact in &self.files {
            if !is_sha256(&artifact.sha256) {
                bail!("{} 의 SHA-256 값이 올바르지 않아요.", artifact.target_path);
            }
        }

        let mut ids: Vec<&str> = self.mods.iter().map(|item| item.id.as_str()).collect();
        ids.sort_unstable();
        let total = ids.len();
        ids.dedup();

        if ids.len() != total {
            bail!("매니페스트에 중복된 모드 id가 있어요.");
        }

        Ok(())
    }

    pub fn required_mods(&self) -> Vec<&ModArtifact> {
        self.mods.iter().filter(|item| item.required).collect()
    }

}

fn cache_file(cache_dir: &Path) -> std::path::PathBuf {
    cache_dir.join(CACHE_FILE)
}

const BUILTIN: &str = include_str!("../launcher-manifest.json");

pub fn builtin() -> Manifest {
    Manifest::parse(BUILTIN).expect("bundled manifest must be valid")
}

pub fn load_local(cache_dir: &Path) -> Manifest {
    load_cached(cache_dir).unwrap_or_else(builtin)
}

pub fn load_cached(cache_dir: &Path) -> Option<Manifest> {
    let text = std::fs::read_to_string(cache_file(cache_dir)).ok()?;

    Manifest::parse(&text).ok()
}

fn store(cache_dir: &Path, text: &str) -> anyhow::Result<()> {
    std::fs::create_dir_all(cache_dir)
        .with_context(|| format!("{} 폴더를 만들지 못했어요.", cache_dir.display()))?;

    let path = cache_file(cache_dir);
    let temp = path.with_extension("json.tmp");

    std::fs::write(&temp, text)
        .with_context(|| format!("{} 에 쓰지 못했어요.", temp.display()))?;
    std::fs::rename(&temp, &path)
        .with_context(|| format!("{} 을(를) 저장하지 못했어요.", path.display()))?;

    Ok(())
}

pub fn fetch(url: &str, cache_dir: &Path) -> anyhow::Result<Manifest> {
    let attempt = crate::http::send(crate::http::client()?.get(url))
        .and_then(|response| response.text().context("매니페스트를 읽지 못했어요."));

    match attempt {
        Ok(text) => {
            let manifest = Manifest::parse(&text)?;

            if let Err(error) = store(cache_dir, &text) {
                log::warn!("매니페스트 캐시 저장 실패: {error:#}");
            }

            log::info!(
                "매니페스트 확인: 런처 v{} · Minecraft {} · 모드 {}개",
                manifest.launcher.version,
                manifest.minecraft.version,
                manifest.mods.len()
            );

            Ok(manifest)
        }
        Err(error) => match load_cached(cache_dir) {
            Some(manifest) => {
                log::warn!("매니페스트를 받지 못해 캐시를 사용합니다: {error:#}");
                Ok(manifest)
            }
            None => Err(error).context("런처 매니페스트를 불러오지 못했어요."),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "schemaVersion": 1,
        "launcher": {
            "version": "0.1.0",
            "url": "https://example.invalid/RendogLauncher.exe",
            "size": 4194304,
            "sha256": "0000000000000000000000000000000000000000000000000000000000000000"
        },
        "minecraft": { "version": "1.20.4", "fabricLoader": "0.15.11" },
        "server": { "address": "rendog.kr" },
        "mods": [
            {
                "id": "rendog-client",
                "name": "RendogClient",
                "description": "서버 자동 접속 · 필수 모드",
                "required": true,
                "url": "https://example.invalid/RendogClient-Delta.jar",
                "fileName": "RendogClient-Delta.jar",
                "size": 8709016,
                "sha256": "72fc258a685734e9cb7914aca0cabf60696facb2253b48dd959eede94b1c111a"
            }
        ],
        "files": []
    }"#;

    #[test]
    fn parses_the_documented_schema() {
        let manifest = Manifest::parse(SAMPLE).unwrap();

        assert_eq!(manifest.launcher.version, "0.1.0");
        assert_eq!(manifest.minecraft.version, "1.20.4");
        assert_eq!(manifest.minecraft.fabric_loader, "0.15.11");
        assert_eq!(manifest.server.address, "rendog.kr");
        assert_eq!(manifest.mods.len(), 1);
    }

    fn client_mod(manifest: &Manifest) -> &ModArtifact {
        manifest
            .required_mods()
            .into_iter()
            .find(|artifact| artifact.id == "rendog-client")
            .expect("the sample pins the client mod")
    }

    #[test]
    fn required_mods_are_selectable() {
        let manifest = Manifest::parse(SAMPLE).unwrap();

        assert_eq!(manifest.required_mods().len(), 1);
        assert_eq!(client_mod(&manifest).file_name, "RendogClient-Delta.jar");
    }

    #[test]
    fn the_client_mod_hash_matches_the_installer_manifest() {
        let manifest = Manifest::parse(SAMPLE).unwrap();

        assert_eq!(
            client_mod(&manifest).sha256,
            "72fc258a685734e9cb7914aca0cabf60696facb2253b48dd959eede94b1c111a"
        );
    }

    #[test]
    fn a_future_schema_version_is_refused() {
        let text = SAMPLE.replace("\"schemaVersion\": 1", "\"schemaVersion\": 2");

        assert!(Manifest::parse(&text).is_err());
    }

    #[test]
    fn a_malformed_hash_is_refused() {
        let text = SAMPLE.replace(
            "72fc258a685734e9cb7914aca0cabf60696facb2253b48dd959eede94b1c111a",
            "not-a-hash",
        );

        assert!(Manifest::parse(&text).is_err());
    }

    #[test]
    fn a_mod_may_pin_a_sha512_instead() {
        let text = SAMPLE.replace(
            r#""sha256": "72fc258a685734e9cb7914aca0cabf60696facb2253b48dd959eede94b1c111a""#,
            &format!(r#""sha512": "{}""#, "b".repeat(128)),
        );

        let manifest = Manifest::parse(&text).unwrap();

        assert_eq!(
            manifest.mods[0].checksum(),
            Some(crate::hash::Checksum::Sha512("b".repeat(128)))
        );
    }

    #[test]
    fn a_sha256_wins_over_a_sha512_when_both_are_present() {
        let text = SAMPLE.replace(
            r#""sha256": "72fc258a685734e9cb7914aca0cabf60696facb2253b48dd959eede94b1c111a""#,
            &format!(
                r#""sha256": "72fc258a685734e9cb7914aca0cabf60696facb2253b48dd959eede94b1c111a", "sha512": "{}""#,
                "c".repeat(128)
            ),
        );

        let manifest = Manifest::parse(&text).unwrap();

        assert!(matches!(
            manifest.mods[0].checksum(),
            Some(crate::hash::Checksum::Sha256(_))
        ));
    }

    #[test]
    fn a_zero_sized_mod_is_refused() {
        let text = SAMPLE.replace("\"size\": 8709016", "\"size\": 0");

        assert!(Manifest::parse(&text).is_err());
    }

    #[test]
    fn duplicate_mod_ids_are_refused() {
        let manifest = Manifest::parse(SAMPLE).unwrap();
        let mut duplicated = manifest.clone();
        duplicated.mods.push(manifest.mods[0].clone());

        assert!(duplicated.validate().is_err());
    }

    #[test]
    fn optional_sections_default_to_empty() {
        let text = r#"{
            "schemaVersion": 1,
            "launcher": {
                "version": "0.1.0",
                "url": "https://example.invalid/x.exe",
                "size": 1,
                "sha256": "0000000000000000000000000000000000000000000000000000000000000000"
            },
            "minecraft": { "version": "1.20.4", "fabricLoader": "0.15.11" },
            "server": { "address": "rendog.kr" }
        }"#;

        let manifest = Manifest::parse(text).unwrap();

        assert!(manifest.mods.is_empty());
        assert!(manifest.files.is_empty());
    }

    #[test]
    fn hex_validation_rejects_wrong_lengths_and_characters() {
        assert!(is_sha256(&"a".repeat(64)));
        assert!(!is_sha256(&"a".repeat(63)));
        assert!(!is_sha256(&"a".repeat(65)));
        assert!(!is_sha256(&"z".repeat(64)));
    }

    #[test]
    fn the_cache_survives_a_round_trip() {
        let dir = std::env::temp_dir().join(format!(
            "rendog-manifest-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        store(&dir, SAMPLE).unwrap();

        assert_eq!(load_cached(&dir), Some(Manifest::parse(SAMPLE).unwrap()));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_bundled_manifest_is_valid() {
        let manifest = builtin();

        assert_eq!(manifest.minecraft.version, "1.20.4");
        assert_eq!(manifest.server.address, "rendog.kr");
        assert_eq!(manifest.required_mods().len(), 2);
    }

    /// The bundled copy can never carry its own size or hash — it lives inside
    /// the very file it would describe. The version still has to be right: a
    /// bundled version ahead of the binary makes an offline launcher announce an
    /// update to itself.
    #[test]
    fn the_bundled_manifest_version_matches_the_binary() {
        assert_eq!(
            builtin().launcher.version,
            env!("CARGO_PKG_VERSION"),
            "launcher-manifest.json 의 launcher.version 을 Cargo.toml 과 맞춰 주세요."
        );
    }

    #[test]
    fn the_bundled_manifest_pins_fabric_api_and_the_client() {
        let manifest = builtin();
        let ids: Vec<&str> = manifest
            .required_mods()
            .iter()
            .map(|artifact| artifact.id.as_str())
            .collect();

        assert!(ids.contains(&"fabric-api"));
        assert!(ids.contains(&"rendog-client"));
    }

    #[test]
    fn the_bundled_loader_clears_the_client_requirement() {
        let pinned = builtin().minecraft.fabric_loader;
        let parts: Vec<u32> = pinned
            .split('.')
            .map(|part| part.parse().expect("loader version is numeric"))
            .collect();

        assert!(
            (parts[0], parts[1], parts[2]) >= (0, 16, 9),
            "RendogClient 는 Fabric Loader 0.16.9 이상을 요구해요 (현재 {pinned})"
        );
    }

    #[test]
    fn the_bundled_manifest_is_used_when_no_cache_exists() {
        let dir = std::env::temp_dir().join(format!("rendog-manifest-nocache-{}", std::process::id()));

        assert_eq!(load_local(&dir), builtin());
    }

    #[test]
    fn a_corrupt_cache_is_ignored() {
        let dir = std::env::temp_dir().join(format!("rendog-manifest-bad-{}", std::process::id()));
        store(&dir, "{ not json").unwrap();

        assert_eq!(load_cached(&dir), None);

        std::fs::remove_dir_all(&dir).ok();
    }
}
