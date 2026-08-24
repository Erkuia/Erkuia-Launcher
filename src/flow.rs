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

/// Everything that has to be on disk before the JVM starts, in one list.
///
/// The download step treats this list as complete: whatever is missing here is
/// missing at launch, and the failure surfaces as a Java stack trace rather than
/// as a missing file. Deduplication is by destination path, since the same
/// artifact reached twice would otherwise be fetched twice.
fn assemble_targets(
    version: &version::VersionPlan,
    libraries: &[version::DownloadTarget],
    index: &assets::AssetIndex,
) -> anyhow::Result<Vec<version::DownloadTarget>> {
    let mut seen = std::collections::HashSet::new();
    let mut targets = Vec::with_capacity(libraries.len() + index.objects.len() + 2);

    let mut push = |target: version::DownloadTarget| {
        if seen.insert(target.relative_path.clone()) {
            targets.push(target);
        }
    };

    for target in libraries.iter().chain(version.natives.iter()) {
        push(target.clone());
    }

    push(version.client.clone());
    push(assets::index_target(&version.asset_index));

    for target in index.targets()? {
        push(target);
    }

    Ok(targets)
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
    let targets = assemble_targets(&version, &libraries, &index)?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        hash::Checksum,
        mc::version::{AssetIndexRef, DownloadTarget, VersionPlan},
    };

    fn library(name: &str, path: &str) -> DownloadTarget {
        DownloadTarget {
            url: format!("https://libraries.invalid/{path}"),
            relative_path: format!("libraries/{path}"),
            checksum: Some(Checksum::Sha1("a".repeat(40))),
            size: 1,
            name: Some(name.to_string()),
        }
    }

    fn plan(libraries: Vec<DownloadTarget>, natives: Vec<DownloadTarget>) -> VersionPlan {
        VersionPlan {
            id: "1.21.4".to_string(),
            main_class: "net.minecraft.client.main.Main".to_string(),
            asset_index: AssetIndexRef {
                id: "12".to_string(),
                url: "https://piston.invalid/12.json".to_string(),
                sha1: "1".repeat(40),
                size: 400,
                total_size: 900,
            },
            assets: "12".to_string(),
            java_major: 21,
            client: DownloadTarget {
                url: "https://piston.invalid/client.jar".to_string(),
                relative_path: "versions/1.21.4/1.21.4.jar".to_string(),
                checksum: Some(Checksum::Sha1("b".repeat(40))),
                size: 24_445_539,
                name: None,
            },
            libraries,
            natives,
        }
    }

    fn index(hashes: &[&str]) -> assets::AssetIndex {
        let objects: Vec<String> = hashes
            .iter()
            .enumerate()
            .map(|(position, hash)| {
                format!(r#""a{position}.ogg": {{ "hash": "{hash}", "size": 10 }}"#)
            })
            .collect();

        serde_json::from_str(&format!(r#"{{ "objects": {{ {} }} }}"#, objects.join(","))).unwrap()
    }

    fn paths(targets: &[DownloadTarget]) -> Vec<&str> {
        targets
            .iter()
            .map(|target| target.relative_path.as_str())
            .collect()
    }

    #[test]
    fn everything_the_jvm_needs_is_in_the_list() {
        let libraries = vec![library("org.lwjgl:lwjgl:3.3.2", "org/lwjgl/lwjgl/3.3.2/lwjgl-3.3.2.jar")];
        let version = plan(libraries.clone(), Vec::new());

        let targets = assemble_targets(&version, &libraries, &index(&["0".repeat(40).as_str()])).unwrap();
        let paths = paths(&targets);

        assert!(paths.contains(&"versions/1.21.4/1.21.4.jar"), "client jar");
        assert!(paths.contains(&"assets/indexes/12.json"), "asset index");
        assert!(paths.iter().any(|path| path.starts_with("libraries/")), "libraries");
        assert!(paths.iter().any(|path| path.starts_with("assets/objects/")), "asset objects");
    }

    /// The natives jar once vanished between the version plan and the download
    /// list, and the game only said so as an LWJGL stack trace. It has to be
    /// carried through by path, whichever list it arrived on.
    #[test]
    fn natives_reach_the_download_list() {
        let libraries = vec![library(
            "org.lwjgl:lwjgl:3.3.2:natives-windows",
            "org/lwjgl/lwjgl/3.3.2/lwjgl-3.3.2-natives-windows.jar",
        )];
        let legacy = vec![library(
            "org.lwjgl:lwjgl:2.9.4:natives-windows",
            "org/lwjgl/lwjgl/2.9.4/lwjgl-2.9.4-natives-windows.jar",
        )];
        let version = plan(libraries.clone(), legacy);

        let targets = assemble_targets(&version, &libraries, &index(&[])).unwrap();
        let paths = paths(&targets);

        assert!(paths.iter().any(|path| path.ends_with("lwjgl-3.3.2-natives-windows.jar")));
        assert!(paths.iter().any(|path| path.ends_with("lwjgl-2.9.4-natives-windows.jar")));
    }

    #[test]
    fn nothing_is_queued_twice() {
        let shared = library("org.lwjgl:lwjgl:3.3.2", "org/lwjgl/lwjgl/3.3.2/lwjgl-3.3.2.jar");
        let libraries = vec![shared.clone()];
        let version = plan(libraries.clone(), vec![shared]);

        let hash = "0".repeat(40);
        let targets =
            assemble_targets(&version, &libraries, &index(&[hash.as_str(), hash.as_str()])).unwrap();
        let mut paths = paths(&targets);
        let total = paths.len();

        paths.sort_unstable();
        paths.dedup();

        assert_eq!(paths.len(), total, "a repeated artifact would be fetched twice");
    }

    #[test]
    fn every_destination_stays_inside_the_minecraft_folder() {
        let libraries = vec![library("org.lwjgl:lwjgl:3.3.2", "org/lwjgl/lwjgl/3.3.2/lwjgl-3.3.2.jar")];
        let version = plan(libraries.clone(), Vec::new());

        let targets = assemble_targets(&version, &libraries, &index(&["0".repeat(40).as_str()])).unwrap();

        for target in &targets {
            let path = std::path::Path::new(&target.relative_path);

            assert!(path.is_relative(), "{} escapes by being absolute", target.relative_path);
            assert!(
                !target.relative_path.contains(".."),
                "{} escapes by traversal",
                target.relative_path
            );
        }
    }

    #[test]
    fn a_world_with_no_assets_still_produces_a_launchable_set() {
        let version = plan(Vec::new(), Vec::new());

        let targets = assemble_targets(&version, &[], &index(&[])).unwrap();

        assert_eq!(targets.len(), 2, "the client jar and the asset index");
    }

    #[test]
    fn a_broken_asset_hash_stops_the_launch_instead_of_downloading_it() {
        let version = plan(Vec::new(), Vec::new());

        assert!(assemble_targets(&version, &[], &index(&["short"])).is_err());
    }
}
