#![allow(dead_code)]

use std::path::{Path, PathBuf};

use anyhow::{bail, Context};

use crate::{manifest::Manifest, mc::version::DownloadTarget};

pub const MODS_RELATIVE: &str = "mods";

pub const JAR_EXTENSION: &str = "jar";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModKind {
    Required,
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub kind: ModKind,
    pub enabled: bool,
    pub file_name: String,
}

impl ModInfo {
    pub fn is_removable(&self) -> bool {
        self.kind == ModKind::Local
    }

    pub fn can_disable(&self) -> bool {
        self.kind == ModKind::Local
    }
}

pub fn is_jar(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(JAR_EXTENSION))
}

fn jars_in(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_jar(path))
        .filter_map(|path| path.file_name()?.to_str().map(str::to_string))
        .collect();

    names.sort_unstable();
    names.dedup();

    names
}

fn required_lookup(manifest: Option<&Manifest>) -> Vec<(String, String, String)> {
    let Some(manifest) = manifest else {
        return Vec::new();
    };

    manifest
        .mods
        .iter()
        .filter(|artifact| artifact.required)
        .map(|artifact| {
            (
                artifact.file_name.clone(),
                artifact.id.clone(),
                artifact.name.clone(),
            )
        })
        .collect()
}

pub fn scan(mods_dir: &Path, disabled_dir: &Path, manifest: Option<&Manifest>) -> Vec<ModInfo> {
    let required = required_lookup(manifest);
    let describe = |file_name: &str| {
        manifest
            .and_then(|manifest| {
                manifest
                    .mods
                    .iter()
                    .find(|artifact| artifact.file_name == file_name)
            })
            .map(|artifact| (artifact.id.clone(), artifact.name.clone(), artifact.description.clone()))
    };

    let mut found: Vec<ModInfo> = Vec::new();

    for (file_name, enabled) in jars_in(mods_dir)
        .into_iter()
        .map(|name| (name, true))
        .chain(jars_in(disabled_dir).into_iter().map(|name| (name, false)))
    {
        if found.iter().any(|entry| entry.file_name == file_name) {
            continue;
        }

        let known = describe(&file_name);
        let is_required = crate::bundled::is_bundled(&file_name)
            || required
                .iter()
                .any(|(required_name, _, _)| required_name == &file_name);

        let source = if enabled { mods_dir } else { disabled_dir };
        let (id, name, description) = known.unwrap_or_else(|| {
            let metadata = read_metadata(&source.join(&file_name)).unwrap_or_default();
            let fallback = file_name.trim_end_matches(".jar").to_string();

            (
                if metadata.id.is_empty() {
                    file_name.clone()
                } else {
                    metadata.id
                },
                if metadata.name.is_empty() {
                    fallback
                } else {
                    metadata.name
                },
                metadata.description,
            )
        });

        found.push(ModInfo {
            id,
            name,
            description,
            kind: if is_required {
                ModKind::Required
            } else {
                ModKind::Local
            },
            enabled,
            file_name,
        });
    }

    for (file_name, id, name) in required {
        if found.iter().any(|entry| entry.file_name == file_name) {
            continue;
        }

        found.push(ModInfo {
            id,
            name,
            description: String::new(),
            kind: ModKind::Required,
            enabled: false,
            file_name,
        });
    }

    found.sort_by(|a, b| {
        (a.kind == ModKind::Local, &a.file_name).cmp(&(b.kind == ModKind::Local, &b.file_name))
    });

    found
}

pub fn local(entries: &[ModInfo]) -> Vec<&ModInfo> {
    entries
        .iter()
        .filter(|entry| entry.kind == ModKind::Local)
        .collect()
}

pub fn missing_required(entries: &[ModInfo]) -> Vec<&ModInfo> {
    entries
        .iter()
        .filter(|entry| entry.kind == ModKind::Required && !entry.enabled)
        .collect()
}

pub fn path_of(mods_dir: &Path, disabled_dir: &Path, entry: &ModInfo) -> PathBuf {
    let root = if entry.enabled { mods_dir } else { disabled_dir };

    root.join(&entry.file_name)
}

