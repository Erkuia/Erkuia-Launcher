use serde::Deserialize;

const MANIFEST_JSON: &str = include_str!("../manifest.json");

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub product: Product,
    pub installer: Installer,
    #[serde(rename = "installPlan")]
    pub install_plan: InstallPlan,
}

#[derive(Debug, Deserialize)]
pub struct Product {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct Installer {
    pub name: String,
    #[serde(rename = "defaultRunAfterInstall")]
    pub default_run_after_install: bool,
    #[serde(rename = "defaultCreateDesktopShortcut")]
    pub default_create_desktop_shortcut: bool,
}

#[derive(Debug, Deserialize)]
pub struct InstallPlan {
    #[serde(rename = "defaultInstallDir")]
    pub default_install_dir: String,
}

pub fn load_manifest() -> anyhow::Result<Manifest> {
    Ok(serde_json::from_str(MANIFEST_JSON)?)
}
