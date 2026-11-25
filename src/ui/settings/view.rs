//! Settings window view: control builders and painting (layout in
//! docs/DESIGN.md). State and behaviour live in the parent module.

use super::*;

/// Top-left of the field frame of timing field `i`.
fn num_frame(i: usize) -> (i32, i32) {
    let (_, _, col, row) = NUMS[i];
    (PAD + col * (num_w() + NUM_GAP), TIMING_Y + 24 + row * NUM_ROW + NUM_LABEL_H + 4)
}

/// Width for a button with this label: bold text + 16 padding each side.
fn btn_w(hdc: HDC, label: &str) -> i32 {
    (units(text_width(hdc, label, fonts().bold)) + 32).max(80)
}

/// An owner-drawn button at (x, y, w, h).
fn mk(hwnd: HWND, id: i32, text: &str, kind: isize, (x, y, w, h): (i32, i32, i32, i32)) -> HWND {
    let btn = button(hwnd, id, text, kind);
    place(btn, x, y, w, h);
    btn
}

/// An edit control inside a field frame whose top-left is (x, frame_y).
fn field(hwnd: HWND, id: i32, style: u32, x: i32, frame_y: i32, frame_w: i32) -> HWND {
    let ed = edit(hwnd, id, style);
    place(ed, x + 10, frame_y + 8, frame_w - 20, FIELD_H - 16);
    ed
}

/// Right-aligned ghost buttons (8 apart) ending at the right padding.
fn buttons_right(hwnd: HWND, hdc: HDC, items: &[(i32, &str)], y: i32) -> Vec<HWND> {
    let mut x = row_left(hdc, items);
    let mut out = Vec::new();
    for (id, label) in items {
        let w = btn_w(hdc, label);
        out.push(mk(hwnd, *id, label, BTN_SECONDARY, (x, y, w, BTN_H)));
        x += w + 8;
    }
    out
}

/// Left edge of a right-aligned row of buttons (8 apart).
fn row_left(hdc: HDC, items: &[(i32, &str)]) -> i32 {
    let total: i32 = items.iter().map(|(_, label)| btn_w(hdc, label) + 8).sum();
    view_w() - PAD - total + 8
}

/// Tabs sized to their labels from the left padding; equal widths if they
/// would not fit.
fn create_tabs(hwnd: HWND, hdc: HDC) -> [HWND; 4] {
    let labels: [String; 4] = std::array::from_fn(|i| t(TAB_KEYS[i]));
    let mut widths: Vec<i32> = labels.iter().map(|l| (units(text_width(hdc, l, fonts().bold)) + 24).max(64)).collect();
    if widths.iter().sum::<i32>() > content_w() {
        widths = vec![content_w() / 4; 4];
    }
    let mut x = PAD;
    std::array::from_fn(|i| {
        // 2px shorter: the strip below is the parent-drawn, sliding underline.
        let tab = mk(hwnd, ID_TAB + i as i32, &labels[i], BTN_TAB, (x, HEADER_H, widths[i], TAB_H - 2));
        x += widths[i];
        tab
    })
}

/// The controls that share the width in columns: theme and language halves,
/// three timing fields per row.
pub(super) fn place_columns(theme_btn: HWND, language_btn: HWND, nums: [HWND; 5]) {
    let half = (content_w() - 8) / 2;
    place(theme_btn, PAD, TOP + 24, half, BTN_H);
    place(language_btn, PAD + half + 8, TOP + 24, half, BTN_H);
    for (i, ed) in nums.into_iter().enumerate() {
        let (x, y) = num_frame(i);
        place(ed, x + 10, y + 8, num_w() - 20, FIELD_H - 16);
    }
}

