//! Color themes. Every color the UI draws comes from the current palette,
//! so switching themes only needs a repaint.

use crate::config::Theme;
use crate::win::wide;
use std::cell::Cell;
use windows_sys::Win32::Foundation::COLORREF;
use windows_sys::Win32::Graphics::Gdi::{CreateSolidBrush, DeleteObject, HBRUSH};
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

/// Win32 colors are 0x00BBGGRR.
pub const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

/// Strength of the motion tints: selection flash (toward `accent`) and run
/// results (toward `ok` / `err`), as a mix fraction.
pub const TINT: f32 = 0.35;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub bg: COLORREF,
    /// Inputs, fields, cards, row icons.
    pub surface: COLORREF,
    /// Selected list row, pressed ghost button.
    pub selected: COLORREF,
    /// Hovered row or ghost button.
    pub hover: COLORREF,
    pub text: COLORREF,
    pub muted: COLORREF,
    /// Control outlines.
    pub border: COLORREF,
    /// Hairlines between areas.
    pub line: COLORREF,
    /// Primary buttons, focus rings, switch "on".
    pub accent: COLORREF,
    pub accent_text: COLORREF,
    /// The "all bindings disabled" banner.
    pub warn: COLORREF,
    pub warn_text: COLORREF,
    /// Keycap fill and its thicker bottom edge.
    pub key: COLORREF,
    pub key_edge: COLORREF,
    /// "Connected" status dot, success tint of a run.
    pub ok: COLORREF,
    /// Failure tint of a run (a red, or white in the contrast theme).
    pub err: COLORREF,
    /// Decorative header pattern (Jewish theme only).
    pub pattern: Option<COLORREF>,
    /// Star of David in the main window header (Jewish theme only).
    pub star: Option<COLORREF>,
    /// Dark title bar.
    pub dark: bool,
}

pub const DARK: Palette = Palette {
    bg: rgb(0x12, 0x12, 0x14),
    surface: rgb(0x1E, 0x1E, 0x22),
    selected: rgb(0x2E, 0x2E, 0x35),
    hover: rgb(0x24, 0x24, 0x29),
    text: rgb(0xF2, 0xF2, 0xF4),
    muted: rgb(0x9C, 0x9C, 0xA6),
    border: rgb(0x3C, 0x3C, 0x44),
    line: rgb(0x25, 0x25, 0x2A),
    accent: rgb(0x2B, 0x66, 0xDE),
    accent_text: rgb(0xFF, 0xFF, 0xFF),
    warn: rgb(0xE8, 0xA8, 0x17),
    warn_text: rgb(0x1A, 0x12, 0x00),
    key: rgb(0x26, 0x26, 0x2C),
    key_edge: rgb(0x08, 0x08, 0x0A),
    ok: rgb(0x3F, 0xB9, 0x50),
    err: rgb(0xE5, 0x48, 0x4D),
    pattern: None,
    star: None,
    dark: true,
};

pub const LIGHT: Palette = Palette {
    bg: rgb(0xFF, 0xFF, 0xFF),
    surface: rgb(0xF4, 0xF4, 0xF6),
    selected: rgb(0xE6, 0xE6, 0xEC),
    hover: rgb(0xF0, 0xF0, 0xF3),
    text: rgb(0x16, 0x16, 0x1A),
    muted: rgb(0x60, 0x60, 0x69),
    border: rgb(0xCF, 0xCF, 0xD6),
    line: rgb(0xEA, 0xEA, 0xEE),
    accent: rgb(0x2B, 0x66, 0xDE),
    accent_text: rgb(0xFF, 0xFF, 0xFF),
    warn: rgb(0xFC, 0xE8, 0xB2),
    warn_text: rgb(0x5C, 0x3C, 0x00),
    key: rgb(0xFF, 0xFF, 0xFF),
    key_edge: rgb(0xC4, 0xC4, 0xCC),
    ok: rgb(0x1F, 0x8A, 0x3B),
    err: rgb(0xC6, 0x28, 0x28),
    pattern: None,
    star: None,
    dark: false,
};

