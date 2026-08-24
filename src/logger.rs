use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use log::{LevelFilter, Log, Metadata, Record};

const FILE_NAME: &str = "launcher.log";
const MAX_BYTES: u64 = 1024 * 1024;
const KEEP: usize = 3;

static LOGGER: OnceLock<FileLogger> = OnceLock::new();

pub fn init(logs_dir: &Path) -> anyhow::Result<()> {
    let sink = Sink::open(logs_dir)?;
    let logger = LOGGER.get_or_init(FileLogger::default);

    *logger
        .sink
        .lock()
        .map_err(|_| anyhow::anyhow!("로그 잠금이 손상됐어요."))? = Some(sink);

    let _ = log::set_logger(logger);
    log::set_max_level(LevelFilter::Info);

    Ok(())
}

#[derive(Default)]
struct FileLogger {
    sink: Mutex<Option<Sink>>,
}

impl Log for FileLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        let line = format!(
            "{} {:<5} {} - {}\n",
            timestamp(now_unix()),
            record.level(),
            record.target(),
            record.args()
        );

        if let Ok(mut guard) = self.sink.lock() {
            if let Some(sink) = guard.as_mut() {
                sink.write_line(&line);
            }
        }
    }

    fn flush(&self) {
        if let Ok(mut guard) = self.sink.lock() {
            if let Some(sink) = guard.as_mut() {
                sink.flush();
            }
        }
    }
}

struct Sink {
    dir: PathBuf,
    file: Option<File>,
    written: u64,
}

impl Sink {
    fn open(dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("{} 폴더를 만들지 못했어요.", dir.display()))?;

        let mut sink = Self {
            dir: dir.to_path_buf(),
            file: None,
            written: 0,
        };
        sink.reopen()
            .with_context(|| format!("{} 을(를) 열지 못했어요.", sink.path().display()))?;

        Ok(sink)
    }

    fn path(&self) -> PathBuf {
        self.dir.join(FILE_NAME)
    }

    fn reopen(&mut self) -> std::io::Result<()> {
        let file = OpenOptions::new().create(true).append(true).open(self.path())?;

        self.written = file.metadata().map(|meta| meta.len()).unwrap_or(0);
        self.file = Some(file);

        Ok(())
    }

    fn write_line(&mut self, line: &str) {
        if self.written >= MAX_BYTES {
            let _ = self.roll();
        }

        if let Some(file) = self.file.as_mut() {
            if file.write_all(line.as_bytes()).is_ok() {
                self.written += line.len() as u64;
            }
        }
    }

    fn flush(&mut self) {
        if let Some(file) = self.file.as_mut() {
            let _ = file.flush();
        }
    }

    fn roll(&mut self) -> std::io::Result<()> {
        // Windows refuses to rename a file that still has an open handle, so the
        // current one has to be dropped before anything moves.
        self.file = None;

        let _ = std::fs::remove_file(rotated_path(&self.dir, KEEP));

        for index in (1..KEEP).rev() {
            let from = rotated_path(&self.dir, index);
            if from.exists() {
                let _ = std::fs::rename(&from, rotated_path(&self.dir, index + 1));
            }
        }

        let _ = std::fs::rename(self.path(), rotated_path(&self.dir, 1));

        self.reopen()
    }
}

fn rotated_path(dir: &Path, index: usize) -> PathBuf {
    dir.join(format!("launcher.{index}.log"))
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// `YYYY-MM-DD HH:MM:SSZ` in UTC.
///
/// UTC keeps this dependency-free; local time on Windows needs the `windows`
/// crate, which only arrives with the DPAPI work in L4-7.
fn timestamp(unix_seconds: i64) -> String {
    let days = unix_seconds.div_euclid(86_400);
    let seconds = unix_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);

    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}Z",
        seconds / 3600,
        (seconds % 3600) / 60,
        seconds % 60
    )
}

/// Howard Hinnant's `civil_from_days`: days since the Unix epoch to a calendar
/// date, valid across the proleptic Gregorian calendar.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    } as u32;

    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "erkuia-log-{tag}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
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

    #[test]
    fn formats_known_instants() {
        assert_eq!(timestamp(0), "1970-01-01 00:00:00Z");
        assert_eq!(timestamp(1_700_000_000), "2023-11-14 22:13:20Z");
        assert_eq!(timestamp(951_782_400), "2000-02-29 00:00:00Z");
        assert_eq!(timestamp(1_709_164_800), "2024-02-29 00:00:00Z");
        assert_eq!(timestamp(-1), "1969-12-31 23:59:59Z");
    }

    #[test]
    fn writes_lines_to_the_log_file() {
        let dir = TempDir::new("write");
        let mut sink = Sink::open(&dir.0).unwrap();

        sink.write_line("hello\n");
        sink.flush();

        let text = std::fs::read_to_string(dir.0.join(FILE_NAME)).unwrap();
        assert_eq!(text, "hello\n");
    }

    #[test]
    fn reopening_appends_instead_of_truncating() {
        let dir = TempDir::new("append");

        let mut first = Sink::open(&dir.0).unwrap();
        first.write_line("one\n");
        first.flush();
        drop(first);

        let mut second = Sink::open(&dir.0).unwrap();
        second.write_line("two\n");
        second.flush();

        let text = std::fs::read_to_string(dir.0.join(FILE_NAME)).unwrap();
        assert_eq!(text, "one\ntwo\n");
    }

    #[test]
    fn rolling_shifts_files_and_keeps_a_bounded_number() {
        let dir = TempDir::new("roll");
        let mut sink = Sink::open(&dir.0).unwrap();

        for generation in 0..6 {
            sink.write_line(&format!("generation-{generation}\n"));
            sink.flush();
            sink.roll().unwrap();
        }

        assert!(dir.0.join(FILE_NAME).exists());
        for index in 1..=KEEP {
            assert!(
                rotated_path(&dir.0, index).exists(),
                "launcher.{index}.log is missing"
            );
        }
        assert!(
            !rotated_path(&dir.0, KEEP + 1).exists(),
            "rotation kept more files than KEEP"
        );

        // The most recent roll must land in slot 1, the oldest kept in slot KEEP.
        assert_eq!(
            std::fs::read_to_string(rotated_path(&dir.0, 1)).unwrap(),
            "generation-5\n"
        );
        assert_eq!(
            std::fs::read_to_string(rotated_path(&dir.0, KEEP)).unwrap(),
            "generation-3\n"
        );
    }

    #[test]
    fn rolling_resets_the_byte_counter() {
        let dir = TempDir::new("counter");
        let mut sink = Sink::open(&dir.0).unwrap();

        sink.write_line("some bytes\n");
        assert!(sink.written > 0);

        sink.roll().unwrap();
        assert_eq!(sink.written, 0);
    }
}
