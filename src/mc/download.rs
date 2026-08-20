use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

use anyhow::{bail, Context};

use crate::{
    mc::version::DownloadTarget,
    task::{Cancel, Reporter, Stage},
};

pub const DEFAULT_CONCURRENCY: usize = 8;
const REPORT_INTERVAL: Duration = Duration::from_millis(120);
const PART_SUFFIX: &str = ".part";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub downloaded: usize,
    pub skipped: usize,
    pub bytes: u64,
}

pub fn is_present(root: &Path, target: &DownloadTarget) -> bool {
    let path = root.join(&target.relative_path);

    let Ok(metadata) = path.metadata() else {
        return false;
    };

    if !metadata.is_file() {
        return false;
    }

    target.size == 0 || metadata.len() == target.size
}

pub fn total_bytes(targets: &[DownloadTarget]) -> u64 {
    targets.iter().map(|target| target.size).sum()
}

fn part_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(PART_SUFFIX);

    PathBuf::from(name)
}

fn store(root: &Path, target: &DownloadTarget) -> anyhow::Result<u64> {
    let path = root.join(&target.relative_path);
    let parent = path
        .parent()
        .with_context(|| format!("{} 의 상위 폴더가 없어요.", path.display()))?;

    std::fs::create_dir_all(parent)
        .with_context(|| format!("{} 폴더를 만들지 못했어요.", parent.display()))?;

    let mut response = crate::http::send(crate::http::client()?.get(&target.url))
        .with_context(|| format!("{} 을(를) 내려받지 못했어요.", target.relative_path))?;

    let temp = part_path(&path);
    let mut file =
        std::fs::File::create(&temp).with_context(|| format!("{} 을(를) 만들지 못했어요.", temp.display()))?;

    let written = std::io::copy(&mut response, &mut file)
        .with_context(|| format!("{} 을(를) 저장하지 못했어요.", target.relative_path))?;

    file.flush().ok();
    drop(file);

    if target.size != 0 && written != target.size {
        std::fs::remove_file(&temp).ok();
        bail!(
            "{} 크기가 맞지 않아요: 예상 {} · 실제 {}",
            target.relative_path,
            target.size,
            written
        );
    }

    std::fs::rename(&temp, &path)
        .with_context(|| format!("{} 을(를) 배치하지 못했어요.", target.relative_path))?;

    Ok(written)
}