fn create_general(hwnd: HWND) -> (HWND, HWND, [HWND; 4], [HWND; 5]) {
    let half = (content_w() - 8) / 2;
    let theme_btn = mk(hwnd, ID_THEME, "", BTN_DROPDOWN, (PAD, TOP + 24, half, BTN_H));
    let language_btn = mk(hwnd, ID_LANGUAGE, "", BTN_DROPDOWN, (PAD + half + 8, TOP + 24, half, BTN_H));
    // Switch labels are painted by the window (normal text, left-aligned).
    let toggles: [HWND; 4] = std::array::from_fn(|i| {
        let y = SWITCH_Y + SWITCH_ROW * i as i32;
        mk(hwnd, ID_TOGGLE + i as i32, "", BTN_SWITCH, (view_w() - PAD + 2 - SWITCH_BTN_W, y, SWITCH_BTN_W, SWITCH_ROW))
    });
    let nums: [HWND; 5] = std::array::from_fn(|i| {
        let (x, y) = num_frame(i);
        field(hwnd, ID_NUM + i as i32, 0, x, y, num_w())
    });
    (theme_btn, language_btn, toggles, nums)
}

fn create_discord(hwnd: HWND, hdc: HDC) -> (HWND, Vec<HWND>) {
    let items = [(ID_CONNECT, t("settings.connect")), (ID_DISCONNECT, t("settings.disconnect"))];
    let items: Vec<(i32, &str)> = items.iter().map(|(id, s)| (*id, s.as_str())).collect();
    let token = field(hwnd, ID_TOKEN, ES_PASSWORD as u32, PAD, TOP + 80, row_left(hdc, &items) - 8 - PAD);
    set_cue(token, &t("settings.token_cue"));
    (token, buttons_right(hwnd, hdc, &items, TOP + 82))
}

/// Returns (tab controls, list, name edit, Record button, room for the shortcut).
fn create_profiles(hwnd: HWND, hdc: HDC) -> (Vec<HWND>, HWND, HWND, HWND, i32) {
    let list = listbox(hwnd, ID_PROFILES, 0);
    place(list, PAD, TOP + 24, content_w(), LIST_ROW * 4);
    kit::track_list_hover(list);
    let labels = [t("settings.profile_add"), t("settings.profile_rename")];
    let items = [(ID_P_ADD, labels[0].as_str()), (ID_P_RENAME, labels[1].as_str())];
    let name = field(hwnd, ID_PROFILE_NAME, 0, PAD, TOP + 196, row_left(hdc, &items) - 8 - PAD);
    set_cue(name, &t("settings.profile_name_cue"));
    let mut group = buttons_right(hwnd, hdc, &items, TOP + 198);
    let activate = t("settings.profile_activate");
    group.push(mk(hwnd, ID_P_ACTIVATE, &activate, BTN_SECONDARY, (PAD, TOP + 244, btn_w(hdc, &activate), BTN_H)));
    let labels = [t("settings.profile_duplicate"), t("settings.profile_delete")];
    group.extend(buttons_right(hwnd, hdc, &[(ID_P_DUPLICATE, &labels[0]), (ID_P_DELETE, &labels[1])], TOP + 244));
    // Record shows "Stop" while recording: size it for the longer label.
    let rec_label = t("editor.record");
    let rec_w = btn_w(hdc, &rec_label).max(btn_w(hdc, &t("editor.stop")));
    let clear = t("settings.clear");
    let clear_w = btn_w(hdc, &clear);
    let clear_x = view_w() - PAD - clear_w;
    let rec_x = clear_x - 8 - rec_w;
    let record = mk(hwnd, ID_SWITCH_RECORD, &rec_label, BTN_SECONDARY, (rec_x, TOP + 324, rec_w, BTN_H));
    group.push(record);
    group.push(mk(hwnd, ID_SWITCH_CLEAR, &clear, BTN_SECONDARY, (clear_x, TOP + 324, clear_w, BTN_H)));
    group.extend([list, name]);
    (group, list, name, record, rec_x - 12 - PAD)
}

