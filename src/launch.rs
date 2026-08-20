#![allow(dead_code)]

use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use anyhow::{bail, Context};

use crate::{
    java::JavaInstall,
    mc::{fabric::LoaderPlan, version::{DownloadTarget, VersionPlan}},
};

pub const LAUNCHER_BRAND: &str = "RendogLauncher";
pub const USER_TYPE: &str = "msa";
pub const VERSION_TYPE: &str = "release";

const MIN_HEAP_MB: u32 = 1024;
const MAX_HEAP_MB: u32 = 8192;
const EARLY_EXIT_GRACE: Duration = Duration::from_secs(5);
const EARLY_EXIT_POLL: Duration = Duration::from_millis(200);

#[cfg(windows)]
pub const CLASSPATH_SEPARATOR: &str = ";";
#[cfg(not(windows))]
pub const CLASSPATH_SEPARATOR: &str = ":";

#[cfg(windows)]
mod memory {
    use std::ffi::c_void;

    #[repr(C)]
    struct MemoryStatusEx {
        length: u32,
        memory_load: u32,
        total_physical: u64,
        available_physical: u64,
        total_page_file: u64,
        available_page_file: u64,
        total_virtual: u64,
        available_virtual: u64,
        available_extended_virtual: u64,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalMemoryStatusEx(buffer: *mut c_void) -> i32;
    }

    pub fn total_megabytes() -> Option<u64> {
        let mut status = MemoryStatusEx {
            length: std::mem::size_of::<MemoryStatusEx>() as u32,
            memory_load: 0,
            total_physical: 0,
            available_physical: 0,
            total_page_file: 0,
            available_page_file: 0,
            total_virtual: 0,
            available_virtual: 0,
            available_extended_virtual: 0,
        };

        let ok = unsafe { GlobalMemoryStatusEx(&mut status as *mut _ as *mut c_void) };

        (ok != 0).then(|| status.total_physical / (1024 * 1024))
    }
}

#[cfg(not(windows))]
mod memory {
    pub fn total_megabytes() -> Option<u64> {
        None
    }
}

/// Minecraft gains little past a few gigabytes and a large heap makes G1 pauses
/// worse, so the share of system memory shrinks as the machine grows.
pub fn heap_megabytes(total_ram_mb: u64) -> u32 {
    let chosen = match total_ram_mb {
        0..=4095 => 1024,
        4096..=8191 => 2048,
        8192..=16383 => 4096,
        _ => 6144,
    };

    chosen.clamp(MIN_HEAP_MB, MAX_HEAP_MB)
}

pub fn detect_heap_megabytes() -> u32 {
    heap_megabytes(memory::total_megabytes().unwrap_or(8192))
}

pub fn classpath(minecraft_dir: &Path, libraries: &[DownloadTarget], client: &DownloadTarget) -> String {
    libraries
        .iter()
        .chain(std::iter::once(client))
        .map(|target| {
            minecraft_dir
                .join(&target.relative_path)
                .display()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join(CLASSPATH_SEPARATOR)
}

pub fn extract_natives(
    minecraft_dir: &Path,
    natives: &[DownloadTarget],
    natives_dir: &Path,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(natives_dir)
        .with_context(|| format!("{} 폴더를 만들지 못했어요.", natives_dir.display()))?;

    for target in natives {
        let path = minecraft_dir.join(&target.relative_path);
        let file = std::fs::File::open(&path)
            .with_context(|| format!("{} 을(를) 열지 못했어요.", path.display()))?;
        let mut archive = zip::ZipArchive::new(file)
            .with_context(|| format!("{} 압축을 열지 못했어요.", path.display()))?;

        for index in 0..archive.len() {
            let mut entry = archive
                .by_index(index)
                .context("네이티브 항목을 읽지 못했어요.")?;

            let Some(relative) = entry.enclosed_name() else {
                continue;
            };

            if entry.is_dir() || relative.starts_with("META-INF") {
                continue;
            }

            let destination = natives_dir.join(&relative);

            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("{} 폴더를 만들지 못했어요.", parent.display()))?;
            }

            let mut out = std::fs::File::create(&destination)
                .with_context(|| format!("{} 을(를) 만들지 못했어요.", destination.display()))?;
            std::io::copy(&mut entry, &mut out)
                .with_context(|| format!("{} 을(를) 풀지 못했어요.", destination.display()))?;
        }
    }

    Ok(())
}

