use std::path::{Path, PathBuf};

const MOD_LIBS: &str = "../mod/build/libs";

fn embed_mod_jar() {
    println!("cargo:rerun-if-changed={MOD_LIBS}");

    let libs = Path::new(MOD_LIBS);

    let jar = std::fs::read_dir(libs)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                return false;
            };

            name.ends_with(".jar") && !name.ends_with("-sources.jar")
        })
        .max_by_key(|path| path.metadata().and_then(|meta| meta.modified()).ok());

    let Some(jar) = jar else {
        panic!(
            "내장할 모드 jar 를 {MOD_LIBS} 에서 찾지 못했습니다.\n\
             mod 폴더에서 `gradle build` 를 먼저 실행해 주세요."
        );
    };

    println!("cargo:rerun-if-changed={}", jar.display());

    let destination = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR is set"))
        .join("rendoglauncher.jar");

    std::fs::copy(&jar, &destination)
        .unwrap_or_else(|error| panic!("{} 을(를) 복사하지 못했습니다: {error}", jar.display()));

    println!("cargo:rustc-env=RENDOG_MOD_JAR={}", destination.display());
}

fn main() {
    slint_build::compile("ui/launcher.slint").expect("failed to compile Slint UI");

    embed_mod_jar();

    #[cfg(target_os = "windows")]
    {
        println!("cargo:rerun-if-changed=assets/rendog-launcher.ico");

        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("assets/rendog-launcher.ico")
            .set("ProductName", "Rendog Launcher")
            .set("FileDescription", "Rendog Launcher")
            .set("CompanyName", "폴리큐")
            .set("OriginalFilename", "RendogLauncher.exe");
        resource
            .compile()
            .expect("failed to compile Windows resources");
    }
}
