use std::path::{Path, PathBuf};

use anyhow::{bail, Context};

use crate::{
    hash::Checksum,
    manifest::LauncherRelease,
    mc::{
        download::{self, Verify},
        version::DownloadTarget,
    },
    task::{Cancel, Reporter, Stage},
};

pub const APPLY_FLAG: &str = "--apply-update";
pub const BACKUP_SUFFIX: &str = ".old";

const PROBE_NAME: &str = ".erkuia-write-probe";

pub fn staged_name(version: &str) -> String {
    format!("Erkuia-Launcher-{version}.exe")
}

pub fn staged_path(cache_dir: &Path, version: &str) -> PathBuf {
    cache_dir.join(staged_name(version))
}

pub fn backup_path(exe: &Path) -> PathBuf {
    let mut name = exe.as_os_str().to_os_string();
    name.push(BACKUP_SUFFIX);

    PathBuf::from(name)
}

pub fn is_backup(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(BACKUP_SUFFIX))
}

fn target(release: &LauncherRelease) -> DownloadTarget {
    DownloadTarget {
        url: release.url.clone(),
        relative_path: staged_name(&release.version),
        checksum: Some(Checksum::Sha256(release.sha256.clone())),
        size: release.size,
        name: Some(format!("Erkuia Launcher v{}", release.version)),
    }
}

/// Downloads the release into the cache and returns where it landed.
///
/// Verified by checksum rather than size: this file becomes the program the
/// person runs next, so "roughly the right number of bytes" is not a standard
/// worth applying to it.
pub fn stage(
    release: &LauncherRelease,
    cache_dir: &Path,
    reporter: &Reporter,
    cancel: &Cancel,
) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(cache_dir)
        .with_context(|| format!("{} 폴더를 만들지 못했어요.", cache_dir.display()))?;

    download::run(
        &[target(release)],
        cache_dir,
        Stage::Download,
        reporter,
        cancel,
        1,
        Verify::Checksum,
    )?;

    Ok(staged_path(cache_dir, &release.version))
}

pub fn is_writable(dir: &Path) -> bool {
    let probe = dir.join(PROBE_NAME);

    match std::fs::write(&probe, b"") {
        Ok(()) => {
            std::fs::remove_file(&probe).ok();
            true
        }
        Err(_) => false,
    }
}

/// Puts `staged` in place of `exe`.
///
/// Windows refuses to overwrite a running image but allows renaming it, so the
/// live executable is moved aside first and the replacement is copied into the
/// freed path. The renamed file cannot be deleted until the process running it
/// exits, which is why cleanup happens on the next start instead of here.
pub fn install(staged: &Path, exe: &Path) -> anyhow::Result<()> {
    if !staged.is_file() {
        bail!(
            "받아둔 파일을 찾지 못했어요: {} ({})",
            staged.display(),
            match std::fs::metadata(staged) {
                Ok(meta) => format!("파일이 아니라 {} 바이트짜리 항목", meta.len()),
                Err(error) => describe(&error),
            }
        );
    }

    let backup = move_aside(exe)?;

    if let Err(error) = std::fs::copy(staged, exe) {
        // Putting the old executable back matters more than the update: without
        // it there is nothing left to launch.
        std::fs::rename(&backup, exe).ok();

        bail!(
            "{} 을(를) 교체하지 못했어요: {}",
            exe.display(),
            describe(&error)
        );
    }

    log::info!("런처를 교체했습니다: {}", exe.display());

    Ok(())
}

/// Moves the live executable out of the way and reports where it went.
///
/// The plain `.old` name is tried first, because cleanup on the next start
/// looks for exactly that. It can fail for a reason retrying will not fix: a
/// backup from an earlier update is still mapped by a process that has not
/// exited, and Windows will neither delete nor overwrite a file in that state.
/// A name nothing can be holding gets the update through anyway, and the same
/// cleanup collects it later.
fn move_aside(exe: &Path) -> anyhow::Result<PathBuf> {
    let backup = backup_path(exe);
    std::fs::remove_file(&backup).ok();

    match std::fs::rename(exe, &backup) {
        Ok(()) => return Ok(backup),
        Err(error) => log::warn!(
            "{} 로 옮기지 못해 다른 이름으로 시도합니다: {}",
            backup.display(),
            describe(&error)
        ),
    }

    let fallback = unique_backup_path(exe);

    if let Err(error) = std::fs::rename(exe, &fallback) {
        bail!(
            "{} 을(를) 옮기지 못했어요: {}",
            exe.display(),
            describe(&error)
        );
    }

    log::info!("이전 런처를 {} 로 옮겼습니다.", fallback.display());

    Ok(fallback)
}

