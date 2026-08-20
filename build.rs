fn main() {
    slint_build::compile("ui/installer.slint").expect("failed to compile Slint UI");

    #[cfg(target_os = "windows")]
    {
        // `slint_build::compile` already emits `rerun-if-changed` directives, so
        // cargo no longer watches the whole package. The icon has to be listed
        // explicitly or a changed icon would not trigger a rebuild.
        println!("cargo:rerun-if-changed=assets/rendog-installer.ico");

        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("assets/rendog-installer.ico")
            .set("ProductName", "Rendog Launcher Installer")
            .set("FileDescription", "Rendog Launcher Installer")
            .set("CompanyName", "Rendog")
            .set("OriginalFilename", "rendog-launcher-installer.exe");
        resource
            .compile()
            .expect("failed to compile Windows resources");
    }
}
