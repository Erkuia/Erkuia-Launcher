use std::{io::Read, path::Path};

use anyhow::Context;
use sha1::Digest as Sha1Digest;
use sha2::Digest as Sha2Digest;

const BUFFER: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Checksum {
    Sha1(String),
    Sha256(String),
}

impl Checksum {
    pub fn algorithm(&self) -> &'static str {
        match self {
            Self::Sha1(_) => "SHA-1",
            Self::Sha256(_) => "SHA-256",
        }
    }

    pub fn expected(&self) -> &str {
        match self {
            Self::Sha1(value) | Self::Sha256(value) => value,
        }
    }

    pub fn of_file(&self, path: &Path) -> anyhow::Result<String> {
        match self {
            Self::Sha1(_) => sha1_file(path),
            Self::Sha256(_) => sha256_file(path),
        }
    }

    pub fn matches_file(&self, path: &Path) -> bool {
        self.of_file(path)
            .is_ok_and(|actual| actual.eq_ignore_ascii_case(self.expected()))
    }
}

fn read_into<F: FnMut(&[u8])>(path: &Path, mut consume: F) -> anyhow::Result<()> {
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("{} 을(를) 열지 못했어요.", path.display()))?;
    let mut buffer = vec![0_u8; BUFFER];

    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("{} 을(를) 읽지 못했어요.", path.display()))?;

        if read == 0 {
            break;
        }

        consume(&buffer[..read]);
    }

    Ok(())
}

pub fn sha1_file(path: &Path) -> anyhow::Result<String> {
    let mut hasher = sha1::Sha1::new();
    read_into(path, |chunk| hasher.update(chunk))?;

    Ok(hex(&hasher.finalize()))
}

pub fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let mut hasher = sha2::Sha256::new();
    read_into(path, |chunk| Sha2Digest::update(&mut hasher, chunk))?;

    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str, contents: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rendog-hash-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let path = dir.join("file.bin");
        std::fs::write(&path, contents).unwrap();

        path
    }

    #[test]
    fn sha1_matches_the_known_digest_of_abc() {
        let path = temp("sha1", b"abc");

        assert_eq!(
            sha1_file(&path).unwrap(),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
    }

    #[test]
    fn sha256_matches_the_known_digest_of_abc() {
        let path = temp("sha256", b"abc");

        assert_eq!(
            sha256_file(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn an_empty_file_hashes_to_the_empty_digest() {
        let path = temp("empty", b"");

        assert_eq!(
            sha1_file(&path).unwrap(),
            "da39a3ee5e6b4b0d3255bfef95601890afd80709"
        );
        assert_eq!(
            sha256_file(&path).unwrap(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn hashing_survives_inputs_larger_than_the_buffer() {
        let path = temp("large", &vec![7_u8; BUFFER * 3 + 17]);

        assert_eq!(sha1_file(&path).unwrap().len(), 40);
        assert_eq!(sha256_file(&path).unwrap().len(), 64);
    }

    #[test]
    fn a_matching_checksum_passes() {
        let path = temp("ok", b"abc");
        let checksum = Checksum::Sha1("a9993e364706816aba3e25717850c26c9cd0d89d".to_string());

        assert!(checksum.matches_file(&path));
    }

    #[test]
    fn case_does_not_matter() {
        let path = temp("case", b"abc");
        let checksum = Checksum::Sha1("A9993E364706816ABA3E25717850C26C9CD0D89D".to_string());

        assert!(checksum.matches_file(&path));
    }

    #[test]
    fn a_wrong_checksum_fails() {
        let path = temp("bad", b"abc");
        let checksum = Checksum::Sha1("0".repeat(40));

        assert!(!checksum.matches_file(&path));
    }

    #[test]
    fn a_missing_file_fails_instead_of_panicking() {
        let checksum = Checksum::Sha256("0".repeat(64));

        assert!(!checksum.matches_file(Path::new("/definitely/not/here.bin")));
    }

    #[test]
    fn the_algorithm_is_reported_for_messages() {
        assert_eq!(Checksum::Sha1(String::new()).algorithm(), "SHA-1");
        assert_eq!(Checksum::Sha256(String::new()).algorithm(), "SHA-256");
    }
}
