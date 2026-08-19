use std::path::PathBuf;

pub fn pick_install_directory(current_path: &str) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new().set_title("Rendog Launcher 설치 경로 선택");

    if !current_path.trim().is_empty() {
        dialog = dialog.set_directory(current_path);
    }

    dialog.pick_folder()
}