pub fn run(
    targets: &[DownloadTarget],
    root: &Path,
    stage: Stage,
    reporter: &Reporter,
    cancel: &Cancel,
    concurrency: usize,
) -> anyhow::Result<Stats> {
    if targets.is_empty() {
        return Ok(Stats::default());
    }

    let total = total_bytes(targets).max(1);
    let cursor = AtomicUsize::new(0);
    let done_bytes = AtomicU64::new(0);
    let done_files = AtomicUsize::new(0);
    let downloaded = AtomicUsize::new(0);
    let skipped = AtomicUsize::new(0);
    let failure: Mutex<Option<anyhow::Error>> = Mutex::new(None);
    let last_report = Mutex::new(
        Instant::now()
            .checked_sub(REPORT_INTERVAL)
            .unwrap_or_else(Instant::now),
    );

    let workers = concurrency.clamp(1, targets.len());

    let cursor = &cursor;
    let done_bytes = &done_bytes;
    let done_files = &done_files;
    let downloaded = &downloaded;
    let skipped = &skipped;
    let failure = &failure;
    let last_report = &last_report;

    std::thread::scope(|scope| {
        for _ in 0..workers {
            let reporter = reporter.clone();

            scope.spawn(move || loop {
                if cancel.is_cancelled() {
                    break;
                }
                if failure.lock().is_ok_and(|held| held.is_some()) {
                    break;
                }

                let index = cursor.fetch_add(1, Ordering::SeqCst);
                if index >= targets.len() {
                    break;
                }

                let target = &targets[index];

                if is_present(root, target) {
                    skipped.fetch_add(1, Ordering::Relaxed);
                    done_bytes.fetch_add(target.size, Ordering::Relaxed);
                } else {
                    match store(root, target) {
                        Ok(written) => {
                            downloaded.fetch_add(1, Ordering::Relaxed);
                            done_bytes.fetch_add(written.max(target.size), Ordering::Relaxed);
                        }
                        Err(error) => {
                            if let Ok(mut held) = failure.lock() {
                                if held.is_none() {
                                    *held = Some(error);
                                }
                            }
                            break;
                        }
                    }
                }

                let finished = done_files.fetch_add(1, Ordering::Relaxed) + 1;
                let should_report = match last_report.lock() {
                    Ok(mut last) if last.elapsed() >= REPORT_INTERVAL => {
                        *last = Instant::now();
                        true
                    }
                    _ => finished == targets.len(),
                };

                if should_report {
                    let fraction = done_bytes.load(Ordering::Relaxed) as f32 / total as f32;

                    reporter.progress(
                        stage,
                        fraction.clamp(0.0, 1.0),
                        format!("파일 준비 중... ({finished}/{})", targets.len()),
                    );
                }
            });
        }
    });

    if let Some(error) = failure.lock().ok().and_then(|mut held| held.take()) {
        return Err(error);
    }

    if cancel.is_cancelled() {
        bail!("작업을 취소했어요.");
    }

    let stats = Stats {
        downloaded: downloaded.load(Ordering::Relaxed),
        skipped: skipped.load(Ordering::Relaxed),
        bytes: done_bytes.load(Ordering::Relaxed),
    };

    log::info!(
        "{:?} 완료 · 내려받음 {} · 건너뜀 {} · {} 바이트",
        stage,
        stats.downloaded,
        stats.skipped,
        stats.bytes
    );

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "rendog-dl-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    fn target(path: &str, size: u64) -> DownloadTarget {
        DownloadTarget {
            url: format!("https://example.invalid/{path}"),
            relative_path: path.to_string(),
            sha1: None,
            size,
            name: None,
        }
    }

    fn write(root: &Path, path: &str, bytes: &[u8]) {
        let full = root.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, bytes).unwrap();
    }

    #[test]
    fn a_missing_file_is_not_present() {
        let dir = TempDir::new("missing");

        assert!(!is_present(&dir.0, &target("libraries/a.jar", 4)));
    }

    #[test]
    fn a_file_with_the_expected_size_is_present() {
        let dir = TempDir::new("match");
        write(&dir.0, "libraries/a.jar", b"abcd");

        assert!(is_present(&dir.0, &target("libraries/a.jar", 4)));
    }

    #[test]
    fn a_truncated_file_is_not_present() {
        let dir = TempDir::new("short");
        write(&dir.0, "libraries/a.jar", b"ab");

        assert!(!is_present(&dir.0, &target("libraries/a.jar", 4)));
    }

    #[test]
    fn an_unknown_size_accepts_whatever_is_on_disk() {
        let dir = TempDir::new("unknown");
        write(&dir.0, "libraries/a.jar", b"anything");

        assert!(is_present(&dir.0, &target("libraries/a.jar", 0)));
    }

    #[test]
    fn a_directory_does_not_count_as_the_file() {
        let dir = TempDir::new("dir");
        std::fs::create_dir_all(dir.0.join("libraries/a.jar")).unwrap();

        assert!(!is_present(&dir.0, &target("libraries/a.jar", 0)));
    }

    #[test]
    fn the_partial_file_sits_next_to_the_target() {
        assert_eq!(
            part_path(Path::new("/root/libraries/a.jar")),
            PathBuf::from("/root/libraries/a.jar.part")
        );
    }

    #[test]
    fn totals_add_up_across_targets() {
        let targets = vec![target("a", 10), target("b", 20), target("c", 0)];

        assert_eq!(total_bytes(&targets), 30);
    }

    #[test]
    fn an_empty_plan_does_nothing() {
        assert_eq!(total_bytes(&[]), 0);
    }
}
