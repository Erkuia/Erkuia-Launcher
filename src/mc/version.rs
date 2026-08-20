use std::collections::HashMap;

use anyhow::{bail, Context};
use serde::Deserialize;

use crate::hash::Checksum;

pub const VERSION_MANIFEST_URL: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Platform {
    pub name: &'static str,
    pub arch: &'static str,
    pub bits: &'static str,
}

impl Platform {
    pub fn current() -> Self {
        Self {
            name: if cfg!(target_os = "windows") {
                "windows"
            } else if cfg!(target_os = "macos") {
                "osx"
            } else {
                "linux"
            },
            arch: if cfg!(target_arch = "x86_64") {
                "x86_64"
            } else if cfg!(target_arch = "aarch64") {
                "arm64"
            } else {
                "x86"
            },
            bits: if cfg!(target_pointer_width = "64") {
                "64"
            } else {
                "32"
            },
        }
    }

    fn natives_os(self) -> &'static str {
        match self.name {
            "osx" => "macos",
            other => other,
        }
    }

    /// 1.20.4 ships every Windows native under the same `os.name` rule, so the
    /// rules alone would let x86 and arm64 binaries onto an x64 classpath. The
    /// classifier is what actually names the architecture: 64-bit gets the bare
    /// `natives-<os>` and everything else carries an explicit suffix.
    pub fn native_classifier(self) -> String {
        let os = self.natives_os();

        match self.arch {
            "x86_64" => format!("natives-{os}"),
            arch => format!("natives-{os}-{arch}"),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct VersionIndex {
    pub versions: Vec<VersionSummary>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VersionSummary {
    pub id: String,
    pub url: String,
    #[serde(rename = "type")]
    pub kind: String,
}

impl VersionIndex {
    pub fn find(&self, id: &str) -> Option<&VersionSummary> {
        self.versions.iter().find(|version| version.id == id)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct VersionDetail {
    pub id: String,
    #[serde(rename = "mainClass")]
    pub main_class: String,
    #[serde(rename = "assetIndex")]
    pub asset_index: AssetIndexRef,
    pub assets: String,
    pub downloads: ClientDownloads,
    #[serde(default)]
    pub libraries: Vec<Library>,
    #[serde(rename = "javaVersion", default)]
    pub java_version: Option<JavaVersion>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssetIndexRef {
    pub id: String,
    pub url: String,
    pub sha1: String,
    pub size: u64,
    #[serde(rename = "totalSize", default)]
    pub total_size: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClientDownloads {
    pub client: Artifact,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JavaVersion {
    #[serde(rename = "majorVersion")]
    pub major_version: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Artifact {
    #[serde(default)]
    pub path: Option<String>,
    pub url: String,
    pub sha1: String,
    pub size: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Library {
    pub name: String,
    #[serde(default)]
    pub downloads: Option<LibraryDownloads>,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub natives: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LibraryDownloads {
    #[serde(default)]
    pub artifact: Option<Artifact>,
    #[serde(default)]
    pub classifiers: HashMap<String, Artifact>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    pub action: String,
    #[serde(default)]
    pub os: Option<OsRule>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OsRule {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arch: Option<String>,
}

pub fn rules_allow(rules: &[Rule], platform: Platform) -> bool {
    if rules.is_empty() {
        return true;
    }

    let mut allowed = false;

    for rule in rules {
        let matches = match &rule.os {
            None => true,
            Some(os) => {
                os.name.as_deref().is_none_or(|name| name == platform.name)
                    && os.arch.as_deref().is_none_or(|arch| arch == platform.arch)
            }
        };

        if matches {
            allowed = rule.action == "allow";
        }
    }

    allowed
}

impl Library {
    pub fn natives_classifier(&self, platform: Platform) -> Option<String> {
        let natives = self.natives.as_ref()?;
        let template = natives.get(platform.name)?;

        Some(template.replace("${arch}", platform.bits))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadTarget {
    pub url: String,
    pub relative_path: String,
    pub checksum: Option<Checksum>,
    pub size: u64,
    pub name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct VersionPlan {
    pub id: String,
    pub main_class: String,
    pub asset_index: AssetIndexRef,
    pub assets: String,
    pub java_major: u32,
    pub client: DownloadTarget,
    pub libraries: Vec<DownloadTarget>,
    pub natives: Vec<DownloadTarget>,
}

/// The version is left out so a newer copy of an artifact replaces an older
/// one, but the classifier is kept: `org.lwjgl:lwjgl:3.3.2:natives-windows`
/// carries the platform binaries and must not collapse into the plain jar.
pub fn maven_key(name: &str) -> Option<String> {
    let mut parts = name.split(':');
    let group = parts.next()?;
    let artifact = parts.next()?;
    let _version = parts.next();

    Some(match parts.next() {
        Some(classifier) => format!("{group}:{artifact}:{classifier}"),
        None => format!("{group}:{artifact}"),
    })
}

pub fn maven_classifier(name: &str) -> Option<&str> {
    name.split(':').nth(3)
}

pub const NATIVES_PREFIX: &str = "natives-";

pub fn maven_path(name: &str, classifier: Option<&str>) -> Option<String> {
    let mut parts = name.split(':');
    let group = parts.next()?.replace('.', "/");
    let artifact = parts.next()?;
    let version = parts.next()?;

    let suffix = match classifier {
        Some(classifier) => format!("-{classifier}"),
        None => String::new(),
    };

    Some(format!(
        "{group}/{artifact}/{version}/{artifact}-{version}{suffix}.jar"
    ))
}

fn target_from(artifact: &Artifact, name: &str, classifier: Option<&str>) -> Option<DownloadTarget> {
    let relative_path = artifact
        .path
        .clone()
        .or_else(|| maven_path(name, classifier))?;

    Some(DownloadTarget {
        url: artifact.url.clone(),
        relative_path: format!("libraries/{relative_path}"),
        checksum: Some(Checksum::Sha1(artifact.sha1.clone())),
        size: artifact.size,
        name: Some(name.to_string()),
    })
}

impl VersionDetail {
    pub fn plan(&self, platform: Platform) -> anyhow::Result<VersionPlan> {
        let mut libraries = Vec::new();
        let mut natives = Vec::new();
        let wanted_natives = platform.native_classifier();

        for library in &self.libraries {
            if !rules_allow(&library.rules, platform) {
                continue;
            }

            let foreign_natives = maven_classifier(&library.name).is_some_and(|classifier| {
                classifier.starts_with(NATIVES_PREFIX) && classifier != wanted_natives
            });

            if foreign_natives {
                continue;
            }

            let Some(downloads) = &library.downloads else {
                continue;
            };

            if let Some(artifact) = &downloads.artifact {
                if let Some(target) = target_from(artifact, &library.name, None) {
                    libraries.push(target);
                }
            }

            if let Some(classifier) = library.natives_classifier(platform) {
                if let Some(artifact) = downloads.classifiers.get(&classifier) {
                    if let Some(target) = target_from(artifact, &library.name, Some(&classifier)) {
                        natives.push(target);
                    }
                }
            }
        }

        let client = DownloadTarget {
            url: self.downloads.client.url.clone(),
            relative_path: format!("versions/{}/{}.jar", self.id, self.id),
            checksum: Some(Checksum::Sha1(self.downloads.client.sha1.clone())),
            size: self.downloads.client.size,
            name: None,
        };

        Ok(VersionPlan {
            id: self.id.clone(),
            main_class: self.main_class.clone(),
            asset_index: self.asset_index.clone(),
            assets: self.assets.clone(),
            java_major: self.java_version.as_ref().map_or(21, |java| java.major_version),
            client,
            libraries,
            natives,
        })
    }
}

pub fn fetch_index() -> anyhow::Result<VersionIndex> {
    crate::http::send(crate::http::client()?.get(VERSION_MANIFEST_URL))
        .context("Minecraft 버전 목록을 받지 못했어요.")?
        .json()
        .context("Minecraft 버전 목록을 해석하지 못했어요.")
}

pub fn fetch_detail(summary: &VersionSummary) -> anyhow::Result<VersionDetail> {
    crate::http::send(crate::http::client()?.get(&summary.url))
        .with_context(|| format!("{} 버전 정보를 받지 못했어요.", summary.id))?
        .json()
        .with_context(|| format!("{} 버전 정보를 해석하지 못했어요.", summary.id))
}

pub fn resolve(version_id: &str) -> anyhow::Result<VersionDetail> {
    let index = fetch_index()?;

    let Some(summary) = index.find(version_id) else {
        bail!("Minecraft {version_id} 버전을 찾지 못했어요.");
    };

    log::info!("Minecraft {} 버전 정보 확인", summary.id);

    fetch_detail(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOWS: Platform = Platform {
        name: "windows",
        arch: "x86_64",
        bits: "64",
    };
    const LINUX: Platform = Platform {
        name: "linux",
        arch: "x86_64",
        bits: "64",
    };

    fn rule(action: &str, os: Option<&str>, arch: Option<&str>) -> Rule {
        Rule {
            action: action.to_string(),
            os: os.map(|name| OsRule {
                name: Some(name.to_string()),
                arch: arch.map(str::to_string),
            }),
        }
    }

    #[test]
    fn no_rules_means_allowed() {
        assert!(rules_allow(&[], WINDOWS));
    }

    #[test]
    fn an_allow_rule_for_another_os_excludes_this_one() {
        let rules = vec![rule("allow", Some("osx"), None)];

        assert!(!rules_allow(&rules, WINDOWS));
    }

    #[test]
    fn a_matching_allow_rule_includes_it() {
        let rules = vec![rule("allow", Some("windows"), None)];

        assert!(rules_allow(&rules, WINDOWS));
    }

    #[test]
    fn a_later_disallow_overrides_an_earlier_allow() {
        let rules = vec![rule("allow", None, None), rule("disallow", Some("osx"), None)];

        assert!(rules_allow(&rules, WINDOWS));
        assert!(!rules_allow(&rules, Platform { name: "osx", arch: "x86_64", bits: "64" }));
    }

    #[test]
    fn arch_narrows_a_rule() {
        let rules = vec![rule("allow", Some("windows"), Some("x86"))];

        assert!(!rules_allow(&rules, WINDOWS));
        assert!(rules_allow(
            &rules,
            Platform { name: "windows", arch: "x86", bits: "32" }
        ));
    }

    #[test]
    fn the_natives_placeholder_is_filled_with_the_bit_width() {
        let library = Library {
            name: "org.lwjgl:lwjgl:2.9.4".to_string(),
            downloads: None,
            rules: Vec::new(),
            natives: Some(HashMap::from([(
                "windows".to_string(),
                "natives-windows-${arch}".to_string(),
            )])),
        };

        assert_eq!(
            library.natives_classifier(WINDOWS).as_deref(),
            Some("natives-windows-64")
        );
        assert_eq!(library.natives_classifier(LINUX), None);
    }

    #[test]
    fn maven_coordinates_become_repository_paths() {
        assert_eq!(
            maven_path("org.lwjgl:lwjgl:3.3.3", None).unwrap(),
            "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3.jar"
        );
        assert_eq!(
            maven_path("org.lwjgl:lwjgl:3.3.3", Some("natives-windows")).unwrap(),
            "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-windows.jar"
        );
        assert!(maven_path("broken", None).is_none());
    }

    fn detail(libraries: &str) -> VersionDetail {
        let json = format!(
            r#"{{
                "id": "1.20.4",
                "mainClass": "net.minecraft.client.main.Main",
                "assets": "12",
                "assetIndex": {{
                    "id": "12",
                    "url": "https://example.invalid/12.json",
                    "sha1": "aaaa",
                    "size": 10,
                    "totalSize": 100
                }},
                "javaVersion": {{ "majorVersion": 17 }},
                "downloads": {{
                    "client": {{
                        "url": "https://example.invalid/client.jar",
                        "sha1": "bbbb",
                        "size": 20
                    }}
                }},
                "libraries": [{libraries}]
            }}"#
        );

        serde_json::from_str(&json).unwrap()
    }

    #[test]
    fn the_client_jar_lands_in_the_versions_folder() {
        let plan = detail("").plan(WINDOWS).unwrap();

        assert_eq!(plan.client.relative_path, "versions/1.20.4/1.20.4.jar");
        assert_eq!(plan.client.checksum, Some(Checksum::Sha1("bbbb".to_string())));
        assert_eq!(plan.java_major, 17);
    }

    #[test]
    fn platform_specific_libraries_are_filtered_out() {
        let libraries = r#"
            {
                "name": "org.lwjgl:lwjgl:3.3.3",
                "downloads": { "artifact": {
                    "path": "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3.jar",
                    "url": "https://example.invalid/a.jar", "sha1": "1", "size": 1
                }}
            },
            {
                "name": "org.lwjgl:lwjgl:3.3.3:natives-macos",
                "rules": [ { "action": "allow", "os": { "name": "osx" } } ],
                "downloads": { "artifact": {
                    "path": "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-macos.jar",
                    "url": "https://example.invalid/b.jar", "sha1": "2", "size": 2
                }}
            }
        "#;

        let plan = detail(libraries).plan(WINDOWS).unwrap();

        assert_eq!(plan.libraries.len(), 1);
        assert_eq!(plan.libraries[0].checksum, Some(Checksum::Sha1("1".to_string())));
    }

    #[test]
    fn every_target_is_relative_to_the_minecraft_directory() {
        let libraries = r#"
            {
                "name": "org.lwjgl:lwjgl:3.3.3",
                "downloads": { "artifact": {
                    "path": "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3.jar",
                    "url": "https://example.invalid/a.jar", "sha1": "1", "size": 1
                }}
            }
        "#;

        let plan = detail(libraries).plan(WINDOWS).unwrap();

        assert_eq!(
            plan.libraries[0].relative_path,
            "libraries/org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3.jar"
        );
        assert!(plan.client.relative_path.starts_with("versions/"));
    }

    #[test]
    fn legacy_classifier_natives_are_collected_separately() {
        let libraries = r#"
            {
                "name": "org.lwjgl:lwjgl:2.9.4",
                "natives": { "windows": "natives-windows" },
                "downloads": {
                    "artifact": {
                        "path": "org/lwjgl/lwjgl/2.9.4/lwjgl-2.9.4.jar",
                        "url": "https://example.invalid/a.jar", "sha1": "1", "size": 1
                    },
                    "classifiers": {
                        "natives-windows": {
                            "path": "org/lwjgl/lwjgl/2.9.4/lwjgl-2.9.4-natives-windows.jar",
                            "url": "https://example.invalid/n.jar", "sha1": "9", "size": 9
                        }
                    }
                }
            }
        "#;

        let plan = detail(libraries).plan(WINDOWS).unwrap();

        assert_eq!(plan.libraries.len(), 1);
        assert_eq!(plan.natives.len(), 1);
        assert_eq!(plan.natives[0].checksum, Some(Checksum::Sha1("9".to_string())));
    }

    #[test]
    fn a_library_without_downloads_is_skipped() {
        let plan = detail(r#"{ "name": "example:only:1.0" }"#).plan(WINDOWS).unwrap();

        assert!(plan.libraries.is_empty());
    }

    #[test]
    fn java_defaults_to_twenty_one_when_absent() {
        let json = r#"{
            "id": "1.20.4",
            "mainClass": "M",
            "assets": "12",
            "assetIndex": { "id": "12", "url": "u", "sha1": "a", "size": 1 },
            "downloads": { "client": { "url": "u", "sha1": "b", "size": 1 } }
        }"#;

        let detail: VersionDetail = serde_json::from_str(json).unwrap();

        assert_eq!(detail.plan(WINDOWS).unwrap().java_major, 21);
    }

    #[test]
    fn the_native_classifier_follows_the_architecture() {
        let windows = |arch| Platform { name: "windows", arch, bits: "64" };

        assert_eq!(windows("x86_64").native_classifier(), "natives-windows");
        assert_eq!(windows("arm64").native_classifier(), "natives-windows-arm64");
        assert_eq!(windows("x86").native_classifier(), "natives-windows-x86");
        assert_eq!(
            Platform { name: "osx", arch: "arm64", bits: "64" }.native_classifier(),
            "natives-macos-arm64"
        );
    }

    #[test]
    fn only_the_matching_architecture_native_survives() {
        let json = r#"{
            "id": "1.20.4",
            "mainClass": "net.minecraft.client.main.Main",
            "assets": "12",
            "assetIndex": {"id":"12","url":"u","sha1":"s","size":1,"totalSize":2},
            "downloads": {"client": {"url":"c","sha1":"cs","size":3}},
            "libraries": [
                {
                    "name": "org.lwjgl:lwjgl:3.3.2",
                    "downloads": {"artifact": {"path":"org/lwjgl/lwjgl/3.3.2/lwjgl-3.3.2.jar","url":"u","sha1":"a","size":1}}
                },
                {
                    "name": "org.lwjgl:lwjgl:3.3.2:natives-windows",
                    "rules": [{"action":"allow","os":{"name":"windows"}}],
                    "downloads": {"artifact": {"path":"org/lwjgl/lwjgl/3.3.2/lwjgl-3.3.2-natives-windows.jar","url":"u","sha1":"b","size":1}}
                },
                {
                    "name": "org.lwjgl:lwjgl:3.3.2:natives-windows-x86",
                    "rules": [{"action":"allow","os":{"name":"windows"}}],
                    "downloads": {"artifact": {"path":"org/lwjgl/lwjgl/3.3.2/lwjgl-3.3.2-natives-windows-x86.jar","url":"u","sha1":"c","size":1}}
                },
                {
                    "name": "org.lwjgl:lwjgl:3.3.2:natives-windows-arm64",
                    "rules": [{"action":"allow","os":{"name":"windows"}}],
                    "downloads": {"artifact": {"path":"org/lwjgl/lwjgl/3.3.2/lwjgl-3.3.2-natives-windows-arm64.jar","url":"u","sha1":"d","size":1}}
                }
            ]
        }"#;

        let detail: VersionDetail = serde_json::from_str(json).unwrap();
        let plan = detail.plan(WINDOWS).unwrap();
        let paths: Vec<&str> = plan
            .libraries
            .iter()
            .map(|target| target.relative_path.as_str())
            .collect();

        assert_eq!(paths.len(), 2, "got {paths:?}");
        assert!(paths.iter().any(|path| path.ends_with("lwjgl-3.3.2.jar")));
        assert!(paths
            .iter()
            .any(|path| path.ends_with("lwjgl-3.3.2-natives-windows.jar")));
    }

    #[test]
    fn the_key_drops_the_version_so_a_newer_copy_replaces_an_older_one() {
        assert_eq!(
            maven_key("org.ow2.asm:asm:9.6").unwrap(),
            maven_key("org.ow2.asm:asm:9.3").unwrap()
        );
    }

    #[test]
    fn the_key_keeps_the_classifier_so_natives_survive_deduplication() {
        let plain = maven_key("org.lwjgl:lwjgl:3.3.2").unwrap();
        let natives = maven_key("org.lwjgl:lwjgl:3.3.2:natives-windows").unwrap();

        assert_ne!(plain, natives);
        assert_ne!(
            natives,
            maven_key("org.lwjgl:lwjgl:3.3.2:natives-windows-arm64").unwrap()
        );
    }

    #[test]
    fn the_index_can_find_a_version() {
        let index: VersionIndex = serde_json::from_str(
            r#"{"versions":[
                {"id":"1.20.5","type":"release","url":"u1"},
                {"id":"1.20.4","type":"release","url":"u2"}
            ]}"#,
        )
        .unwrap();

        assert_eq!(index.find("1.20.4").unwrap().url, "u2");
        assert!(index.find("1.19.0").is_none());
    }
}
