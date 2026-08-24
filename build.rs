fn main() {
    slint_build::compile("ui/installer.slint").expect("failed to compile Slint UI");

    #[cfg(target_os = "windows")]
    {
        println!("cargo:rerun-if-changed=assets/erkuia-installer.ico");

        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("assets/erkuia-installer.ico")
            .set("ProductName", "Rendog Launcher Installer")
            .set("FileDescription", "Rendog Launcher Installer")
            .set("CompanyName", "폴리큐")
            .set("OriginalFilename", "RendogLauncherInstaller.exe");
        resource
            .compile()
            .expect("failed to compile Windows resources");
    }
}
