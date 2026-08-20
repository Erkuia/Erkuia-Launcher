#![allow(dead_code)]

use std::path::{Path, PathBuf};

use anyhow::bail;

pub const RELEASE_FILE: &str = "release";

#[cfg(windows)]
pub const JAVA_EXE: &str = "java.exe";
#[cfg(windows)]
pub const JAVAW_EXE: &str = "javaw.exe";

#[cfg(not(windows))]
pub const JAVA_EXE: &str = "java";
#[cfg(not(windows))]
pub const JAVAW_EXE: &str = "java";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaInstall {
    pub home: PathBuf,
    pub major: u32,
}

impl JavaInstall {
    pub fn java(&self) -> PathBuf {
        self.home.join("bin").join(JAVA_EXE)
    }

    pub fn javaw(&self) -> PathBuf {
        let javaw = self.home.join("bin").join(JAVAW_EXE);

        if javaw.is_file() {
            javaw
        } else {
            self.java()
        }
    }
}

/// Accepts both the `release` file form (`21.0.3`) and the banner printed by
/// `java -version`. Legacy versions report `1.8.0_402`, where the major number
/// is the second component.
pub fn parse_major(value: &str) -> Option<u32> {
    let trimmed = value.trim().trim_matches('"');
    let mut parts = trimmed.split(['.', '_', '-', '+']);
    let first: u32 = parts.next()?.parse().ok()?;

    if first == 1 {
        return parts.next()?.parse().ok();
    }

    Some(first)
}

pub fn parse_release_file(text: &str) -> Option<u32> {
    text.lines()
        .find_map(|line| line.strip_prefix("JAVA_VERSION="))
        .and_then(parse_major)
}

pub fn parse_version_banner(text: &str) -> Option<u32> {
    text.lines()
        .find(|line| line.contains(" version \""))
        .and_then(|line| line.split('"').nth(1))
        .and_then(parse_major)
}

fn spawn_probe(java: &Path) -> Option<String> {
    let mut command = std::process::Command::new(java);
    command.arg("-version");

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let output = command.output().ok()?;
    let mut text = String::from_utf8_lossy(&output.stderr).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stdout));

    Some(text)
}

pub fn probe(home: &Path) -> Option<JavaInstall> {
    if !home.join("bin").join(JAVA_EXE).is_file() {
        return None;
    }

    let from_release = std::fs::read_to_string(home.join(RELEASE_FILE))
        .ok()
        .and_then(|text| parse_release_file(&text));

    let major = match from_release {
        Some(major) => major,
        None => spawn_probe(&home.join("bin").join(JAVA_EXE))
            .as_deref()
            .and_then(parse_version_banner)?,
    };

    Some(JavaInstall {
        home: home.to_path_buf(),
        major,
    })
}

fn bundled_homes(runtime_dir: &Path) -> Vec<PathBuf> {
    let mut homes = vec![runtime_dir.to_path_buf()];

    if let Ok(entries) = std::fs::read_dir(runtime_dir) {
        let mut nested: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();

        nested.sort_unstable();
        homes.extend(nested);
    }

    homes
}

fn path_homes() -> Vec<PathBuf> {
    let Some(path) = std::env::var_os("PATH") else {
        return Vec::new();
    };

    std::env::split_paths(&path)
        .filter(|dir| dir.join(JAVA_EXE).is_file())
        .filter_map(|dir| dir.parent().map(Path::to_path_buf))
        .collect()
}

/// Bundled runtime first, then `JAVA_HOME`, then whatever is on `PATH`.
pub fn candidates(runtime_dir: &Path) -> Vec<PathBuf> {
    let mut homes = bundled_homes(runtime_dir);

    if let Some(java_home) = std::env::var_os("JAVA_HOME") {
        homes.push(PathBuf::from(java_home));
    }

    homes.extend(path_homes());

    let mut seen = std::collections::HashSet::new();
    homes.retain(|home| seen.insert(home.clone()));

    homes
}

