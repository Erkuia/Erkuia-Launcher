use crate::{manifest::Manifest, powershell};

/// Size the manifest expects to place on disk. Components without a declared
/// size are still pending, so they contribute nothing yet.
pub fn required_bytes(manifest: &Manifest) -> u64 {
    manifest
        .install_plan
        .components
        .iter()
        .filter_map(|component| component.size)
        .sum()
}

pub fn capacity_text(required: u64, free: Option<u64>) -> String {
    match free {
        Some(free) => format!(
            "필요 용량 {} · 사용 가능 {}",
            format_bytes(required),
            format_bytes(free)
        ),
        None => format!("필요 용량 {}", format_bytes(required)),
    }
}

pub fn free_bytes_for_path(path: &str) -> Option<u64> {
    let drive = drive_letter(path)?;
    let script = format!("(Get-PSDrive -Name '{}' -ErrorAction Stop).Free", drive);
    let output = powershell::output(&["-NoProfile", "-NonInteractive", "-Command", &script]).ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

fn drive_letter(path: &str) -> Option<char> {
    let mut characters = path.chars();
    let letter = characters.next()?;

    if characters.next() != Some(':') || !letter.is_ascii_alphabetic() {
        return None;
    }

    Some(letter.to_ascii_uppercase())
}

pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    const TB: f64 = GB * 1024.0;

    let value = bytes as f64;

    if value >= TB {
        format!("{:.1}TB", value / TB)
    } else if value >= GB {
        format!("{:.1}GB", value / GB)
    } else if value >= MB {
        format!("{:.1}MB", value / MB)
    } else if value >= KB {
        format!("{:.0}KB", value / KB)
    } else {
        format!("{}B", bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_common_sizes() {
        assert_eq!(format_bytes(512), "512B");
        assert_eq!(format_bytes(8_709_016), "8.3MB");
        assert_eq!(format_bytes(137_438_953_472), "128.0GB");
    }

    #[test]
    fn reads_drive_letter_only_from_windows_paths() {
        assert_eq!(drive_letter(r"c:\Program Files\Erkuia"), Some('C'));
        assert_eq!(drive_letter(r"\\server\share"), None);
        assert_eq!(drive_letter(""), None);
    }

    #[test]
    fn hides_available_space_when_unknown() {
        assert_eq!(capacity_text(8_709_016, None), "필요 용량 8.3MB");
    }
}
