use std::path::PathBuf;

pub fn pick_install_directory(current_path: &PathBuf) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new().set_title("Erkuia Launcher 설치 경로 선택");

    if !current_path.as_os_str().is_empty() {
        dialog = dialog.set_directory(current_path);
    }

    dialog.pick_folder()
}