pub fn detect(runtime_dir: &Path, required_major: u32) -> Option<JavaInstall> {
    for home in candidates(runtime_dir) {
        let Some(install) = probe(&home) else {
            continue;
        };

        if install.major >= required_major {
            log::info!(
                "Java {} 사용: {}",
                install.major,
                install.home.display()
            );

            return Some(install);
        }

        log::info!(
            "Java {} 은(는) 요구 버전 {required_major} 보다 낮아 건너뜁니다: {}",
            install.major,
            install.home.display()
        );
    }

    None
}

pub fn require(runtime_dir: &Path, required_major: u32) -> anyhow::Result<JavaInstall> {
    match detect(runtime_dir, required_major) {
        Some(install) => Ok(install),
        None => bail!("Java {required_major} 런타임을 찾지 못했어요."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modern_versions_report_their_first_component() {
        assert_eq!(parse_major("21"), Some(21));
        assert_eq!(parse_major("21.0.3"), Some(21));
        assert_eq!(parse_major("17.0.11+9"), Some(17));
        assert_eq!(parse_major("21.0.3-LTS"), Some(21));
    }

    #[test]
    fn legacy_versions_use_the_second_component() {
        assert_eq!(parse_major("1.8.0_402"), Some(8));
        assert_eq!(parse_major("1.7.0"), Some(7));
    }

    #[test]
    fn quotes_and_padding_are_stripped() {
        assert_eq!(parse_major("  \"21.0.3\"  "), Some(21));
    }

    #[test]
    fn nonsense_is_rejected() {
        assert_eq!(parse_major("openjdk"), None);
        assert_eq!(parse_major(""), None);
    }

    #[test]
    fn the_release_file_is_read() {
        let text = "IMPLEMENTOR=\"Eclipse Adoptium\"\nJAVA_VERSION=\"21.0.3\"\nOS_ARCH=\"x86_64\"\n";

        assert_eq!(parse_release_file(text), Some(21));
    }

    #[test]
    fn a_release_file_without_the_version_yields_nothing() {
        assert_eq!(parse_release_file("OS_ARCH=\"x86_64\"\n"), None);
    }

    #[test]
    fn the_version_banner_is_read() {
        let banner = "openjdk version \"21.0.3\" 2024-04-16\nOpenJDK Runtime Environment Temurin-21.0.3+9\n";

        assert_eq!(parse_version_banner(banner), Some(21));
    }

    #[test]
    fn a_legacy_banner_is_read() {
        let banner = "java version \"1.8.0_402\"\nJava(TM) SE Runtime Environment\n";

        assert_eq!(parse_version_banner(banner), Some(8));
    }

    #[test]
    fn an_unexpected_banner_yields_nothing() {
        assert_eq!(parse_version_banner("command not found"), None);
    }

    #[test]
    fn the_bundled_runtime_comes_first() {
        let runtime = std::env::temp_dir().join(format!("rendog-java-{}", std::process::id()));
        std::fs::create_dir_all(runtime.join("jdk-21.0.3+9")).unwrap();

        let homes = candidates(&runtime);

        assert_eq!(homes[0], runtime);
        assert_eq!(homes[1], runtime.join("jdk-21.0.3+9"));

        std::fs::remove_dir_all(&runtime).ok();
    }

    #[test]
    fn candidates_are_deduplicated() {
        let runtime = std::env::temp_dir().join(format!("rendog-java-dup-{}", std::process::id()));
        std::fs::create_dir_all(&runtime).unwrap();

        let homes = candidates(&runtime);
        let mut unique = homes.clone();
        unique.dedup();

        assert_eq!(homes.len(), unique.len());

        std::fs::remove_dir_all(&runtime).ok();
    }

    #[test]
    fn a_directory_without_a_java_binary_is_not_an_install() {
        let empty = std::env::temp_dir().join(format!("rendog-java-empty-{}", std::process::id()));
        std::fs::create_dir_all(&empty).unwrap();

        assert!(probe(&empty).is_none());

        std::fs::remove_dir_all(&empty).ok();
    }

    #[test]
    fn the_executables_sit_under_bin() {
        let install = JavaInstall {
            home: PathBuf::from("/opt/java"),
            major: 21,
        };

        assert!(install.java().ends_with(format!("bin/{JAVA_EXE}")));
    }
}
