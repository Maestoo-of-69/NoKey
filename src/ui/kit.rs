//! Design-system pieces shared by every window (see docs/DESIGN.md):
//! keycaps, the switch, banners, status dots, header bars, row backgrounds
//! and the tab underline. Motion amounts (0..1) come from `anim`; hover and
//! press tracking for buttons and list rows lives in `kit/track.rs`.

mod slider;
mod track;

pub use slider::*;
pub use track::*;

use super::anim::{self, lerp, lerp_color};
use super::theme::{palette, TINT};
use super::{fill, fonts, round_rect, text, text_width};
use crate::win::px;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{COLORREF, HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;

/// Keycap height and the gap between keys of one shortcut.
pub const KEY_H: i32 = 22;
const KEY_GAP: i32 = 4;
const KEY_PAD: i32 = 7;
const KEY_MIN_W: i32 = 22;
/// Space for the arrow between steps of a sequence.
const ARROW_W: i32 = 14;
/// Gap between alternative shortcuts of one binding.
const ALT_GAP: i32 = 10;
/// How far a hovered row pulls keycap borders toward the text color.
const KEY_HOVER_BORDER: f32 = 0.35;

// Motion durations (ms), docs/DESIGN.md "Motion".
pub const HOVER_MS: u32 = 180;
pub const ROW_HOVER_MS: u32 = 220;
pub const RELEASE_MS: u32 = 240;
pub const TOGGLE_MS: u32 = 280;
pub const SWITCH_MS: u32 = 280;
pub const TAB_MS: u32 = 320;
pub const SELECT_FLASH_MS: u32 = 600;
pub const RUN_FLASH_MS: u32 = 1100;
/// Click ripple on a button.
pub const RIPPLE_MS: u32 = 480;
/// How far a clicked row first tints toward the accent.
const FLASH_TINT: f32 = 0.6;
pub const TINT_MS: u32 = 200;
pub const BANNER_MS: u32 = 200;
pub const DOT_PULSE_MS: u32 = 1000;
pub const RECORD_PULSE_MS: u32 = 1200;
pub const KEY_FADE_MS: u32 = 240;

// Animation slots on a button window.
pub const SLOT_HOVER: u32 = 1;
pub const SLOT_PRESS: u32 = 2;
/// Switch knob / checkbox fill (0 = off, 1 = on).
pub const SLOT_ON: u32 = 3;
/// Run-result tint (0 = none, 1 = full).
pub const SLOT_TINT: u32 = 4;
pub const SLOT_PULSE: u32 = 5;
/// Most the tab underline passes its target (96-DPI units).
const TAB_OVERSHOOT: i32 = 6;
thread_local! {
    /// Direction of the latest underline slide (1 = rightward, -1 = left).
    static TAB_DIRECTION: std::cell::Cell<f32> = const { std::cell::Cell::new(1.0) };
}
/// Click ripple progress (1 = gone).
pub const SLOT_RIPPLE: u32 = 6;
// Animation slots on a parent window.
const SLOT_TAB_LEFT: u32 = 0x7AB0_0001;
const SLOT_TAB_WIDTH: u32 = 0x7AB0_0002;

// Row effects on a list window: `row_slot(kind, key)`, key = item data.
pub const ROW_HOVER: u32 = 1;
pub const ROW_SELECT: u32 = 2;
pub const ROW_OK: u32 = 3;
pub const ROW_ERR: u32 = 4;

/// Animation slot of a row effect; `key` is the row's item data (a binding
/// or profile id, or a step index), so effects survive list refills.
pub fn row_slot(kind: u32, key: u32) -> u32 {
    kind << 28 | (key & 0x0FFF_FFFF)
}

/// One drawable piece of a shortcut label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyPart {
    Key(String),
    /// "→" between steps of a sequence.
    Then,
    /// Space between alternative shortcuts ("  |  ").
    Or,
}

