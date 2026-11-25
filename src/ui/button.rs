//! Owner-drawn buttons (docs/DESIGN.md, Components and Motion). Hover,
//! press, toggle, tint and pulse amounts come from `anim`; the transitions
//! themselves are started by the mouse handlers in `kit/track.rs`.

use super::anim::{self, lerp_color};
use super::kit::{self, BTN_HOVER, BTN_PULSE, BTN_TINT, BTN_TINT_ERR, SLOT_HOVER, SLOT_ON, SLOT_PRESS, SLOT_PULSE, SLOT_TINT};
use super::kit::SLOT_RIPPLE;
use super::theme::{palette, Palette, TINT};
use super::*;

/// Primary / warn hover: the fill moves this far toward its text color.
/// How far a hovered primary/warn fill moves toward its text colour. 0.06
/// keeps white-on-accent text above 4.5:1 in every theme (0.12 fell to 4.17).
pub(super) const FILL_HOVER: f32 = 0.06;
/// Pressed: the fill moves this far toward `key_edge` (darker in most themes).
const PRESS_DARKEN: f32 = 0.25;
/// Pressed: each side of a button moves in this far (96-DPI units).
const PRESS_SQUISH: i32 = 2;
/// Strength of the click ripple where it starts.
const RIPPLE_TINT: f32 = 0.28;
/// Hover ring: how far it moves from the fill toward its colour.
const RING: f32 = 0.85;
/// Recording pulse: how far a ghost button moves toward `warn`, and how far
/// a warn button dims toward `bg` between beats.
const PULSE_GHOST: f32 = 0.4;
const PULSE_WARN_DIM: f32 = 0.3;

/// Everything a button draw needs, read once.
struct Btn {
    hwnd: HWND,
    rc: RECT,
    kind: isize,
    raw: isize,
    checked: bool,
    pressed: bool,
    disabled: bool,
    focused: bool,
    /// Hover amount 0..1.
    hover: f32,
}

impl Btn {
    fn read(dis: &DRAWITEMSTRUCT) -> Btn {
        let raw = unsafe { GetWindowLongPtrW(dis.hwndItem, GWLP_USERDATA) };
        let disabled = dis.itemState & ODS_DISABLED != 0;
        let hovered = raw & BTN_HOVER != 0;
        let hover = if disabled { 0.0 } else { anim::value(dis.hwndItem, SLOT_HOVER, hovered as u8 as f32) };
        Btn {
            hwnd: dis.hwndItem,
            rc: dis.rcItem,
            kind: raw & KIND_MASK,
            raw,
            checked: raw & BTN_CHECKED != 0,
            pressed: dis.itemState & ODS_SELECTED != 0,
            disabled,
            focused: dis.itemState & ODS_FOCUS != 0,
            hover,
        }
    }

    /// Shown on/off amount of a switch or checkbox (pressing previews the flip).
    fn on_amount(&self) -> f32 {
        anim::value(self.hwnd, SLOT_ON, (self.checked != self.pressed) as u8 as f32)
    }
}

/// Paints an owner-drawn button (styles: docs/DESIGN.md, Components).
/// Drawn off-screen and copied at once (`buffered_item`): no flicker while
/// a transition repaints it every frame.
pub fn draw_button(dis: &DRAWITEMSTRUCT) {
    let b = Btn::read(dis);
    let label = crate::win::get_text(b.hwnd);
    buffered_item(dis.hDC, &dis.rcItem, |hdc, rc| {
        let b = Btn { rc: *rc, ..b };
        fill(hdc, rc, palette().bg);
        match b.kind {
            BTN_SWITCH => draw_switch(hdc, &b, &label),
            BTN_TOGGLE => draw_checkbox(hdc, &b, &label),
            BTN_TAB => draw_tab(hdc, &b, &label),
            BTN_LOGO => draw_logo(hdc, &b, &label),
            _ => draw_push(hdc, &b, &label),
        }
    });
}

