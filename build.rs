fn main() {
    const ICON: &str = "assets/soundcloud-lite.ico";
    println!("cargo:rerun-if-changed={ICON}");

    if std::env::var_os("CARGO_CFG_TARGET_OS").as_deref() == Some(std::ffi::OsStr::new("windows")) {
        winresource::WindowsResource::new()
            .set_icon(ICON)
            .set("FileDescription", "SoundCloud Lite")
            .set("ProductName", "SoundCloud Lite")
            .compile()
            .expect("compile SoundCloud Lite Windows icon");
    }
}