fn unique_backup_path(exe: &Path) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);

    let mut name = exe.as_os_str().to_os_string();
    name.push(format!(".{stamp}{BACKUP_SUFFIX}"));

    PathBuf::from(name)
}

/// Keeps the OS error number in the message. "액세스가 거부되었습니다" alone does
/// not say which of the several things this function touches was refused.
fn describe(error: &std::io::Error) -> String {
    match error.raw_os_error() {
        Some(code) => format!("{error} (OS 오류 {code})"),
        None => error.to_string(),
    }
}

/// Removes executables left behind by an earlier update. Failure is ignored:
/// a stale file wastes a few megabytes, while refusing to start over one would
/// waste the whole session.
pub fn clean_backups(install_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(install_dir) else {
        return;
    };

    for path in entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_backup(path))
    {
        match std::fs::remove_file(&path) {
            Ok(()) => log::info!("이전 런처 정리: {}", path.display()),
            Err(error) => log::info!("{} 정리는 다음 기회에: {error}", path.display()),
        }
    }
}

pub fn staged_from_args() -> Option<PathBuf> {
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        if arg == APPLY_FLAG {
            return args.next().map(PathBuf::from);
        }
    }

    None
}

/// Hands the privileged half to an elevated copy of this executable and waits
/// for it, so the relaunch afterwards happens from *this* process and does not
/// inherit administrator rights.
pub fn install_elevated(staged: &Path, exe: &Path) -> anyhow::Result<()> {
    let parameters = format!("{APPLY_FLAG} \"{}\"", staged.display());
    let code = crate::shell::run_as_admin_and_wait(&exe.display().to_string(), &parameters)?;

    if code != 0 {
        // The elevated half writes the reason to the same log before it exits,
        // so the code is only a pointer to where the answer already is.
        bail!("업데이트 적용에 실패했어요. 자세한 내용은 launcher.log 를 봐주세요. (종료 코드 {code})");
    }

    Ok(())
}