fn draw_switch(hdc: HDC, b: &Btn, label: &str) {
    let p = palette();
    let sw = RECT { left: b.rc.right - px(38), right: b.rc.right - px(2), ..b.rc };
    kit::switch_at(hdc, &sw, b.on_amount(), b.focused, b.hover, b.disabled);
    if !label.is_empty() {
        let tr = RECT { right: sw.left - px(8), ..b.rc };
        let color = if b.disabled { p.muted } else { p.text };
        text(hdc, label, &tr, fonts().small, color, DT_RIGHT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
    }
}

/// 16x16 box; the accent fill and the ✓ fade in together.
fn draw_checkbox(hdc: HDC, b: &Btn, label: &str) {
    let p = palette();
    let f = fonts();
    let size = px(16);
    let top = b.rc.top + (b.rc.bottom - b.rc.top - size) / 2;
    let bx = RECT { left: b.rc.left, top, right: b.rc.left + size, bottom: top + size };
    let on = b.on_amount();
    let off_outline = if b.focused { p.accent } else { lerp_color(p.border, p.accent, b.hover) };
    let outline = if b.disabled { p.line } else { lerp_color(off_outline, p.accent, on) };
    let box_fill = lerp_color(lerp_color(p.bg, p.hover, b.hover), p.accent, on);
    // Mid-flip the box shrinks, then the spring pops it back.
    let pop = ((4.0 * on * (1.0 - on)).max(0.0) * px(3) as f32).round() as i32;
    let shown = RECT { left: bx.left + pop, top: bx.top + pop, right: bx.right - pop, bottom: bx.bottom - pop };
    round_rect(hdc, &shown, outline, Some(box_fill), 4);
    if on > 0.01 {
        text(hdc, "✓", &bx, f.caps, lerp_color(box_fill, p.accent_text, on), DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    }
    let tr = RECT { left: bx.right + px(10), ..b.rc };
    let color = if b.disabled { p.muted } else { p.text };
    text(hdc, label, &tr, f.normal, color, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
}

/// Tab label; the selected tab's underline is drawn by the parent
/// (`kit::tab_underline`) so it can slide between tabs.
fn draw_tab(hdc: HDC, b: &Btn, label: &str) {
    let p = palette();
    let color = if b.checked { p.text } else { lerp_color(p.muted, p.text, b.hover) };
    if b.hover > 0.01 && !b.checked {
        let pill = RECT { left: b.rc.left + px(2), top: b.rc.top + px(4), right: b.rc.right - px(2), bottom: b.rc.bottom - px(4) };
        let c = lerp_color(p.bg, p.hover, b.hover);
        round_rect(hdc, &pill, c, Some(c), 6);
    }
    text(hdc, label, &b.rc, fonts().bold, color, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
}

/// Background and text color of a push button at rest plus hover.
fn push_base(b: &Btn, p: &Palette) -> (COLORREF, COLORREF) {
    let primary = matches!(b.kind, BTN_PRIMARY | BTN_BIG);
    let warn = b.kind == BTN_WARN;
    match (b.disabled, primary || warn) {
        (true, true) => (p.surface, p.muted),
        (true, false) => (p.bg, p.muted),
        (false, true) => {
            let (fill_c, fg) = if warn { (p.warn, p.warn_text) } else { (p.accent, p.accent_text) };
            (lerp_color(fill_c, lerp_color(fill_c, fg, FILL_HOVER), b.hover), fg)
        }
        (false, false) => (lerp_color(p.bg, p.hover, b.hover), p.text),
    }
}

/// Push-button fill with press, recording pulse and run tint applied.
fn push_fill(b: &Btn, p: &Palette, base: COLORREF, press: f32) -> COLORREF {
    let mut bg = lerp_color(base, p.key_edge, PRESS_DARKEN * press);
    if b.raw & BTN_PULSE != 0 && !b.disabled {
        // Rest beat (animations off, or the first frame): warn = full warn, ghost = plain.
        let rest = if b.kind == BTN_WARN { 1.0 } else { 0.0 };
        let beat = anim::pulse(b.hwnd, SLOT_PULSE, kit::RECORD_PULSE_MS, rest, None);
        bg = if b.kind == BTN_WARN {
            lerp_color(lerp_color(p.warn, p.bg, PULSE_WARN_DIM), bg, beat)
        } else {
            lerp_color(bg, p.warn, PULSE_GHOST * beat)
        };
    }
    let tint = anim::value(b.hwnd, SLOT_TINT, (b.raw & BTN_TINT != 0) as u8 as f32);
    if tint > 0.0 {
        let color = if b.raw & BTN_TINT_ERR != 0 { p.err } else { p.ok };
        bg = lerp_color(bg, lerp_color(p.bg, color, TINT), tint);
    }
    bg
}

/// The header's "NoKey" button: on hover the name turns accent and a line
/// grows under it from its start (also shown with keyboard focus).
fn draw_logo(hdc: HDC, b: &Btn, label: &str) {
    let p = palette();
    let press = anim::value(b.hwnd, SLOT_PRESS, b.pressed as u8 as f32);
    let dy = (press * px(2) as f32).round() as i32;
    ripple(hdc, b, &b.rc, p.bg, p.accent);
    let color = lerp_color(p.text, p.accent, b.hover.max(press));
    let tr = RECT { top: b.rc.top + dy, bottom: b.rc.bottom + dy, ..b.rc };
    text(hdc, label, &tr, fonts().title, color, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    let amount = if b.focused { 1.0 } else { b.hover };
    let width = ((b.rc.right - b.rc.left - px(16)) as f32 * amount).round() as i32;
    if width > 0 {
        let left = b.rc.left + px(8);
        fill(hdc, &RECT { left, top: b.rc.bottom - px(4), right: left + width, bottom: b.rc.bottom - px(1) }, p.accent);
    }
}

/// A circle growing from the clicked point and fading into `base`, clipped
/// to the button. Drawn between the fill and the label.
fn ripple(hdc: HDC, b: &Btn, area: &RECT, base: COLORREF, toward: COLORREF) {
    let t = anim::value(b.hwnd, SLOT_RIPPLE, 1.0);
    let Some((x, y)) = kit::ripple_origin(b.hwnd).filter(|_| t < 1.0 && !b.disabled) else { return };
    let dx = (x - area.left).max(area.right - x) as f32;
    let dy = (y - area.top).max(area.bottom - y) as f32;
    let radius = (dx.hypot(dy) * t) as i32 + px(4);
    let color = lerp_color(base, toward, RIPPLE_TINT * (1.0 - t));
    unsafe {
        let saved = SaveDC(hdc);
        BeginPath(hdc);
        RoundRect(hdc, area.left, area.top, area.right + 1, area.bottom + 1, px(10), px(10));
        EndPath(hdc);
        SelectClipPath(hdc, RGN_AND);
        let brush = CreateSolidBrush(color);
        SelectObject(hdc, brush as _);
        SelectObject(hdc, GetStockObject(NULL_PEN));
        Ellipse(hdc, x - radius, y - radius, x + radius, y + radius);
        RestoreDC(hdc, saved);
        DeleteObject(brush as _);
    }
}

/// Primary, ghost, warn, dropdown and icon buttons. Hover draws a ring;
/// pressed, the button squeezes in and darkens, and a ripple spreads from
/// the click.
fn draw_push(hdc: HDC, b: &Btn, label: &str) {
    let p = palette();
    let f = fonts();
    let press = if b.disabled { 0.0 } else { anim::value(b.hwnd, SLOT_PRESS, b.pressed as u8 as f32) };
    let (base, fg) = push_base(b, &p);
    let bg = push_fill(b, &p, base, press);
    // A ring in a contrasting colour appears with the hover; the label's
    // background is left alone, so its contrast does not change.
    let ring_color = if matches!(b.kind, BTN_PRIMARY | BTN_BIG | BTN_WARN) { fg } else { p.accent };
    let hover = if b.disabled { 0.0 } else { b.hover };
    let ring = lerp_color(bg, ring_color, RING * hover);
    let outline = if b.focused && !b.disabled { p.accent } else { ring };
    let squish = (press * px(PRESS_SQUISH) as f32).round() as i32;
    let rc = RECT {
        left: b.rc.left + squish,
        top: b.rc.top + squish,
        right: b.rc.right - squish,
        bottom: b.rc.bottom - squish,
    };
    let inner = RECT { right: rc.right - 1, bottom: rc.bottom - 1, ..rc };
    round_rect(hdc, &inner, outline, Some(bg), 6);
    // On filled buttons the wave darkens: toward the label colour it would
    // cost the label its contrast.
    let wave = if matches!(b.kind, BTN_PRIMARY | BTN_BIG | BTN_WARN) { p.key_edge } else { fg };
    ripple(hdc, b, &inner, bg, wave);
    if hover > 0.01 {
        let second = RECT { left: inner.left + 1, top: inner.top + 1, right: inner.right - 1, bottom: inner.bottom - 1 };
        round_rect(hdc, &inner, outline, None, 6);
        round_rect(hdc, &second, ring, None, 5);
    }
    match b.kind {
        BTN_DROPDOWN => {
            let tr = RECT { left: rc.left + px(10), right: rc.right - px(24), ..rc };
            text(hdc, label, &tr, f.normal, fg, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
            let ar = RECT { left: rc.right - px(24), ..rc };
            text(hdc, "▾", &ar, f.small, p.muted, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        }
        BTN_ICON => {
            text(hdc, label, &rc, f.big, if b.disabled { p.muted } else { p.text }, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        }
        _ => {
            let font = if b.kind == BTN_BIG || b.kind == BTN_WARN { f.big } else { f.bold };
            let tr = RECT { left: rc.left + px(8), right: rc.right - px(8), ..rc };
            text(hdc, label, &tr, font, fg, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
        }
    }
}
