fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        tauri_build::try_build(
            tauri_build::Attributes::new()
                .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest()),
        )
        .expect("Tauri build configuration failed");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        let compatibility = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("windows")
            .join("compatibility.manifest");
        println!("cargo:rerun-if-changed={}", compatibility.display());
        println!(
            "cargo:rustc-link-arg=/MANIFESTINPUT:{}",
            compatibility.display()
        );
        println!(
            "cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
        );
    } else {
        tauri_build::build();
    }
}
