use serde::Deserialize;

const MANIFEST_JSON: &str = include_str!("../manifest.json");

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub product: Product,
    pub installer: Installer,
    #[serde(rename = "installPlan")]
    pub install_plan: InstallPlan,
    pub uninstall: Uninstall,
    #[serde(rename = "progressWeights")]
    pub progress_weights: ProgressWeights,
}

#[derive(Debug, Deserialize)]
pub struct Product {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct Installer {
    pub name: String,
    #[serde(rename = "requiresAdminOnInstall")]
    pub requires_admin_on_install: bool,
    #[serde(rename = "allowPendingRequiredComponents")]
    pub allow_pending_required_components: bool,
    #[serde(rename = "defaultRunAfterInstall")]
    pub default_run_after_install: bool,
    #[serde(rename = "defaultCreateDesktopShortcut")]
    pub default_create_desktop_shortcut: bool,
}

#[derive(Debug, Deserialize)]
pub struct InstallPlan {
    #[serde(rename = "defaultInstallDir")]
    pub default_install_dir: String,
    #[serde(rename = "dataDir")]
    pub data_dir: String,
    pub components: Vec<Component>,
}

#[derive(Debug, Deserialize)]
pub struct Component {
    pub id: String,
    pub name: String,
    pub required: bool,
    pub status: ComponentStatus,
    pub url: Option<String>,
    #[serde(rename = "fileName")]
    pub file_name: String,
    #[serde(rename = "targetRoot", default)]
    pub target_root: TargetRoot,
    #[serde(rename = "targetPath")]
    pub target_path: String,
    pub size: Option<u64>,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TargetRoot {
    #[default]
    Install,
    Data,
}

#[derive(Debug, Deserialize)]
pub struct Uninstall {
    #[serde(rename = "displayName")]
    pub display_name: String,
    #[serde(rename = "removeDesktopShortcut")]
    pub remove_desktop_shortcut: bool,
    #[serde(rename = "removeStartMenuShortcut")]
    pub remove_start_menu_shortcut: bool,
    #[serde(rename = "preserveUserDataByDefault")]
    pub preserve_user_data_by_default: bool,
    #[serde(rename = "userDataBackupName")]
    pub user_data_backup_name: String,
}

#[derive(Debug, Deserialize)]
pub struct ProgressWeights {
    pub prepare: ProgressWeight,
    pub download: ProgressWeight,
    pub verify: ProgressWeight,
    #[serde(rename = "installFiles")]
    pub install_files: ProgressWeight,
    pub shortcuts: ProgressWeight,
    #[serde(rename = "registerUninstaller")]
    pub register_uninstaller: ProgressWeight,
    pub finalize: ProgressWeight,
}

#[derive(Debug, Deserialize)]
pub struct ProgressWeight {
    pub start: f32,
    pub end: f32,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ComponentStatus {
    Ready,
    Pending,
}

pub fn load_manifest() -> anyhow::Result<Manifest> {
    Ok(serde_json::from_str(MANIFEST_JSON)?)
}