/// High contrast: black, white and yellow only.
pub const CONTRAST: Palette = Palette {
    bg: rgb(0, 0, 0),
    surface: rgb(0, 0, 0),
    selected: rgb(0x33, 0x2B, 0x00),
    hover: rgb(0x1A, 0x16, 0x00),
    text: rgb(0xFF, 0xFF, 0xFF),
    muted: rgb(0xFF, 0xFF, 0xFF),
    border: rgb(0xFF, 0xFF, 0xFF),
    line: rgb(0xFF, 0xFF, 0xFF),
    accent: rgb(0xFF, 0xD4, 0x00),
    accent_text: rgb(0, 0, 0),
    warn: rgb(0xFF, 0xD4, 0x00),
    warn_text: rgb(0, 0, 0),
    key: rgb(0, 0, 0),
    key_edge: rgb(0xFF, 0xFF, 0xFF),
    ok: rgb(0xFF, 0xD4, 0x00),
    err: rgb(0xFF, 0xFF, 0xFF),
    pattern: None,
    star: None,
    dark: true,
};

/// Saturated flag blue on white: blue header stripes, a geometric lattice
/// and a large Star of David in the main window header.
pub const JEWISH: Palette = Palette {
    bg: rgb(0xF5, 0xF8, 0xFF),
    surface: rgb(0xE3, 0xEC, 0xFF),
    selected: rgb(0xC7, 0xD8, 0xFF),
    hover: rgb(0xDA, 0xE5, 0xFF),
    text: rgb(0x0A, 0x1A, 0x47),
    muted: rgb(0x2E, 0x4A, 0x8C),
    border: rgb(0x00, 0x38, 0xB8),
    line: rgb(0xB9, 0xCC, 0xF5),
    accent: rgb(0x00, 0x38, 0xB8),
    accent_text: rgb(0xFF, 0xFF, 0xFF),
    warn: rgb(0x0A, 0x1A, 0x47),
    warn_text: rgb(0xFF, 0xFF, 0xFF),
    key: rgb(0xFF, 0xFF, 0xFF),
    key_edge: rgb(0x00, 0x38, 0xB8),
    ok: rgb(0x00, 0x38, 0xB8),
    err: rgb(0xC0, 0x1C, 0x28),
    pattern: Some(rgb(0xBF, 0xD1, 0xFA)),
    star: Some(rgb(0x00, 0x38, 0xB8)),
    dark: false,
};

/// Reads "Choose your app mode" from Windows (light = true).
pub fn system_uses_light() -> bool {
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide("AppsUseLightTheme");
    let mut value: u32 = 0;
    let mut size = 4u32;
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            &mut value as *mut u32 as *mut _,
            &mut size,
        )
    };
    rc == 0 && value != 0
}

pub fn palette_for(theme: Theme, system_light: bool) -> Palette {
    match theme {
        Theme::Dark => DARK,
        Theme::Light => LIGHT,
        Theme::Contrast => CONTRAST,
        Theme::Jewish => JEWISH,
        Theme::System => {
            if system_light {
                LIGHT
            } else {
                DARK
            }
        }
    }
}

thread_local! {
    static THEME: Cell<Theme> = const { Cell::new(Theme::Dark) };
    static PALETTE: Cell<Palette> = const { Cell::new(DARK) };
    static BRUSHES: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
}

/// Switches the palette (UI thread). Returns true if the colors changed.
pub fn set_theme(theme: Theme) -> bool {
    let p = palette_for(theme, system_uses_light());
    THEME.with(|t| t.set(theme));
    let changed = PALETTE.with(|c| c.replace(p)) != p || BRUSHES.with(|b| b.get().0 == 0);
    if changed {
        let new = unsafe { (CreateSolidBrush(p.bg) as usize, CreateSolidBrush(p.surface) as usize) };
        let old = BRUSHES.with(|b| b.replace(new));
        if old.0 != 0 {
            unsafe {
                DeleteObject(old.0 as _);
                DeleteObject(old.1 as _);
            }
        }
    }
    changed
}

pub fn current_theme() -> Theme {
    THEME.with(|t| t.get())
}

pub fn palette() -> Palette {
    PALETTE.with(|c| c.get())
}

fn brushes() -> (HBRUSH, HBRUSH) {
    if BRUSHES.with(|b| b.get().0 == 0) {
        set_theme(current_theme());
    }
    let (bg, surface) = BRUSHES.with(|b| b.get());
    (bg as HBRUSH, surface as HBRUSH)
}

