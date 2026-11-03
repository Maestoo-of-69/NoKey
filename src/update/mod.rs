//! Update check through GitHub Releases, with SHA-256 verification and a
//! safe restart-and-replace. Nothing is downloaded or replaced without the
//! user confirming it.
//!
//! A release must contain two assets: `NoKey.exe` and `NoKey.exe.sha256`
//! (the hex digest, optionally followed by the file name).

use crate::net::http;
use serde::Deserialize;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Security::Cryptography::{BCryptHash, BCRYPT_SHA256_ALG_HANDLE};

pub const EXE_ASSET: &str = "NoKey.exe";
pub const SHA_ASSET: &str = "NoKey.exe.sha256";
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const MAX_EXE_SIZE: u64 = 50 * 1024 * 1024;

pub fn is_valid_repo(repo: &str) -> bool {
    let ok_part = |p: &str| {
        !p.is_empty() && p.len() <= 100 && p.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    };
    match repo.split_once('/') {
        // GitHub user/org names never contain dots; repo names may, but not "." or "..".
        Some((owner, name)) => ok_part(owner) && !owner.contains('.') && ok_part(name) && name != "." && name != "..",
        None => false,
    }
}

pub fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let core = s.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u32>());
    let major = parts.next()?.ok()?;
    let minor = parts.next().unwrap_or(Ok(0)).ok()?;
    let patch = parts.next().unwrap_or(Ok(0)).ok()?;
    Some((major, minor, patch))
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Clone, Debug)]
pub struct ReleaseInfo {
    pub version: String,
    pub notes: String,
    exe_url: String,
    sha_url: String,
    size: u64,
}

const GH_HEADERS: [(&str, &str); 3] = [
    ("User-Agent", "NoKey-Updater"),
    ("Accept", "application/vnd.github+json"),
    ("X-GitHub-Api-Version", "2022-11-28"),
];

/// `Ok(None)` when the installed version is the latest.
pub fn check(repo: &str) -> Result<Option<ReleaseInfo>, String> {
    if !is_valid_repo(repo) {
        return Err("Set the update repository as owner/name in Settings.".into());
    }
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let resp = http::request("GET", &url, &GH_HEADERS, None)?;
    match resp.status {
        200 => {}
        404 => return Err(format!("No published releases found for {repo}.")),
        403 | 429 => return Err("GitHub rate limit reached. Try again later.".into()),
        s => return Err(format!("GitHub returned HTTP {s}.")),
    }
    let rel: GhRelease = serde_json::from_slice(&resp.body).map_err(|e| format!("Unexpected GitHub response: {e}"))?;
    if !is_newer(&rel.tag_name, CURRENT_VERSION) {
        return Ok(None);
    }
    let find = |name: &str| rel.assets.iter().find(|a| a.name.eq_ignore_ascii_case(name));
    let exe = find(EXE_ASSET).ok_or_else(|| format!("Release {} has no {EXE_ASSET} file.", rel.tag_name))?;
    let sha = find(SHA_ASSET)
        .ok_or_else(|| format!("Release {} has no {SHA_ASSET} file, so it can't be verified.", rel.tag_name))?;
    // Only download from this repository's own release files.
    for url in [&exe.browser_download_url, &sha.browser_download_url] {
        if !is_release_url(url, repo) {
            return Err(format!("Release {} points to an unexpected download location.", rel.tag_name));
        }
    }
    Ok(Some(ReleaseInfo {
        version: rel.tag_name.clone(),
        notes: rel.body.clone().unwrap_or_default(),
        exe_url: exe.browser_download_url.clone(),
        sha_url: sha.browser_download_url.clone(),
        size: exe.size,
    }))
}

pub fn is_release_url(url: &str, repo: &str) -> bool {
    let prefix = format!("https://github.com/{repo}/releases/download/").to_ascii_lowercase();
    url.to_ascii_lowercase().starts_with(&prefix) && !url.contains("..")
}

pub fn sha256(data: &[u8]) -> Result<[u8; 32], String> {
    let mut out = [0u8; 32];
    let status = unsafe {
        BCryptHash(BCRYPT_SHA256_ALG_HANDLE, std::ptr::null(), 0, data.as_ptr(), data.len() as u32, out.as_mut_ptr(), 32)
    };
    if status != 0 {
        return Err(format!("SHA-256 failed (status {status:#x})"));
    }
    Ok(out)
}

pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Accepts "abc123…" or "abc123…  NoKey.exe" (sha256sum format).
pub fn parse_sha_file(text: &str) -> Option<String> {
    let first = text.split_whitespace().next()?.to_ascii_lowercase();
    (first.len() == 64 && first.bytes().all(|b| b.is_ascii_hexdigit())).then_some(first)
}

fn update_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("NoKey").join("update")
}

/// A verified download, ready to install.
#[derive(Clone, Debug)]
pub struct Downloaded {
    pub path: PathBuf,
    sha256: String,
}

/// Downloads the new exe and verifies its SHA-256.
pub fn download(info: &ReleaseInfo) -> Result<Downloaded, String> {
    let dir = update_dir();
    let path = dir.join("NoKey.exe.new");
    let result = download_to(info, &dir, &path);
    if result.is_err() {
        let _ = fs::remove_file(&path); // never leave a partial or rejected file behind
    }
    result
}

