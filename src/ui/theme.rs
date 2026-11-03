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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub bg: COLORREF,
    /// Inputs, list rows, cards.
    pub surface: COLORREF,
    /// Selected list row.
    pub selected: COLORREF,
    pub text: COLORREF,
    pub muted: COLORREF,
    pub border: COLORREF,
    pub line: COLORREF,
    /// Primary buttons and highlights.
    pub accent: COLORREF,
    pub accent_text: COLORREF,
    /// Background of the "all bindings disabled" banner.
    pub warn: COLORREF,
    pub warn_text: COLORREF,
    /// Decorative header pattern (Jewish theme only).
    pub pattern: Option<COLORREF>,
    /// Star of David in the main window header (Jewish theme only).
    pub star: Option<COLORREF>,
    /// Dark title bar.
    pub dark: bool,
}

pub const DARK: Palette = Palette {
    bg: rgb(0, 0, 0),
    surface: rgb(0x17, 0x17, 0x17),
    selected: rgb(0x26, 0x26, 0x26),
    text: rgb(0xFF, 0xFF, 0xFF),
    muted: rgb(0x77, 0x77, 0x77),
    border: rgb(0x77, 0x77, 0x77),
    line: rgb(0x2A, 0x2A, 0x2A),
    accent: rgb(0xFF, 0xFF, 0xFF),
    accent_text: rgb(0, 0, 0),
    warn: rgb(0xE5, 0xE5, 0xE5),
    warn_text: rgb(0, 0, 0),
    pattern: None,
    star: None,
    dark: true,
};

pub const LIGHT: Palette = Palette {
    bg: rgb(0xFF, 0xFF, 0xFF),
    surface: rgb(0xF2, 0xF2, 0xF2),
    selected: rgb(0xE5, 0xE5, 0xE5),
    text: rgb(0x11, 0x11, 0x11),
    muted: rgb(0x66, 0x66, 0x66),
    border: rgb(0xB5, 0xB5, 0xB5),
    line: rgb(0xE5, 0xE5, 0xE5),
    accent: rgb(0x11, 0x11, 0x11),
    accent_text: rgb(0xFF, 0xFF, 0xFF),
    warn: rgb(0x17, 0x17, 0x17),
    warn_text: rgb(0xFF, 0xFF, 0xFF),
    pattern: None,
    star: None,
    dark: false,
};

/// High contrast: black, white and yellow only.
pub const CONTRAST: Palette = Palette {
    bg: rgb(0, 0, 0),
    surface: rgb(0, 0, 0),
    selected: rgb(0x33, 0x2B, 0x00),
    text: rgb(0xFF, 0xFF, 0xFF),
    muted: rgb(0xFF, 0xFF, 0xFF),
    border: rgb(0xFF, 0xFF, 0xFF),
    line: rgb(0xFF, 0xFF, 0xFF),
    accent: rgb(0xFF, 0xD4, 0x00),
    accent_text: rgb(0, 0, 0),
    warn: rgb(0xFF, 0xD4, 0x00),
    warn_text: rgb(0, 0, 0),
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
    text: rgb(0x0A, 0x1A, 0x47),
    muted: rgb(0x2E, 0x4A, 0x8C),
    border: rgb(0x00, 0x38, 0xB8),
    line: rgb(0xB9, 0xCC, 0xF5),
    accent: rgb(0x00, 0x38, 0xB8),
    accent_text: rgb(0xFF, 0xFF, 0xFF),
    warn: rgb(0x0A, 0x1A, 0x47),
    warn_text: rgb(0xFF, 0xFF, 0xFF),
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
        assert!(contrast(CONTRAST.accent, CONTRAST.bg) >= 12.0, "contrast theme accent");
        assert!(contrast(JEWISH.star.unwrap(), JEWISH.bg) >= 7.0, "jewish theme star");
    }

    #[test]
    fn system_theme_follows_windows() {
        assert_eq!(palette_for(Theme::System, true), LIGHT);
        assert_eq!(palette_for(Theme::System, false), DARK);
    }
}
