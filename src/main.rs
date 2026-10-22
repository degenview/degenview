// Release builds on Windows are GUI-subsystem apps: no console window behind the UI.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    if let Err(error) = degenview::run_gui_with(icloud()) {
        report_fatal(&error.to_string());
        std::process::exit(1);
    }
}

/// The CloudKit bridge, in builds that include it. The C functions are declared here, in the
/// binary, rather than in the library: the background alert agent shares the library and must
/// not link CloudKit.
#[cfg(all(target_os = "macos", feature = "icloud"))]
fn icloud() -> Option<std::sync::Arc<dyn degenview::TransportFactory>> {
    use std::ffi::c_void;

    unsafe extern "C" {
        fn dv_sync_abi_version() -> u32;
        fn dv_sync_create(
            config: *const u8,
            len: usize,
            callback: unsafe extern "C" fn(*mut c_void, *const u8, usize),
            user: *mut c_void,
        ) -> *mut c_void;
        fn dv_sync_command(handle: *mut c_void, json: *const u8, len: usize);
        fn dv_sync_destroy(handle: *mut c_void);
    }

    let api = degenview::NativeApi {
        abi_version: dv_sync_abi_version,
        create: dv_sync_create,
        command: dv_sync_command,
        destroy: dv_sync_destroy,
    };
    Some(std::sync::Arc::new(degenview::CloudKitFactory::new(api)))
}

#[cfg(not(all(target_os = "macos", feature = "icloud")))]
fn icloud() -> Option<std::sync::Arc<dyn degenview::TransportFactory>> {
    None
}

/// A GUI-subsystem build has no console, so a startup failure would vanish; show it in a dialog.
#[cfg(windows)]
fn report_fatal(message: &str) {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(hwnd: *mut (), text: *const u16, caption: *const u16, kind: u32) -> i32;
    }

    eprintln!("Error: {message}");
    let wide = |text: &str| {
        std::ffi::OsStr::new(text)
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<u16>>()
    };
    let text = wide(message);
    let caption = wide("DegenView failed to start");
    // MB_OK | MB_ICONERROR
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), 0x10) };
}

#[cfg(not(windows))]
fn report_fatal(message: &str) {
    eprintln!("Error: {message}");
}
