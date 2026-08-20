use std::collections::{HashMap, HashSet};

use anyhow::{bail, Context};
use serde::Deserialize;

use crate::mc::version::{AssetIndexRef, DownloadTarget};

pub const RESOURCE_BASE: &str = "https://resources.download.minecraft.net";

const SHA1_HEX_LEN: usize = 40;

#[derive(Debug, Clone, Deserialize)]
pub struct AssetIndex {
    #[serde(default)]
    pub objects: HashMap<String, AssetObject>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AssetObject {
    pub hash: String,
    pub size: u64,
}

fn is_sha1(value: &str) -> bool {
    value.len() == SHA1_HEX_LEN && value.chars().all(|c| c.is_ascii_hexdigit())
}

pub fn object_url(hash: &str) -> String {
    format!("{RESOURCE_BASE}/{}/{hash}", &hash[..2])
}

pub fn object_path(hash: &str) -> String {
    format!("assets/objects/{}/{hash}", &hash[..2])
}

pub fn index_path(id: &str) -> String {
    format!("assets/indexes/{id}.json")
}

impl AssetIndex {
    pub fn targets(&self) -> anyhow::Result<Vec<DownloadTarget>> {
        let mut seen = HashSet::new();
        let mut targets = Vec::new();

        for (name, object) in &self.objects {
            if !is_sha1(&object.hash) {
                bail!("에셋 해시가 올바르지 않아요: {name}");
            }

            if !seen.insert(object.hash.clone()) {
                continue;
            }

            targets.push(DownloadTarget {
                url: object_url(&object.hash),
                relative_path: object_path(&object.hash),
                sha1: Some(object.hash.clone()),
                size: object.size,
                name: None,
            });
        }

        targets.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

        Ok(targets)
    }

    pub fn total_size(&self) -> u64 {
        let mut seen = HashSet::new();

        self.objects
            .values()
            .filter(|object| seen.insert(object.hash.as_str()))
            .map(|object| object.size)
            .sum()
    }
}

pub fn index_target(reference: &AssetIndexRef) -> DownloadTarget {
    DownloadTarget {
        url: reference.url.clone(),
        relative_path: index_path(&reference.id),
        sha1: Some(reference.sha1.clone()),
        size: reference.size,
        name: None,
    }
}

pub fn fetch(reference: &AssetIndexRef) -> anyhow::Result<AssetIndex> {
    let index: AssetIndex = crate::http::send(crate::http::client()?.get(&reference.url))
        .context("에셋 목록을 받지 못했어요.")?
        .json()
        .context("에셋 목록을 해석하지 못했어요.")?;

    log::info!(
        "에셋 인덱스 {} · 항목 {}개 · {} 바이트",
        reference.id,
        index.objects.len(),
        index.total_size()
    );

    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "objects": {
            "minecraft/sounds/ambient/cave/cave1.ogg": {
                "hash": "0f4b5a1c1d0f1b3a5c7e9f2b4d6a8c0e2f4b6d8a",
                "size": 1024
            },
            "minecraft/lang/ko_kr.json": {
                "hash": "1a2b3c4d5e6f70819293a4b5c6d7e8f901234567",
                "size": 2048
            }
        }
    }"#;

    #[test]
    fn parses_the_index() {
        let index: AssetIndex = serde_json::from_str(SAMPLE).unwrap();

        assert_eq!(index.objects.len(), 2);
        assert_eq!(index.total_size(), 3072);
    }

    #[test]
    fn the_url_and_path_use_the_first_two_hash_characters() {
        let hash = "0f4b5a1c1d0f1b3a5c7e9f2b4d6a8c0e2f4b6d8a";

        assert_eq!(
            object_url(hash),
            "https://resources.download.minecraft.net/0f/0f4b5a1c1d0f1b3a5c7e9f2b4d6a8c0e2f4b6d8a"
        );
        assert_eq!(
            object_path(hash),
            "assets/objects/0f/0f4b5a1c1d0f1b3a5c7e9f2b4d6a8c0e2f4b6d8a"
        );
    }

    #[test]
    fn the_index_file_lands_next_to_the_objects() {
        assert_eq!(index_path("12"), "assets/indexes/12.json");
    }

    #[test]
    fn targets_are_deduplicated_by_hash() {
        let json = r#"{
            "objects": {
                "a.ogg": { "hash": "1a2b3c4d5e6f70819293a4b5c6d7e8f901234567", "size": 10 },
                "b.ogg": { "hash": "1a2b3c4d5e6f70819293a4b5c6d7e8f901234567", "size": 10 },
                "c.ogg": { "hash": "0f4b5a1c1d0f1b3a5c7e9f2b4d6a8c0e2f4b6d8a", "size": 20 }
            }
        }"#;

        let index: AssetIndex = serde_json::from_str(json).unwrap();

        assert_eq!(index.objects.len(), 3);
        assert_eq!(index.targets().unwrap().len(), 2);
        assert_eq!(index.total_size(), 30);
    }

    #[test]
    fn targets_come_back_in_a_stable_order() {
        let index: AssetIndex = serde_json::from_str(SAMPLE).unwrap();

        let first: Vec<String> = index
            .targets()
            .unwrap()
            .into_iter()
            .map(|target| target.relative_path)
            .collect();
        let second: Vec<String> = index
            .targets()
            .unwrap()
            .into_iter()
            .map(|target| target.relative_path)
            .collect();

        assert_eq!(first, second);
        assert!(first[0] < first[1]);
    }

    #[test]
    fn a_malformed_hash_is_refused() {
        let json = r#"{ "objects": { "a.ogg": { "hash": "short", "size": 1 } } }"#;
        let index: AssetIndex = serde_json::from_str(json).unwrap();

        assert!(index.targets().is_err());
    }

    #[test]
    fn an_empty_index_is_allowed() {
        let index: AssetIndex = serde_json::from_str(r#"{ "objects": {} }"#).unwrap();

        assert!(index.targets().unwrap().is_empty());
        assert_eq!(index.total_size(), 0);
    }

    #[test]
    fn the_index_file_itself_is_a_download_target() {
        let reference = AssetIndexRef {
            id: "12".to_string(),
            url: "https://example.invalid/12.json".to_string(),
            sha1: "1a2b3c4d5e6f70819293a4b5c6d7e8f901234567".to_string(),
            size: 400,
            total_size: 900,
        };

        let target = index_target(&reference);

        assert_eq!(target.relative_path, "assets/indexes/12.json");
        assert_eq!(target.size, 400);
    }
}
