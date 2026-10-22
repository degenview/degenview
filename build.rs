fn main() {
    build_id();
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

/// Exposes `DEGENVIEW_BUILD_ID` to the crate: the short commit hash the build was made from, with
/// `-dirty` when the tree had uncommitted changes so a local build never claims to be a clean
/// commit. A release is built from its tag's commit, so every release gets its own id.
/// `DEGENVIEW_BUILD_ID` in the environment overrides it; with no git (a source tarball) it is
/// `unknown`. Informational only: Settings > About shows it.
fn build_id() {
    use std::process::Command;

    println!("cargo:rerun-if-env-changed=DEGENVIEW_BUILD_ID");
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    // Rebuild when the checked-out commit changes. `--git-path` finds the right files in a linked
    // worktree, where `.git` is a file.
    if let Some(head) = git(&["rev-parse", "--git-path", "HEAD"]) {
        println!("cargo:rerun-if-changed={head}");
    }
    if let Some(reference) = git(&["symbolic-ref", "-q", "HEAD"])
        && let Some(path) = git(&["rev-parse", "--git-path", &reference])
    {
        println!("cargo:rerun-if-changed={path}");
    }
    let id = std::env::var("DEGENVIEW_BUILD_ID")
        .ok()
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty() && !id.contains(char::is_whitespace))
        .or_else(|| {
            let hash = git(&["rev-parse", "--short=9", "HEAD"]).filter(|hash| !hash.is_empty())?;
            // Untracked files (docs scratch, build output) do not change what was compiled.
            let dirty = git(&["status", "--porcelain", "--untracked-files=no"])
                .is_some_and(|status| !status.is_empty());
            Some(if dirty { format!("{hash}-dirty") } else { hash })
        })
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=DEGENVIEW_BUILD_ID={id}");
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
