use std::process::Child;

use anyhow::{bail, Context};

use crate::{
    auth::{session::Session, store::SecretStore},
    bundled,
    config::Config,
    launch::{self, LaunchInputs},
    manifest::{self, Manifest},
    mc::{assets, download, fabric, version},
    modconfig::{self, ModConfig},
    mods, paths::Paths,
    runtime,
    task::{Cancel, Reporter, Stage},
};

pub const REQUIRED_JAVA: u32 = 21;
pub const GAME_LOG_FILE: &str = "minecraft.log";

pub struct LaunchOutcome {
    pub child: Child,
    pub managed_mods: Vec<String>,
}

pub fn run(
    paths: &Paths,
    settings: &Config,
    secrets: &SecretStore,
    reporter: &Reporter,
    cancel: &Cancel,
) -> anyhow::Result<LaunchOutcome> {
    reporter.progress(Stage::Prepare, 0.0, "준비 중...");
    paths.bootstrap()?;
    mods::ensure_dirs(&paths.mods_dir(), &paths.disabled_mods_dir())?;

    let Some(account) = settings.selected() else {
        bail!("먼저 로그인해 주세요.");
    };
    let Some(refresh_token) = secrets.refresh_token(&account.id) else {
        bail!("저장된 로그인이 없어요. 다시 로그인해 주세요.");
    };

    reporter.progress(Stage::Auth, 0.0, "계정을 확인하는 중...");
    let mut session =
        Session::from_refresh_token(secrets.identity()?, refresh_token.to_string());
    let token = session.minecraft_token()?;
    let profile = session.profile()?;

    reporter.progress(Stage::Java, 0.0, "Java 런타임을 확인하는 중...");
    let java = runtime::ensure(
        &paths.runtime_dir(),
        &paths.cache_dir(),
        REQUIRED_JAVA,
        reporter,
        cancel,
    )?;

    reporter.progress(Stage::Manifest, 0.0, "구성 정보를 확인하는 중...");
    let manifest = manifest::fetch(manifest::DEFAULT_URL, &paths.cache_dir())
        .unwrap_or_else(|error| {
            log::warn!("매니페스트를 갱신하지 못해 로컬 사본을 사용합니다: {error:#}");
            manifest::load_local(&paths.cache_dir())
        });

    let plan = build_plan(&manifest, reporter)?;

    reporter.progress(Stage::Download, 0.0, "게임 파일을 확인하는 중...");
    let minecraft_dir = paths.minecraft_dir();
    download::run(
        &plan.targets,
        &minecraft_dir,
        Stage::Download,
        reporter,
        cancel,
        download::DEFAULT_CONCURRENCY,
        download::Verify::Size,
    )?;

    reporter.progress(Stage::Mods, 0.0, "모드를 확인하는 중...");
    if bundled::ensure(&paths.mods_dir(), &paths.disabled_mods_dir())? {
        log::info!("내장 모드 {} 을(를) 기록했습니다.", bundled::FILE_NAME);
    }

    let sync = mods::prepare_required(
        &paths.mods_dir(),
        &paths.disabled_mods_dir(),
        &manifest,
        &settings.managed_mods,
    )?;
    download::run(
        &sync.targets,
        &minecraft_dir,
        Stage::Mods,
        reporter,
        cancel,
        download::DEFAULT_CONCURRENCY,
        download::Verify::Checksum,
    )?;

    reporter.progress(Stage::Launch, 0.0, "실행 준비 중...");
    modconfig::write(
        &minecraft_dir,
        &ModConfig::from_settings(
            settings,
            &manifest.server.address,
            env!("CARGO_PKG_VERSION"),
        ),
    )?;

    let natives_dir = launch::natives_version_dir(&paths.natives_dir(), &plan.version.id);
    launch::extract_natives(&minecraft_dir, &plan.version.natives, &natives_dir)?;

    let log_path = paths.logs_dir().join(GAME_LOG_FILE);

    let inputs = LaunchInputs {
        minecraft_dir: &minecraft_dir,
        natives_dir: &natives_dir,
        java: &java,
        version: &plan.version,
        loader: &plan.loader,
        libraries: &plan.libraries,
        username: &profile.name,
        uuid: &profile.id,
        access_token: &token.access_token,
        server_address: &manifest.server.address,
        heap_megabytes: launch::detect_heap_megabytes(),
        launcher_version: env!("CARGO_PKG_VERSION"),
        log_path: &log_path,
    };

    reporter.progress(Stage::Launch, 0.6, "Minecraft 를 실행하는 중...");
    let child = launch::spawn(launch::build_command(&inputs)?, &log_path)?;

    reporter.progress(Stage::Launch, 1.0, "실행됐어요.");

    Ok(LaunchOutcome {
        child,
        managed_mods: mods::required_file_names(&manifest),
    })
}

struct Plan {
    version: version::VersionPlan,
    loader: fabric::LoaderPlan,
    libraries: Vec<version::DownloadTarget>,
    targets: Vec<version::DownloadTarget>,
}

fn build_plan(manifest: &Manifest, reporter: &Reporter) -> anyhow::Result<Plan> {
    let game_version = manifest.minecraft.version.as_str();

    reporter.progress(Stage::Manifest, 0.3, "Minecraft 버전 정보를 받는 중...");
    let detail = version::resolve(game_version)?;
    let version = detail
        .plan(version::Platform::current())
        .context("Minecraft 구성을 만들지 못했어요.")?;

    reporter.progress(Stage::Manifest, 0.6, "Fabric 로더 정보를 받는 중...");
    let loader = fabric::resolve(game_version, &manifest.minecraft.fabric_loader)?;

    reporter.progress(Stage::Manifest, 0.9, "에셋 목록을 받는 중...");
    let index = assets::fetch(&version.asset_index)?;

    let libraries = fabric::merge_libraries(&loader.libraries, &version.libraries);

    let mut targets = libraries.clone();
    targets.extend(version.natives.iter().cloned());
    targets.push(version.client.clone());
    targets.push(assets::index_target(&version.asset_index));
    targets.extend(index.targets()?);

    log::info!(
        "다운로드 대상 {}개 · 라이브러리 {} · 에셋 {}",
        targets.len(),
        libraries.len(),
        index.objects.len()
    );

    Ok(Plan {
        version,
        loader,
        libraries,
        targets,
    })
}