pub struct LaunchInputs<'a> {
    pub minecraft_dir: &'a Path,
    pub natives_dir: &'a Path,
    pub java: &'a JavaInstall,
    pub version: &'a VersionPlan,
    pub loader: &'a LoaderPlan,
    pub libraries: &'a [DownloadTarget],
    pub username: &'a str,
    pub uuid: &'a str,
    pub access_token: &'a str,
    pub server_address: &'a str,
    pub heap_megabytes: u32,
    pub launcher_version: &'a str,
    pub log_path: &'a Path,
}

pub fn jvm_arguments(inputs: &LaunchInputs<'_>) -> Vec<String> {
    let heap = inputs.heap_megabytes.clamp(MIN_HEAP_MB, MAX_HEAP_MB);
    let initial = (heap / 2).max(MIN_HEAP_MB.min(heap));

    let mut arguments = vec![
        format!("-Xmx{heap}M"),
        format!("-Xms{initial}M"),
        "-XX:+UnlockExperimentalVMOptions".to_string(),
        "-XX:+UseG1GC".to_string(),
        "-XX:G1NewSizePercent=20".to_string(),
        "-XX:G1ReservePercent=20".to_string(),
        "-XX:MaxGCPauseMillis=50".to_string(),
        "-XX:G1HeapRegionSize=32M".to_string(),
        "-Dfile.encoding=UTF-8".to_string(),
        format!("-Djava.library.path={}", inputs.natives_dir.display()),
        format!(
            "-Dorg.lwjgl.system.SharedLibraryExtractPath={}",
            inputs.natives_dir.display()
        ),
        format!("-Djna.tmpdir={}", inputs.natives_dir.display()),
        format!("-Dio.netty.native.workdir={}", inputs.natives_dir.display()),
        format!("-Dminecraft.launcher.brand={LAUNCHER_BRAND}"),
        format!("-Dminecraft.launcher.version={}", inputs.launcher_version),
    ];

    arguments.extend(inputs.loader.jvm_arguments.iter().cloned());

    arguments.push("-cp".to_string());
    arguments.push(classpath(
        inputs.minecraft_dir,
        inputs.libraries,
        &inputs.version.client,
    ));

    arguments
}

/// Quick Play is the vanilla way in: 1.20.4 declares the flag behind the
/// `is_quick_play_multiplayer` feature, so the client dials the server itself
/// and the launcher needs no agreement with any mod. An empty address simply
/// leaves the player on the multiplayer screen.
pub fn quick_play_arguments(server_address: &str) -> Vec<String> {
    let address = server_address.trim();

    if address.is_empty() {
        return Vec::new();
    }

    vec!["--quickPlayMultiplayer".to_string(), address.to_string()]
}

pub fn game_arguments(inputs: &LaunchInputs<'_>) -> Vec<String> {
    let mut arguments = vec![
        "--username".to_string(),
        inputs.username.to_string(),
        "--version".to_string(),
        inputs.version.id.clone(),
        "--gameDir".to_string(),
        inputs.minecraft_dir.display().to_string(),
        "--assetsDir".to_string(),
        inputs.minecraft_dir.join("assets").display().to_string(),
        "--assetIndex".to_string(),
        inputs.version.asset_index.id.clone(),
        "--uuid".to_string(),
        inputs.uuid.replace('-', ""),
        "--accessToken".to_string(),
        inputs.access_token.to_string(),
        "--clientId".to_string(),
        String::new(),
        "--xuid".to_string(),
        String::new(),
        "--userType".to_string(),
        USER_TYPE.to_string(),
        "--versionType".to_string(),
        VERSION_TYPE.to_string(),
    ];

    arguments.extend(quick_play_arguments(inputs.server_address));

    arguments
}

