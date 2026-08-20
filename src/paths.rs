#![allow(dead_code)]

use std::path::{Path, PathBuf};

use anyhow::Context;

pub const DATA_DIR_NAME: &str = "RendogLauncher";

#[derive(Debug, Clone)]
pub struct Paths {
    data_dir: PathBuf,
}

impl Paths {
    pub fn resolve() -> anyhow::Result<Self> {
        let app_data =
            std::env::var("APPDATA").context("APPDATA environment variable is missing")?;

        Ok(Self::with_data_dir(Path::new(&app_data).join(DATA_DIR_NAME)))
    }

    pub fn with_data_dir(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn minecraft_dir(&self) -> PathBuf {
        self.data_dir.join("minecraft")
    }

    pub fn mods_dir(&self) -> PathBuf {
        self.minecraft_dir().join("mods")
    }

    pub fn disabled_mods_dir(&self) -> PathBuf {
        self.minecraft_dir().join("mods-disabled")
    }

    pub fn runtime_dir(&self) -> PathBuf {
        self.data_dir.join("runtime")
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.data_dir.join("cache")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }

    pub fn config_file(&self) -> PathBuf {
        self.data_dir.join("config.json")
    }

    pub fn bootstrap(&self) -> anyhow::Result<()> {
        for dir in self.required_dirs() {
            std::fs::create_dir_all(&dir)
                .with_context(|| format!("{} 폴더를 만들지 못했어요.", dir.display()))?;
        }

        Ok(())
    }

    pub fn required_dirs(&self) -> Vec<PathBuf> {
        vec![
            self.data_dir.clone(),
            self.minecraft_dir(),
            self.mods_dir(),
            self.disabled_mods_dir(),
            self.runtime_dir(),
            self.cache_dir(),
            self.logs_dir(),
        ]
    }
}

pub fn install_dir() -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe().context("failed to resolve current executable")?;

    exe.parent()
        .map(Path::to_path_buf)
        .context("current executable has no parent directory")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> Paths {
        Paths::with_data_dir(PathBuf::from(r"C:\Users\test\AppData\Roaming\RendogLauncher"))
    }

    #[test]
    fn mods_path_matches_the_installer_target() {
        let mods = paths().mods_dir();

        assert!(
            mods.ends_with(Path::new("minecraft").join("mods")),
            "{} does not end with minecraft/mods",
            mods.display()
        );
    }

    #[test]
    fn every_directory_sits_under_the_data_dir() {
        let paths = paths();

        for dir in paths.required_dirs() {
            assert!(
                dir.starts_with(paths.data_dir()),
                "{} escapes the data directory",
                dir.display()
            );
        }
    }

    #[test]
    fn bootstrap_creates_every_directory_and_is_repeatable() {
        let root = std::env::temp_dir().join(format!(
            "rendog-launcher-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let paths = Paths::with_data_dir(root.clone());

        paths.bootstrap().expect("first bootstrap");
        paths.bootstrap().expect("bootstrap is idempotent");

        for dir in paths.required_dirs() {
            assert!(dir.is_dir(), "{} was not created", dir.display());
        }

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn resolve_uses_appdata() {
        std::env::set_var("APPDATA", r"C:\Users\test\AppData\Roaming");
        let paths = Paths::resolve().expect("APPDATA is set");

        assert_eq!(
            paths.data_dir(),
            Path::new(r"C:\Users\test\AppData\Roaming").join(DATA_DIR_NAME)
        );
    }
}
