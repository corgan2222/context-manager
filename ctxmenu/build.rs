fn main() {
    println!("cargo:rerun-if-changed=assets/app.ico");
    println!("cargo:rerun-if-changed=assets/app.manifest");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/app.ico");
        res.set_manifest_file("assets/app.manifest");
        // FileVersion and ProductVersion come from CARGO_PKG_VERSION without
        // being asked, so the properties dialog can never disagree with
        // `ctxmenu --version`. What does not come for free is the description:
        // it defaults to the crate name, and "ctxmenu" in the Windows file
        // properties says less than the sentence Cargo.toml already has.
        // `std::env::var`, not `env!`: Cargo sets these while *running* the
        // build script, which is not the same moment as compiling it.
        let description = std::env::var("CARGO_PKG_DESCRIPTION")
            .unwrap_or_else(|_| "Windows Context Menu Manager".into());
        res.set("FileDescription", &description);
        res.set("ProductName", "Kontextmenü-Manager");
        res.compile().expect("compiling the resources failed");
    }
}
