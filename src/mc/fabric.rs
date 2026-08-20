use std::collections::HashSet;

use anyhow::{bail, Context};
use serde::Deserialize;

use crate::{hash::Checksum, mc::version::{maven_key, maven_path, DownloadTarget}};

pub const META_BASE: &str = "https://meta.fabricmc.net/v2";
pub const DEFAULT_MAVEN: &str = "https://maven.fabricmc.net/";

#[derive(Debug, Clone, Deserialize)]
pub struct LoaderEntry {
    pub loader: LoaderInfo,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoaderInfo {
    pub version: String,
    #[serde(default)]
    pub stable: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Profile {
    pub id: String,
    #[serde(rename = "inheritsFrom")]
    pub inherits_from: String,
    #[serde(rename = "mainClass")]
    pub main_class: String,
    #[serde(default)]
    pub libraries: Vec<ProfileLibrary>,
    #[serde(default)]
    pub arguments: ProfileArguments,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProfileLibrary {
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProfileArguments {
    #[serde(default)]
    pub jvm: Vec<String>,
    #[serde(default)]
    pub game: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct LoaderPlan {
    pub loader_version: String,
    pub main_class: String,
    pub libraries: Vec<DownloadTarget>,
    pub jvm_arguments: Vec<String>,
}

pub fn loaders_url(game_version: &str) -> String {
    format!("{META_BASE}/versions/loader/{game_version}")
}

pub fn profile_url(game_version: &str, loader_version: &str) -> String {
    format!("{META_BASE}/versions/loader/{game_version}/{loader_version}/profile/json")
}

fn maven_base(url: Option<&str>) -> String {
    let base = url.unwrap_or(DEFAULT_MAVEN);

    if base.ends_with('/') {
        base.to_string()
    } else {
        format!("{base}/")
    }
}

impl ProfileLibrary {
    pub fn target(&self) -> Option<DownloadTarget> {
        let path = maven_path(&self.name, None)?;

        Some(DownloadTarget {
            url: format!("{}{path}", maven_base(self.url.as_deref())),
            relative_path: format!("libraries/{path}"),
            checksum: self.sha1.clone().map(Checksum::Sha1),
            size: self.size.unwrap_or(0),
            name: Some(self.name.clone()),
        })
    }
}

impl Profile {
    pub fn plan(&self, loader_version: &str) -> LoaderPlan {
        LoaderPlan {
            loader_version: loader_version.to_string(),
            main_class: self.main_class.clone(),
            libraries: self
                .libraries
                .iter()
                .filter_map(ProfileLibrary::target)
                .collect(),
            jvm_arguments: self.arguments.jvm.clone(),
        }
    }
}

/// Fabric ships newer copies of libraries that vanilla also provides (asm and
/// friends). Its versions win, so the loader entries go first and vanilla only
/// contributes artifacts Fabric did not already cover.
pub fn merge_libraries(loader: &[DownloadTarget], vanilla: &[DownloadTarget]) -> Vec<DownloadTarget> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut merged = Vec::with_capacity(loader.len() + vanilla.len());

    for target in loader.iter().chain(vanilla.iter()) {
        let key = target
            .name
            .as_deref()
            .and_then(maven_key)
            .unwrap_or_else(|| target.relative_path.clone());

        if seen.insert(key) {
            merged.push(target.clone());
        }
    }

    merged
}

pub fn pick_loader(entries: &[LoaderEntry]) -> Option<&LoaderInfo> {
    entries
        .iter()
        .map(|entry| &entry.loader)
        .find(|loader| loader.stable)
        .or_else(|| entries.first().map(|entry| &entry.loader))
}

pub fn fetch_loaders(game_version: &str) -> anyhow::Result<Vec<LoaderEntry>> {
    crate::http::send(crate::http::client()?.get(loaders_url(game_version)))
        .context("Fabric 로더 목록을 받지 못했어요.")?
        .json()
        .context("Fabric 로더 목록을 해석하지 못했어요.")
}

pub fn fetch_profile(game_version: &str, loader_version: &str) -> anyhow::Result<Profile> {
    let profile: Profile =
        crate::http::send(crate::http::client()?.get(profile_url(game_version, loader_version)))
            .context("Fabric 프로필을 받지 못했어요.")?
            .json()
            .context("Fabric 프로필을 해석하지 못했어요.")?;

    if profile.inherits_from != game_version {
        bail!(
            "Fabric 프로필이 다른 버전을 가리켜요: {} (요청 {game_version})",
            profile.inherits_from
        );
    }

    Ok(profile)
}

pub fn resolve(game_version: &str, pinned: &str) -> anyhow::Result<LoaderPlan> {
    let loader_version = if pinned.is_empty() {
        let entries = fetch_loaders(game_version)?;
        let Some(loader) = pick_loader(&entries) else {
            bail!("{game_version} 용 Fabric 로더를 찾지 못했어요.");
        };
        loader.version.clone()
    } else {
        pinned.to_string()
    };

    let profile = fetch_profile(game_version, &loader_version)?;

    log::info!(
        "Fabric 로더 {} · 라이브러리 {}개 · mainClass {}",
        loader_version,
        profile.libraries.len(),
        profile.main_class
    );

    Ok(profile.plan(&loader_version))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROFILE: &str = r#"{
        "id": "fabric-loader-0.15.11-1.20.4",
        "inheritsFrom": "1.20.4",
        "mainClass": "net.fabricmc.loader.impl.launch.knot.KnotClient",
        "arguments": { "jvm": ["-DFabricMcEmu= net.minecraft.client.main.Main "], "game": [] },
        "libraries": [
            { "name": "net.fabricmc:sponge-mixin:0.13.2+mixin.0.8.5", "url": "https://maven.fabricmc.net/" },
            { "name": "org.ow2.asm:asm:9.6", "url": "https://maven.fabricmc.net" },
            { "name": "net.fabricmc:fabric-loader:0.15.11" }
        ]
    }"#;

    fn vanilla(name: &str, path: &str) -> DownloadTarget {
        DownloadTarget {
            url: format!("https://libraries.minecraft.net/{path}"),
            relative_path: format!("libraries/{path}"),
            checksum: Some(Checksum::Sha1("a".repeat(40))),
            size: 1,
            name: Some(name.to_string()),
        }
    }

    #[test]
    fn a_natives_jar_is_not_swallowed_by_its_plain_artifact() {
        let vanilla = vec![
            vanilla("org.lwjgl:lwjgl:3.3.2", "org/lwjgl/lwjgl/3.3.2/lwjgl-3.3.2.jar"),
            vanilla(
                "org.lwjgl:lwjgl:3.3.2:natives-windows",
                "org/lwjgl/lwjgl/3.3.2/lwjgl-3.3.2-natives-windows.jar",
            ),
        ];

        let merged = merge_libraries(&[], &vanilla);

        assert_eq!(merged.len(), 2);
        assert!(merged
            .iter()
            .any(|target| target.relative_path.ends_with("natives-windows.jar")));
    }

    #[test]
    fn builds_the_meta_urls() {
        assert_eq!(
            loaders_url("1.20.4"),
            "https://meta.fabricmc.net/v2/versions/loader/1.20.4"
        );
        assert_eq!(
            profile_url("1.20.4", "0.15.11"),
            "https://meta.fabricmc.net/v2/versions/loader/1.20.4/0.15.11/profile/json"
        );
    }

    #[test]
    fn parses_the_profile() {
        let profile: Profile = serde_json::from_str(PROFILE).unwrap();
        let plan = profile.plan("0.15.11");

        assert_eq!(
            plan.main_class,
            "net.fabricmc.loader.impl.launch.knot.KnotClient"
        );
        assert_eq!(plan.libraries.len(), 3);
        assert_eq!(plan.jvm_arguments.len(), 1);
    }

    #[test]
    fn library_urls_are_built_from_the_maven_base() {
        let profile: Profile = serde_json::from_str(PROFILE).unwrap();
        let plan = profile.plan("0.15.11");

        assert_eq!(
            plan.libraries[0].url,
            "https://maven.fabricmc.net/net/fabricmc/sponge-mixin/0.13.2+mixin.0.8.5/sponge-mixin-0.13.2+mixin.0.8.5.jar"
        );
        assert_eq!(
            plan.libraries[0].relative_path,
            "libraries/net/fabricmc/sponge-mixin/0.13.2+mixin.0.8.5/sponge-mixin-0.13.2+mixin.0.8.5.jar"
        );
    }

    #[test]
    fn a_maven_base_without_a_trailing_slash_still_works() {
        let profile: Profile = serde_json::from_str(PROFILE).unwrap();
        let plan = profile.plan("0.15.11");

        assert_eq!(
            plan.libraries[1].url,
            "https://maven.fabricmc.net/org/ow2/asm/asm/9.6/asm-9.6.jar"
        );
    }

    #[test]
    fn a_library_without_a_url_falls_back_to_the_fabric_maven() {
        let profile: Profile = serde_json::from_str(PROFILE).unwrap();
        let plan = profile.plan("0.15.11");

        assert!(plan.libraries[2].url.starts_with(DEFAULT_MAVEN));
    }

    #[test]
    fn fabric_libraries_have_no_hash_to_verify() {
        let profile: Profile = serde_json::from_str(PROFILE).unwrap();
        let plan = profile.plan("0.15.11");

        assert!(plan.libraries.iter().all(|target| target.checksum.is_none()));
    }

    #[test]
    fn the_loader_copy_of_a_shared_library_wins() {
        let profile: Profile = serde_json::from_str(PROFILE).unwrap();
        let loader = profile.plan("0.15.11").libraries;
        let vanilla = vec![
            vanilla("org.ow2.asm:asm:9.3", "org/ow2/asm/asm/9.3/asm-9.3.jar"),
            vanilla("com.google.guava:guava:32.1.2", "com/google/guava/guava/32.1.2/guava-32.1.2.jar"),
        ];

        let merged = merge_libraries(&loader, &vanilla);
        let asm: Vec<&DownloadTarget> = merged
            .iter()
            .filter(|target| target.name.as_deref().is_some_and(|n| n.contains(":asm:")))
            .collect();

        assert_eq!(asm.len(), 1);
        assert_eq!(asm[0].name.as_deref(), Some("org.ow2.asm:asm:9.6"));
        assert_eq!(merged.len(), 4);
    }

    #[test]
    fn merging_keeps_the_loader_entries_first() {
        let profile: Profile = serde_json::from_str(PROFILE).unwrap();
        let loader = profile.plan("0.15.11").libraries;
        let vanilla = vec![vanilla(
            "com.google.guava:guava:32.1.2",
            "com/google/guava/guava/32.1.2/guava-32.1.2.jar",
        )];

        let merged = merge_libraries(&loader, &vanilla);

        assert_eq!(merged[0].name, loader[0].name);
        assert_eq!(merged.last().unwrap().name.as_deref(), Some("com.google.guava:guava:32.1.2"));
    }

    #[test]
    fn a_stable_loader_is_preferred() {
        let entries: Vec<LoaderEntry> = serde_json::from_str(
            r#"[
                { "loader": { "version": "0.16.0-beta.1", "stable": false } },
                { "loader": { "version": "0.15.11", "stable": true } }
            ]"#,
        )
        .unwrap();

        assert_eq!(pick_loader(&entries).unwrap().version, "0.15.11");
    }

    #[test]
    fn the_newest_loader_is_used_when_none_are_stable() {
        let entries: Vec<LoaderEntry> = serde_json::from_str(
            r#"[
                { "loader": { "version": "0.16.0-beta.2", "stable": false } },
                { "loader": { "version": "0.16.0-beta.1", "stable": false } }
            ]"#,
        )
        .unwrap();

        assert_eq!(pick_loader(&entries).unwrap().version, "0.16.0-beta.2");
    }

    #[test]
    fn an_empty_loader_list_yields_nothing() {
        assert!(pick_loader(&[]).is_none());
    }
}