/// Splits a shortcut label ("Ctrl + K", "Ctrl → K → C", "F8  |  F9") into keys.
pub fn split_keys(label: &str) -> Vec<KeyPart> {
    let mut parts = Vec::new();
    for (i, alt) in label.split("  |  ").enumerate() {
        if i > 0 {
            parts.push(KeyPart::Or);
        }
        for (j, step) in alt.split(" → ").enumerate() {
            if j > 0 {
                parts.push(KeyPart::Then);
            }
            parts.extend(step.split(" + ").map(str::trim).filter(|k| !k.is_empty()).map(|k| KeyPart::Key(k.to_string())));
        }
    }
    parts
}

/// Number of keys in a shortcut label.
pub fn key_count(label: &str) -> usize {
    split_keys(label).iter().filter(|p| matches!(p, KeyPart::Key(_))).count()
}

fn key_w(hdc: HDC, k: &str) -> i32 {
    (text_width(hdc, k, fonts().caps) + px(2 * KEY_PAD)).max(px(KEY_MIN_W))
}

/// Width (device pixels) that `keycaps` needs for this label.
pub fn keycaps_width(hdc: HDC, label: &str) -> i32 {
    let parts = split_keys(label);
    let mut w = 0;
    for (i, part) in parts.iter().enumerate() {
        if i > 0 && matches!((&parts[i - 1], part), (KeyPart::Key(_), KeyPart::Key(_))) {
            w += px(KEY_GAP);
        }
        w += match part {
            KeyPart::Key(k) => key_w(hdc, k),
            KeyPart::Then => px(ARROW_W),
            KeyPart::Or => px(ALT_GAP),
        };
    }
    w
}

/// Motion state of a keycap group.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct KeyStyle {
    /// Hover amount of the row (0..1): borders pull toward the text color.
    pub hover: f32,
    /// Mouse held on the row: the bottom edge collapses, caps sink 1 px.
    pub pressed: bool,
    /// Opacity (0..1) of the last key while it fades in; None = solid.
    pub last_alpha: Option<f32>,
}

/// One keycap at `x` (device pixels); `alpha` fades it in from `bg`.
fn draw_key(hdc: HDC, k: &str, x: i32, top: i32, w: i32, style: &KeyStyle, alpha: f32) {
    let p = palette();
    let mix = |c: COLORREF| lerp_color(p.bg, c, alpha);
    let border = mix(lerp_color(p.border, p.text, KEY_HOVER_BORDER * style.hover));
    let cap = RECT { left: x, top, right: x + w, bottom: top + px(KEY_H) };
    let face = if style.pressed {
        RECT { top: cap.top + px(1), bottom: cap.bottom - px(1), ..cap }
    } else {
        round_rect(hdc, &RECT { top: cap.top + px(2), ..cap }, mix(p.key_edge), Some(mix(p.key_edge)), 6);
        RECT { bottom: cap.bottom - px(2), ..cap }
    };
    round_rect(hdc, &face, border, Some(mix(p.key)), 6);
    text(hdc, k, &face, fonts().caps, mix(p.text), DT_CENTER | DT_VCENTER | DT_SINGLELINE);
}

/// Draws a shortcut as keycaps starting at `left` (device pixels), vertically
/// centered on `center_y`. Returns the right edge.
pub fn keycaps(hdc: HDC, label: &str, left: i32, center_y: i32) -> i32 {
    keycaps_ex(hdc, label, left, center_y, &KeyStyle::default())
}

