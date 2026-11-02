//! Unpacking and inspecting a downloaded update, bounded so a hostile zip can't fill the disk or
//! plant extra launchable code: the zip's directory is checked before extraction, the extraction
//! itself is size- and time-limited, and the unpacked bundle must match the app's known layout.
//!
//! All of this is structural. It cannot prove the binaries are genuine; that comes from the signed
//! tag, the immutable release and the SHA-256 checked before anything is unpacked, and from the
//! Developer ID signature check in `code_signature`.
// The macOS bundle checks and `ditto` extraction below are unused on Windows and Linux.
#![cfg_attr(any(windows, target_os = "linux"), allow(dead_code))]

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
        match directory_bytes(watch) {
            Ok(bytes) if bytes <= max_bytes => {}
            Ok(_) => return fail(&mut child, "it unpacks to more data than an update should"),
            // A size that cannot be measured cannot be bounded: stop rather than let it grow.
            Err(error) => return fail(&mut child, &format!("could not measure it ({error})")),
        }
        if started.elapsed() > timeout {
            return fail(&mut child, "unpacking took too long");
        }
        std::thread::sleep(POLL);
    }
}

/// Total size of the regular files under `dir` (symlinks are not followed). A folder that does not
/// exist yet counts as empty (the extractor has not made it); any other read error is reported, so
/// a tree that cannot be fully measured is never taken to be small.
fn directory_bytes(dir: &Path) -> std::io::Result<u64> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    let mut total = 0_u64;
    for entry in entries {
        let entry = entry?;
        let meta = match entry.metadata() {
            Ok(meta) => meta,
            // The extractor renamed or removed a temporary file between listing and reading.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if meta.is_dir() {
            total = total.saturating_add(directory_bytes(&entry.path())?);
        } else if meta.is_file() {
            total = total.saturating_add(meta.len());
        }
    }
    Ok(total)
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
    // Two entries of one name would leave it to the reader which one is "the" executable.
    if archive.has_duplicate_names() {
        return Err("The download contains the same file twice.".to_owned());
    }
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

// MARK: Linux binary

/// The one file a Linux update replaces: the release tarball carries it at its root (as
/// `./degenview`), next to the icon, `.desktop` file and licence texts, which are left alone.
#[cfg(any(target_os = "linux", test))]
pub const BINARY_NAME: &str = "degenview";

/// Inflates `BINARY_NAME` from the gzipped tarball in `bytes` into `dest_dir` (mode 0755), refusing
/// more than `MAX_UNPACKED_BYTES` of decompressed data in total, more than `MAX_ENTRIES` entries,
/// an unsafe entry name anywhere, and a `BINARY_NAME` that is not a regular file or appears twice.
/// The output name is fixed, so nothing in the archive picks where a file lands.
#[cfg(any(target_os = "linux", test))]
pub fn extract_tar_gz_binary(bytes: &[u8], dest_dir: &Path) -> Result<std::path::PathBuf, String> {
    extract_tar_gz_binary_within(bytes, dest_dir, MAX_UNPACKED_BYTES)
}

/// `extract_tar_gz_binary` with the decompressed-size limit as a parameter, so a test can hit it
/// without inflating half a gigabyte.
#[cfg(any(target_os = "linux", test))]
fn extract_tar_gz_binary_within(
    bytes: &[u8],
    dest_dir: &Path,
    max_bytes: u64,
) -> Result<std::path::PathBuf, String> {
    use std::io::Read;

    let failed = |error: &dyn std::fmt::Display| format!("Could not unpack the download: {error}");
    let not_degenview = || "The download is not a DegenView app.".to_owned();
    if bytes.get(..2) != Some(&[0x1f, 0x8b]) {
        return Err("The download is not a valid archive.".to_owned());
    }
    // Declared sizes can lie, and so can compression ratios: bound what actually comes out.
    let stream = flate2::read::GzDecoder::new(bytes).take(max_bytes);
    let mut archive = tar::Archive::new(stream);
    let mut written = None;
    for (index, entry) in archive.entries().map_err(|e| failed(&e))?.enumerate() {
        if index >= MAX_ENTRIES {
            return Err("The download has an unexpected number of files.".to_owned());
        }
        let mut entry = entry.map_err(|e| failed(&e))?;
        let path = entry.path().map_err(|e| failed(&e))?;
        let name = path
            .to_str()
            .ok_or("The download contains an unsafe file path.")?;
        if !is_safe_entry_name(name.as_bytes()) {
            return Err("The download contains an unsafe file path.".to_owned());
        }
        if name.trim_start_matches("./") != BINARY_NAME {
            continue;
        }
        if written.is_some() || !entry.header().entry_type().is_file() {
            return Err(not_degenview());
        }
        std::fs::create_dir_all(dest_dir).map_err(|e| failed(&e))?;
        let dest = dest_dir.join(BINARY_NAME);
        let mut out = std::fs::File::create(&dest).map_err(|e| failed(&e))?;
        std::io::copy(&mut (&mut entry).take(max_bytes), &mut out).map_err(|e| failed(&e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| failed(&e))?;
        }
        written = Some(dest);
    }
    written.ok_or_else(not_degenview)
}

