fn main() {
    slint_build::compile("ui/launcher.slint").expect("failed to compile Slint UI");

    #[cfg(target_os = "windows")]
    {
        // `slint_build::compile` already emits `rerun-if-changed` directives, so
        // cargo no longer watches the whole package. The icon has to be listed
        // explicitly or a changed icon would not trigger a rebuild.
        println!("cargo:rerun-if-changed=assets/rendog-launcher.ico");

        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("assets/rendog-launcher.ico")
            .set("ProductName", "Rendog Launcher")
            .set("FileDescription", "Rendog Launcher")
            .set("CompanyName", "Rendog")
            .set("OriginalFilename", "RendogLauncher.exe");
        resource
            .compile()
            .expect("failed to compile Windows resources");
    }
}
