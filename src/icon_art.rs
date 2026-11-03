// NoKey icon, drawn in code so the same pixels feed both the .ico embedded
// at build time (build.rs includes this file) and the tray icon at runtime.
// A white keycap outline crossed by a slash, on a black rounded square.

/// Returns `size * size` RGBA pixels, top row first.
pub fn icon_rgba(size: u32) -> Vec<u8> {
    let n = size as f32;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let u = (x as f32 + 0.5) / n;
            let v = (y as f32 + 0.5) / n;
            let px = 1.0 / n;
            let cov = |d: f32| (0.5 - d / px).clamp(0.0, 1.0);

            let bg = cov(rounded_box(u, v, 0.5, 0.5, 0.5, 0.5, 0.22));
            let cap = cov(rounded_box(u, v, 0.5, 0.5, 0.27, 0.27, 0.07).abs() - 0.045);
            let slash_d = segment(u, v, 0.24, 0.76, 0.76, 0.24);
            let gap = cov(slash_d - 0.09);
            let slash = cov(slash_d - 0.045);

            let white = (cap * (1.0 - gap)).max(slash);
            let c = (white * 255.0).round() as u8;
            out.extend_from_slice(&[c, c, c, (bg * 255.0).round() as u8]);
        }
    }
    out
}

fn rounded_box(u: f32, v: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let qx = (u - cx).abs() - (hw - r);
    let qy = (v - cy).abs() - (hh - r);
    let ox = qx.max(0.0);
    let oy = qy.max(0.0);
    (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - r
}

fn segment(u: f32, v: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (pax, pay) = (u - ax, v - ay);
    let (bax, bay) = (bx - ax, by - ay);
    let h = ((pax * bax + pay * bay) / (bax * bax + bay * bay)).clamp(0.0, 1.0);
    let (dx, dy) = (pax - bax * h, pay - bay * h);
    (dx * dx + dy * dy).sqrt()
}
