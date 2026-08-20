fn main() {
    println!("cargo:rerun-if-changed=assets/app.ico");
    println!("cargo:rerun-if-changed=assets/app.manifest");

    build_handler_dll();

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
        // English, not the bilingual window title: the file properties dialog
        // has no language switch, so whichever language goes in here is the
        // one every Explorer user sees, regardless of `--lang` or the saved
        // setting. English also matches a public repository's audience.
        res.set("ProductName", "Context Menu Manager");
        res.compile().expect("compiling the resources failed");
    }
}

/// Builds `ctxmenu_handler.dll` and hands its path to `include_bytes!`.
///
/// The window carries the handler inside itself — the promise is one `.exe`
/// with no installer, and the DLL is written to `%LOCALAPPDATA%` when the
/// user turns the Windows 11 entries on. A cdylib cannot be an ordinary
/// dependency, so this is cargo inside cargo, with two guards that matter:
///
/// * Its own `--target-dir` under `OUT_DIR`. Sharing the outer target
///   directory would deadlock on cargo's build lock — and a shared target
///   directory is exactly the trap `.claude`'s worktree rule exists for.
/// * Always `--release`: the DLL is an artefact the shell loads, not a
///   debug target of its own; a debug build of the window still embeds the
///   optimised handler.
///
/// `rerun-if-changed` keeps this from running on every build: cargo skips
/// the whole script while the handler's sources are untouched.
fn build_handler_dll() {
    println!("cargo:rerun-if-changed=../ctxmenu-handler/src");
    println!("cargo:rerun-if-changed=../ctxmenu-handler/Cargo.toml");
    println!("cargo:rerun-if-changed=../ctxmenu-handler/AppxManifest.xml");
    println!("cargo:rerun-if-changed=../ctxmenu-handler/handler.msix");

    let out_dir = std::env::var("OUT_DIR").expect("cargo sets OUT_DIR");
    let target_dir = std::path::Path::new(&out_dir).join("handler-target");
    let target = std::env::var("TARGET").expect("cargo sets TARGET while running a build script");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());

    let status = std::process::Command::new(&cargo)
        .args(["build", "--release", "-p", "ctxmenu-handler", "--target"])
        .arg(&target)
        .arg("--target-dir")
        .arg(&target_dir)
        // The outer invocation may have set this; letting it through would
        // point the inner build back at the shared directory.
        .env_remove("CARGO_TARGET_DIR")
        .status()
        .expect("running cargo for the handler DLL");
    assert!(status.success(), "the handler DLL did not build");

    let dll = target_dir
        .join(&target)
        .join("release")
        .join("ctxmenu_handler.dll");
    assert!(dll.exists(), "expected the DLL at {}", dll.display());
    println!("cargo:rustc-env=CTXMENU_HANDLER_DLL={}", dll.display());
}