fn create_updates(hwnd: HWND, hdc: HDC) -> (HWND, HWND) {
    let label = t("settings.check_updates");
    let w = btn_w(hdc, &label);
    let repo = field(hwnd, ID_REPO, 0, PAD, TOP + 92, content_w() - w - 8);
    let btn_check = mk(hwnd, ID_CHECK, &label, BTN_SECONDARY, (view_w() - PAD - w, TOP + 94, w, BTN_H));
    set_cue(repo, "owner/repository");
    (repo, btn_check)
}

/// Action bar: Close (ghost) then Save (primary, the window's one primary) on the right.
fn create_bar(hwnd: HWND, hdc: HDC) {
    let (save, close) = (t("common.save"), t("common.close"));
    let (sw, cw) = (btn_w(hdc, &save), btn_w(hdc, &close));
    let y = bar_y() + (BAR_H - BTN_H) / 2;
    mk(hwnd, ID_CLOSE, &close, BTN_SECONDARY, (view_w() - PAD - sw - 8 - cw, y, cw, BTN_H));
    mk(hwnd, ID_SAVE, &save, BTN_PRIMARY, (view_w() - PAD - sw, y, sw, BTN_H));
}

pub(super) fn on_create(hwnd: HWND) {
    let hdc = unsafe { GetDC(hwnd) };
    page::home_button(hwnd); // first: it is also first in the Tab order
    let tabs = create_tabs(hwnd, hdc);
    let (theme_btn, language_btn, toggles, nums) = create_general(hwnd);
    let mut general = vec![theme_btn, language_btn];
    general.extend(toggles);
    general.extend(nums);
    let (token, mut discord) = create_discord(hwnd, hdc);
    let btn_disconnect = discord[1];
    discord.push(token);
    let (profiles_group, profiles, profile_name, switch_record, shortcut_w) = create_profiles(hwnd, hdc);
    let (repo, btn_check) = create_updates(hwnd, hdc);
    create_bar(hwnd, hdc);
    unsafe { ReleaseDC(hwnd, hdc) };

    for (ed, (_, tip, ..)) in nums.iter().zip(NUMS) {
        add_tooltip(hwnd, *ed, &t(tip));
    }
    add_tooltip(hwnd, theme_btn, &t("tip.theme"));
    add_tooltip(hwnd, switch_record, &t("tip.profile_switch"));

    let mut edits = nums.to_vec();
    edits.extend([token, profile_name, repo]);
    let groups = [general, discord, profiles_group, vec![repo, btn_check]];
    let (tab, profile_ids, recording, switch_info, update_status) = (TAB_GENERAL, Vec::new(), false, String::new(), String::new());
    let state = Se { hwnd, tab, tabs, groups, edits, theme_btn, language_btn, toggles, nums, token, btn_disconnect, profiles,
        profile_ids, profile_name, switch_record, shortcut_w, recording, switch_info, repo, btn_check, update_status };
    SE.with(|s| *s.borrow_mut() = Some(state));
}

/// Small muted text whose bottom sits at `bottom` (wraps upward when long).
fn label_above(hdc: HDC, s: &str, x: i32, bottom: i32, w: i32) {
    let wide_s: Vec<u16> = s.encode_utf16().collect();
    let mut calc = r(x, 0, w, 0);
    unsafe {
        SelectObject(hdc, fonts().small as _);
        DrawTextW(hdc, wide_s.as_ptr(), wide_s.len() as i32, &mut calc, DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX);
    }
    let h = (calc.bottom - calc.top).min(px(NUM_LABEL_H));
    let rc = RECT { top: px(bottom) - h, bottom: px(bottom), ..r(x, 0, w, 0) };
    text(hdc, s, &rc, fonts().small, palette().muted, DT_WORDBREAK | DT_END_ELLIPSIS);
}

