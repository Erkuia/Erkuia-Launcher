fn main() {
    slint_build::compile("ui/installer.slint").expect("failed to compile Slint UI");

    #[cfg(target_os = "windows")]
    {
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
