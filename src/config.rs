use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};

pub const MIN_FPS: i32 = 30;
pub const MAX_FPS: i32 = 150;
pub const DEFAULT_FPS: i32 = 60;

/// Non-secret half of a signed-in account.
///
/// Refresh tokens never live here — `config.json` is plain text. They go into
/// the DPAPI-encrypted store built in L4-7, keyed by `id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountRecord {
    pub id: String,
    pub name: String,
}

impl AccountRecord {
    pub fn initial(&self) -> String {
        self.name
            .chars()
            .next()
            .map(|first| first.to_uppercase().to_string())
            .unwrap_or_else(|| "?".to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_fps")]
    pub target_fps: i32,
    #[serde(default = "default_adaptive_rendering")]
    pub adaptive_rendering: bool,
    #[serde(default)]
    pub managed_mods: Vec<String>,
    #[serde(default)]
    pub accounts: Vec<AccountRecord>,
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
            managed_mods: Vec::new(),
            accounts: Vec::new(),
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
        self.accounts.dedup_by(|a, b| a.id == b.id);

        if !self
            .selected_account
            .as_ref()
            .is_some_and(|id| self.accounts.iter().any(|account| &account.id == id))
        {
            self.selected_account = self.accounts.first().map(|account| account.id.clone());
        }

        self
    }

    pub fn selected(&self) -> Option<&AccountRecord> {
        let id = self.selected_account.as_ref()?;

        self.accounts.iter().find(|account| &account.id == id)
    }

    pub fn others(&self) -> Vec<&AccountRecord> {
        self.accounts
            .iter()
            .filter(|account| Some(&account.id) != self.selected_account.as_ref())
            .collect()
    }

    pub fn upsert_account(&mut self, account: AccountRecord) {
        match self
            .accounts
            .iter_mut()
            .find(|existing| existing.id == account.id)
        {
            Some(existing) => *existing = account.clone(),
            None => self.accounts.push(account.clone()),
        }

        self.selected_account = Some(account.id);
    }

    pub fn remove_account(&mut self, id: &str) {
        self.accounts.retain(|account| account.id != id);

        if self.selected_account.as_deref() == Some(id) {
            self.selected_account = self.accounts.first().map(|account| account.id.clone());
        }
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

    fn account(id: &str, name: &str) -> AccountRecord {
        AccountRecord {
            id: id.to_string(),
            name: name.to_string(),
        }
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = TempDir::new("roundtrip");
        let config = Config {
            target_fps: 90,
            adaptive_rendering: false,
            accounts: vec![account("a", "Rendog_Player"), account("b", "Rendog_Player2")],
            selected_account: Some("b".to_string()),
        };

        config.save(&dir.file()).unwrap();

        assert_eq!(Config::load(&dir.file()).unwrap(), config);
    }

    #[test]
    fn selection_falls_back_when_it_points_at_a_missing_account() {
        let config = Config {
            accounts: vec![account("a", "Alice")],
            selected_account: Some("gone".to_string()),
            ..Config::default()
        }
        .normalized();

        assert_eq!(config.selected_account.as_deref(), Some("a"));
    }

    #[test]
    fn selection_is_cleared_when_no_accounts_remain() {
        let config = Config {
            selected_account: Some("a".to_string()),
            ..Config::default()
        }
        .normalized();

        assert_eq!(config.selected_account, None);
    }

    #[test]
    fn upsert_replaces_by_id_and_selects() {
        let mut config = Config::default();
        config.upsert_account(account("a", "Alice"));
        config.upsert_account(account("b", "Bob"));
        config.upsert_account(account("a", "Alice_Renamed"));

        assert_eq!(config.accounts.len(), 2);
        assert_eq!(config.selected_account.as_deref(), Some("a"));
        assert_eq!(config.selected().map(|a| a.name.as_str()), Some("Alice_Renamed"));
        assert_eq!(
            config.others().iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            vec!["b"]
        );
    }

    #[test]
    fn removing_the_selected_account_moves_selection() {
        let mut config = Config::default();
        config.upsert_account(account("a", "Alice"));
        config.upsert_account(account("b", "Bob"));
        config.remove_account("b");

        assert_eq!(config.selected_account.as_deref(), Some("a"));

        config.remove_account("a");
        assert_eq!(config.selected_account, None);
        assert!(config.accounts.is_empty());
    }

    #[test]
    fn initial_comes_from_the_first_character() {
        assert_eq!(account("a", "rendog").initial(), "R");
        assert_eq!(account("a", "").initial(), "?");
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
