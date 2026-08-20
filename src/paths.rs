//! Filesystem layout for the launcher.
//!
//! Two roots, matching what the installer writes (see `installer/README.md`):
//!
//! - **install dir** — `%ProgramFiles%\Rendog Launcher`, holds the executables.
//!   Only writable while elevated, so the launcher treats it as read-only.
//! - **data dir** — `%APPDATA%\RendogLauncher`, holds everything the launcher
//!   mutates at runtime. Always writable without administrator rights.
//!
//! This module only *resolves* paths. Creating the directories is L3-1.

// The full layout is declared up front so every phase agrees on it. Most
// accessors gain their first caller in Phase 3 and later.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use anyhow::Context;

/// Folder name under `%APPDATA%`. Must stay in sync with `dataDir` in
/// `installer/manifest.json`.
pub const DATA_DIR_NAME: &str = "RendogLauncher";

/// Resolved locations the launcher reads and writes.
#[derive(Debug, Clone)]
pub struct Paths {
    data_dir: PathBuf,
}

impl Paths {
    /// Resolve the data directory from `%APPDATA%`.
    pub fn resolve() -> anyhow::Result<Self> {
        let app_data =
            std::env::var("APPDATA").context("APPDATA environment variable is missing")?;

        Ok(Self::with_data_dir(Path::new(&app_data).join(DATA_DIR_NAME)))
    }

    /// Build the layout around an explicit data directory. Used by tests.
    pub fn with_data_dir(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    /// `%APPDATA%\RendogLauncher`
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Minecraft game directory. The installer places the required mod here, so
    /// the trailing `minecraft\mods` segment must match its `targetPath`.
    pub fn minecraft_dir(&self) -> PathBuf {
        self.data_dir.join("minecraft")
    }

    /// Enabled mods. Fabric loads exactly what is in this folder.
    pub fn mods_dir(&self) -> PathBuf {
        self.minecraft_dir().join("mods")
    }

    /// Mods toggled off in the settings modal. Kept next to `mods` so turning a
    /// mod back on is a rename rather than a re-download.
    pub fn disabled_mods_dir(&self) -> PathBuf {
        self.minecraft_dir().join("mods-disabled")
    }

    /// Bundled Java runtime, downloaded when the system has no Java 21.
    pub fn runtime_dir(&self) -> PathBuf {
        self.data_dir.join("runtime")
    }

    /// Scratch space for in-flight downloads. Safe to delete at any time.
    pub fn cache_dir(&self) -> PathBuf {
        self.data_dir.join("cache")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }

    /// Launcher settings: target FPS, adaptive rendering, mod states, accounts.
    pub fn config_file(&self) -> PathBuf {
        self.data_dir.join("config.json")
    }

    /// Every directory that must exist before the launcher can run. L3-1
    /// creates them; listing them here keeps the layout in one place.
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

/// Directory the running executable sits in.
///
/// Shown read-only as "프로그램 디렉토리" in the settings modal, and used by its
/// `열기` button. Never written to.
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
        // installer/manifest.json installs to `minecraft\mods\RendogClient-Delta.jar`
        // relative to the data dir. Drifting from that would silently orphan the
        // mod the installer already placed.
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
    fn resolve_uses_appdata() {
        std::env::set_var("APPDATA", r"C:\Users\test\AppData\Roaming");
        let paths = Paths::resolve().expect("APPDATA is set");

        assert_eq!(
            paths.data_dir(),
            Path::new(r"C:\Users\test\AppData\Roaming").join(DATA_DIR_NAME)
        );
    }
}
