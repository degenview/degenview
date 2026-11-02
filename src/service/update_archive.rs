//! Unpacking and inspecting a downloaded update, bounded so a hostile zip can't fill the disk or
//! plant extra launchable code: the zip's directory is checked before extraction, the extraction
//! itself is size- and time-limited, and the unpacked bundle must match the app's known layout.
//!
//! All of this is structural. It cannot prove the binaries are genuine; that comes from the signed
//! tag, the immutable release and the SHA-256 checked before anything is unpacked, and from the
//! Developer ID signature check in `code_signature`.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A real app unpacks to about 60 MB.
pub const MAX_UNPACKED_BYTES: u64 = 512 * 1024 * 1024;
const MAX_ENTRIES: usize = 2_000;
const EXTRACT_TIMEOUT: Duration = Duration::from_secs(120);
const POLL: Duration = Duration::from_millis(100);

const BUNDLE_ID: &str = "es.nico.degenview";
const HELPER_ID: &str = "es.nico.degenview.alert-agent";
const MAIN_EXECUTABLE: &str = "Contents/MacOS/DegenView";
pub(crate) const HELPER_APP: &str = "Contents/Library/LoginItems/DegenViewAlertAgent.app";
const HELPER_EXECUTABLE: &str = "Contents/MacOS/DegenViewAlertAgent";

// MARK: Zip directory

fn le16(bytes: &[u8], at: usize) -> Option<usize> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?).into())
}

fn le32(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?).into())
}

/// Reads the zip's central directory (no decompression) and refuses archives that declare too
/// many entries, too much data, ZIP64 sizes, or an entry name that is absolute or climbs out.
pub fn precheck(zip: &[u8]) -> Result<(), String> {
    let invalid = || "The download is not a valid zip.".to_owned();
    let last = zip.len().checked_sub(22).ok_or_else(invalid)?;
    let first = last.saturating_sub(usize::from(u16::MAX));
    let eocd = (first..=last)
        .rev()
        .find(|&at| zip[at..at + 4] == [0x50, 0x4b, 0x05, 0x06])
        .ok_or_else(invalid)?;
    let count = le16(zip, eocd + 10).ok_or_else(invalid)?;
    let directory_size = le32(zip, eocd + 12).ok_or_else(invalid)?;
    let directory_at = le32(zip, eocd + 16).ok_or_else(invalid)?;
    if count == 0xFFFF || directory_size == 0xFFFF_FFFF || directory_at == 0xFFFF_FFFF {
        return Err("The download uses an unsupported zip format.".to_owned());
    }
    if count == 0 || count > MAX_ENTRIES {
        return Err("The download has an unexpected number of files.".to_owned());
    }
    let mut at = usize::try_from(directory_at).map_err(|_| invalid())?;
    let mut total: u64 = 0;
    for _ in 0..count {
        if zip.get(at..at + 4) != Some(&[0x50, 0x4b, 0x01, 0x02]) {
            return Err(invalid());
        }
        let size = le32(zip, at + 24).ok_or_else(invalid)?;
        let (name_len, extra_len, comment_len) = (
            le16(zip, at + 28).ok_or_else(invalid)?,
            le16(zip, at + 30).ok_or_else(invalid)?,
            le16(zip, at + 32).ok_or_else(invalid)?,
        );
        let name = zip.get(at + 46..at + 46 + name_len).ok_or_else(invalid)?;
        if !is_safe_entry_name(name) {
            return Err("The download contains an unsafe file path.".to_owned());
        }
        total = total
            .checked_add(size)
            .filter(|total| *total <= MAX_UNPACKED_BYTES && size != 0xFFFF_FFFF)
            .ok_or("The download unpacks to more data than an update should.")?;
        at += 46 + name_len + extra_len + comment_len;
    }
    Ok(())
}

fn is_safe_entry_name(name: &[u8]) -> bool {
    let Ok(name) = std::str::from_utf8(name) else {
        return false;
    };
    !name.is_empty()
        && !name.starts_with('/')
        && !name.contains(['\\', '\0'])
        && name.split('/').all(|part| part != "..")
}

// MARK: Bounded extraction

/// Unpacks `zip` into `dest` with `ditto`, killing it if `dest` grows past `MAX_UNPACKED_BYTES`
/// (declared sizes can lie) or it runs past the timeout.
pub fn extract(zip: &Path, dest: &Path) -> Result<(), String> {
    let mut command = Command::new("/usr/bin/ditto");
    command.arg("-x").arg("-k").arg(zip).arg(dest);
    run_bounded(&mut command, dest, MAX_UNPACKED_BYTES, EXTRACT_TIMEOUT)
        .map_err(|error| format!("Could not unpack the download: {error}"))
}

