use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::config::{Config, MAX_FPS, MIN_FPS};

pub const SCHEMA_VERSION: u32 = 1;
pub const DIR_NAME: &str = "config";
pub const FILE_NAME: &str = "rendoglauncher.json";

/// The contract between the launcher and the mod it carries. The launcher owns
/// every value here, so the file is rewritten on each launch rather than merged
/// — whatever the settings screen last showed is the truth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModConfig {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    #[serde(rename = "serverAddress")]
    pub server_address: String,
    #[serde(rename = "targetFps")]
    pub target_fps: i32,
    #[serde(rename = "adaptiveRendering")]
    pub adaptive_rendering: bool,
    #[serde(rename = "launcherVersion")]
    pub launcher_version: String,
}

impl ModConfig {
    pub fn from_settings(settings: &Config, server_address: &str, launcher_version: &str) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            server_address: server_address.trim().to_string(),
            target_fps: settings.target_fps.clamp(MIN_FPS, MAX_FPS),
            adaptive_rendering: settings.adaptive_rendering,
            launcher_version: launcher_version.to_string(),
        }
    }

    pub fn to_json(&self) -> anyhow::Result<String> {
        serde_json::to_string_pretty(self).context("모드 설정을 만들지 못했어요.")
    }
}

pub fn path_in(minecraft_dir: &Path) -> PathBuf {
    minecraft_dir.join(DIR_NAME).join(FILE_NAME)
}

pub fn write(minecraft_dir: &Path, config: &ModConfig) -> anyhow::Result<()> {
    let path = path_in(minecraft_dir);
    let dir = path.parent().context("설정 폴더 경로가 올바르지 않아요.")?;

    std::fs::create_dir_all(dir)
        .with_context(|| format!("{} 폴더를 만들지 못했어요.", dir.display()))?;

    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, config.to_json()?)
        .with_context(|| format!("{} 에 쓰지 못했어요.", temp.display()))?;
    std::fs::rename(&temp, &path)
        .with_context(|| format!("{} 을(를) 저장하지 못했어요.", path.display()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(target_fps: i32, adaptive_rendering: bool) -> Config {
        Config {
            target_fps,
            adaptive_rendering,
            ..Config::default()
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rendog-modconfig-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn the_file_lands_where_fabric_mods_look() {
        assert_eq!(
            path_in(Path::new("/mc")),
            Path::new("/mc").join("config").join("rendoglauncher.json")
        );
    }

    #[test]
    fn the_settings_are_carried_over() {
        let config = ModConfig::from_settings(&settings(120, false), "rendog.kr", "0.1.0");

        assert_eq!(config.schema_version, SCHEMA_VERSION);
        assert_eq!(config.server_address, "rendog.kr");
        assert_eq!(config.target_fps, 120);
        assert!(!config.adaptive_rendering);
        assert_eq!(config.launcher_version, "0.1.0");
    }

    #[test]
    fn an_out_of_range_fps_is_clamped_before_the_mod_sees_it() {
        assert_eq!(
            ModConfig::from_settings(&settings(5000, true), "rendog.kr", "0.1.0").target_fps,
            MAX_FPS
        );
        assert_eq!(
            ModConfig::from_settings(&settings(1, true), "rendog.kr", "0.1.0").target_fps,
            MIN_FPS
        );
    }

    #[test]
    fn the_address_is_trimmed() {
        let config = ModConfig::from_settings(&settings(60, true), "  rendog.kr\n", "0.1.0");

        assert_eq!(config.server_address, "rendog.kr");
    }

    #[test]
    fn the_json_uses_the_documented_key_names() {
        let json = ModConfig::from_settings(&settings(90, true), "rendog.kr", "0.1.0")
            .to_json()
            .unwrap();

        for key in [
            "schemaVersion",
            "serverAddress",
            "targetFps",
            "adaptiveRendering",
            "launcherVersion",
        ] {
            assert!(json.contains(key), "{key} is missing from {json}");
        }
    }

    #[test]
    fn it_survives_a_round_trip() {
        let config = ModConfig::from_settings(&settings(90, true), "rendog.kr", "0.1.0");
        let parsed: ModConfig = serde_json::from_str(&config.to_json().unwrap()).unwrap();

        assert_eq!(parsed, config);
    }

    #[test]
    fn writing_creates_the_config_folder() {
        let root = temp_dir("write");
        let config = ModConfig::from_settings(&settings(60, true), "rendog.kr", "0.1.0");

        write(&root, &config).unwrap();

        let text = std::fs::read_to_string(path_in(&root)).unwrap();
        assert_eq!(serde_json::from_str::<ModConfig>(&text).unwrap(), config);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_second_write_replaces_the_first() {
        let root = temp_dir("replace");

        write(&root, &ModConfig::from_settings(&settings(60, true), "rendog.kr", "0.1.0")).unwrap();
        let second = ModConfig::from_settings(&settings(120, false), "rendog.kr", "0.1.0");
        write(&root, &second).unwrap();

        let text = std::fs::read_to_string(path_in(&root)).unwrap();
        assert_eq!(serde_json::from_str::<ModConfig>(&text).unwrap(), second);

        std::fs::remove_dir_all(&root).ok();
    }
}
