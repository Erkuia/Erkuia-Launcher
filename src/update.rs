use std::fmt;

use crate::manifest::Manifest;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    /// Accepts what a release tag realistically looks like: an optional `v`, one
    /// to three numbers, and a pre-release or build suffix that takes no part in
    /// the ordering.
    pub fn parse(text: &str) -> Option<Self> {
        let trimmed = text.trim();
        let without_prefix = trimmed
            .strip_prefix('v')
            .or_else(|| trimmed.strip_prefix('V'))
            .unwrap_or(trimmed);

        let core = without_prefix.split(['-', '+']).next()?;
        let mut parts = core.split('.');

        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().map_or(Ok(0), |part| part.parse()).ok()?;
        let patch = parts.next().map_or(Ok(0), |part| part.parse()).ok()?;

        if parts.next().is_some() {
            return None;
        }

        Some(Self { major, minor, patch })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// An unreadable version on either side means "no update", never "update".
/// Announcing a release that cannot be compared would push people towards a
/// download that may well be older than what they are running.
pub fn newer(remote: &str, current: &str) -> Option<Version> {
    let remote = Version::parse(remote)?;
    let current = Version::parse(current)?;

    (remote > current).then_some(remote)
}

pub fn available(manifest: &Manifest, current: &str) -> Option<Version> {
    newer(&manifest.launcher.version, current)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(major: u32, minor: u32, patch: u32) -> Version {
        Version { major, minor, patch }
    }

    #[test]
    fn a_plain_triple_parses() {
        assert_eq!(Version::parse("1.2.3"), Some(version(1, 2, 3)));
    }

    #[test]
    fn a_leading_v_is_optional() {
        assert_eq!(Version::parse("v1.2.3"), Version::parse("1.2.3"));
        assert_eq!(Version::parse("V1.2.3"), Version::parse("1.2.3"));
    }

    #[test]
    fn missing_components_count_as_zero() {
        assert_eq!(Version::parse("2"), Some(version(2, 0, 0)));
        assert_eq!(Version::parse("2.1"), Some(version(2, 1, 0)));
    }

    #[test]
    fn a_suffix_is_ignored_for_ordering() {
        assert_eq!(Version::parse("1.2.3-beta.1"), Some(version(1, 2, 3)));
        assert_eq!(Version::parse("1.2.3+build9"), Some(version(1, 2, 3)));
    }

    #[test]
    fn nonsense_is_refused() {
        assert!(Version::parse("").is_none());
        assert!(Version::parse("next").is_none());
        assert!(Version::parse("1.2.").is_none());
        assert!(Version::parse("1.2.3.4").is_none());
        assert!(Version::parse("-1.0.0").is_none());
    }

    #[test]
    fn ordering_walks_the_components_left_to_right() {
        assert!(version(1, 0, 0) > version(0, 9, 9));
        assert!(version(0, 2, 0) > version(0, 1, 9));
        assert!(version(0, 1, 2) > version(0, 1, 1));
    }

    #[test]
    fn a_double_digit_component_is_not_compared_as_text() {
        assert!(version(0, 10, 0) > version(0, 9, 0));
        assert!(newer("0.10.0", "0.9.0").is_some());
    }

    #[test]
    fn only_a_strictly_higher_version_counts() {
        assert_eq!(newer("0.2.0", "0.1.0"), Some(version(0, 2, 0)));
        assert!(newer("0.1.0", "0.1.0").is_none());
        assert!(newer("0.0.9", "0.1.0").is_none());
    }

    #[test]
    fn the_padding_does_not_invent_a_difference() {
        assert!(newer("1", "1.0.0").is_none());
        assert!(newer("1.0", "1.0.0").is_none());
    }

    #[test]
    fn an_unreadable_version_never_announces_an_update() {
        assert!(newer("nightly", "0.1.0").is_none());
        assert!(newer("0.2.0", "nightly").is_none());
    }

    #[test]
    fn the_manifest_version_is_what_gets_compared() {
        let manifest = Manifest::parse(
            r#"{
                "schemaVersion": 1,
                "launcher": {
                    "version": "9.9.9",
                    "url": "https://example.invalid/x.exe",
                    "size": 1,
                    "sha256": "0000000000000000000000000000000000000000000000000000000000000000"
                },
                "minecraft": { "version": "1.20.4", "fabricLoader": "0.19.3" },
                "server": { "address": "rendog.kr" }
            }"#,
        )
        .unwrap();

        assert_eq!(available(&manifest, "0.1.0"), Some(version(9, 9, 9)));
        assert!(available(&manifest, "10.0.0").is_none());
    }

    #[test]
    fn the_shipped_version_is_readable() {
        assert!(
            Version::parse(env!("CARGO_PKG_VERSION")).is_some(),
            "Cargo.toml version must stay comparable"
        );
    }
}
