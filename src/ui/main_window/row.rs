//! One binding row (docs/DESIGN.md, "Row"): icon square, name + summary on
//! one line, keycaps or an "Off"/"Paused" chip on the right.

use super::view::Row;
use crate::i18n::t;
use crate::ui::theme::palette;
use crate::ui::{buffered_item, fonts, kit, round_rect, text, text_width};
use crate::win::px;
use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::Controls::{DRAWITEMSTRUCT, ODS_SELECTED};

const GAP: i32 = 8;

/// Draws what sits at the right of a row (keycaps, "+N", or a state chip)
/// ending at `right`; never further left than `min_left`. Returns its left edge.
fn trailing(hdc: HDC, row: &Row, right: i32, min_left: i32, cy: i32, keys: &kit::KeyStyle) -> i32 {
    let p = palette();
    let f = fonts();
    let single = DT_SINGLELINE | DT_VCENTER;
    let half = px(kit::KEY_H) / 2;
    if let Some(state) = &row.state_chip {
        let w = text_width(hdc, state, f.small) + px(16);
        let chip = RECT { left: right - w, top: cy - half, right, bottom: cy + half };
        round_rect(hdc, &chip, p.border, None, 6);
        text(hdc, state, &chip, f.small, p.muted, single | DT_CENTER);
        return chip.left;
    }
    let mut right = right;
    if row.extra > 0 {
        let more = format!("+{}", row.extra);
        let w = text_width(hdc, &more, f.small);
        text(hdc, &more, &RECT { left: right - w, top: cy - half, right, bottom: cy + half }, f.small, p.muted, single);
        right -= w + px(6);
    }
    let fits = !row.keys.is_empty() && right - kit::keycaps_width(hdc, &row.keys) >= min_left;
    if fits {
        return kit::keycaps_right_ex(hdc, &row.keys, right, cy, keys);
    }
    // No shortcut, or keycaps too wide: plain muted text.
    let label = if row.keys.is_empty() { t("main.no_shortcut") } else { row.keys.clone() };
    let left = (right - text_width(hdc, &label, f.small)).max(min_left);
    text(hdc, &label, &RECT { left, top: cy - half, right, bottom: cy + half }, f.small, p.muted, single | DT_END_ELLIPSIS);
    left
}

/// Name (bold) followed by the summary (small, muted) on one line.
fn name_and_summary(hdc: HDC, row: &Row, left: i32, right: i32, rc: &RECT) {
    let p = palette();
    let f = fonts();
    let flags = DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS;
    let name = if row.problem { format!("⚠ {}", row.name) } else { row.name.clone() };
    let color = if row.enabled { p.text } else { p.muted };
    let name_right = (left + text_width(hdc, &name, f.bold)).min(right);
    text(hdc, &name, &RECT { left, right: name_right, ..*rc }, f.bold, color, flags);
    let sum_left = name_right + px(GAP);
    if sum_left + px(24) < right {
        // One pixel lower so the smaller text shares the name's baseline.
        let sum_rc = RECT { left: sum_left, top: rc.top + px(1), right, bottom: rc.bottom + px(1) };
        text(hdc, &row.summary, &sum_rc, f.small, p.muted, flags);
    }
}

/// Paints `row` into the list item described by `dis`, with its motion
/// state (hover glide, selection and run flashes, pressed keycaps).
/// Drawn off-screen and copied at once (`buffered_item`).
pub(super) fn draw(dis: &DRAWITEMSTRUCT, row: &Row) {
    buffered_item(dis.hDC, &dis.rcItem, |hdc, rc| {
        let rc = *rc;
        let p = palette();
        let selected = dis.itemState & ODS_SELECTED != 0;
        let fx = kit::RowFx::read(dis.hwndItem, dis.itemID, row.id);
        kit::row_background_fx(hdc, &rc, selected, &fx);

        let cy = (rc.top + rc.bottom) / 2;
        let icon_left = rc.left + px(12);
        let icon = RECT { left: icon_left, top: cy - px(12), right: icon_left + px(24), bottom: cy + px(12) };
        round_rect(hdc, &icon, p.surface, Some(p.surface), 6);
        let color = if row.enabled { p.text } else { p.muted };
        text(hdc, &row.icon, &icon, fonts().bold, color, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

        let text_left = icon.right + px(10);
        let trailing_left = trailing(hdc, row, rc.right - px(12), text_left + px(120), cy, &fx.keys());
        name_and_summary(hdc, row, text_left, trailing_left - px(12), &rc);
    });
}