fn paint_general(hdc: HDC) {
    let p = palette();
    let f = fonts();
    let half = (content_w() - 8) / 2;
    caption(hdc, &t("settings.theme"), PAD, TOP, half);
    caption(hdc, &t("settings.language"), PAD + half + 8, TOP, half);
    let label_w = content_w() - SWITCH_BTN_W - 12;
    for (i, key) in TOGGLES.iter().enumerate() {
        let rc = r(PAD, SWITCH_Y + SWITCH_ROW * i as i32, label_w, SWITCH_ROW);
        text(hdc, &t(key), &rc, f.normal, p.text, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
    }
    caption(hdc, &t("settings.timing"), PAD, TIMING_Y, content_w());
    for (i, (key, ..)) in NUMS.iter().enumerate() {
        let (x, y) = num_frame(i);
        label_above(hdc, &t(key), x, y - 4, num_w());
    }
    let hint_y = num_frame(4).1 + FIELD_H + 12;
    text(hdc, &t("settings.timing_hint"), &r(PAD, hint_y, content_w(), bar_y() - 8 - hint_y), f.small, p.muted, DT_WORDBREAK | DT_END_ELLIPSIS);
}

/// `dot_pulse`: 0..1 while the token is being checked (the dot pulses).
fn paint_discord(hdc: HDC, dot_pulse: Option<f32>) {
    let p = palette();
    let f = fonts();
    let (connected, conn_line, masked) =
        app::with(|a| (matches!(a.conn, Conn::Connected(_)), a.conn.line(), a.token.as_deref().map(mask)));
    caption(hdc, &t("settings.discord_bot"), PAD, TOP, content_w());
    kit::status_dot_ex(hdc, px(PAD + 4), px(TOP + 36), connected || dot_pulse.is_some(), dot_pulse.unwrap_or(0.0));
    text(hdc, &conn_line, &r(PAD + 16, TOP + 24, content_w() - 16, 24), f.bold, p.text, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
    let note = match masked {
        Some(m) => tf("settings.token_saved", &[("mask", &m)]),
        None => t("settings.token_note"),
    };
    text(hdc, &note, &r(PAD, TOP + 50, content_w(), 18), f.small, p.muted, DT_SINGLELINE | DT_END_ELLIPSIS);
    text(hdc, &t("settings.discord_help"), &r(PAD, TOP + 132, content_w(), 48), f.small, p.muted, DT_WORDBREAK | DT_END_ELLIPSIS);
}

fn paint_profiles(hdc: HDC, switch_info: &str, recording: bool, shortcut_w: i32) {
    let p = palette();
    let f = fonts();
    let switch = app::with(|a| a.config.profile_switch.as_ref().map(|s| s.label()));
    caption(hdc, &t("settings.profiles"), PAD, TOP, content_w());
    caption(hdc, &t("settings.profile_switch"), PAD, TOP + 300, content_w());
    let row = r(PAD, TOP + 324, shortcut_w, BTN_H);
    match (recording, switch) {
        (false, Some(label)) => {
            kit::keycaps(hdc, &label, row.left, (row.top + row.bottom) / 2);
        }
        (true, _) => text(hdc, &t("editor.press_keys"), &row, f.bold, p.text, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS),
        (false, None) => text(hdc, &t("settings.not_set"), &row, f.normal, p.muted, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS),
    }
    text(hdc, switch_info, &r(PAD, TOP + 364, content_w(), 36), f.small, p.muted, DT_WORDBREAK | DT_END_ELLIPSIS);
}

fn paint_updates(hdc: HDC, status: &str) {
    let p = palette();
    let f = fonts();
    caption(hdc, &t("settings.updates"), PAD, TOP, content_w());
    let ver = tf("settings.installed_version", &[("version", CURRENT_VERSION)]);
    text(hdc, &ver, &r(PAD, TOP + 24, content_w(), 20), f.normal, p.text, DT_SINGLELINE | DT_END_ELLIPSIS);
    caption(hdc, &t("settings.repo"), PAD, TOP + 68, content_w());
    text(hdc, status, &r(PAD, TOP + 144, content_w(), 36), f.small, p.text, DT_WORDBREAK | DT_END_ELLIPSIS);
    text(hdc, &t("settings.update_help"), &r(PAD, TOP + 188, content_w(), 48), f.small, p.muted, DT_WORDBREAK | DT_END_ELLIPSIS);
}

/// Header bar with the title, the hairline under the tabs and the action bar line.
fn paint_chrome(hdc: HDC, hwnd: HWND) {
    page::paint_header(hdc, hwnd, &t("settings.title"));
    hline(hdc, 0, HEADER_H + TAB_H, view_w());
    hline(hdc, 0, bar_y(), view_w());
}

pub(super) fn on_paint(hwnd: HWND) {
    let Some((tab, tabs, edits, status, switch_info, recording, shortcut_w)) = se(|s| {
        (s.tab, s.tabs, s.edits.clone(), s.update_status.clone(), s.switch_info.clone(), s.recording, s.shortcut_w)
    }) else {
        return;
    };
    let checking = tab == TAB_DISCORD && app::with(|a| matches!(a.conn, Conn::Checking));
    // Each pulse frame repaints only the dot (centered at PAD + 4, TOP + 36).
    let dot_area = r(PAD - 2, TOP + 30, 12, 12);
    let dot_pulse = checking.then(|| anim::pulse(hwnd, SLOT_DOT, kit::DOT_PULSE_MS, 0.0, Some(dot_area)));
    paint(hwnd, |hdc, _| {
        paint_chrome(hdc, hwnd);
        kit::tab_underline(hdc, hwnd, tabs[tab]);
        edit_frames(hwnd, hdc, &edits);
        match tab {
            TAB_GENERAL => paint_general(hdc),
            TAB_DISCORD => paint_discord(hdc, dot_pulse),
            TAB_PROFILES => paint_profiles(hdc, &switch_info, recording, shortcut_w),
            _ => paint_updates(hdc, &status),
        }
    });
}

/// Profile row (DESIGN.md "Row"): icon square (✓ on accent when active,
/// else the first letter), bold name, binding count right-aligned.
pub(super) fn draw_profile_row(dis: &DRAWITEMSTRUCT) {
    if dis.itemID == u32::MAX {
        return;
    }
    let Some(id) = se(|s| s.profile_ids.get(dis.itemID as usize).copied()).flatten() else { return };
    let (name, active, count) = app::with(|a| {
        let name = a.config.profile(id).map(|p| p.name.clone()).unwrap_or_default();
        let count = a.config.bindings.iter().filter(|b| b.profile == id).count();
        (name, a.config.active_profile == id, count)
    });
    let p = palette();
    let f = fonts();
    buffered_item(dis.hDC, &dis.rcItem, |hdc, rc| {
        let rc = *rc;
        let selected = dis.itemState & ODS_SELECTED != 0;
        kit::row_background_fx(hdc, &rc, selected, &kit::RowFx::read(dis.hwndItem, dis.itemID, id));
        let size = px(24);
        let top = rc.top + (rc.bottom - rc.top - size) / 2;
        let icon = RECT { left: rc.left + px(12), top, right: rc.left + px(12) + size, bottom: top + size };
        let (fill_c, fg, glyph) = if active {
            (p.accent, p.accent_text, "✓".to_string())
        } else {
            (p.surface, p.text, name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default())
        };
        round_rect(hdc, &icon, fill_c, Some(fill_c), 6);
        text(hdc, &glyph, &icon, f.bold, fg, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        let mut note = tf("settings.profile_bindings", &[("count", &count.to_string())]);
        if is_builtin_profile(id) {
            note = format!("{} · {note}", t("settings.profile_builtin"));
        }
        let note_left = rc.right - px(12) - text_width(hdc, &note, f.small);
        let right = RECT { left: note_left, right: rc.right - px(12), ..rc };
        text(hdc, &note, &right, f.small, p.muted, DT_SINGLELINE | DT_VCENTER | DT_RIGHT);
        let left = RECT { left: icon.right + px(12), right: note_left - px(12), ..rc };
        text(hdc, &name, &left, f.bold, p.text, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
    });
}
