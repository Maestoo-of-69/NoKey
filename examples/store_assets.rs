//! Writes the PNG icons the Microsoft Store (MSIX) package needs.
//!
//! Usage: `cargo run --example store_assets -- <assets_dir> [<listing_dir>]`
//!
//! `<assets_dir>` receives the package logos (named with MRT resource
//! qualifiers such as `.scale-200` / `.targetsize-32` so `makepri` can index
//! them). `<listing_dir>` (optional) receives the 300x300 icon that is
//! uploaded by hand to the Store listing. No image crate is used: the PNG
//! encoder below writes uncompressed ("stored") deflate blocks.

use std::path::Path;
use std::{env, fs, process};

use nokey::icon_art::icon_rgba;

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
const MAX_STORED_BLOCK: usize = 65_535;

/// CRC-32 (IEEE, as used by PNG), computed bit by bit.
fn crc32(chunks: &[&[u8]]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in chunks.iter().flat_map(|c| c.iter()) {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// Adler-32 checksum, the trailer of a zlib stream.
fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for byte in data {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

/// Wraps `data` in a zlib stream made only of stored (uncompressed) blocks.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01]; // CMF/FLG: deflate, 32K window, no compression
    let mut blocks = data.chunks(MAX_STORED_BLOCK).peekable();
    if blocks.peek().is_none() {
        out.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]); // one empty final block
    }
    while let Some(block) = blocks.next() {
        let is_last = blocks.peek().is_none();
        let len = block.len() as u16;
        out.push(u8::from(is_last));
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn push_chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out.extend_from_slice(&crc32(&[kind, body]).to_be_bytes());
}

/// Encodes `size * size` RGBA pixels (top row first) as an RGBA8 PNG.
fn encode_png(size: u32, rgba: &[u8]) -> Vec<u8> {
    let row_bytes = size as usize * 4;
    assert_eq!(rgba.len(), row_bytes * size as usize, "pixel buffer size mismatch");

    let mut raw = Vec::with_capacity((row_bytes + 1) * size as usize);
    for row in rgba.chunks(row_bytes) {
        raw.push(0); // filter type 0: none
        raw.extend_from_slice(row);
    }

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit, RGBA, deflate, no filter set, no interlace

    let mut png = PNG_SIGNATURE.to_vec();
    push_chunk(&mut png, b"IHDR", &ihdr);
    push_chunk(&mut png, b"IDAT", &zlib_stored(&raw));
    push_chunk(&mut png, b"IEND", &[]);
    png
}

/// (file name, pixel size) of every package asset.
fn asset_list() -> Vec<(String, u32)> {
    let mut list = Vec::new();
    for (name, base) in [("StoreLogo", 50), ("Square150x150Logo", 150), ("Square44x44Logo", 44)] {
        for scale in [1u32, 2, 4] {
            list.push((format!("{name}.scale-{}.png", scale * 100), base * scale));
        }
    }
    for target in [16, 24, 32, 48, 256] {
        list.push((format!("Square44x44Logo.targetsize-{target}.png"), target));
        list.push((format!("Square44x44Logo.targetsize-{target}_altform-unplated.png"), target));
    }
    list
}

fn write_png(dir: &Path, name: &str, size: u32) -> Result<(), String> {
    let png = encode_png(size, &icon_rgba(size));
    fs::write(dir.join(name), png).map_err(|e| format!("cannot write {name}: {e}"))
}

fn run(assets_dir: &Path, listing_dir: Option<&Path>) -> Result<(), String> {
    fs::create_dir_all(assets_dir).map_err(|e| format!("cannot create {}: {e}", assets_dir.display()))?;
    let assets = asset_list();
    for (name, size) in &assets {
        write_png(assets_dir, name, *size)?;
    }
    println!("wrote {} assets to {}", assets.len(), assets_dir.display());
    if let Some(dir) = listing_dir {
        fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        write_png(dir, "store-listing-icon-300.png", 300)?;
        println!("wrote store-listing-icon-300.png to {}", dir.display());
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args.len() > 2 {
        eprintln!("usage: store_assets <assets_dir> [<listing_dir>]");
        process::exit(2);
    }
    let listing = args.get(1).map(Path::new);
    if let Err(message) = run(Path::new(&args[0]), listing) {
        eprintln!("error: {message}");
        process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_check_value() {
        assert_eq!(crc32(&[b"123456789"]), 0xCBF4_3926);
        assert_eq!(crc32(&[b"1234", b"56789"]), 0xCBF4_3926);
    }

    #[test]
    fn adler32_matches_known_value() {
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn png_has_signature_and_ihdr() {
        let png = encode_png(2, &[255u8; 16]);
        assert_eq!(&png[..8], &PNG_SIGNATURE);
        assert_eq!(&png[8..12], &13u32.to_be_bytes());
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(&png[16..20], &2u32.to_be_bytes());
        assert_eq!(&png[20..24], &2u32.to_be_bytes());
        assert_eq!(&png[24..29], &[8, 6, 0, 0, 0]);
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
    }

    #[test]
    fn large_images_split_into_multiple_stored_blocks() {
        let data = vec![7u8; MAX_STORED_BLOCK + 10];
        let z = zlib_stored(&data);
        // header(2) + 2 block headers(5 each) + payload + adler(4)
        assert_eq!(z.len(), 2 + 10 + data.len() + 4);
        assert_eq!(z[2], 0); // first block not final
        assert_eq!(z[2 + 5 + MAX_STORED_BLOCK], 1); // second block final
    }

    #[test]
    fn asset_list_has_expected_files() {
        let list = asset_list();
        assert_eq!(list.len(), 19);
        assert!(list.contains(&("Square150x150Logo.scale-400.png".to_string(), 600)));
        assert!(list.contains(&("Square44x44Logo.targetsize-16_altform-unplated.png".to_string(), 16)));
    }
}
