// Embeds the app icon and the Windows manifest into NoKey.exe.
// The icon is generated from src/icon_art.rs, so there is no binary asset
// to keep in sync. Needs rc.exe from the Windows SDK (installed with the
// MSVC build tools); without it only the manifest is embedded.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

include!("src/icon_art.rs");

const ICON_SIZES: [u32; 7] = [16, 20, 24, 32, 40, 48, 64];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/icon_art.rs");
    println!("cargo:rerun-if-changed=assets/app.manifest");
    println!("cargo:rerun-if-env-changed=RC");

    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let manifest = root.join("assets").join("app.manifest");

    let ico = out.join("nokey.ico");
    fs::write(&ico, make_ico(&ICON_SIZES)).expect("write icon");

    match find_rc() {
        Some(rc) => {
            let rc_file = out.join("nokey.rc");
            let res = out.join("nokey.res");
            let script = format!("1 ICON \"{}\"\n1 24 \"{}\"\n", escape(&ico), escape(&manifest));
            fs::write(&rc_file, script).expect("write rc");
            let status = Command::new(&rc)
                .arg("/nologo")
                .arg("/fo")
                .arg(&res)
                .arg(&rc_file)
                .status()
                .expect("run rc.exe");
            assert!(status.success(), "rc.exe failed");
            println!("cargo:rustc-link-arg-bins={}", res.display());
        }
        None => {
            println!("cargo:warning=rc.exe not found: the exe will have no embedded icon");
            println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
            println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}", manifest.display());
        }
    }
}

fn escape(p: &Path) -> String {
    p.display().to_string().replace('\\', "\\\\")
}

fn find_rc() -> Option<PathBuf> {
    if let Some(rc) = env::var_os("RC").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(rc);
    }
    let kits = PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\bin");
    let mut versions: Vec<PathBuf> = fs::read_dir(&kits).ok()?.flatten().map(|e| e.path()).collect();
    versions.sort();
    versions.into_iter().rev().map(|v| v.join("x64").join("rc.exe")).find(|p| p.is_file())
}

/// ICO file with 32-bit BMP images (BGRA, bottom-up, plus an empty AND mask).
fn make_ico(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes.iter().map(|&s| bmp_image(s)).collect();
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&s, img) in sizes.iter().zip(&images) {
        out.push(s as u8);
        out.push(s as u8);
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(img.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in images {
        out.extend_from_slice(&img);
    }
    out
}

fn bmp_image(size: u32) -> Vec<u8> {
    let rgba = icon_rgba(size);
    let mask_row = size.div_ceil(32) * 4;
    let mut out = Vec::new();
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(size as i32).to_le_bytes());
    out.extend_from_slice(&((size * 2) as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(size * size * 4 + mask_row * size).to_le_bytes());
    out.extend_from_slice(&[0u8; 16]);
    for y in (0..size).rev() {
        for x in 0..size {
            let i = ((y * size + x) * 4) as usize;
            out.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
        }
    }
    out.extend(std::iter::repeat_n(0u8, (mask_row * size) as usize));
    out
}
