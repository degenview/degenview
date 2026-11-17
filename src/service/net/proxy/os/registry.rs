//! The per-user Internet Settings key, where Windows apps (and VPN clients in "system proxy"
//! mode) publish the proxy. A PAC script (`AutoConfigURL`) is not followed.

use super::SystemProxy;
use super::server_list;
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW,
};

const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn number(name: &str) -> Option<u32> {
    let (key, name) = (wide(KEY), wide(name));
    let mut value = 0_u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: both strings are NUL-terminated, and `value` is a DWORD-sized buffer that `size` describes.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            std::ptr::from_mut(&mut value).cast(),
            &mut size,
        )
    };
    (status == ERROR_SUCCESS).then_some(value)
}

fn text(name: &str) -> Option<String> {
    let (key, name) = (wide(KEY), wide(name));
    let mut size = 0_u32;
    // SAFETY: a null data pointer asks only for the size in bytes, including the terminator.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS || size == 0 {
        return None;
    }
    let mut buffer = vec![0_u16; (size as usize).div_ceil(2)];
    let mut size = (buffer.len() * 2) as u32;
    // SAFETY: `buffer` holds `size` bytes, as the first call reported.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let length = buffer
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..length]))
}

pub fn read() -> Option<SystemProxy> {
    if number("ProxyEnable")? != 1 {
        return None;
    }
    let spec = server_list::choose(&text("ProxyServer")?)?;
    let bypass = text("ProxyOverride").map_or_else(Vec::new, |value| server_list::bypass(&value));
    Some(SystemProxy { spec, bypass })
}
