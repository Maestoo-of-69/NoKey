//! "Start with Windows" through the per-user Run key (no admin rights needed).

use super::wide;
use std::ffi::c_void;
use std::ptr::null_mut;
use windows_sys::Win32::System::Registry::*;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "NoKey";
pub const AUTOSTART_ARG: &str = "--autostart";

pub fn command_line(exe: &std::path::Path) -> String {
    format!("\"{}\" {AUTOSTART_ARG}", exe.display())
}

fn current_value() -> Option<String> {
    let key = wide(RUN_KEY);
    let name = wide(VALUE_NAME);
    let mut size = 0u32;
    unsafe {
        let rc = RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, null_mut(), null_mut(), &mut size);
        if rc != 0 || size == 0 {
            return None;
        }
        let mut buf = vec![0u16; size as usize / 2 + 1];
        let rc = RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            buf.as_mut_ptr() as *mut c_void,
            &mut size,
        );
        if rc != 0 {
            return None;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }
}

pub fn is_enabled() -> bool {
    current_value().is_some()
}

/// Turns autostart on or off. Also refreshes the path if the exe moved.
pub fn set_enabled(enabled: bool) -> Result<(), String> {
    let key = wide(RUN_KEY);
    let name = wide(VALUE_NAME);
    if !enabled {
        let rc = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) };
        return if rc == 0 || rc == 2 { Ok(()) } else { Err(format!("Could not remove autostart (error {rc}).")) };
    }
    let exe = std::env::current_exe().map_err(|e| format!("Cannot find NoKey.exe: {e}"))?;
    let cmd = command_line(&exe);
    if current_value().as_deref() == Some(cmd.as_str()) {
        return Ok(());
    }
    let data = wide(&cmd);
    let rc = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            REG_SZ,
            data.as_ptr() as *const c_void,
            (data.len() * 2) as u32,
        )
    };
    if rc == 0 { Ok(()) } else { Err(format!("Could not enable autostart (error {rc}).")) }
}
