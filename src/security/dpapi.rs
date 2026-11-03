//! Windows DPAPI: encrypts data with a key tied to the current Windows user.
//! Another user account (or another PC) cannot decrypt the token file.

use std::fs;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};

const ENTROPY: &[u8] = b"NoKey.bot-token.v1";
pub const TOKEN_FILE: &str = "token.bin";

fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 }
}

unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    unsafe {
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(out.pbData as _);
        v
    }
}

pub fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
    let input = blob(plain);
    let entropy = blob(ENTROPY);
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: null_mut() };
    let ok = unsafe { CryptProtectData(&input, null(), &entropy, null(), null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out) };
    if ok == 0 {
        return Err(format!("Windows could not encrypt the token (error {}).", crate::win::last_error()));
    }
    Ok(unsafe { take(out) })
}

pub fn unprotect(cipher: &[u8]) -> Result<Vec<u8>, String> {
    let input = blob(cipher);
    let entropy = blob(ENTROPY);
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: null_mut() };
    let ok = unsafe {
        CryptUnprotectData(&input, null_mut(), &entropy, null(), null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
    };
    if ok == 0 {
        return Err("The saved token can't be decrypted (it belongs to another Windows user or is damaged).".into());
    }
    Ok(unsafe { take(out) })
}

pub fn token_path(dir: &Path) -> PathBuf {
    dir.join(TOKEN_FILE)
}

pub fn save_token(dir: &Path, token: &str) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    let cipher = protect(token.as_bytes())?;
    fs::write(token_path(dir), cipher).map_err(|e| format!("Cannot save the token: {e}"))
}

/// `Ok(None)` when no token is saved.
pub fn load_token(dir: &Path) -> Result<Option<String>, String> {
    let cipher = match fs::read(token_path(dir)) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Cannot read the token file: {e}")),
    };
    let plain = unprotect(&cipher)?;
    String::from_utf8(plain).map(Some).map_err(|_| "The saved token is damaged.".into())
}

pub fn delete_token(dir: &Path) -> Result<(), String> {
    match fs::remove_file(token_path(dir)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("Cannot delete the token: {e}")),
    }
}

/// Shows only the last 4 characters, e.g. "••••••••x9Qa".
pub fn mask(token: &str) -> String {
    let chars: Vec<char> = token.chars().collect();
    if chars.len() <= 8 {
        return "••••••••".into();
    }
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("••••••••{tail}")
}