fn download_to(info: &ReleaseInfo, dir: &Path, path: &Path) -> Result<Downloaded, String> {
    if info.size > MAX_EXE_SIZE {
        return Err("The update file is unexpectedly large; refusing to download it.".into());
    }
    let sha_resp = http::request("GET", &info.sha_url, &[("User-Agent", "NoKey-Updater")], None)?;
    if sha_resp.status != 200 {
        return Err(format!("Could not download the checksum (HTTP {}).", sha_resp.status));
    }
    let expected = parse_sha_file(&String::from_utf8_lossy(&sha_resp.body))
        .ok_or("The checksum file is not a valid SHA-256 value.")?;

    fs::create_dir_all(dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    let resp = {
        // The block closes the file before it is hashed below.
        let mut file = fs::File::create(path).map_err(|e| format!("Cannot write the update: {e}"))?;
        let mut received = 0u64;
        let mut sink = |chunk: &[u8]| {
            received += chunk.len() as u64;
            if received > MAX_EXE_SIZE {
                return Err("The update file is unexpectedly large; download stopped.".to_string());
            }
            file.write_all(chunk).map_err(|e| format!("Cannot write the update: {e}"))
        };
        http::request_with("GET", &info.exe_url, &[("User-Agent", "NoKey-Updater")], None, Some(&mut sink))?
    };
    if resp.status != 200 {
        return Err(format!("Download failed (HTTP {}).", resp.status));
    }
    let data = fs::read(path).map_err(|e| e.to_string())?;
    if to_hex(&sha256(&data)?) != expected {
        return Err("The downloaded file does not match its checksum. It was deleted and nothing was changed.".into());
    }
    if !data.starts_with(b"MZ") {
        return Err("The downloaded file is not a Windows program.".into());
    }
    Ok(Downloaded { path: path.to_path_buf(), sha256: expected })
}

fn old_exe_path(exe: &Path) -> PathBuf {
    exe.with_extension("exe.old")
}

/// Replaces the running exe and starts the new one. Windows can't overwrite
/// a running exe, but it can rename it: current -> .old, new -> current.
/// On any failure the original is put back.
pub fn apply_and_restart(update: &Downloaded, config_dir: &Path) -> Result<(), String> {
    let new_exe = update.path.as_path();
    // Check again: the file sat on disk since the download.
    let data = fs::read(new_exe).map_err(|e| format!("The downloaded update is gone: {e}"))?;
    if to_hex(&sha256(&data)?) != update.sha256 {
        let _ = fs::remove_file(new_exe);
        return Err("The downloaded update changed on disk and was deleted. Nothing was installed.".into());
    }
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    let old = old_exe_path(&current);
    let cfg = crate::config::config_path(config_dir);
    if cfg.exists() {
        fs::copy(&cfg, config_dir.join("config.backup.json")).map_err(|e| format!("Could not back up settings: {e}"))?;
    }
    let _ = fs::remove_file(&old);
    fs::rename(&current, &old).map_err(|e| format!("Could not prepare the update: {e}"))?;
    let placed = fs::rename(new_exe, &current).or_else(|_| fs::copy(new_exe, &current).map(|_| ()));
    if let Err(e) = placed {
        let _ = fs::rename(&old, &current);
        return Err(format!("Could not install the update: {e}. The old version was restored."));
    }
    let _ = fs::remove_file(new_exe);
    std::process::Command::new(&current)
        .arg("--updated")
        .spawn()
        .map_err(|e| format!("Update installed, but NoKey could not restart: {e}. Start it manually."))?;
    Ok(())
}

/// Removes the previous version left behind by an update.
pub fn cleanup_old() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = fs::remove_file(old_exe_path(&exe));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare() {
        assert!(is_newer("v1.1.0", "1.0.0"));
        assert!(is_newer("0.2", "0.1.9"));
        assert!(!is_newer("v1.0.0", "1.0.0"));
        assert!(!is_newer("v0.9.9", "1.0.0"));
        assert!(!is_newer("nightly", "1.0.0"));
        assert_eq!(parse_version("v2.3.4-beta.1"), Some((2, 3, 4)));
    }

    #[test]
    fn repo_validation() {
        assert!(is_valid_repo("someone/nokey"));
        assert!(!is_valid_repo("someone"));
        assert!(!is_valid_repo("a/b/c"));
        assert!(!is_valid_repo("../x"));
        assert!(!is_valid_repo("a/.."));
    }

    #[test]
    fn only_own_release_downloads_are_accepted() {
        let repo = "someone/nokey";
        assert!(is_release_url("https://github.com/someone/nokey/releases/download/v1.0.0/NoKey.exe", repo));
        assert!(!is_release_url("https://evil.example/NoKey.exe", repo));
        assert!(!is_release_url("https://github.com/other/nokey/releases/download/v1/NoKey.exe", repo));
        assert!(!is_release_url("https://github.com/someone/nokey/releases/download/../../x", repo));
    }

    #[test]
    fn sha256_known_vector() {
        assert_eq!(
            to_hex(&sha256(b"abc").unwrap()),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn parses_sha_files() {
        let h = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(parse_sha_file(&format!("{h}  NoKey.exe\n")), Some(h.to_string()));
        assert_eq!(parse_sha_file(&h.to_uppercase()), Some(h.to_string()));
        assert_eq!(parse_sha_file("not-a-hash"), None);
    }
}
