//! Keeps the installer's entry in Settings > Installed apps in step with self-updates (Windows).
//!
//! The Inno Setup installer (`packaging/windows/degenview.iss`) records the version it installed; a
//! self-updated exe swaps the file but not that entry. Compiled everywhere under test so the pure
//! logic is tested on every platform; only the registry calls are Windows-only.
#![cfg_attr(not(windows), allow(dead_code))]

use std::path::Path;

#[cfg(windows)]
use super::portable_binary::plain;

/// The installer's `AppId` (`packaging/windows/degenview.iss`), which Inno Setup also uses to name
/// the per-user uninstall entry. A test keeps the two in step.
const INSTALLER_APP_ID: &str = "79717C79-2D1E-4206-B25A-F3E626438640";

/// Where Inno Setup records a per-user install, under `HKEY_CURRENT_USER`.
fn uninstall_key_path() -> String {
    format!(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{{{INSTALLER_APP_ID}}}_is1")
}

/// Whether the installer's entry should be rewritten with `version`: it describes the install in
/// this exe's folder (`app_path` is what Inno recorded), and shows another version. A portable copy
/// elsewhere must not edit the entry of a separate installed one.
fn needs_version_sync(app_path: &str, exe: &Path, shown: Option<&str>, version: &str) -> bool {
    // Plain string work on purpose: a Windows path is only split by `Path` on Windows, and this is
    // tested everywhere.
    let exe = exe.to_string_lossy();
    let trim = |path: &str| path.trim_end_matches(['\\', '/']).to_ascii_lowercase();
    exe.rsplit_once(['\\', '/'])
        .is_some_and(|(folder, _)| trim(app_path) == trim(folder))
        && shown != Some(version)
}

/// Run once the app has started: after a self-update the installer's entry in Settings > Installed
/// apps still shows the version that was installed, so it is brought up to date. Does nothing for a
/// portable copy (no entry) or off Windows.
#[cfg(windows)]
pub fn sync_installed_version() {
    let Ok(exe) = std::env::current_exe().and_then(|exe| exe.canonicalize()) else {
        return;
    };
    registry::sync_version_at(
        registry::CURRENT_USER,
        &uninstall_key_path(),
        &plain(&exe),
        env!("CARGO_PKG_VERSION"),
    );
}

/// The few registry calls this needs, declared by hand like the other Win32 calls (no crate).
#[cfg(windows)]
mod registry {
    use std::path::Path;
    use std::ptr::null_mut;

    type Hkey = isize;
    /// `HKEY_CURRENT_USER`, sign-extended like the `(HKEY)(LONG)0x80000001` the header defines.
    pub const CURRENT_USER: Hkey = 0x8000_0001u32 as i32 as isize;
    const KEY_QUERY_VALUE: u32 = 0x0001;
    const KEY_SET_VALUE: u32 = 0x0002;
    const REG_SZ: u32 = 1;

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegOpenKeyExW(
            key: Hkey,
            sub_key: *const u16,
            options: u32,
            access: u32,
            out: *mut Hkey,
        ) -> i32;
        fn RegQueryValueExW(
            key: Hkey,
            name: *const u16,
            reserved: *mut u32,
            kind: *mut u32,
            data: *mut u8,
            len: *mut u32,
        ) -> i32;
        fn RegSetValueExW(
            key: Hkey,
            name: *const u16,
            reserved: u32,
            kind: u32,
            data: *const u8,
            len: u32,
        ) -> i32;
        fn RegCloseKey(key: Hkey) -> i32;
    }

    struct Key(Hkey);

    impl Drop for Key {
        fn drop(&mut self) {
            unsafe { RegCloseKey(self.0) };
        }
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    fn open(root: Hkey, path: &str) -> Option<Key> {
        let mut key = 0;
        let status = unsafe {
            RegOpenKeyExW(
                root,
                wide(path).as_ptr(),
                0,
                KEY_QUERY_VALUE | KEY_SET_VALUE,
                &mut key,
            )
        };
        (status == 0).then_some(Key(key))
    }

    fn read(key: &Key, name: &str) -> Option<String> {
        let name = wide(name);
        let (mut kind, mut len) = (0u32, 0u32);
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                null_mut(),
                &mut kind,
                null_mut(),
                &mut len,
            )
        };
        if status != 0 || kind != REG_SZ {
            return None;
        }
        // Room for the terminator the registry does not promise to store.
        let mut buffer = vec![0u16; len as usize / 2 + 1];
        let mut len = (buffer.len() * 2) as u32;
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                null_mut(),
                &mut kind,
                buffer.as_mut_ptr().cast(),
                &mut len,
            )
        };
        if status != 0 {
            return None;
        }
        let units = (len as usize / 2).min(buffer.len());
        Some(
            String::from_utf16_lossy(&buffer[..units])
                .trim_end_matches('\0')
                .to_owned(),
        )
    }

    fn write(key: &Key, name: &str, value: &str) -> bool {
        let (name, data) = (wide(name), wide(value));
        let status = unsafe {
            RegSetValueExW(
                key.0,
                name.as_ptr(),
                0,
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            )
        };
        status == 0
    }

    /// Sets `DisplayVersion` under `root\path` to `version` when `super::needs_version_sync` says
    /// so. Returns whether it wrote. A missing key is not an error: it is a portable copy.
    pub fn sync_version_at(root: Hkey, path: &str, exe: &Path, version: &str) -> bool {
        let Some(key) = open(root, path) else {
            return false;
        };
        let Some(app_path) = read(&key, "Inno Setup: App Path") else {
            return false;
        };
        let shown = read(&key, "DisplayVersion");
        super::needs_version_sync(&app_path, exe, shown.as_deref(), version)
            && write(&key, "DisplayVersion", version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_installer_script_and_the_app_agree_on_the_app_id() {
        let script = include_str!("../../../packaging/windows/degenview.iss");
        assert!(
            script.contains(&format!("AppId={{{{{INSTALLER_APP_ID}}}")),
            "packaging/windows/degenview.iss must declare AppId={{{{{INSTALLER_APP_ID}}}"
        );
        assert!(uninstall_key_path().ends_with(&format!(r"\{{{INSTALLER_APP_ID}}}_is1")));
    }

    #[test]
    fn only_the_install_in_this_exes_folder_has_its_version_synced() {
        let exe = Path::new(r"C:\Users\me\AppData\Local\Programs\DegenView\degenview.exe");
        let installed = r"C:\Users\me\AppData\Local\Programs\DegenView";
        let sync = |app_path: &str, exe: &Path, shown: Option<&str>| {
            needs_version_sync(app_path, exe, shown, "0.6.1")
        };
        // Same folder, other version: sync (case and a trailing slash do not matter).
        assert!(sync(installed, exe, Some("0.6.0")));
        assert!(sync(&format!("{}\\", installed.to_uppercase()), exe, None));
        assert!(!sync(installed, exe, Some("0.6.1")));
        // A portable copy elsewhere leaves the installed entry alone.
        assert!(!sync(
            installed,
            Path::new(r"D:\Tools\degenview.exe"),
            Some("0.6.0")
        ));
    }

    /// Windows only: the real registry, through a scratch key under `HKEY_CURRENT_USER` made with
    /// `reg.exe`.
    #[cfg(windows)]
    #[test]
    fn the_installed_version_follows_the_running_exe() {
        use std::process::Command;

        let path = format!(r"Software\DegenViewTest-{}", std::process::id());
        let full = format!(r"HKCU\{path}");
        let reg = |args: &[&str]| {
            let output = Command::new("reg").args(args).output().unwrap();
            assert!(output.status.success(), "reg {args:?}: {output:?}");
            String::from_utf8_lossy(&output.stdout).into_owned()
        };
        let sync = |exe: &str| {
            registry::sync_version_at(registry::CURRENT_USER, &path, Path::new(exe), "2.0.0")
        };
        reg(&[
            "add",
            &full,
            "/v",
            "Inno Setup: App Path",
            "/d",
            r"C:\Apps\DegenView",
            "/f",
        ]);
        reg(&["add", &full, "/v", "DisplayVersion", "/d", "1.0.0", "/f"]);

        // Another folder's exe does not touch it.
        assert!(!sync(r"C:\Elsewhere\degenview.exe"));
        assert!(reg(&["query", &full, "/v", "DisplayVersion"]).contains("1.0.0"));
        // This folder's does, once.
        assert!(sync(r"C:\Apps\DegenView\degenview.exe"));
        assert!(reg(&["query", &full, "/v", "DisplayVersion"]).contains("2.0.0"));
        assert!(!sync(r"C:\Apps\DegenView\degenview.exe"));
        // No key at all (a portable copy) is not an error.
        reg(&["delete", &full, "/f"]);
        assert!(!sync(r"C:\Apps\DegenView\degenview.exe"));
    }
}
