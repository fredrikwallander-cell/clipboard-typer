fn main() {
    println!("cargo:rerun-if-changed=assets/crosshair.ico");
    println!("cargo:rerun-if-changed=assets/crosshair-dark-theme.ico");
    println!("cargo:rerun-if-changed=app.manifest");

    if cfg!(windows) {
        let mut resources = winresource::WindowsResource::new();
        resources
            .set_icon("assets/crosshair.ico")
            .set_icon_with_id("assets/crosshair-dark-theme.ico", "2");
        if std::env::var("PROFILE").as_deref() == Ok("release") {
            resources.set_manifest_file("app.manifest");
        }
        resources
            .compile()
            .expect("failed to embed the Clipboard Typer icon");
    }
}
