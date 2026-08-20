use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};

pub const MIN_FPS: i32 = 30;
pub const MAX_FPS: i32 = 150;
pub const DEFAULT_FPS: i32 = 105;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_fps")]
    pub target_fps: i32,
    #[serde(default = "default_adaptive_rendering")]
    pub adaptive_rendering: bool,
    #[serde(default)]
    pub selected_account: Option<String>,
}

fn default_fps() -> i32 {
    DEFAULT_FPS
}

fn default_adaptive_rendering() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            target_fps: default_fps(),
            adaptive_rendering: default_adaptive_rendering(),
            selected_account: None,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let text = std::fs::read_to_string(path)
            .with_context(|| format!("{} 을(를) 읽지 못했어요.", path.display()))?;

        let config: Self = serde_json::from_str(&text)
            .with_context(|| format!("{} 형식이 올바르지 않아요.", path.display()))?;

        Ok(config.normalized())
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let parent = path
            .parent()
            .context("설정 파일 경로에 상위 폴더가 없어요.")?;
        std::fs::create_dir_all(parent)
            .with_context(|| format!("{} 폴더를 만들지 못했어요.", parent.display()))?;

        let text = serde_json::to_string_pretty(&self.clone().normalized())
            .context("설정을 직렬화하지 못했어요.")?;

        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, text)
            .with_context(|| format!("{} 에 쓰지 못했어요.", temp.display()))?;

        std::fs::rename(&temp, path).with_context(|| {
            format!("{} 을(를) 저장하지 못했어요.", path.display())
        })?;

        Ok(())
    }

    pub fn normalized(mut self) -> Self {
        self.target_fps = self.target_fps.clamp(MIN_FPS, MAX_FPS);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "rendog-config-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn file(&self) -> std::path::PathBuf {
            self.0.join("config.json")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    #[test]
    fn missing_file_yields_defaults() {
        let dir = TempDir::new("missing");

        assert_eq!(Config::load(&dir.file()).unwrap(), Config::default());
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = TempDir::new("roundtrip");
        let config = Config {
            target_fps: 60,
            adaptive_rendering: false,
            selected_account: Some("abc".to_string()),
        };

        config.save(&dir.file()).unwrap();

        assert_eq!(Config::load(&dir.file()).unwrap(), config);
    }

    #[test]
    fn unknown_and_missing_fields_fall_back_to_defaults() {
        let dir = TempDir::new("partial");
        std::fs::write(dir.file(), r#"{"target_fps": 90, "future_option": 1}"#).unwrap();

        let config = Config::load(&dir.file()).unwrap();

        assert_eq!(config.target_fps, 90);
        assert!(config.adaptive_rendering);
        assert_eq!(config.selected_account, None);
    }

    #[test]
    fn fps_is_clamped_to_the_slider_range() {
        let dir = TempDir::new("clamp");
        std::fs::write(dir.file(), r#"{"target_fps": 9000}"#).unwrap();
        assert_eq!(Config::load(&dir.file()).unwrap().target_fps, MAX_FPS);

        std::fs::write(dir.file(), r#"{"target_fps": -5}"#).unwrap();
        assert_eq!(Config::load(&dir.file()).unwrap().target_fps, MIN_FPS);
    }

    #[test]
    fn malformed_json_is_reported_not_silently_reset() {
        let dir = TempDir::new("malformed");
        std::fs::write(dir.file(), "{ not json").unwrap();

        assert!(Config::load(&dir.file()).is_err());
    }

    #[test]
    fn save_leaves_no_temporary_file_behind() {
        let dir = TempDir::new("atomic");
        Config::default().save(&dir.file()).unwrap();
        Config::default().save(&dir.file()).unwrap();

        let leftovers: Vec<_> = std::fs::read_dir(&dir.0)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.ends_with(".tmp"))
            .collect();

        assert!(leftovers.is_empty(), "leftover temp files: {leftovers:?}");
    }
}
