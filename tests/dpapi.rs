//! Token storage with Windows DPAPI.

use nokey::security::dpapi;
use std::fs;

const FAKE_TOKEN: &str = "TEST.not-a-real-token.0123456789abcdef";

#[test]
fn protect_unprotect_round_trip() {
    let cipher = dpapi::protect(FAKE_TOKEN.as_bytes()).unwrap();
    assert!(!cipher.windows(FAKE_TOKEN.len()).any(|w| w == FAKE_TOKEN.as_bytes()), "ciphertext must not contain the token");
    assert_eq!(dpapi::unprotect(&cipher).unwrap(), FAKE_TOKEN.as_bytes());
}

#[test]
fn tampered_data_is_rejected() {
    let mut cipher = dpapi::protect(FAKE_TOKEN.as_bytes()).unwrap();
    let last = cipher.len() - 1;
    cipher[last] ^= 0xFF;
    assert!(dpapi::unprotect(&cipher).is_err());
    assert!(dpapi::unprotect(b"garbage").is_err());
}

#[test]
fn token_file_save_load_delete() {
    let dir = std::env::temp_dir().join(format!("nokey-test-dpapi-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(dpapi::load_token(&dir).unwrap(), None);
    dpapi::save_token(&dir, FAKE_TOKEN).unwrap();
    let raw = fs::read(dpapi::token_path(&dir)).unwrap();
    assert!(!String::from_utf8_lossy(&raw).contains(FAKE_TOKEN), "token file must not be plain text");
    assert_eq!(dpapi::load_token(&dir).unwrap().as_deref(), Some(FAKE_TOKEN));
    dpapi::delete_token(&dir).unwrap();
    assert_eq!(dpapi::load_token(&dir).unwrap(), None);
    dpapi::delete_token(&dir).unwrap(); // deleting twice is fine
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn mask_shows_only_the_tail() {
    assert_eq!(dpapi::mask(FAKE_TOKEN), "••••••••cdef");
    assert_eq!(dpapi::mask("short"), "••••••••");
}
