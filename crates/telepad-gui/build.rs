//! Embeds the app icon and version info into `telepad.exe` on Windows, so
//! Explorer, the taskbar, the Start menu and the installer show the Telepad icon.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon/telepad.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon/telepad.ico")
            .set("ProductName", "Telepad")
            .set("FileDescription", "Telepad")
            .set("LegalCopyright", "MIT OR Apache-2.0");
        if let Err(e) = res.compile() {
            // A build-time failure (no resource compiler) should be loud, not a silent icon-less exe.
            panic!("embedding the Windows icon failed: {e}");
        }
    }
}