pub const METADATA_ENTRY: &str = "fabric.mod.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct ModMetadata {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: String,
}

pub fn parse_metadata(text: &str) -> Option<ModMetadata> {
    let metadata: ModMetadata = serde_json::from_str(text).ok()?;

    (!metadata.id.is_empty() || !metadata.name.is_empty()).then_some(metadata)
}

pub fn read_metadata(jar: &Path) -> Option<ModMetadata> {
    let file = std::fs::File::open(jar).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut entry = archive.by_name(METADATA_ENTRY).ok()?;

    let mut text = String::new();
    std::io::Read::read_to_string(&mut entry, &mut text).ok()?;

    parse_metadata(&text)
}

pub fn add_local(mods_dir: &Path, disabled_dir: &Path, source: &Path) -> anyhow::Result<String> {
    if !is_jar(source) {
        bail!("jar 파일만 추가할 수 있어요.");
    }

    if !source.is_file() {
        bail!("파일을 찾지 못했어요: {}", source.display());
    }

    let file_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .context("파일 이름을 읽지 못했어요.")?
        .to_string();

    ensure_dirs(mods_dir, disabled_dir)?;

    let destination = mods_dir.join(&file_name);

    if destination == source {
        return Ok(file_name);
    }

    std::fs::copy(source, &destination)
        .with_context(|| format!("{file_name} 을(를) 복사하지 못했어요."))?;

    let parked = disabled_dir.join(&file_name);
    if parked.is_file() {
        std::fs::remove_file(&parked).ok();
    }

    log::info!("로컬 모드 추가: {file_name}");

    Ok(file_name)
}

pub fn set_enabled(
    mods_dir: &Path,
    disabled_dir: &Path,
    entry: &ModInfo,
    enabled: bool,
) -> anyhow::Result<()> {
    if !enabled && !entry.can_disable() {
        bail!("{} 은(는) 필수 모드라 끌 수 없어요.", entry.name);
    }

    ensure_dirs(mods_dir, disabled_dir)?;

    let (from, to) = if enabled {
        (disabled_dir, mods_dir)
    } else {
        (mods_dir, disabled_dir)
    };

    let source = from.join(&entry.file_name);
    let destination = to.join(&entry.file_name);

    if !source.is_file() {
        if destination.is_file() {
            return Ok(());
        }

        bail!("{} 파일을 찾지 못했어요.", entry.file_name);
    }

    if destination.is_file() {
        std::fs::remove_file(&source)
            .with_context(|| format!("{} 중복 파일을 정리하지 못했어요.", entry.file_name))?;

        return Ok(());
    }

    std::fs::rename(&source, &destination)
        .with_context(|| format!("{} 상태를 바꾸지 못했어요.", entry.file_name))?;

    log::info!(
        "모드 {} -> {}",
        entry.file_name,
        if enabled { "켜짐" } else { "꺼짐" }
    );

    Ok(())
}

pub fn remove(mods_dir: &Path, disabled_dir: &Path, entry: &ModInfo) -> anyhow::Result<()> {
    if !entry.is_removable() {
        bail!("{} 은(는) 필수 모드라 삭제할 수 없어요.", entry.name);
    }

    let mut deleted = false;

    for dir in [mods_dir, disabled_dir] {
        let path = dir.join(&entry.file_name);

        if path.is_file() {
            std::fs::remove_file(&path)
                .with_context(|| format!("{} 을(를) 삭제하지 못했어요.", entry.file_name))?;
            deleted = true;
        }
    }

    if !deleted {
        bail!("{} 파일을 찾지 못했어요.", entry.file_name);
    }

    log::info!("모드 삭제: {}", entry.file_name);

    Ok(())
}

fn find<'a>(entries: &'a [ModInfo], id: &str) -> anyhow::Result<&'a ModInfo> {
    entries
        .iter()
        .find(|entry| entry.id == id)
        .with_context(|| format!("모드를 찾지 못했어요: {id}"))
}

