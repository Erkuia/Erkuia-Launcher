fn main() {
    slint_build::compile("ui/installer.slint").expect("failed to compile Slint UI");

    #[cfg(target_os = "windows")]
    {
        println!("cargo:rerun-if-changed=assets/erkuia-installer.ico");

        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("assets/erkuia-installer.ico")
            .set("ProductName", "Erkuia Launcher Installer")
            .set("FileDescription", "Erkuia Launcher Installer")
            .set("CompanyName", "Erkuia")
            .set("OriginalFilename", "Erkuia-Launcher-Installer.exe");
        resource
            .compile()
            .expect("failed to compile Windows resources");
    }
}