fn output_file(path: &Path) -> anyhow::Result<std::fs::File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("{} 폴더를 만들지 못했어요.", parent.display()))?;
    }

    std::fs::File::create(path)
        .with_context(|| format!("{} 을(를) 만들지 못했어요.", path.display()))
}

/// The launcher exits as soon as the game is up, so nothing would be left to
/// drain a pipe. Minecraft writes far more than a pipe buffer holds during
/// startup and would block forever on a full one, so both streams go to a file.
pub fn build_command(inputs: &LaunchInputs<'_>) -> anyhow::Result<Command> {
    let log = output_file(inputs.log_path)?;
    let errors = log
        .try_clone()
        .context("게임 로그 파일을 열어두지 못했어요.")?;

    let mut command = Command::new(inputs.java.javaw());

    command
        .current_dir(inputs.minecraft_dir)
        .args(jvm_arguments(inputs))
        .arg(&inputs.loader.main_class)
        .args(game_arguments(inputs))
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(errors))
        .stdin(Stdio::null());

    Ok(command)
}

/// A broken classpath or a missing native shows up within a second or two, so a
/// short watch catches the failures worth reporting without keeping the
/// launcher resident for the whole session.
pub fn spawn(mut command: Command, log_path: &Path) -> anyhow::Result<Child> {
    let mut child = command
        .spawn()
        .context("Minecraft 를 실행하지 못했어요. Java 설치를 확인해 주세요.")?;

    let started = Instant::now();

    while started.elapsed() < EARLY_EXIT_GRACE {
        match child.try_wait() {
            Ok(Some(status)) => {
                let detail = std::fs::read_to_string(log_path).unwrap_or_default();

                log::error!("Minecraft 조기 종료 ({status})\n{detail}");

                bail!(
                    "{}\n자세한 기록: {}",
                    describe_exit(status.code(), &detail),
                    log_path.display()
                );
            }
            Ok(None) => std::thread::sleep(EARLY_EXIT_POLL),
            Err(error) => {
                log::warn!("실행 상태를 확인하지 못했습니다: {error}");
                break;
            }
        }
    }

    log::info!(
        "Minecraft 실행 확인 (pid {}) · 기록 {}",
        child.id(),
        log_path.display()
    );

    Ok(child)
}

pub fn describe_exit(code: Option<i32>, detail: &str) -> String {
    let tail: String = detail
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .rev()
        .take(3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" ");

    match code {
        Some(code) if tail.trim().is_empty() => {
            format!("Minecraft 가 바로 종료됐어요. (종료 코드 {code})")
        }
        Some(code) => format!("Minecraft 가 바로 종료됐어요. (종료 코드 {code}) {tail}"),
        None => "Minecraft 가 예기치 않게 종료됐어요.".to_string(),
    }
}

