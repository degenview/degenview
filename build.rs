fn main() {
    // Embed the app icon in the Windows executable (Explorer, taskbar, shortcuts).
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=assets/brand/DegenView.ico");
        winresource::WindowsResource::new()
            .set_icon("assets/brand/DegenView.ico")
            .compile()
            .expect("embed Windows icon");
    }
    swift_bridge();
}

/// Compiles the CloudKit bridge (`native/swift`) into a static library and links it into the
/// `degenview` binary only: the background alert agent shares the library crate but not CloudKit.
///
/// Only with the `icloud` feature, and only when building *for* macOS. `cfg(target_os)` in a
/// build script describes the machine running the build, so the target is read from Cargo's
/// environment instead.
fn swift_bridge() {
    use std::path::PathBuf;
    use std::process::Command;

    println!("cargo:rerun-if-changed=native/swift");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_ICLOUD");
    if std::env::var_os("CARGO_FEATURE_ICLOUD").is_none()
        || std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos")
    {
        return;
    }

    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86_64") => "x86_64",
        other => panic!("the iCloud bridge supports arm64 and x86_64 Macs, not {other:?}"),
    };
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let library = out.join("libdvsync.a");
    let mut sources: Vec<PathBuf> = std::fs::read_dir("native/swift")
        .expect("native/swift")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "swift"))
        .collect();
    sources.sort();

    let status = Command::new("xcrun")
        .args([
            "swiftc",
            "-swift-version",
            "5",
            "-parse-as-library",
            "-O",
            "-wmo",
        ])
        .args(["-emit-library", "-static", "-module-name", "DVSync"])
        .args(["-target", &format!("{arch}-apple-macos14.0")])
        .arg("-o")
        .arg(&library)
        .args(&sources)
        .status()
        .expect("run xcrun swiftc (install the Xcode command line tools to build with iCloud)");
    assert!(status.success(), "compiling the Swift iCloud bridge failed");

    let sdk = Command::new("xcrun")
        .args(["--show-sdk-path"])
        .output()
        .expect("run xcrun --show-sdk-path");
    let sdk = String::from_utf8_lossy(&sdk.stdout).trim().to_string();

    let link = |arg: String| println!("cargo:rustc-link-arg-bin=degenview={arg}");
    link(format!("-L{}", out.display()));
    link("-ldvsync".into());
    // The Swift runtime ships with macOS; link against the SDK's stubs and find it at run time.
    link(format!("-L{sdk}/usr/lib/swift"));
    link("-Wl,-rpath,/usr/lib/swift".into());
    for framework in ["CloudKit", "CryptoKit", "Network", "Foundation"] {
        link("-framework".into());
        link(framework.into());
    }
}
