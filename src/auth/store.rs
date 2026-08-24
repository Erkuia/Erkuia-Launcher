use std::path::Path;

use anyhow::{bail, Context};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};

use crate::auth::{device::DeviceIdentity, dpapi};

const CURRENT_VERSION: u32 = 1;
const DEVICE_KEY_BYTES: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredAccount {
    pub id: String,
    pub refresh_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretStore {
    #[serde(default = "current_version")]
    pub version: u32,
    pub device_id: String,
    pub device_key: String,
    #[serde(default)]
    pub accounts: Vec<StoredAccount>,
}

fn current_version() -> u32 {
    CURRENT_VERSION
}

impl SecretStore {
    pub fn new() -> Self {
        Self::from_identity(&DeviceIdentity::generate())
    }

    pub fn from_identity(identity: &DeviceIdentity) -> Self {
        Self {
            version: CURRENT_VERSION,
            device_id: identity.id.clone(),
            device_key: STANDARD.encode(identity.key.to_scalar()),
            accounts: Vec::new(),
        }
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::new());
        }

        let sealed = std::fs::read(path)
            .with_context(|| format!("{} 을(를) 읽지 못했어요.", path.display()))?;
        let plain = dpapi::unprotect(&sealed)?;

        serde_json::from_slice(&plain).context("계정 저장소 형식이 올바르지 않아요.")
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let parent = path
            .parent()
            .context("계정 저장소 경로에 상위 폴더가 없어요.")?;
        std::fs::create_dir_all(parent)
            .with_context(|| format!("{} 폴더를 만들지 못했어요.", parent.display()))?;

        let plain = serde_json::to_vec(self).context("계정 저장소를 직렬화하지 못했어요.")?;
        let sealed = dpapi::protect(&plain)?;

        let temp = path.with_extension("dat.tmp");
        std::fs::write(&temp, &sealed)
            .with_context(|| format!("{} 에 쓰지 못했어요.", temp.display()))?;
        std::fs::rename(&temp, path)
            .with_context(|| format!("{} 을(를) 저장하지 못했어요.", path.display()))?;

        Ok(())
    }

    pub fn identity(&self) -> anyhow::Result<DeviceIdentity> {
        let scalar = STANDARD
            .decode(&self.device_key)
            .context("디바이스 키를 해독하지 못했어요.")?;

        if scalar.len() != DEVICE_KEY_BYTES {
            bail!("디바이스 키 길이가 올바르지 않아요: {}", scalar.len());
        }

        DeviceIdentity::restore(self.device_id.clone(), &scalar)
    }

    pub fn refresh_token(&self, id: &str) -> Option<&str> {
        self.accounts
            .iter()
            .find(|account| account.id == id)
            .map(|account| account.refresh_token.as_str())
    }

    pub fn upsert(&mut self, id: impl Into<String>, refresh_token: impl Into<String>) {
        let id = id.into();
        let refresh_token = refresh_token.into();

        match self.accounts.iter_mut().find(|account| account.id == id) {
            Some(existing) => existing.refresh_token = refresh_token,
            None => self.accounts.push(StoredAccount { id, refresh_token }),
        }
    }

    pub fn remove(&mut self, id: &str) {
        self.accounts.retain(|account| account.id != id);
    }

    pub fn ids(&self) -> Vec<&str> {
        self.accounts
            .iter()
            .map(|account| account.id.as_str())
            .collect()
    }
}

impl Default for SecretStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_store_carries_a_usable_device_identity() {
        let store = SecretStore::new();
        let identity = store.identity().unwrap();

        assert_eq!(identity.id, store.device_id);
        assert_eq!(identity.id.len(), 36);
    }

    #[test]
    fn the_device_key_survives_a_serde_round_trip() {
        let original = SecretStore::new();
        let json = serde_json::to_string(&original).unwrap();
        let restored: SecretStore = serde_json::from_str(&json).unwrap();

        assert_eq!(restored, original);
        assert_eq!(
            restored.identity().unwrap().key.proof_key(),
            original.identity().unwrap().key.proof_key()
        );
    }

    #[test]
    fn upsert_replaces_the_token_for_a_known_account() {
        let mut store = SecretStore::new();
        store.upsert("a", "old");
        store.upsert("b", "other");
        store.upsert("a", "new");

        assert_eq!(store.accounts.len(), 2);
        assert_eq!(store.refresh_token("a"), Some("new"));
        assert_eq!(store.refresh_token("b"), Some("other"));
    }

    #[test]
    fn removing_an_account_drops_its_token() {
        let mut store = SecretStore::new();
        store.upsert("a", "token");
        store.remove("a");

        assert_eq!(store.refresh_token("a"), None);
        assert!(store.accounts.is_empty());
    }

    #[test]
    fn an_unknown_account_has_no_token() {
        assert_eq!(SecretStore::new().refresh_token("missing"), None);
    }

    #[test]
    fn a_corrupt_device_key_is_reported() {
        let mut store = SecretStore::new();
        store.device_key = STANDARD.encode([0_u8; 8]);

        assert!(store.identity().is_err());
    }

    #[test]
    fn a_non_base64_device_key_is_reported() {
        let mut store = SecretStore::new();
        store.device_key = "!!!not base64!!!".to_string();

        assert!(store.identity().is_err());
    }

    #[test]
    fn the_version_defaults_when_absent() {
        let json = r#"{"device_id":"id","device_key":"AA==","accounts":[]}"#;
        let store: SecretStore = serde_json::from_str(json).unwrap();

        assert_eq!(store.version, CURRENT_VERSION);
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "erkuia-store-{tag}-{}-{}",
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
            self.0.join("accounts.dat")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    #[test]
    fn a_missing_file_yields_a_fresh_store() {
        let dir = TempDir::new("missing");
        let store = SecretStore::load(&dir.file()).unwrap();

        assert!(store.accounts.is_empty());
        assert!(store.identity().is_ok());
    }

    #[test]
    fn secrets_round_trip_through_disk() {
        let dir = TempDir::new("roundtrip");
        let mut store = SecretStore::new();
        store.upsert("069a79f4-44e9-4726-a5be-fca90e38aaf5", "REFRESH");
        store.save(&dir.file()).unwrap();

        let loaded = SecretStore::load(&dir.file()).unwrap();

        assert_eq!(loaded, store);
        assert_eq!(
            loaded.refresh_token("069a79f4-44e9-4726-a5be-fca90e38aaf5"),
            Some("REFRESH")
        );
    }

    #[test]
    fn the_file_on_disk_does_not_contain_the_token_in_clear_text() {
        let dir = TempDir::new("opaque");
        let mut store = SecretStore::new();
        store.upsert("id", "SUPER_SECRET_REFRESH_TOKEN");
        store.save(&dir.file()).unwrap();

        let raw = std::fs::read(dir.file()).unwrap();

        assert!(!raw
            .windows(26)
            .any(|window| window == b"SUPER_SECRET_REFRESH_TOKEN"));
    }

    #[test]
    fn saving_leaves_no_temporary_file() {
        let dir = TempDir::new("atomic");
        SecretStore::new().save(&dir.file()).unwrap();
        SecretStore::new().save(&dir.file()).unwrap();

        let leftovers: Vec<_> = std::fs::read_dir(&dir.0)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.ends_with(".tmp"))
            .collect();

        assert!(leftovers.is_empty(), "leftover: {leftovers:?}");
    }
}
