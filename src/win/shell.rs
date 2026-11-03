//! Opening apps, files, folders and URLs (ShellExecute), plus file/folder
//! pickers. No command interpreter is ever invoked.

use super::wide;
use std::path::Path;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows_sys::Win32::UI::Controls::Dialogs::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// Why ShellExecute could not open something (shown translated by the UI).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellFail {
    FileNotFound,
    FolderNotFound,
    AccessDenied,
    InUse,
    NoAssociation,
    Other(isize),
}

impl ShellFail {
    fn from_code(code: isize) -> Self {
        match code {
            2 => ShellFail::FileNotFound,
            3 => ShellFail::FolderNotFound,
            5 => ShellFail::AccessDenied,
            26 | 32 => ShellFail::InUse,
            27 | 31 => ShellFail::NoAssociation,
            other => ShellFail::Other(other),
        }
    }
}

/// Runs ShellExecute "open" (for URLs this is the default browser). Call
/// from a worker thread: opening a network path can take a while and must
/// not freeze the window.
pub fn open(target: &str, args: &str) -> Result<(), ShellFail> {
    unsafe { CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32) };
    let verb = wide("open");
    let file = wide(target);
    let params = wide(args);
    let p = if args.is_empty() { null() } else { params.as_ptr() };
    let rc = unsafe { ShellExecuteW(null_mut(), verb.as_ptr(), file.as_ptr(), p, null(), SW_SHOWNORMAL) } as isize;
    unsafe { CoUninitialize() };
    if rc > 32 { Ok(()) } else { Err(ShellFail::from_code(rc)) }
}

pub fn open_existing_file(path: &str, args: &str) -> Result<(), ShellFail> {
    if !Path::new(path).is_file() {
        return Err(ShellFail::FileNotFound);
    }
    open(path, args)
}

pub fn open_folder(path: &str) -> Result<(), ShellFail> {
    if !Path::new(path).is_dir() {
        return Err(ShellFail::FolderNotFound);
    }
    open(path, "")
}

/// Shows the standard "Open" dialog. `filter` pairs: ("Programs", "*.exe;*.lnk").
pub fn pick_file(owner: HWND, filter: &[(&str, &str)]) -> Option<String> {
    let mut filter_w: Vec<u16> = Vec::new();
    for (name, pattern) in filter {
        filter_w.extend(name.encode_utf16());
        filter_w.push(0);
        filter_w.extend(pattern.encode_utf16());
        filter_w.push(0);
    }
    filter_w.push(0);
    let mut buf = vec![0u16; 1024];
    let mut ofn: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = owner;
    ofn.lpstrFilter = filter_w.as_ptr();
    ofn.lpstrFile = buf.as_mut_ptr();
    ofn.nMaxFile = buf.len() as u32;
    // NODEREFERENCELINKS: picking a .lnk keeps the shortcut itself.
    ofn.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | OFN_NODEREFERENCELINKS | OFN_EXPLORER;
    if unsafe { GetOpenFileNameW(&mut ofn) } == 0 {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
    Some(String::from_utf16_lossy(&buf[..len]))
}

pub fn pick_folder(owner: HWND) -> Option<String> {
    let title = wide("Choose a folder");
    let mut bi: BROWSEINFOW = unsafe { std::mem::zeroed() };
    bi.hwndOwner = owner;
    bi.lpszTitle = title.as_ptr();
    bi.ulFlags = BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE;
    unsafe {
        let pidl = SHBrowseForFolderW(&bi);
        if pidl.is_null() {
            return None;
        }
        let mut buf = vec![0u16; 1024];
        let ok = SHGetPathFromIDListW(pidl, buf.as_mut_ptr());
        CoTaskMemFree(pidl as *const _);
        if ok == 0 {
            return None;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
        Some(String::from_utf16_lossy(&buf[..len]))
    }
}
