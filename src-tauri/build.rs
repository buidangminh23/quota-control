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
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        macos::build_swift_library();
    }
}

/// The Dynamic Island, the popup's window style and the widget reload live in Swift
/// (`macos/Shared` and `macos/Host`), compiled into a static library linked into the app. The
/// widget extension itself is built separately by `scripts/macos-widget.mjs`.
mod macos {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    const MINIMUM_MACOS: &str = "14.0";
    const LIBRARY: &str = "quotacontrol_macos";

    fn capture(command: &str, arguments: &[&str]) -> String {
        let output = Command::new(command)
            .args(arguments)
            .output()
            .unwrap_or_else(|error| panic!("{command} could not start: {error}"));
        assert!(
            output.status.success(),
            "{command} {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("tool output is not UTF-8")
            .trim()
            .to_owned()
    }

    fn swift_sources(folder: &Path) -> Vec<PathBuf> {
        println!("cargo:rerun-if-changed={}", folder.display());
        let mut sources: Vec<PathBuf> = std::fs::read_dir(folder)
            .unwrap_or_else(|error| panic!("cannot list {}: {error}", folder.display()))
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "swift")
            })
            .collect();
        sources.sort();
        for source in &sources {
            println!("cargo:rerun-if-changed={}", source.display());
        }
        sources
    }

    pub fn build_swift_library() {
        let manifest =
            PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
        let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
        let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
            Ok("aarch64") => "arm64",
            Ok("x86_64") => "x86_64",
            other => panic!("unsupported macOS architecture {other:?}"),
        };
        let root = manifest.join("macos");
        let mut sources = swift_sources(&root.join("Shared"));
        sources.extend(swift_sources(&root.join("Host")));
        let sdk = capture("xcrun", &["--sdk", "macosx", "--show-sdk-path"]);
        let library = out.join(format!("lib{LIBRARY}.a"));
        let target = format!("{arch}-apple-macos{MINIMUM_MACOS}");
        let status = Command::new("xcrun")
            .args([
                "swiftc",
                "-parse-as-library",
                "-emit-library",
                "-static",
                "-module-name",
                "QuotaControlMac",
                "-swift-version",
                "5",
                "-target",
                &target,
                "-sdk",
                &sdk,
                "-O",
                "-whole-module-optimization",
                "-o",
            ])
            .arg(&library)
            .args(&sources)
            .status()
            .expect("swiftc could not start; install Xcode or the Command Line Tools");
        assert!(
            status.success(),
            "swiftc failed to build the macOS host library"
        );

        let swiftc = PathBuf::from(capture("xcrun", &["--find", "swiftc"]));
        let toolchain = swiftc
            .parent()
            .and_then(Path::parent)
            .expect("unexpected swiftc location")
            .join("lib")
            .join("swift")
            .join("macosx");
        println!("cargo:rustc-link-search=native={}", out.display());
        println!("cargo:rustc-link-lib=static={LIBRARY}");
        println!("cargo:rustc-link-search=native={}", toolchain.display());
        println!("cargo:rustc-link-search=native=/usr/lib/swift");
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
        for framework in ["AppKit", "SwiftUI", "WidgetKit", "QuartzCore", "Foundation"] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
    }
}
