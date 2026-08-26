use std::path::Path;

use anyhow::Context;

pub const FILE_NAME: &str = "ErkuiaSupport.jar";

/// The id declared in the jar's `fabric.mod.json`.
///
/// The real identity of the bundled mod. The file name is only a convention and
/// drifts — it has already been renamed once, and a stale copy under any name
/// must still be recognised as ours rather than offered to the player as a mod
/// they can delete or switch off.
pub const MOD_ID: &str = "erkuialauncher";

/// What the jar used to be called.
///
/// The mod id inside has not changed, so a copy left over under the old name
/// would load alongside the new one and Fabric would refuse to start on the
/// duplicate. Renaming the file is only safe if the old one is swept up.
const LEGACY_FILE_NAMES: [&str; 1] = ["ErkuiaLauncherMod.jar"];

pub const BYTES: &[u8] = include_bytes!(env!("ERKUIA_MOD_JAR"));

pub fn is_bundled(file_name: &str) -> bool {
    file_name.eq_ignore_ascii_case(FILE_NAME)
}

fn matches_disk(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| meta.len() == BYTES.len() as u64)
        && std::fs::read(path).is_ok_and(|current| current == BYTES)
}

/// The jar travels inside the executable, so the copy in `mods/` is a cache
/// rather than a download. Rewriting it whenever it differs means a user who
/// deletes or edits it simply gets it back on the next launch.
pub fn ensure(mods_dir: &Path, disabled_dir: &Path) -> anyhow::Result<bool> {
    sweep_legacy(mods_dir);
    sweep_legacy(disabled_dir);

    let parked = disabled_dir.join(FILE_NAME);
    if parked.is_file() {
        std::fs::remove_file(&parked)
            .with_context(|| format!("{} 을(를) 정리하지 못했어요.", parked.display()))?;
    }

    let path = mods_dir.join(FILE_NAME);
    if matches_disk(&path) {
        return Ok(false);
    }

    std::fs::create_dir_all(mods_dir)
        .with_context(|| format!("{} 폴더를 만들지 못했어요.", mods_dir.display()))?;

    let temp = path.with_extension("jar.part");
    std::fs::write(&temp, BYTES)
        .with_context(|| format!("{} 을(를) 쓰지 못했어요.", temp.display()))?;
    std::fs::rename(&temp, &path)
        .with_context(|| format!("{} 을(를) 배치하지 못했어요.", path.display()))?;

    Ok(true)
}

/// Removes copies under names this mod used to ship as.
///
/// Best effort on purpose: a jar held open by something else must not stop the
/// game from starting. The duplicate would be reported by Fabric with a far
/// clearer message than anything raised from here.
fn sweep_legacy(dir: &Path) {
    for legacy in LEGACY_FILE_NAMES {
        let path = dir.join(legacy);

        if !path.is_file() {
            continue;
        }

        match std::fs::remove_file(&path) {
            Ok(()) => log::info!("옛 이름의 내장 모드를 정리했습니다: {}", path.display()),
            Err(error) => log::warn!("{} 을(를) 지우지 못했습니다: {error}", path.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(tag: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "erkuia-bundled-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("mods")).unwrap();
        std::fs::create_dir_all(root.join("mods-disabled")).unwrap();

        root
    }

    #[test]
    fn the_embedded_jar_is_a_real_zip() {
        assert!(BYTES.len() > 512, "jar is suspiciously small");
        assert_eq!(&BYTES[..2], b"PK");
    }

    #[test]
    fn the_embedded_jar_declares_the_mod() {
        let reader = std::io::Cursor::new(BYTES);
        let mut archive = zip::ZipArchive::new(reader).expect("jar opens as a zip");
        let mut entry = archive
            .by_name("fabric.mod.json")
            .expect("jar carries fabric.mod.json");

        let mut text = String::new();
        std::io::Read::read_to_string(&mut entry, &mut text).unwrap();

        assert!(text.contains("\"erkuialauncher\""), "{text}");
    }

    #[test]
    fn a_missing_jar_is_written_out() {
        let root = fixture("write");

        let written = ensure(&root.join("mods"), &root.join("mods-disabled")).unwrap();

        assert!(written);
        assert_eq!(std::fs::read(root.join("mods").join(FILE_NAME)).unwrap(), BYTES);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_identical_jar_is_left_alone() {
        let root = fixture("skip");
        ensure(&root.join("mods"), &root.join("mods-disabled")).unwrap();

        assert!(!ensure(&root.join("mods"), &root.join("mods-disabled")).unwrap());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_tampered_jar_is_replaced() {
        let root = fixture("replace");
        let path = root.join("mods").join(FILE_NAME);
        std::fs::write(&path, b"not a jar").unwrap();

        let written = ensure(&root.join("mods"), &root.join("mods-disabled")).unwrap();

        assert!(written);
        assert_eq!(std::fs::read(&path).unwrap(), BYTES);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_parked_copy_is_removed_so_it_cannot_stay_disabled() {
        let root = fixture("parked");
        let parked = root.join("mods-disabled").join(FILE_NAME);
        std::fs::write(&parked, BYTES).unwrap();

        ensure(&root.join("mods"), &root.join("mods-disabled")).unwrap();

        assert!(!parked.exists());
        assert!(root.join("mods").join(FILE_NAME).is_file());

        std::fs::remove_dir_all(&root).ok();
    }

    /// Renaming the jar without this leaves the old copy in place, and Fabric
    /// refuses to start when two files declare the same mod id.
    #[test]
    fn a_copy_under_the_old_name_is_swept_up() {
        let root = fixture("legacy");
        let legacy = root.join("mods").join("ErkuiaLauncherMod.jar");
        let parked = root.join("mods-disabled").join("ErkuiaLauncherMod.jar");

        std::fs::write(&legacy, b"an older build").unwrap();
        std::fs::write(&parked, b"an older build").unwrap();

        ensure(&root.join("mods"), &root.join("mods-disabled")).unwrap();

        assert!(!legacy.exists(), "the old jar is still in mods/");
        assert!(!parked.exists(), "the old jar is still parked");
        assert!(root.join("mods").join(FILE_NAME).is_file());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_name_check_ignores_case() {
        assert!(is_bundled(FILE_NAME));
        assert!(is_bundled("erkuiasupport.jar"));
        assert!(!is_bundled("fabric-api-0.119.4+1.21.4.jar"));
    }
}