fn run_bounded(
    command: &mut Command,
    watch: &Path,
    max_bytes: u64,
    timeout: Duration,
) -> Result<(), String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    let started = Instant::now();
    let fail = |child: &mut std::process::Child, message: &str| {
        let _ = child.kill();
        let _ = child.wait();
        Err(message.to_owned())
    };
    loop {
        match child.try_wait().map_err(|error| error.to_string())? {
            Some(status) if status.success() => return Ok(()),
            Some(status) => return Err(format!("unpacking failed ({status})")),
            None => {}
        }
        if directory_bytes(watch) > max_bytes {
            return fail(&mut child, "it unpacks to more data than an update should");
        }
        if started.elapsed() > timeout {
            return fail(&mut child, "unpacking took too long");
        }
        std::thread::sleep(POLL);
    }
}

/// Total size of the regular files under `dir` (symlinks are not followed).
fn directory_bytes(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.metadata() {
            Ok(meta) if meta.is_dir() => directory_bytes(&entry.path()),
            Ok(meta) if meta.is_file() => meta.len(),
            _ => 0,
        })
        .sum()
}

// MARK: Windows executable

/// The one file a Windows update replaces: the release zip carries it at its root, next to licence
/// texts that are not code and are left alone.
#[cfg(any(windows, test))]
pub const EXE_NAME: &str = "degenview.exe";

/// Inflates `EXE_NAME` from the zip in `bytes` into `dest_dir`, refusing more than
/// `MAX_UNPACKED_BYTES`. The output name is fixed, so nothing in the archive picks where a file
/// lands, and every other entry is ignored. `precheck` has already bounded the directory itself.
#[cfg(any(windows, test))]
pub fn extract_exe(bytes: &[u8], dest_dir: &Path) -> Result<std::path::PathBuf, String> {
    use std::io::Read;

    let failed = |error: &dyn std::fmt::Display| format!("Could not unpack the download: {error}");
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|error| failed(&error))?;
    let mut entry = archive
        .by_name(EXE_NAME)
        .map_err(|_| "The download is not a DegenView app.".to_owned())?;
    if !entry.is_file() || entry.size() > MAX_UNPACKED_BYTES {
        return Err("The download is not a DegenView app.".to_owned());
    }
    std::fs::create_dir_all(dest_dir).map_err(|error| failed(&error))?;
    let dest = dest_dir.join(EXE_NAME);
    let mut out = std::fs::File::create(&dest).map_err(|error| failed(&error))?;
    // Declared sizes can lie: count what actually comes out.
    let copied = std::io::copy(&mut (&mut entry).take(MAX_UNPACKED_BYTES + 1), &mut out)
        .map_err(|error| failed(&error))?;
    if copied > MAX_UNPACKED_BYTES {
        return Err("The download unpacks to more data than an update should.".to_owned());
    }
    Ok(dest)
}

/// The PE machine type of the running build, which an update for another CPU must not replace.
#[cfg(any(windows, test))]
const fn expected_machine() -> u16 {
    if cfg!(target_arch = "aarch64") {
        0xAA64
    } else {
        0x8664
    }
}

/// The machine type in the PE header of `exe`, or an error if it is not a PE image.
#[cfg(any(windows, test))]
fn pe_machine(exe: &Path) -> Result<u16, String> {
    use std::io::{Read, Seek, SeekFrom};

    let not_degenview = || "The download is not a DegenView app.".to_owned();
    let mut file = std::fs::File::open(exe).map_err(|_| not_degenview())?;
    let mut dos = [0u8; 64];
    file.read_exact(&mut dos).map_err(|_| not_degenview())?;
    if &dos[..2] != b"MZ" {
        return Err(not_degenview());
    }
    let pe_at = u32::from_le_bytes([dos[0x3C], dos[0x3D], dos[0x3E], dos[0x3F]]);
    let mut pe = [0u8; 6];
    file.seek(SeekFrom::Start(pe_at.into()))
        .and_then(|_| file.read_exact(&mut pe))
        .map_err(|_| not_degenview())?;
    if &pe[..4] != b"PE\0\0" {
        return Err(not_degenview());
    }
    Ok(u16::from_le_bytes([pe[4], pe[5]]))
}

/// Structural check of an unpacked Windows executable: a PE image for this CPU. It cannot prove the
/// file is genuine; that is the signed tag, the immutable release and the SHA-256.
#[cfg(any(windows, test))]
pub fn check_exe(exe: &Path) -> Result<(), String> {
    if pe_machine(exe)? != expected_machine() {
        return Err("The download is for a different kind of processor.".to_owned());
    }
    Ok(())
}