pub fn natives_version_dir(natives_root: &Path, version_id: &str) -> PathBuf {
    natives_root.join(version_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mc::version::AssetIndexRef;

    fn target(path: &str) -> DownloadTarget {
        DownloadTarget {
            url: String::new(),
            relative_path: path.to_string(),
            checksum: None,
            size: 0,
            name: None,
        }
    }

    fn version() -> VersionPlan {
        VersionPlan {
            id: "1.20.4".to_string(),
            main_class: "net.minecraft.client.main.Main".to_string(),
            asset_index: AssetIndexRef {
                id: "12".to_string(),
                url: String::new(),
                sha1: String::new(),
                size: 0,
                total_size: 0,
            },
            assets: "12".to_string(),
            java_major: 17,
            client: target("versions/1.20.4/1.20.4.jar"),
            libraries: Vec::new(),
            natives: Vec::new(),
        }
    }

    fn loader() -> LoaderPlan {
        LoaderPlan {
            loader_version: "0.15.11".to_string(),
            main_class: "net.fabricmc.loader.impl.launch.knot.KnotClient".to_string(),
            libraries: Vec::new(),
            jvm_arguments: vec!["-DFabricMcEmu= net.minecraft.client.main.Main ".to_string()],
        }
    }

    fn inputs<'a>(
        minecraft: &'a Path,
        natives: &'a Path,
        java: &'a JavaInstall,
        version: &'a VersionPlan,
        loader: &'a LoaderPlan,
        libraries: &'a [DownloadTarget],
    ) -> LaunchInputs<'a> {
        LaunchInputs {
            minecraft_dir: minecraft,
            natives_dir: natives,
            java,
            version,
            loader,
            libraries,
            username: "KkulBee_",
            uuid: "069a79f4-44e9-4726-a5be-fca90e38aaf5",
            access_token: "TOKEN",
            server_address: "rendog.kr",
            heap_megabytes: 4096,
            launcher_version: "0.1.0",
            log_path: Path::new("/mc/logs/minecraft.log"),
        }
    }

    #[test]
    fn the_heap_grows_with_system_memory_but_stops() {
        assert_eq!(heap_megabytes(2048), 1024);
        assert_eq!(heap_megabytes(4096), 2048);
        assert_eq!(heap_megabytes(8192), 4096);
        assert_eq!(heap_megabytes(16384), 6144);
        assert_eq!(heap_megabytes(131_072), 6144);
    }

    #[test]
    fn a_machine_with_unknown_memory_still_gets_a_sane_heap() {
        let heap = heap_megabytes(0);

        assert!((MIN_HEAP_MB..=MAX_HEAP_MB).contains(&heap));
    }

    #[test]
    fn the_client_jar_goes_last_on_the_classpath() {
        let libraries = vec![target("libraries/a.jar"), target("libraries/b.jar")];
        let client = target("versions/1.20.4/1.20.4.jar");

        let value = classpath(Path::new("/mc"), &libraries, &client);
        let entries: Vec<&str> = value.split(CLASSPATH_SEPARATOR).collect();

        assert_eq!(entries.len(), 3);
        assert!(entries[0].ends_with("a.jar"));
        assert!(entries[2].ends_with("1.20.4.jar"));
    }

    #[test]
    fn jvm_arguments_carry_heap_natives_and_the_loader_extras() {
        let java = JavaInstall {
            home: PathBuf::from("/java"),
            major: 21,
        };
        let version = version();
        let loader = loader();
        let libraries = vec![target("libraries/a.jar")];
        let inputs = inputs(
            Path::new("/mc"),
            Path::new("/mc/natives"),
            &java,
            &version,
            &loader,
            &libraries,
        );

        let arguments = jvm_arguments(&inputs);

        assert!(arguments.contains(&"-Xmx4096M".to_string()));
        assert!(arguments.contains(&"-Xms2048M".to_string()));
        assert!(arguments.contains(&"-XX:+UseG1GC".to_string()));
        assert!(arguments
            .iter()
            .any(|argument| argument.starts_with("-Djava.library.path=")));
        assert!(arguments.contains(&"-DFabricMcEmu= net.minecraft.client.main.Main ".to_string()));
        assert_eq!(arguments[arguments.len() - 2], "-cp");
    }

    #[test]
    fn the_initial_heap_never_exceeds_the_maximum() {
        let java = JavaInstall {
            home: PathBuf::from("/java"),
            major: 21,
        };
        let version = version();
        let loader = loader();
        let mut inputs = inputs(
            Path::new("/mc"),
            Path::new("/mc/natives"),
            &java,
            &version,
            &loader,
            &[],
        );
        inputs.heap_megabytes = MIN_HEAP_MB;

        let arguments = jvm_arguments(&inputs);

        assert!(arguments.contains(&format!("-Xmx{MIN_HEAP_MB}M")));
        assert!(arguments.contains(&format!("-Xms{MIN_HEAP_MB}M")));
    }

    #[test]
    fn game_arguments_send_the_undashed_uuid() {
        let java = JavaInstall {
            home: PathBuf::from("/java"),
            major: 21,
        };
        let version = version();
        let loader = loader();
        let inputs = inputs(
            Path::new("/mc"),
            Path::new("/mc/natives"),
            &java,
            &version,
            &loader,
            &[],
        );

        let arguments = game_arguments(&inputs);
        let uuid = arguments
            .iter()
            .position(|argument| argument == "--uuid")
            .map(|index| &arguments[index + 1])
            .unwrap();

        assert_eq!(uuid, "069a79f444e94726a5befca90e38aaf5");
    }

    #[test]
    fn game_arguments_cover_what_the_client_expects() {
        let java = JavaInstall {
            home: PathBuf::from("/java"),
            major: 21,
        };
        let version = version();
        let loader = loader();
        let inputs = inputs(
            Path::new("/mc"),
            Path::new("/mc/natives"),
            &java,
            &version,
            &loader,
            &[],
        );

        let arguments = game_arguments(&inputs);

        for flag in [
            "--username",
            "--version",
            "--gameDir",
            "--assetsDir",
            "--assetIndex",
            "--uuid",
            "--accessToken",
            "--userType",
            "--versionType",
        ] {
            assert!(
                arguments.iter().any(|argument| argument == flag),
                "{flag} is missing"
            );
        }

        assert!(arguments.contains(&"msa".to_string()));
    }

    #[test]
    fn the_server_address_is_handed_over_as_quick_play() {
        let java = JavaInstall {
            home: PathBuf::from("/java"),
            major: 21,
        };
        let version = version();
        let loader = loader();
        let inputs = inputs(
            Path::new("/mc"),
            Path::new("/mc/natives"),
            &java,
            &version,
            &loader,
            &[],
        );

        let arguments = game_arguments(&inputs);
        let address = arguments
            .iter()
            .position(|argument| argument == "--quickPlayMultiplayer")
            .map(|index| arguments[index + 1].as_str());

        assert_eq!(address, Some("rendog.kr"));
    }

    #[test]
    fn no_address_leaves_the_player_on_the_multiplayer_screen() {
        assert!(quick_play_arguments("").is_empty());
        assert!(quick_play_arguments("   ").is_empty());
    }

    #[test]
    fn the_address_is_trimmed_before_it_reaches_the_client() {
        assert_eq!(
            quick_play_arguments("  rendog.kr \n"),
            vec!["--quickPlayMultiplayer".to_string(), "rendog.kr".to_string()]
        );
    }

    #[test]
    fn the_loader_main_class_replaces_the_vanilla_one() {
        let version = version();
        let loader = loader();

        assert_ne!(version.main_class, loader.main_class);
        assert!(loader.main_class.contains("Knot"));
    }

    #[test]
    fn an_exit_code_is_reported_with_the_tail_of_the_output() {
        let message = describe_exit(Some(1), "line one\nline two\nCould not find or load main class");

        assert!(message.contains("종료 코드 1"));
        assert!(message.contains("main class"));
    }

    #[test]
    fn an_exit_without_output_still_reports_the_code() {
        assert!(describe_exit(Some(3), "   ").contains("종료 코드 3"));
    }

    #[test]
    fn a_terminated_process_is_reported_without_a_code() {
        assert!(describe_exit(None, "").contains("예기치 않게"));
    }

    #[test]
    fn the_tail_skips_blank_lines() {
        let message = describe_exit(Some(1), "real cause\n\n\n   \n");

        assert!(message.contains("real cause"));
    }

    #[test]
    fn both_streams_land_in_the_same_file_so_neither_can_block() {
        let dir = std::env::temp_dir().join(format!("rendog-launch-log-{}", std::process::id()));
        let path = dir.join("logs").join("minecraft.log");

        let first = output_file(&path).unwrap();
        let second = first.try_clone().unwrap();

        assert!(path.is_file());
        drop((first, second));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn natives_are_kept_per_version() {
        assert!(natives_version_dir(Path::new("/mc/natives"), "1.20.4").ends_with("1.20.4"));
    }
}