/// The ELF `e_machine` of the running build, which an update for another CPU must not replace.
#[cfg(any(target_os = "linux", test))]
const fn expected_elf_machine() -> u16 {
    if cfg!(target_arch = "aarch64") {
        0xB7
    } else {
        0x3E
    }
}

/// The ELF `e_machine` of `path`, once its header says it is a 64-bit little-endian ELF executable
/// or PIE.
#[cfg(any(target_os = "linux", test))]
fn elf_machine(path: &Path) -> Result<u16, String> {
    use std::io::Read;

    let not_degenview = || "The download is not a DegenView app.".to_owned();
    let mut header = [0u8; 20];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .map_err(|_| not_degenview())?;
    let kind = u16::from_le_bytes([header[16], header[17]]);
    // ELF magic, ELFCLASS64, ELFDATA2LSB; ET_EXEC or ET_DYN.
    if header[..4] != *b"\x7fELF" || header[4] != 2 || header[5] != 1 || !matches!(kind, 2 | 3) {
        return Err(not_degenview());
    }
    Ok(u16::from_le_bytes([header[18], header[19]]))
}

/// Structural check of an unpacked Linux binary: a 64-bit little-endian ELF executable (or PIE) for
/// this CPU. It cannot prove the file is genuine; that is the signed tag, the immutable release and
/// the SHA-256.
#[cfg(any(target_os = "linux", test))]
pub fn check_elf(path: &Path) -> Result<(), String> {
    if elf_machine(path)? != expected_elf_machine() {
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
    for entry in entries {
        let path = entry.map_err(|error| error.to_string())?.path();
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

    /// A file that passes `check_elf`: a 64-bit little-endian ELF header with `machine`.
    pub fn fake_elf(machine: u16) -> Vec<u8> {
        let mut bytes = vec![0u8; 64];
        bytes[..4].copy_from_slice(b"\x7fELF");
        bytes[4] = 2;
        bytes[5] = 1;
        bytes[16..18].copy_from_slice(&3u16.to_le_bytes());
        bytes[18..20].copy_from_slice(&machine.to_le_bytes());
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
    use super::testing::{fake_app, fake_elf, fake_pe, make_executable};
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
    fn a_zip_with_the_executable_twice_is_refused() {
        let root = scratch("exe-twice");
        let exe = fake_pe(expected_machine());
        // The writer refuses two entries of one name, so write a same-length twin and rename it
        // in both the local header and the central directory.
        let twin = EXE_NAME.replace(".exe", ".exf");
        let zip = make_zip(
            &[(EXE_NAME, &exe), (&twin, &exe)],
            zip::CompressionMethod::Stored,
        );
        let mut doubled = zip.clone();
        let (from, to) = (twin.as_bytes(), EXE_NAME.as_bytes());
        let mut at = 0;
        while let Some(found) = doubled[at..].windows(from.len()).position(|w| w == from) {
            doubled[at + found..at + found + from.len()].copy_from_slice(to);
            at += found + from.len();
        }
        assert!(extract_exe(&zip, &root.join("fine")).is_ok());
        let error = extract_exe(&doubled, &root.join("out")).unwrap_err();
        assert!(error.contains("twice"), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn an_unreadable_tree_is_not_measured_as_small() {
        let root = scratch("measure");
        // Not made yet: empty.
        assert_eq!(directory_bytes(&root.join("later")).unwrap(), 0);
        let sub = root.join("a");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("file"), [0_u8; 100]).unwrap();
        fs::write(root.join("top"), [0_u8; 50]).unwrap();
        assert_eq!(directory_bytes(&root).unwrap(), 150);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&sub, fs::Permissions::from_mode(0o000)).unwrap();
            let readable = fs::read_dir(&sub).is_ok(); // running as root ignores the mode
            let result = directory_bytes(&root);
            fs::set_permissions(&sub, fs::Permissions::from_mode(0o755)).unwrap();
            assert!(readable || result.is_err(), "{result:?}");
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

    // MARK: Linux binary

    enum Kind {
        File,
        Symlink,
    }

    /// A gzipped tarball with the given entries. Names go into the header unchecked (the `tar`
    /// crate refuses to write `..` itself, which is exactly what some tests need to write).
    fn make_targz(entries: &[(&str, &[u8], Kind)]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, body, kind) in entries {
            let mut header = tar::Header::new_gnu();
            header.as_old_mut().name[..name.len()].copy_from_slice(name.as_bytes());
            header.set_mode(0o644);
            match kind {
                Kind::File => {
                    header.set_entry_type(tar::EntryType::Regular);
                    header.set_size(body.len() as u64);
                }
                Kind::Symlink => {
                    header.set_entry_type(tar::EntryType::Symlink);
                    header.set_link_name("/bin/sh").unwrap();
                    header.set_size(0);
                }
            }
            header.set_cksum();
            builder.append(&header, *body).unwrap();
        }
        let tar = builder.into_inner().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &tar).unwrap();
        gz.finish().unwrap()
    }

    #[test]
    fn only_the_binary_is_unpacked_from_a_linux_tarball() {
        let root = scratch("tar-extract");
        let elf = fake_elf(expected_elf_machine());
        // The release tarball is made with `tar -czf ... -C pkg .`: every name starts with `./`.
        let targz = make_targz(&[
            ("./", b"", Kind::File),
            ("./degenview.desktop", b"[Desktop Entry]", Kind::File),
            ("./degenview", &elf, Kind::File),
            ("./licenses/FONTS.md", b"fonts", Kind::File),
            ("./degenview.png", b"png", Kind::File),
        ]);
        let path = extract_tar_gz_binary(&targz, &root.join("out")).unwrap();
        assert_eq!(fs::read(&path).unwrap(), elf);
        assert_eq!(fs::read_dir(root.join("out")).unwrap().count(), 1);
        assert!(check_elf(&path).is_ok());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o755
            );
        }
        // A tarball without the leading `./` works too.
        let plain = make_targz(&[("degenview", &elf, Kind::File)]);
        assert!(extract_tar_gz_binary(&plain, &root.join("plain")).is_ok());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_tarball_without_the_binary_or_with_it_nested_or_doubled_is_refused() {
        let root = scratch("tar-missing");
        let elf = fake_elf(expected_elf_machine());
        for entries in [
            vec![("./README.md", &b"x"[..], Kind::File)],
            vec![("./bin/degenview", &elf[..], Kind::File)],
            vec![("./DEGENVIEW", &elf[..], Kind::File)],
            vec![
                ("./degenview", &elf[..], Kind::File),
                ("degenview", &elf[..], Kind::File),
            ],
            vec![("./degenview", &b""[..], Kind::Symlink)],
        ] {
            let targz = make_targz(&entries);
            assert!(extract_tar_gz_binary(&targz, &root.join("out")).is_err());
        }
        assert!(extract_tar_gz_binary(b"not gzip", &root.join("out")).is_err());
        assert!(extract_tar_gz_binary(&[0x1f, 0x8b, 0, 0], &root.join("out")).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unsafe_names_and_floods_anywhere_in_the_tarball_are_refused() {
        let root = scratch("tar-unsafe");
        let elf = fake_elf(expected_elf_machine());
        for name in ["../evil", "/etc/passwd", "./a/../../b"] {
            let targz = make_targz(&[("./degenview", &elf, Kind::File), (name, b"x", Kind::File)]);
            assert!(
                extract_tar_gz_binary(&targz, &root.join("out")).is_err(),
                "{name}"
            );
        }
        let many: Vec<_> = (0..=MAX_ENTRIES)
            .map(|index| (format!("./f{index}"), Vec::new(), Kind::File))
            .collect();
        let entries: Vec<_> = many
            .iter()
            .map(|(name, body, kind)| {
                (
                    name.as_str(),
                    body.as_slice(),
                    match kind {
                        Kind::File => Kind::File,
                        Kind::Symlink => Kind::Symlink,
                    },
                )
            })
            .collect();
        assert!(extract_tar_gz_binary(&make_targz(&entries), &root.join("out")).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_tarball_that_inflates_past_the_limit_is_cut_off() {
        let root = scratch("tar-bomb");
        let elf = fake_elf(expected_elf_machine());
        // 256 KiB of zeros gzips to a few hundred bytes; with the limit at 64 KiB it must fail,
        // wherever in the archive the binary sits.
        let filler = vec![0u8; 256 * 1024];
        for entries in [
            vec![
                ("./filler", &filler[..], Kind::File),
                ("./degenview", &elf[..], Kind::File),
            ],
            vec![
                ("./degenview", &elf[..], Kind::File),
                ("./filler", &filler[..], Kind::File),
            ],
        ] {
            let targz = make_targz(&entries);
            assert!(targz.len() < 4096, "the fixture must be a small bomb");
            let out = root.join("out");
            assert!(extract_tar_gz_binary_within(&targz, &out, 64 * 1024).is_err());
            assert!(extract_tar_gz_binary_within(&targz, &out, 1024 * 1024).is_ok());
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn only_an_elf_for_this_cpu_passes() {
        let root = scratch("elf-check");
        let check = |name: &str, bytes: &[u8]| {
            let path = root.join(name);
            fs::write(&path, bytes).unwrap();
            check_elf(&path)
        };
        assert!(check("ok", &fake_elf(expected_elf_machine())).is_ok());
        let other = if expected_elf_machine() == 0x3E {
            0xB7
        } else {
            0x3E
        };
        assert!(check("other-cpu", &fake_elf(other)).is_err());
        assert!(check("script", b"#!/bin/sh\nexit 0\n#################").is_err());
        assert!(check("pe", &fake_pe(0x8664)).is_err());
        assert!(check("short", b"\x7fELF").is_err());
        let mut thirty_two_bit = fake_elf(expected_elf_machine());
        thirty_two_bit[4] = 1;
        assert!(check("elf32", &thirty_two_bit).is_err());
        let mut big_endian = fake_elf(expected_elf_machine());
        big_endian[5] = 2;
        assert!(check("big-endian", &big_endian).is_err());
        let mut relocatable = fake_elf(expected_elf_machine());
        relocatable[16] = 1;
        assert!(check("object", &relocatable).is_err());
        assert!(check_elf(&root.join("missing")).is_err());
        let _ = fs::remove_dir_all(root);
    }

    /// Needs a real release tarball: `DEGENVIEW_TEST_LINUX_TARBALL=<path> cargo test -- --ignored
    /// a_real_linux_release_tarball_unpacks`. The CPU is read from the file name (`-arm64` or
    /// `-x64`); `check_elf` is not used because it compares with the machine running the test.
    #[test]
    #[ignore = "needs a downloaded release tarball"]
    fn a_real_linux_release_tarball_unpacks() {
        let path =
            std::env::var("DEGENVIEW_TEST_LINUX_TARBALL").expect("DEGENVIEW_TEST_LINUX_TARBALL");
        let expected = if path.contains("arm64") { 0xB7 } else { 0x3E };
        let targz = fs::read(&path).unwrap();
        let root = scratch("real-linux-tarball");
        let binary = extract_tar_gz_binary(&targz, &root).unwrap();
        assert_eq!(elf_machine(&binary).unwrap(), expected);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        let _ = fs::remove_dir_all(root);
    }
}