// MARK: Bundle layout

/// The unpacked bundle must be exactly DegenView: both of its executables with our bundle ids, no
/// other launchable file, and no link that leaves the bundle.
pub fn check_bundle(app: &Path) -> Result<(), String> {
    let not_degenview = || "The download is not a DegenView app.".to_owned();
    let helper = app.join(HELPER_APP);
    let declares = |bundle: &Path, id: &str| {
        std::fs::read_to_string(bundle.join("Contents/Info.plist"))
            .is_ok_and(|plist| plist.contains(&format!("<string>{id}</string>")))
    };
    let main_executable = app.join(MAIN_EXECUTABLE);
    let helper_executable = helper.join(HELPER_EXECUTABLE);
    if !declares(app, BUNDLE_ID)
        || !declares(&helper, HELPER_ID)
        || !main_executable.is_file()
        || !helper_executable.is_file()
    {
        return Err(not_degenview());
    }
    // If the bundle's layout changes on purpose, update the constants above.
    let expected = [main_executable, helper_executable];
    inspect(app, app, &expected)
        .map_err(|problem| format!("The download is not a DegenView app ({problem})."))
}

fn inspect(root: &Path, dir: &Path, expected: &[std::path::PathBuf]) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|error| error.to_string())?;
    for entry in entries.flatten() {
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        if meta.file_type().is_symlink() {
            let target = std::fs::read_link(&path).map_err(|error| error.to_string())?;
            let escapes = target.is_absolute()
                || target
                    .components()
                    .any(|part| matches!(part, std::path::Component::ParentDir));
            if escapes {
                return Err(format!("{relative} links outside the app"));
            }
        } else if meta.is_dir() {
            inspect(root, &path, expected)?;
        } else if is_executable(&meta) && !expected.contains(&path) {
            return Err(format!("unexpected executable {relative}"));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn is_executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_: &std::fs::Metadata) -> bool {
    false
}

/// Test helpers shared with `self_update`'s tests.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::path::PathBuf;

    /// A bundle with the layout `tools/bundle.sh` produces; both executables contain `marker`.
    pub fn fake_app(root: &Path, marker: &str) -> PathBuf {
        let app = root.join("DegenView.app");
        let helper = app.join(HELPER_APP);
        for (bundle, id, executable) in [
            (&app, BUNDLE_ID, MAIN_EXECUTABLE),
            (&helper, HELPER_ID, HELPER_EXECUTABLE),
        ] {
            let path = bundle.join(executable);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, marker).unwrap();
            make_executable(&path);
            std::fs::write(
                bundle.join("Contents/Info.plist"),
                format!("<key>CFBundleIdentifier</key><string>{id}</string>"),
            )
            .unwrap();
        }
        app
    }

    /// A file that passes `check_exe`: the DOS header, a PE signature and `machine`.
    pub fn fake_pe(machine: u16) -> Vec<u8> {
        let mut bytes = vec![0u8; 0x80];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
        bytes[0x40..0x44].copy_from_slice(b"PE\0\0");
        bytes[0x44..0x46].copy_from_slice(&machine.to_le_bytes());
        bytes
    }

    #[cfg(unix)]
    pub fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(not(unix))]
    pub fn make_executable(_: &Path) {}
}

