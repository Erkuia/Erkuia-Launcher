use std::path::{Path, PathBuf};

use anyhow::{bail, Context};

/// Placeholders the manifest is allowed to use in a path.
///
/// Only this fixed set is expanded. Anything else is rejected so a malformed
/// manifest cannot make the installer write somewhere unexpected.
const SUPPORTED_VARIABLES: [&str; 5] = [
    "%ProgramFiles%",
    "%ProgramData%",
    "%APPDATA%",
    "%LOCALAPPDATA%",
    "%USERPROFILE%",
];

/// Expand a manifest path such as `%APPDATA%\RendogLauncher` into a real path.
pub fn expand(path: &str) -> anyhow::Result<PathBuf> {
    for variable in SUPPORTED_VARIABLES {
        // Manifest placeholders are matched case-insensitively because Windows
        // environment variable names are not case sensitive.
        if path.len() >= variable.len() && path[..variable.len()].eq_ignore_ascii_case(variable) {
            let name = variable.trim_matches('%');
            let base = std::env::var(name)
                .with_context(|| format!("{} environment variable is missing", name))?;
            let rest = trim_path_separator(&path[variable.len()..]);

            return Ok(if rest.is_empty() {
                PathBuf::from(base)
            } else {
                Path::new(&base).join(rest)
            });
        }
    }

    if path.contains('%') {
        bail!("unsupported environment variable in path: {}", path);
    }

    Ok(PathBuf::from(path))
}

/// Same as [`expand`], but falls back to the literal string when the path uses
/// something the installer cannot resolve. Used on UI paths the user typed.
pub fn expand_or_literal(path: &str) -> PathBuf {
    expand(path).unwrap_or_else(|_| PathBuf::from(path))
}

fn trim_path_separator(path: &str) -> &str {
    path.trim_start_matches(['\\', '/'])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_plain_paths_untouched() {
        assert_eq!(
            expand(r"D:\Games\Rendog").unwrap(),
            PathBuf::from(r"D:\Games\Rendog")
        );
    }

    #[test]
    fn rejects_unknown_variables() {
        assert!(expand(r"%WINDIR%\Rendog").is_err());
    }

    #[test]
    fn expands_known_variables() {
        std::env::set_var("APPDATA", r"C:\Users\test\AppData\Roaming");
        assert_eq!(
            expand(r"%APPDATA%\RendogLauncher").unwrap(),
            PathBuf::from(r"C:\Users\test\AppData\Roaming\RendogLauncher")
        );
        assert_eq!(
            expand(r"%appdata%\RendogLauncher").unwrap(),
            PathBuf::from(r"C:\Users\test\AppData\Roaming\RendogLauncher")
        );
    }
}