pub fn bg_brush() -> HBRUSH {
    brushes().0
}

pub fn surface_brush() -> HBRUSH {
    brushes().1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG relative luminance contrast ratio.
    fn contrast(a: COLORREF, b: COLORREF) -> f64 {
        let lum = |c: COLORREF| {
            let ch = |v: u32| {
                let s = v as f64 / 255.0;
                if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * ch(c & 0xFF) + 0.7152 * ch((c >> 8) & 0xFF) + 0.0722 * ch((c >> 16) & 0xFF)
        };
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn hovered_buttons_keep_readable_text() {
        use crate::ui::{anim::lerp_color, button::FILL_HOVER};
        for (name, p) in [("dark", DARK), ("light", LIGHT), ("contrast", CONTRAST), ("jewish", JEWISH)] {
            let primary = lerp_color(p.accent, p.accent_text, FILL_HOVER);
            assert!(contrast(p.accent_text, primary) >= 4.5, "{name}: hovered primary button");
            let warn = lerp_color(p.warn, p.warn_text, FILL_HOVER);
            assert!(contrast(p.warn_text, warn) >= 4.5, "{name}: hovered warn button");
        }
    }

    #[test]
    fn rgb_is_bgr_packed() {
        assert_eq!(rgb(0x00, 0x38, 0xB8), 0x00B8_3800);
    }

    #[test]
    fn text_is_readable_in_every_theme() {
        for (name, p) in [("dark", DARK), ("light", LIGHT), ("contrast", CONTRAST), ("jewish", JEWISH)] {
            assert!(contrast(p.text, p.bg) >= 7.0, "{name}: text");
            assert!(contrast(p.text, p.surface) >= 7.0, "{name}: text on surface");
            assert!(contrast(p.accent_text, p.accent) >= 4.5, "{name}: button text");
            assert!(contrast(p.warn_text, p.warn) >= 4.5, "{name}: banner text");
            assert!(contrast(p.muted, p.bg) >= 4.0, "{name}: muted text");
        }
        for (name, p) in [("dark", DARK), ("light", LIGHT), ("contrast", CONTRAST), ("jewish", JEWISH)] {
            assert!(contrast(p.text, p.key) >= 7.0, "{name}: keycap text");
            assert!(contrast(p.text, p.selected) >= 7.0, "{name}: text on selected row");
            assert!(contrast(p.text, p.hover) >= 7.0, "{name}: text on hovered row");
            assert!(contrast(p.accent, p.bg) >= 3.0, "{name}: accent (focus ring, switch) is visible");
            assert!(contrast(p.ok, p.bg) >= 3.0, "{name}: status dot is visible");
        }
        assert!(contrast(CONTRAST.accent, CONTRAST.bg) >= 12.0, "contrast theme accent");
        assert!(contrast(JEWISH.star.unwrap(), JEWISH.bg) >= 7.0, "jewish theme star");
    }

    #[test]
    fn text_stays_readable_on_motion_tints() {
        use crate::ui::anim::lerp_color;
        for (name, p) in [("dark", DARK), ("light", LIGHT), ("contrast", CONTRAST), ("jewish", JEWISH)] {
            let peaks = [
                ("selection flash", lerp_color(p.selected, p.accent, TINT)),
                ("ok on bg", lerp_color(p.bg, p.ok, TINT)),
                ("err on bg", lerp_color(p.bg, p.err, TINT)),
                ("ok on selected", lerp_color(p.selected, p.ok, TINT)),
                ("err on selected", lerp_color(p.selected, p.err, TINT)),
            ];
            for (what, bg) in peaks {
                assert!(contrast(p.text, bg) >= 4.5, "{name}: text on {what}");
            }
            assert_ne!(lerp_color(p.bg, p.ok, TINT), lerp_color(p.bg, p.err, TINT), "{name}: ok and err tints differ");
        }
    }

    #[test]
    fn system_theme_follows_windows() {
        assert_eq!(palette_for(Theme::System, true), LIGHT);
        assert_eq!(palette_for(Theme::System, false), DARK);
    }
}