pub fn apply(staged: &Path, exe: &Path) -> anyhow::Result<()> {
    let dir = exe.parent().context("설치 폴더를 찾지 못했어요.")?;

    if is_writable(dir) {
        return install(staged, exe);
    }

    log::info!("{} 에 쓸 수 없어 관리자 권한을 요청합니다.", dir.display());

    install_elevated(staged, exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "erkuia-selfupdate-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        dir
    }

    fn release(version: &str) -> LauncherRelease {
        LauncherRelease {
            version: version.to_string(),
            url: "https://example.invalid/Erkuia-Launcher.exe".to_string(),
            size: 4,
            sha256: "0".repeat(64),
        }
    }

    #[test]
    fn the_staged_name_carries_the_version_so_releases_do_not_collide() {
        assert_eq!(staged_name("0.2.0"), "Erkuia-Launcher-0.2.0.exe");
        assert_ne!(staged_name("0.2.0"), staged_name("0.3.0"));
    }

    #[test]
    fn the_backup_keeps_the_original_name_intact() {
        let backup = backup_path(Path::new(r"C:\Program Files\Erkuia\Erkuia-Launcher.exe"));

        assert!(backup.ends_with("Erkuia-Launcher.exe.old"));
        assert!(is_backup(&backup));
        assert!(!is_backup(Path::new("Erkuia-Launcher.exe")));
    }

    #[test]
    fn the_fallback_backup_is_still_collected_by_cleanup() {
        let exe = Path::new(r"C:\Program Files\Erkuia\Erkuia-Launcher.exe");
        let fallback = unique_backup_path(exe);

        // Cleanup only looks at the suffix, so a name it cannot recognise would
        // leave the file behind forever.
        assert!(is_backup(&fallback));
        assert_ne!(fallback, backup_path(exe));
        assert_eq!(fallback.parent(), exe.parent());
    }

    #[test]
    fn install_reports_a_missing_download_instead_of_touching_the_executable() {
        let dir = temp_dir("missing");
        let exe = dir.join("Erkuia-Launcher.exe");
        std::fs::write(&exe, b"original").unwrap();

        let error = install(&dir.join("nope.exe"), &exe).unwrap_err();

        assert!(error.to_string().contains("찾지 못했어요"));
        assert_eq!(std::fs::read(&exe).unwrap(), b"original");
        assert!(!backup_path(&exe).exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn install_moves_the_old_executable_aside_and_puts_the_new_one_in_place() {
        let dir = temp_dir("install");
        let exe = dir.join("Erkuia-Launcher.exe");
        let staged = dir.join("Erkuia-Launcher-0.2.0.exe");
        std::fs::write(&exe, b"old").unwrap();
        std::fs::write(&staged, b"new").unwrap();

        install(&staged, &exe).unwrap();

        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
        assert_eq!(std::fs::read(backup_path(&exe)).unwrap(), b"old");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_download_is_pinned_to_the_release_checksum() {
        let target = target(&release("0.2.0"));

        assert_eq!(target.relative_path, "Erkuia-Launcher-0.2.0.exe");
        assert_eq!(target.checksum, Some(Checksum::Sha256("0".repeat(64))));
    }

    #[test]
    fn installing_moves_the_old_file_aside_and_puts_the_new_one_in_place() {
        let dir = temp_dir("install");
        let exe = dir.join("Erkuia-Launcher.exe");
        let staged = dir.join("staged.exe");

        std::fs::write(&exe, b"old").unwrap();
        std::fs::write(&staged, b"new").unwrap();

        install(&staged, &exe).unwrap();

        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
        assert_eq!(std::fs::read(backup_path(&exe)).unwrap(), b"old");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_second_update_replaces_the_previous_backup() {
        let dir = temp_dir("twice");
        let exe = dir.join("Erkuia-Launcher.exe");
        let staged = dir.join("staged.exe");

        std::fs::write(&exe, b"v1").unwrap();
        std::fs::write(&staged, b"v2").unwrap();
        install(&staged, &exe).unwrap();

        std::fs::write(&staged, b"v3").unwrap();
        install(&staged, &exe).unwrap();

        assert_eq!(std::fs::read(&exe).unwrap(), b"v3");
        assert_eq!(std::fs::read(backup_path(&exe)).unwrap(), b"v2");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_download_leaves_the_installed_launcher_alone() {
        let dir = temp_dir("missing");
        let exe = dir.join("Erkuia-Launcher.exe");
        std::fs::write(&exe, b"old").unwrap();

        assert!(install(&dir.join("ghost.exe"), &exe).is_err());
        assert_eq!(std::fs::read(&exe).unwrap(), b"old");
        assert!(!backup_path(&exe).exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cleanup_removes_backups_and_nothing_else() {
        let dir = temp_dir("clean");
        std::fs::write(dir.join("Erkuia-Launcher.exe"), b"live").unwrap();
        std::fs::write(dir.join("Erkuia-Launcher.exe.old"), b"stale").unwrap();
        std::fs::write(dir.join("notes.txt"), b"keep").unwrap();

        clean_backups(&dir);

        assert!(dir.join("Erkuia-Launcher.exe").is_file());
        assert!(dir.join("notes.txt").is_file());
        assert!(!dir.join("Erkuia-Launcher.exe.old").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cleanup_survives_a_folder_that_is_not_there() {
        clean_backups(Path::new("/definitely/not/here"));
    }

    #[test]
    fn a_writable_folder_is_recognised_and_left_clean() {
        let dir = temp_dir("probe");

        assert!(is_writable(&dir));
        assert!(!dir.join(PROBE_NAME).exists(), "the probe must not linger");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_unreachable_folder_is_not_writable() {
        assert!(!is_writable(Path::new("/definitely/not/here")));
    }
}
