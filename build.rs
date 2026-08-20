fn main() {
    slint_build::compile("ui/launcher.slint").expect("failed to compile Slint UI");

    #[cfg(target_os = "windows")]
    {
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