#[cfg(test)]
mod tests {
    use super::testing::{fake_app, fake_pe, make_executable};
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "degenview-update-archive-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A zip that is only a central directory (all `precheck` reads) declaring these entries.
    fn directory_only_zip(entries: &[(&str, u32)]) -> Vec<u8> {
        let mut directory = Vec::new();
        for (name, size) in entries {
            directory.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02]);
            directory.extend_from_slice(&[0; 20]); // versions, flags, method, time, date, crc, compressed size
            directory.extend_from_slice(&size.to_le_bytes());
            directory.extend_from_slice(&(name.len() as u16).to_le_bytes());
            directory.extend_from_slice(&[0; 16]); // extra, comment, disk, attributes, offset
            directory.extend_from_slice(name.as_bytes());
        }
        let mut zip = directory.clone();
        zip.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0]);
        zip.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        zip.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        zip.extend_from_slice(&(directory.len() as u32).to_le_bytes());
        zip.extend_from_slice(&0_u32.to_le_bytes());
        zip.extend_from_slice(&[0, 0]);
        zip
    }

    #[test]
    fn an_ordinary_directory_passes_the_precheck() {
        let zip = directory_only_zip(&[
            ("DegenView.app/", 0),
            ("DegenView.app/Contents/MacOS/DegenView", 30_000_000),
            ("__MACOSX/._DegenView.app", 100),
        ]);
        assert_eq!(precheck(&zip), Ok(()));
    }

    #[test]
    fn bombs_floods_and_traversal_are_refused() {
        let big = directory_only_zip(&[("a", 400_000_000), ("b", 400_000_000)]);
        assert!(precheck(&big).unwrap_err().contains("more data"));
        let names: Vec<String> = (0..=MAX_ENTRIES).map(|i| format!("f{i}")).collect();
        let many: Vec<(&str, u32)> = names.iter().map(|n| (n.as_str(), 1)).collect();
        assert!(precheck(&directory_only_zip(&many)).is_err());
        for name in ["../x", "a/../../x", "/etc/passwd", "a\\b", ""] {
            let zip = directory_only_zip(&[(name, 1)]);
            assert!(precheck(&zip).is_err(), "{name:?}");
        }
        assert!(precheck(&directory_only_zip(&[("a", 0xFFFF_FFFF)])).is_err());
    }

    #[test]
    fn garbage_and_truncated_zips_are_refused() {
        assert!(precheck(b"").is_err());
        assert!(precheck(b"not a zip at all, definitely").is_err());
        let zip = directory_only_zip(&[("a", 1)]);
        assert!(precheck(&zip[..zip.len() - 5]).is_err());
        // A directory offset pointing past the end.
        let mut broken = zip.clone();
        let eocd = broken.len() - 22;
        broken[eocd + 16..eocd + 20].copy_from_slice(&1_000_000_u32.to_le_bytes());
        assert!(precheck(&broken).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_runaway_extraction_is_killed() {
        let root = scratch("runaway");
        let out = root.join("out");
        fs::create_dir_all(&out).unwrap();
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg("dd if=/dev/zero of=\"$0/big\" bs=1048576 count=8 2>/dev/null; sleep 5")
            .arg(&out);
        let started = Instant::now();
        let error =
            run_bounded(&mut command, &out, 1024 * 1024, Duration::from_secs(10)).unwrap_err();
        assert!(error.contains("more data"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(4));
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn a_slow_extraction_times_out_and_a_small_one_succeeds() {
        let root = scratch("timeout");
        let mut slow = Command::new("/bin/sh");
        slow.arg("-c").arg("sleep 5");
        let error = run_bounded(&mut slow, &root, 1024, Duration::from_millis(300)).unwrap_err();
        assert!(error.contains("too long"), "{error}");
        let mut small = Command::new("/bin/sh");
        small.arg("-c").arg("echo hi > \"$0/f\"").arg(&root);
        assert_eq!(
            run_bounded(&mut small, &root, 1024, Duration::from_secs(5)),
            Ok(())
        );
        let mut failing = Command::new("/bin/sh");
        failing.arg("-c").arg("exit 3");
        assert!(run_bounded(&mut failing, &root, 1024, Duration::from_secs(5)).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(not(windows))]
    #[test]
    fn the_real_bundle_layout_passes() {
        let root = scratch("layout-ok");
        fs::write(root.join("note.txt"), "ignored outside the bundle").unwrap();
        let app = fake_app(&root, "x");
        // Plain files (icon, plist) are fine, and so is a relative link inside the bundle.
        fs::create_dir_all(app.join("Contents/Resources")).unwrap();
        fs::write(app.join("Contents/Resources/DegenView.icns"), "icon").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("DegenView.icns", app.join("Contents/Resources/alias")).unwrap();
        assert_eq!(check_bundle(&app), Ok(()));
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(not(windows))]
    #[test]
    fn a_missing_or_wrong_helper_is_refused() {
        let root = scratch("helper");
        let app = fake_app(&root, "x");
        let helper = app.join(HELPER_APP);
        fs::write(
            helper.join("Contents/Info.plist"),
            "<string>com.example.other</string>",
        )
        .unwrap();
        assert!(check_bundle(&app).is_err());
        fs::remove_dir_all(&helper).unwrap();
        assert!(check_bundle(&app).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(not(windows))]
    #[test]
    fn wrong_main_id_and_substring_ids_are_refused() {
        let root = scratch("ids");
        let app = fake_app(&root, "x");
        // The helper's id contains the main id; the main bundle must declare the exact one.
        fs::write(
            app.join("Contents/Info.plist"),
            format!("<string>{HELPER_ID}</string>"),
        )
        .unwrap();
        assert!(check_bundle(&app).is_err());
        assert!(check_bundle(&root.join("Missing.app")).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn extra_executables_and_escaping_links_are_refused() {
        let root = scratch("extras");
        let app = fake_app(&root, "x");
        let extra = app.join("Contents/Resources/payload");
        fs::create_dir_all(extra.parent().unwrap()).unwrap();
        fs::write(&extra, "#!/bin/sh").unwrap();
        make_executable(&extra);
        let error = check_bundle(&app).unwrap_err();
        assert!(error.contains("unexpected executable"), "{error}");
        fs::remove_file(&extra).unwrap();
        assert_eq!(check_bundle(&app), Ok(()));
        for target in ["/etc/passwd", "../../outside"] {
            let link = app.join("Contents/Resources/link");
            let _ = fs::remove_file(&link);
            std::os::unix::fs::symlink(target, &link).unwrap();
            let error = check_bundle(&app).unwrap_err();
            assert!(error.contains("links outside"), "{target}: {error}");
        }
        let _ = fs::remove_dir_all(root);
    }

    // MARK: Windows executable

    fn make_zip(entries: &[(&str, &[u8])], method: zip::CompressionMethod) -> Vec<u8> {
        use std::io::Write;
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default().compression_method(method);
        for (name, body) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(body).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn only_the_executable_is_unpacked_from_a_windows_zip() {
        let root = scratch("exe-extract");
        let exe = fake_pe(expected_machine());
        for method in [
            zip::CompressionMethod::Stored,
            zip::CompressionMethod::Deflated,
        ] {
            let zip = make_zip(
                &[
                    ("OFL-JetBrainsMono.txt", b"licence"),
                    (EXE_NAME, &exe),
                    ("evil.exe", b"MZ"),
                ],
                method,
            );
            precheck(&zip).unwrap();
            let dest = root.join(format!("{method:?}"));
            let path = extract_exe(&zip, &dest).unwrap();
            assert_eq!(fs::read(&path).unwrap(), exe);
            assert_eq!(fs::read_dir(&dest).unwrap().count(), 1);
            assert!(check_exe(&path).is_ok());
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_zip_without_the_executable_or_with_it_nested_is_refused() {
        let root = scratch("exe-missing");
        let exe = fake_pe(expected_machine());
        let method = zip::CompressionMethod::Stored;
        for entries in [
            vec![("README.md", &b"x"[..])],
            vec![("DegenView/degenview.exe", &exe[..])],
            vec![("DEGENVIEW.EXE", &exe[..])],
        ] {
            let zip = make_zip(&entries, method);
            assert!(extract_exe(&zip, &root.join("out")).is_err(), "{entries:?}");
        }
        assert!(extract_exe(b"not a zip", &root.join("out")).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn only_a_pe_image_for_this_cpu_passes() {
        let root = scratch("exe-check");
        let check = |name: &str, bytes: &[u8]| {
            let path = root.join(name);
            fs::write(&path, bytes).unwrap();
            check_exe(&path)
        };
        assert!(check("ok.exe", &fake_pe(expected_machine())).is_ok());
        let other = if expected_machine() == 0x8664 {
            0xAA64
        } else {
            0x8664
        };
        assert!(check("other.exe", &fake_pe(other)).is_err());
        assert!(check("short.exe", b"MZ").is_err());
        assert!(check("elf.exe", &[0x7f, b'E', b'L', b'F', 0, 0, 0, 0]).is_err());
        let mut no_pe = fake_pe(expected_machine());
        no_pe[0x40] = b'X';
        assert!(check("no-pe.exe", &no_pe).is_err());
        let mut far = fake_pe(expected_machine());
        far[0x3C..0x40].copy_from_slice(&0x00FF_FFFFu32.to_le_bytes());
        assert!(check("far.exe", &far).is_err());
        assert!(check_exe(&root.join("missing.exe")).is_err());
        let _ = fs::remove_dir_all(root);
    }

    /// Needs a real release zip: `DEGENVIEW_TEST_WINDOWS_ZIP=<path> cargo test -- --ignored
    /// a_real_windows_release_zip_unpacks`. Reads the PE machine directly rather than `check_exe`,
    /// which compares with the CPU of the machine running the test.
    #[test]
    #[ignore = "needs a downloaded release zip"]
    fn a_real_windows_release_zip_unpacks() {
        let path = std::env::var("DEGENVIEW_TEST_WINDOWS_ZIP").expect("DEGENVIEW_TEST_WINDOWS_ZIP");
        let zip = fs::read(path).unwrap();
        precheck(&zip).unwrap();
        let root = scratch("real-windows-zip");
        let exe = extract_exe(&zip, &root).unwrap();
        assert_eq!(pe_machine(&exe).unwrap(), 0x8664);
        let _ = fs::remove_dir_all(root);
    }
}