pub fn set_enabled_by_id(
    mods_dir: &Path,
    disabled_dir: &Path,
    entries: &[ModInfo],
    id: &str,
    enabled: bool,
) -> anyhow::Result<()> {
    set_enabled(mods_dir, disabled_dir, find(entries, id)?, enabled)
}

pub fn remove_by_id(
    mods_dir: &Path,
    disabled_dir: &Path,
    entries: &[ModInfo],
    id: &str,
) -> anyhow::Result<()> {
    remove(mods_dir, disabled_dir, find(entries, id)?)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequiredSync {
    pub targets: Vec<DownloadTarget>,
    pub restored: Vec<String>,
    pub removed: Vec<String>,
}

pub fn required_file_names(manifest: &Manifest) -> Vec<String> {
    manifest
        .required_mods()
        .into_iter()
        .map(|artifact| artifact.file_name.clone())
        .collect()
}

pub fn required_targets(manifest: &Manifest) -> Vec<DownloadTarget> {
    manifest
        .required_mods()
        .into_iter()
        .map(|artifact| DownloadTarget {
            url: artifact.url.clone(),
            relative_path: format!("{MODS_RELATIVE}/{}", artifact.file_name),
            checksum: artifact.checksum(),
            size: artifact.size,
            name: Some(artifact.id.clone()),
        })
        .collect()
}

pub fn prepare_required(
    mods_dir: &Path,
    disabled_dir: &Path,
    manifest: &Manifest,
    managed: &[String],
) -> anyhow::Result<RequiredSync> {
    ensure_dirs(mods_dir, disabled_dir)?;

    let wanted = required_file_names(manifest);
    let mut sync = RequiredSync {
        targets: required_targets(manifest),
        ..RequiredSync::default()
    };

    for file_name in &wanted {
        let parked = disabled_dir.join(file_name);
        let active = mods_dir.join(file_name);

        if parked.is_file() && !active.exists() {
            std::fs::rename(&parked, &active).with_context(|| {
                format!("{file_name} 을(를) 다시 활성화하지 못했어요.")
            })?;
            sync.restored.push(file_name.clone());
        } else if parked.is_file() {
            std::fs::remove_file(&parked).ok();
        }
    }

    for file_name in managed {
        if wanted.iter().any(|name| name == file_name) {
            continue;
        }

        for dir in [mods_dir, disabled_dir] {
            let path = dir.join(file_name);
            if path.is_file() {
                std::fs::remove_file(&path)
                    .with_context(|| format!("{file_name} 을(를) 정리하지 못했어요."))?;
                sync.removed.push(file_name.clone());
            }
        }
    }

    Ok(sync)
}

pub fn ensure_dirs(mods_dir: &Path, disabled_dir: &Path) -> anyhow::Result<()> {
    for dir in [mods_dir, disabled_dir] {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("{} 폴더를 만들지 못했어요.", dir.display()))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "erkuia-mods-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(root.join("mods")).unwrap();
            std::fs::create_dir_all(root.join("mods-disabled")).unwrap();

            Self { root }
        }

        fn mods(&self) -> PathBuf {
            self.root.join("mods")
        }

        fn disabled(&self) -> PathBuf {
            self.root.join("mods-disabled")
        }

        fn put(&self, dir: &str, name: &str) {
            std::fs::write(self.root.join(dir).join(name), b"jar").unwrap();
        }

        fn scan(&self, manifest: Option<&Manifest>) -> Vec<ModInfo> {
            scan(&self.mods(), &self.disabled(), manifest)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).ok();
        }
    }

    fn manifest() -> Manifest {
        Manifest::parse(
            r#"{
                "schemaVersion": 1,
                "launcher": {
                    "version": "0.1.0",
                    "url": "https://example.invalid/x.exe",
                    "size": 1,
                    "sha256": "0000000000000000000000000000000000000000000000000000000000000000"
                },
                "minecraft": { "version": "1.20.4", "fabricLoader": "0.15.11" },
                "server": { "address": "erkuia.kr" },
                "mods": [{
                    "id": "rendog-client",
                    "name": "RendogClient",
                    "description": "서버 자동 접속 · 필수 모드",
                    "required": true,
                    "url": "https://example.invalid/RendogClient-Delta.jar",
                    "fileName": "RendogClient-Delta.jar",
                    "size": 8709016,
                    "sha256": "72fc258a685734e9cb7914aca0cabf60696facb2253b48dd959eede94b1c111a"
                }]
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn only_jar_files_count() {
        assert!(is_jar(Path::new("a.jar")));
        assert!(is_jar(Path::new("a.JAR")));
        assert!(!is_jar(Path::new("a.zip")));
        assert!(!is_jar(Path::new("a")));
    }

    #[test]
    fn a_jar_in_the_mods_folder_is_enabled() {
        let fixture = Fixture::new("enabled");
        fixture.put("mods", "custom.jar");

        let entries = fixture.scan(None);

        assert_eq!(entries.len(), 1);
        assert!(entries[0].enabled);
        assert_eq!(entries[0].kind, ModKind::Local);
    }

    #[test]
    fn a_jar_in_the_disabled_folder_is_off() {
        let fixture = Fixture::new("disabled");
        fixture.put("mods-disabled", "custom.jar");

        let entries = fixture.scan(None);

        assert_eq!(entries.len(), 1);
        assert!(!entries[0].enabled);
    }

    #[test]
    fn the_enabled_copy_wins_when_a_jar_sits_in_both_folders() {
        let fixture = Fixture::new("both");
        fixture.put("mods", "custom.jar");
        fixture.put("mods-disabled", "custom.jar");

        let entries = fixture.scan(None);

        assert_eq!(entries.len(), 1);
        assert!(entries[0].enabled);
    }

    #[test]
    fn a_manifest_mod_is_marked_required() {
        let fixture = Fixture::new("required");
        fixture.put("mods", "RendogClient-Delta.jar");

        let entries = fixture.scan(Some(&manifest()));

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, ModKind::Required);
        assert_eq!(entries[0].id, "rendog-client");
        assert_eq!(entries[0].name, "RendogClient");
        assert_eq!(entries[0].description, "서버 자동 접속 · 필수 모드");
    }

    #[test]
    fn a_required_mod_that_is_not_installed_still_shows_up() {
        let fixture = Fixture::new("missing");

        let entries = fixture.scan(Some(&manifest()));

        assert_eq!(entries.len(), 1);
        assert!(!entries[0].enabled);
        assert_eq!(missing_required(&entries).len(), 1);
    }

    #[test]
    fn required_mods_are_listed_before_local_ones() {
        let fixture = Fixture::new("order");
        fixture.put("mods", "aaa.jar");
        fixture.put("mods", "RendogClient-Delta.jar");
        fixture.put("mods", "zzz.jar");

        let entries = fixture.scan(Some(&manifest()));

        assert_eq!(entries[0].kind, ModKind::Required);
        assert_eq!(
            entries[1..].iter().map(|e| e.file_name.as_str()).collect::<Vec<_>>(),
            vec!["aaa.jar", "zzz.jar"]
        );
    }

    #[test]
    fn required_mods_cannot_be_removed_or_disabled() {
        let fixture = Fixture::new("locked");
        fixture.put("mods", "RendogClient-Delta.jar");
        fixture.put("mods", "custom.jar");

        let entries = fixture.scan(Some(&manifest()));
        let required = &entries[0];
        let custom = &entries[1];

        assert!(!required.is_removable());
        assert!(!required.can_disable());
        assert!(custom.is_removable());
        assert!(custom.can_disable());
    }

    #[test]
    fn the_bundled_mod_is_required_even_without_a_manifest_entry() {
        let fixture = Fixture::new("bundled");
        fixture.put("mods", crate::bundled::FILE_NAME);

        let entries = fixture.scan(Some(&manifest()));
        let bundled = entries
            .iter()
            .find(|entry| crate::bundled::is_bundled(&entry.file_name))
            .expect("the bundled mod shows up");

        assert_eq!(bundled.kind, ModKind::Required);
        assert!(!bundled.is_removable());
        assert!(!bundled.can_disable());
    }

    #[test]
    fn only_local_mods_reach_the_settings_list() {
        let fixture = Fixture::new("local");
        fixture.put("mods", "RendogClient-Delta.jar");
        fixture.put("mods", "custom.jar");

        let entries = fixture.scan(Some(&manifest()));

        assert_eq!(local(&entries).len(), 1);
        assert_eq!(local(&entries)[0].file_name, "custom.jar");
    }

    #[test]
    fn an_unknown_jar_falls_back_to_its_file_name() {
        let fixture = Fixture::new("fallback");
        fixture.put("mods", "some-mod-1.2.3.jar");

        let entries = fixture.scan(Some(&manifest()));
        let local = local(&entries);

        assert_eq!(local[0].name, "some-mod-1.2.3");
        assert!(local[0].description.is_empty());
    }

    #[test]
    fn non_jar_files_are_ignored() {
        let fixture = Fixture::new("noise");
        fixture.put("mods", "readme.txt");
        fixture.put("mods", "custom.jar");

        assert_eq!(fixture.scan(None).len(), 1);
    }

    #[test]
    fn missing_folders_are_treated_as_empty() {
        let root = std::env::temp_dir().join(format!("erkuia-mods-none-{}", std::process::id()));

        assert!(scan(&root.join("mods"), &root.join("mods-disabled"), None).is_empty());
    }

    #[test]
    fn fabric_metadata_is_read_from_the_manifest_entry() {
        let metadata = parse_metadata(
            r#"{
                "schemaVersion": 1,
                "id": "inventory-sorter",
                "name": "Inventory Sorter",
                "version": "1.2.3",
                "description": "인벤토리 정렬 도구"
            }"#,
        )
        .unwrap();

        assert_eq!(metadata.id, "inventory-sorter");
        assert_eq!(metadata.name, "Inventory Sorter");
        assert_eq!(metadata.description, "인벤토리 정렬 도구");
    }

    #[test]
    fn metadata_without_an_id_or_name_is_ignored() {
        assert!(parse_metadata(r#"{"schemaVersion": 1}"#).is_none());
        assert!(parse_metadata("not json").is_none());
    }

    #[test]
    fn metadata_missing_a_description_still_parses() {
        let metadata = parse_metadata(r#"{"id":"a","name":"A"}"#).unwrap();

        assert!(metadata.description.is_empty());
    }

    #[test]
    fn a_jar_without_fabric_metadata_yields_nothing() {
        let fixture = Fixture::new("nometa");
        fixture.put("mods", "plain.jar");

        assert!(read_metadata(&fixture.mods().join("plain.jar")).is_none());
    }

    #[test]
    fn adding_a_local_mod_copies_it_into_mods() {
        let fixture = Fixture::new("add");
        let source = fixture.root.join("downloaded.jar");
        std::fs::write(&source, b"jar").unwrap();

        let name = add_local(&fixture.mods(), &fixture.disabled(), &source).unwrap();

        assert_eq!(name, "downloaded.jar");
        assert!(fixture.mods().join("downloaded.jar").is_file());
        assert!(source.is_file());
    }

    #[test]
    fn adding_replaces_a_parked_copy_of_the_same_name() {
        let fixture = Fixture::new("add-parked");
        fixture.put("mods-disabled", "custom.jar");
        let source = fixture.root.join("custom.jar");
        std::fs::write(&source, b"jar").unwrap();

        add_local(&fixture.mods(), &fixture.disabled(), &source).unwrap();

        assert!(fixture.mods().join("custom.jar").is_file());
        assert!(!fixture.disabled().join("custom.jar").exists());
    }

    #[test]
    fn only_jar_files_can_be_added() {
        let fixture = Fixture::new("add-bad");
        let source = fixture.root.join("notes.txt");
        std::fs::write(&source, b"text").unwrap();

        assert!(add_local(&fixture.mods(), &fixture.disabled(), &source).is_err());
    }

    #[test]
    fn adding_a_missing_file_is_reported() {
        let fixture = Fixture::new("add-missing");

        assert!(add_local(
            &fixture.mods(),
            &fixture.disabled(),
            &fixture.root.join("ghost.jar")
        )
        .is_err());
    }

    #[test]
    fn turning_a_local_mod_off_moves_it_to_the_disabled_folder() {
        let fixture = Fixture::new("off");
        fixture.put("mods", "custom.jar");
        let entries = fixture.scan(None);

        set_enabled(&fixture.mods(), &fixture.disabled(), &entries[0], false).unwrap();

        assert!(!fixture.mods().join("custom.jar").exists());
        assert!(fixture.disabled().join("custom.jar").is_file());
    }

    #[test]
    fn turning_it_back_on_moves_it_home() {
        let fixture = Fixture::new("on");
        fixture.put("mods-disabled", "custom.jar");
        let entries = fixture.scan(None);

        set_enabled(&fixture.mods(), &fixture.disabled(), &entries[0], true).unwrap();

        assert!(fixture.mods().join("custom.jar").is_file());
        assert!(!fixture.disabled().join("custom.jar").exists());
    }

    #[test]
    fn a_required_mod_cannot_be_turned_off() {
        let fixture = Fixture::new("locked-off");
        fixture.put("mods", "RendogClient-Delta.jar");
        let entries = fixture.scan(Some(&manifest()));

        let error = set_enabled(&fixture.mods(), &fixture.disabled(), &entries[0], false)
            .unwrap_err()
            .to_string();

        assert!(error.contains("필수"));
        assert!(fixture.mods().join("RendogClient-Delta.jar").is_file());
    }

    #[test]
    fn a_required_mod_cannot_be_removed() {
        let fixture = Fixture::new("locked-rm");
        fixture.put("mods", "RendogClient-Delta.jar");
        let entries = fixture.scan(Some(&manifest()));

        assert!(remove(&fixture.mods(), &fixture.disabled(), &entries[0]).is_err());
        assert!(fixture.mods().join("RendogClient-Delta.jar").is_file());
    }

    #[test]
    fn toggling_to_the_state_it_is_already_in_is_harmless() {
        let fixture = Fixture::new("noop");
        fixture.put("mods", "custom.jar");
        let entries = fixture.scan(None);

        set_enabled(&fixture.mods(), &fixture.disabled(), &entries[0], true).unwrap();

        assert!(fixture.mods().join("custom.jar").is_file());
    }

    #[test]
    fn a_stray_copy_in_the_other_folder_is_cleaned_up_on_toggle() {
        let fixture = Fixture::new("stray");
        fixture.put("mods", "custom.jar");
        fixture.put("mods-disabled", "custom.jar");
        let entries = fixture.scan(None);

        set_enabled(&fixture.mods(), &fixture.disabled(), &entries[0], false).unwrap();

        assert!(fixture.disabled().join("custom.jar").is_file());
        assert!(!fixture.mods().join("custom.jar").exists());
    }

    #[test]
    fn removing_a_local_mod_clears_both_folders() {
        let fixture = Fixture::new("remove");
        fixture.put("mods", "custom.jar");
        fixture.put("mods-disabled", "custom.jar");
        let entries = fixture.scan(None);

        remove(&fixture.mods(), &fixture.disabled(), &entries[0]).unwrap();

        assert!(!fixture.mods().join("custom.jar").exists());
        assert!(!fixture.disabled().join("custom.jar").exists());
    }

    #[test]
    fn removing_something_that_is_not_there_is_reported() {
        let fixture = Fixture::new("gone");
        fixture.put("mods", "custom.jar");
        let entries = fixture.scan(None);
        std::fs::remove_file(fixture.mods().join("custom.jar")).unwrap();

        assert!(remove(&fixture.mods(), &fixture.disabled(), &entries[0]).is_err());
    }

    #[test]
    fn lookups_go_through_the_mod_id() {
        let fixture = Fixture::new("by-id");
        fixture.put("mods", "custom.jar");
        let entries = fixture.scan(None);

        set_enabled_by_id(
            &fixture.mods(),
            &fixture.disabled(),
            &entries,
            &entries[0].id,
            false,
        )
        .unwrap();

        assert!(fixture.disabled().join("custom.jar").is_file());
    }

    #[test]
    fn an_unknown_id_is_reported() {
        let fixture = Fixture::new("unknown-id");

        assert!(set_enabled_by_id(
            &fixture.mods(),
            &fixture.disabled(),
            &[],
            "nope",
            false
        )
        .is_err());
    }

    #[test]
    fn required_targets_carry_the_manifest_hash_and_land_in_mods() {
        let targets = required_targets(&manifest());

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].relative_path, "mods/RendogClient-Delta.jar");
        assert_eq!(
            targets[0].checksum,
            Some(crate::hash::Checksum::Sha256(
                "72fc258a685734e9cb7914aca0cabf60696facb2253b48dd959eede94b1c111a".to_string()
            ))
        );
        assert_eq!(targets[0].size, 8_709_016);
    }

    #[test]
    fn a_parked_required_mod_is_moved_back_into_mods() {
        let fixture = Fixture::new("restore");
        fixture.put("mods-disabled", "RendogClient-Delta.jar");

        let sync =
            prepare_required(&fixture.mods(), &fixture.disabled(), &manifest(), &[]).unwrap();

        assert_eq!(sync.restored, vec!["RendogClient-Delta.jar"]);
        assert!(fixture.mods().join("RendogClient-Delta.jar").is_file());
        assert!(!fixture.disabled().join("RendogClient-Delta.jar").exists());
    }

    #[test]
    fn a_duplicate_parked_copy_is_dropped() {
        let fixture = Fixture::new("dup");
        fixture.put("mods", "RendogClient-Delta.jar");
        fixture.put("mods-disabled", "RendogClient-Delta.jar");

        let sync =
            prepare_required(&fixture.mods(), &fixture.disabled(), &manifest(), &[]).unwrap();

        assert!(sync.restored.is_empty());
        assert!(fixture.mods().join("RendogClient-Delta.jar").is_file());
        assert!(!fixture.disabled().join("RendogClient-Delta.jar").exists());
    }

    #[test]
    fn a_managed_mod_dropped_from_the_manifest_is_removed() {
        let fixture = Fixture::new("stale");
        fixture.put("mods", "RendogClient-Charlie.jar");
        fixture.put("mods", "RendogClient-Delta.jar");

        let managed = vec!["RendogClient-Charlie.jar".to_string()];
        let sync =
            prepare_required(&fixture.mods(), &fixture.disabled(), &manifest(), &managed).unwrap();

        assert_eq!(sync.removed, vec!["RendogClient-Charlie.jar"]);
        assert!(!fixture.mods().join("RendogClient-Charlie.jar").exists());
        assert!(fixture.mods().join("RendogClient-Delta.jar").is_file());
    }

    #[test]
    fn a_user_added_mod_is_never_removed() {
        let fixture = Fixture::new("keep");
        fixture.put("mods", "my-favourite.jar");

        let sync =
            prepare_required(&fixture.mods(), &fixture.disabled(), &manifest(), &[]).unwrap();

        assert!(sync.removed.is_empty());
        assert!(fixture.mods().join("my-favourite.jar").is_file());
    }

    #[test]
    fn a_mod_still_required_is_not_treated_as_stale() {
        let fixture = Fixture::new("current");
        fixture.put("mods", "RendogClient-Delta.jar");

        let managed = vec!["RendogClient-Delta.jar".to_string()];
        let sync =
            prepare_required(&fixture.mods(), &fixture.disabled(), &manifest(), &managed).unwrap();

        assert!(sync.removed.is_empty());
        assert!(fixture.mods().join("RendogClient-Delta.jar").is_file());
    }

    #[test]
    fn the_managed_list_comes_from_the_manifest() {
        assert_eq!(
            required_file_names(&manifest()),
            vec!["RendogClient-Delta.jar"]
        );
    }

    #[test]
    fn the_path_follows_the_enabled_state() {
        let fixture = Fixture::new("path");
        fixture.put("mods-disabled", "custom.jar");

        let entries = fixture.scan(None);
        let path = path_of(&fixture.mods(), &fixture.disabled(), &entries[0]);

        assert!(path.starts_with(fixture.disabled()));
    }
}