/// `keycaps` with hover / press / fade-in state.
pub fn keycaps_ex(hdc: HDC, label: &str, left: i32, center_y: i32, style: &KeyStyle) -> i32 {
    let p = palette();
    let top = center_y - px(KEY_H) / 2;
    let parts = split_keys(label);
    let last_key = parts.iter().rposition(|part| matches!(part, KeyPart::Key(_)));
    let mut x = left;
    for (i, part) in parts.iter().enumerate() {
        if i > 0 && matches!((&parts[i - 1], part), (KeyPart::Key(_), KeyPart::Key(_))) {
            x += px(KEY_GAP);
        }
        match part {
            KeyPart::Key(k) => {
                let w = key_w(hdc, k);
                let alpha = if Some(i) == last_key { style.last_alpha.unwrap_or(1.0) } else { 1.0 };
                draw_key(hdc, k, x, top, w, style, alpha);
                x += w;
            }
            KeyPart::Then => {
                let rc = RECT { left: x, top, right: x + px(ARROW_W), bottom: top + px(KEY_H) };
                // The DC is mirrored in Hebrew, so keys run right to left; the arrow must too.
                let arrow = if crate::i18n::is_rtl() { "←" } else { "→" };
                text(hdc, arrow, &rc, fonts().small, p.muted, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                x += px(ARROW_W);
            }
            KeyPart::Or => x += px(ALT_GAP),
        }
    }
    x
}

/// Draws the keycaps right-aligned so they end at `right`. Returns the left edge.
pub fn keycaps_right(hdc: HDC, label: &str, right: i32, center_y: i32) -> i32 {
    keycaps_right_ex(hdc, label, right, center_y, &KeyStyle::default())
}

/// `keycaps_right` with hover / press state.
pub fn keycaps_right_ex(hdc: HDC, label: &str, right: i32, center_y: i32, style: &KeyStyle) -> i32 {
    let left = right - keycaps_width(hdc, label);
    keycaps_ex(hdc, label, left, center_y, style);
    left
}

/// A pill switch inside `rc` (device pixels), vertically centered.
pub fn switch(hdc: HDC, rc: &RECT, on: bool, focused: bool, hovered: bool, disabled: bool) {
    switch_at(hdc, rc, on as u8 as f32, focused, hovered as u8 as f32, disabled);
}

/// A switch with the knob at `pos` (0 = off, 1 = on) and a hover amount;
/// track and knob colors follow the knob.
pub fn switch_at(hdc: HDC, rc: &RECT, pos: f32, focused: bool, hover: f32, disabled: bool) {
    let p = palette();
    let (w, h) = (px(36), px(20));
    let top = rc.top + (rc.bottom - rc.top - h) / 2;
    let left = rc.right - w;
    let track = RECT { left, top, right: left + w, bottom: top + h };
    let (track_fill, outline, knob) = if disabled {
        (p.surface, p.line, p.line)
    } else {
        let off_fill = lerp_color(p.bg, p.hover, hover);
        let off_outline = if focused { p.accent } else { p.border };
        (lerp_color(off_fill, p.accent, pos), lerp_color(off_outline, p.accent, pos), lerp_color(p.muted, p.accent_text, pos))
    };
    round_rect(hdc, &track, outline, Some(track_fill), 20);
    let k = px(14);
    let ky = top + (h - k) / 2;
    // The knob may overshoot a little, and stretches while it travels.
    let at = pos.clamp(-0.12, 1.12);
    let kx = lerp((track.left + px(3)) as f32, (track.right - px(3) - k) as f32, at).round() as i32;
    let stretch = ((4.0 * pos * (1.0 - pos)).max(0.0) * px(6) as f32).round() as i32;
    let knob_rc = RECT { left: kx - stretch / 2, top: ky, right: kx + k + stretch / 2, bottom: ky + k };
    round_rect(hdc, &knob_rc, knob, Some(knob), 14);
    if focused && pos > 0.5 && !disabled {
        let ring = RECT { left: track.left - px(2), top: track.top - px(2), right: track.right + px(2), bottom: track.bottom + px(2) };
        round_rect(hdc, &ring, p.accent, None, 24);
    }
}

/// Full-width warning strip with centered small text.
pub fn banner(hdc: HDC, rc: &RECT, message: &str) {
    let p = palette();
    fill(hdc, rc, p.warn);
    let inner = RECT { left: rc.left + px(16), right: rc.right - px(16), ..*rc };
    text(hdc, message, &inner, fonts().small, p.warn_text, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
}

/// 8px status dot centered at (cx, cy): filled `ok` when on, outlined when off.
pub fn status_dot(hdc: HDC, cx: i32, cy: i32, on: bool) {
    status_dot_ex(hdc, cx, cy, on, 0.0);
}

/// `status_dot` whose fill fades toward `bg` by `fade` (0..1, a pulse).
pub fn status_dot_ex(hdc: HDC, cx: i32, cy: i32, on: bool, fade: f32) {
    let p = palette();
    let rad = px(4);
    let rc = RECT { left: cx - rad, top: cy - rad, right: cx + rad, bottom: cy + rad };
    if on {
        let c = lerp_color(p.ok, p.bg, 0.7 * fade);
        round_rect(hdc, &rc, c, Some(c), 8);
    } else {
        round_rect(hdc, &rc, p.border, None, 8);
    }
}

/// Header bar background: plain `bg` with a bottom hairline, or the Jewish
/// lattice and stripes when the theme has a pattern. `width`/`height` in
/// 96-DPI units.
pub fn header_bar(hdc: HDC, width: i32, height: i32) {
    let p = palette();
    if p.pattern.is_some() {
        super::header_band(hdc, width, height);
    } else {
        fill(hdc, &RECT { left: 0, top: px(height) - px(1), right: px(width), bottom: px(height) }, p.line);
    }
}

/// Motion state of one list row (all amounts 0..1).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RowFx {
    pub hover: f32,
    /// Selection flash: accent tint fading to `selected`.
    pub flash: f32,
    /// Binding-ran flashes.
    pub ok: f32,
    pub err: f32,
    /// Mouse held on this row.
    pub pressed: bool,
}

impl RowFx {
    /// Reads the row's effects; `item` is its index, `key` its item data.
    pub fn read(list: HWND, item: u32, key: u32) -> RowFx {
        let hovered = hovered_item(list) == Some(item);
        RowFx {
            hover: anim::value(list, row_slot(ROW_HOVER, key), hovered as u8 as f32),
            flash: anim::value(list, row_slot(ROW_SELECT, key), 0.0),
            ok: anim::value(list, row_slot(ROW_OK, key), 0.0),
            err: anim::value(list, row_slot(ROW_ERR, key), 0.0),
            pressed: pressed_item(list) == Some(item),
        }
    }

    /// Keycap style for this row.
    pub fn keys(&self) -> KeyStyle {
        KeyStyle { hover: self.hover, pressed: self.pressed, last_alpha: None }
    }
}

/// Row fill over `base`, or None when the row shows only `base`.
pub fn row_fill(base: COLORREF, selected: bool, fx: &RowFx) -> Option<COLORREF> {
    let p = palette();
    let mut c = if selected {
        lerp_color(p.selected, lerp_color(p.selected, p.accent, FLASH_TINT), fx.flash)
    } else {
        lerp_color(base, p.hover, fx.hover)
    };
    c = lerp_color(c, p.ok, TINT * fx.ok);
    c = lerp_color(c, p.err, TINT * fx.err);
    (c != base).then_some(c)
}

/// Selected / hovered row background, inset 4px and rounded.
pub fn row_background(hdc: HDC, rc: &RECT, selected: bool, hovered: bool) {
    row_background_fx(hdc, rc, selected, &RowFx { hover: hovered as u8 as f32, ..RowFx::default() });
}

/// `row_background` with motion amounts.
pub fn row_background_fx(hdc: HDC, rc: &RECT, selected: bool, fx: &RowFx) {
    let p = palette();
    fill(hdc, rc, p.bg);
    if let Some(c) = row_fill(p.bg, selected, fx) {
        let inner = RECT { left: rc.left + px(4), top: rc.top + px(1), right: rc.right - px(4), bottom: rc.bottom - px(1) };
        round_rect(hdc, &inner, c, Some(c), 6);
    }
    // Accent bar at the row's start: it grows from the middle on hover and
    // stays on the selected row.
    let amount = if selected { 1.0 } else { fx.hover };
    let half = (((rc.bottom - rc.top - px(14)) as f32 * amount) / 2.0).round() as i32;
    if half > 0 {
        let cy = (rc.top + rc.bottom) / 2;
        let bar = RECT { left: rc.left + px(4), top: cy - half, right: rc.left + px(7), bottom: cy + half };
        fill(hdc, &bar, p.accent);
    }
}

/// A child window's rectangle in its parent's client coordinates (device
/// pixels, left <= right also in mirrored Hebrew windows).
fn child_rect(parent: HWND, child: HWND) -> RECT {
    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe {
        GetWindowRect(child, &mut rc);
        MapWindowPoints(null_mut(), parent, &mut rc as *mut RECT as *mut POINT, 2);
    }
    RECT { left: rc.left.min(rc.right), right: rc.left.max(rc.right), ..rc }
}

/// Tab strip underline (2px accent) under the selected tab, drawn by the
/// parent in the 2px strip below the tab buttons. Slides when `slide_tab`
/// started a transition.
pub fn tab_underline(hdc: HDC, parent: HWND, tab: HWND) {
    let rc = child_rect(parent, tab);
    // The spring may pass the target tab by a few units, however far it came.
    let past = anim::value(parent, SLOT_TAB_LEFT, rc.left as f32) - rc.left as f32;
    let direction = TAB_DIRECTION.with(std::cell::Cell::get);
    let past = if past * direction > px(TAB_OVERSHOOT) as f32 { direction * px(TAB_OVERSHOOT) as f32 } else { past };
    let left = (rc.left as f32 + past).round() as i32;
    let width = anim::value(parent, SLOT_TAB_WIDTH, (rc.right - rc.left) as f32).round() as i32;
    fill(hdc, &RECT { left, top: rc.bottom, right: left + width, bottom: rc.bottom + px(2) }, palette().accent);
}

/// Slides the underline from tab `from` to tab `to` (mouse clicks only).
pub fn slide_tab(parent: HWND, from: HWND, to: HWND) {
    if from == to {
        return;
    }
    let (a, b) = (child_rect(parent, from), child_rect(parent, to));
    TAB_DIRECTION.with(|d| d.set((b.left - a.left).signum() as f32));
    anim::spring(parent, SLOT_TAB_LEFT, a.left as f32, b.left as f32, TAB_MS);
    anim::spring(parent, SLOT_TAB_WIDTH, (a.right - a.left) as f32, (b.right - b.left) as f32, TAB_MS);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(v: &[&str]) -> Vec<KeyPart> {
        v.iter().map(|k| KeyPart::Key(k.to_string())).collect()
    }

    #[test]
    fn splits_chords() {
        assert_eq!(split_keys("Ctrl + Shift + P"), keys(&["Ctrl", "Shift", "P"]));
        assert_eq!(split_keys("F8"), keys(&["F8"]));
    }

    #[test]
    fn splits_sequences_and_alternatives() {
        assert_eq!(
            split_keys("Ctrl → K"),
            vec![KeyPart::Key("Ctrl".into()), KeyPart::Then, KeyPart::Key("K".into())]
        );
        assert_eq!(split_keys("F8  |  A + S"), vec![KeyPart::Key("F8".into()), KeyPart::Or, KeyPart::Key("A".into()), KeyPart::Key("S".into())]);
    }

    #[test]
    fn sentence_case_keeps_brand_capitals() {
        use crate::ui::sentence_case;
        assert_eq!(sentence_case("BINDINGS"), "Bindings");
        assert_eq!(sentence_case("bindings"), "Bindings");
        assert_eq!(sentence_case("GitHub repo"), "GitHub repo");
        assert_eq!(sentence_case("Бот Discord"), "Бот Discord");
        assert_eq!(sentence_case("קישורים"), "קישורים");
        assert_eq!(sentence_case(""), "");
    }

    #[test]
    fn plus_key_inside_a_name_survives() {
        assert_eq!(split_keys("Ctrl + Num +"), keys(&["Ctrl", "Num +"]));
        assert!(split_keys("").is_empty());
    }

    #[test]
    fn counts_keys_for_the_fade_in() {
        assert_eq!(key_count("Ctrl + Shift"), 2);
        assert_eq!(key_count("Ctrl → K → C"), 3);
        assert_eq!(key_count(""), 0);
    }

    #[test]
    fn row_slots_keep_kinds_apart() {
        assert_ne!(row_slot(ROW_HOVER, 7), row_slot(ROW_SELECT, 7));
        assert_ne!(row_slot(ROW_OK, 7), row_slot(ROW_OK, 8));
        assert_eq!(row_slot(ROW_ERR, 7) >> 28, ROW_ERR);
    }
}
