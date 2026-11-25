//! Main window (docs/DESIGN.md, "Main"): header bar with the master switch,
//! profile and Discord status, a banner while binds are off, search and sort,
//! the binding list, a detail line and the bottom action bar. Motion
//! (docs/DESIGN.md "Motion"): the banner slides only after a click on the
//! switch, the Run button crossfades its result only for a clicked run, and a
//! row flashes when its binding ran (hotkey too) while the window is visible.

mod menus;
mod paint;
mod row;
mod view;

use super::theme::palette;
use super::*;
use crate::app;
use crate::config::Binding;
use crate::i18n::t;
use menus::{more_menu, profile_menu, sort_menu};
use std::cell::RefCell;
use view::{build_view, run_label, run_labels, run_text, sort_short, Row, SortKey, View, SORT_KEYS};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::HDC;

const CLASS: &str = "NoKeyMain";
fn view_w() -> i32 {
    super::page::width()
}
/// Window side padding.
const PAD: i32 = 16;
const GAP: i32 = 8;
const ROW_H: i32 = 40;
const HEADER_H: i32 = 52;
/// Taller header for themes with a Star of David, so the star can be large.
const STAR_HEADER_H: i32 = 96;
const BANNER_H: i32 = 32;
const BAR_H: i32 = 56;
const DETAIL_H: i32 = 24;
const BTN_H: i32 = 32;
/// The sort dropdown fits its widest label within these bounds.
const SORT_MIN_W: i32 = 96;
const SORT_MAX_W: i32 = 200;
const SWITCH_W: i32 = 40;
const SWITCH_H: i32 = 24;
const PROFILE_MAX_W: i32 = 160;
const PROFILE_MIN_W: i32 = 64;
/// Horizontal padding of a text button (both sides together).
const TEXT_BTN_PAD: i32 = 32;
/// Label padding of a dropdown: 10 left, 24 for the arrow, plus slack.
const DROPDOWN_PAD: i32 = 40;
/// Status dot plus its gap before the connection text.
const DOT_SPACE: i32 = 14;
/// Gap between the title (and star) and the header controls.
const HEADER_GAP: i32 = 12;

/// Header height for the current theme.
fn header_h() -> i32 {
    if palette().star.is_some() { STAR_HEADER_H } else { HEADER_H }
}

/// Client height (96-DPI units), shared with the pages. A taller themed
/// header takes its space from the binding list.
fn height() -> i32 {
    super::page::height()
}

/// Places everything again after the window changed size.
pub fn relayout() {
    let Some((ctl, laid, sort_w)) = st(|s| (s.ctl, s.banner_laid, s.sort_w)) else { return };
    bar_layout(&ctl);
    body_layout(&ctl, laid, sort_w);
    refresh(); // places the header controls
    unsafe { RedrawWindow(ctl.hwnd, std::ptr::null(), null_mut(), RDW_INVALIDATE | RDW_ALLCHILDREN) };
}

/// The binding list, which gets the focus back when a page closes.
pub fn list() -> HWND {
    st(|s| s.ctl.list).unwrap_or(null_mut())
}

/// `banner_h`: the banner's current height (0..BANNER_H, animated).
fn search_top(banner_h: i32) -> i32 {
    header_h() + banner_h + 12
}

fn caption_y(banner_h: i32) -> i32 {
    search_top(banner_h) + 48
}

fn list_top(banner_h: i32) -> i32 {
    caption_y(banner_h) + 22
}

/// Banner height for a shown / hidden banner.
fn banner_target(shown: bool) -> i32 {
    if shown { BANNER_H } else { 0 }
}

/// Animation slots on the main window.
const SLOT_BANNER: u32 = 1;
const SLOT_DOT: u32 = 2;

fn list_bottom() -> i32 {
    height() - BAR_H - DETAIL_H - 4
}

const ID_LIST: i32 = 100;
const ID_NEW: i32 = 101;
const ID_EDIT: i32 = 102;
const ID_MORE: i32 = 103;
const ID_RUN: i32 = 105;
const ID_SETTINGS: i32 = 106;
const ID_SWITCH: i32 = 107;
const ID_PROFILE: i32 = 108;
const ID_SEARCH: i32 = 109;
const ID_SORT: i32 = 110;

const TIMER_FLASH: usize = 1;
const TIMER_COOLDOWN: usize = 2;
const FLASH_MS: u32 = 1500;
/// Cooldowns shorter than this don't lock the run button.
const MIN_LOCK_MS: u32 = 250;

/// Window handles (copied out of the state so layout code never holds a borrow
/// while Windows sends messages back).
#[derive(Clone, Copy)]
struct Controls {
    hwnd: HWND,
    list: HWND,
    search: HWND,
    switch: HWND,
    profile: HWND,
    sort: HWND,
    new: HWND,
    edit: HWND,
    more: HWND,
    run: HWND,
    settings: HWND,
}

struct State {
    ctl: Controls,
    selected: Option<u32>,
    view: View,
    flash: Option<(u32, bool)>,
    cooling: bool,
    sort: SortKey,
    show_all: bool,
    told_about_tray: bool,
    /// Layout inputs last applied: banner shown, profile label, connection text.
    laid_out: Option<(bool, String, String)>,
    /// Discord status area in the header (left, right; 96-DPI units).
    status: (i32, i32),
    /// Last banner text, still drawn while the banner slides away.
    banner_msg: String,
    /// The switch is being clicked with the mouse: a banner change slides.
    banner_armed: bool,
    /// Banner height the body was last laid out for.
    banner_laid: i32,
    /// Sort dropdown width (measured on a real relayout, reused per banner frame).
    sort_w: i32,
    /// Binding started with a mouse click on Run: its result crossfades.
    run_armed: Option<u32>,
}

thread_local! {
    static ST: RefCell<Option<State>> = const { RefCell::new(None) };
    /// View options that survive a rebuild (language change).
    static KEEP: RefCell<(SortKey, bool, Option<u32>)> = const { RefCell::new((SortKey::Name, false, None)) };
}

fn st<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    ST.with(|s| s.try_borrow_mut().ok()?.as_mut().map(f))
}

pub fn register(icon: HICON) {
    register_class(CLASS, Some(wndproc), icon);
}

pub fn create() {
    super::page::reset_size();
    create_window(CLASS, "NoKey", view_w(), height(), null_mut());
    refresh();
}

pub fn hwnd() -> HWND {
    st(|s| s.ctl.hwnd).unwrap_or(null_mut())
}

pub fn is_foreground() -> bool {
    let h = hwnd();
    // With a page open the status line is covered: notifications still show.
    !h.is_null() && !super::page::is_open() && unsafe { GetForegroundWindow() } == h
}

pub fn show_main() {
    let h = hwnd();
    if h.is_null() {
        return;
    }
    unsafe {
        if IsIconic(h) != 0 {
            ShowWindow(h, SW_RESTORE);
        } else {
            ShowWindow(h, SW_SHOW);
        }
        SetForegroundWindow(h);
    }
}

pub fn show_minimized() {
    let h = hwnd();
    if !h.is_null() {
        unsafe { ShowWindow(h, SW_SHOWMINNOACTIVE) };
    }
}

pub fn destroy() {
    let h = hwnd();
    if let Some(keep) = st(|s| (s.sort, s.show_all, s.selected)) {
        KEEP.with(|k| *k.borrow_mut() = keep);
    }
    ST.with(|s| *s.borrow_mut() = None);
    if !h.is_null() {
        unsafe { DestroyWindow(h) };
    }
}

/// Re-creates the window (new language / layout direction).
pub fn rebuild() {
    let old = hwnd();
    let was_visible = is_visible(old);
    // Size, position and "maximized" carry over to the new window.
    let mut placement: WINDOWPLACEMENT = unsafe { std::mem::zeroed() };
    placement.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
    let known = was_visible && unsafe { GetWindowPlacement(old, &mut placement) } != 0;
    destroy();
    create();
    if known {
        unsafe { SetWindowPlacement(hwnd(), &placement) };
    }
    if was_visible {
        show_main();
    }
}

pub fn select(id: u32) {
    st(|s| {
        s.selected = Some(id);
        s.cooling = false;
    });
    refresh();
}

/// Width taken by the star next to the title (Jewish theme), with its gap.
fn star_space() -> i32 {
    if palette().star.is_some() { HEADER_GAP + header_h() - 18 } else { 0 }
}

// ---------- refresh ----------

fn fill_list(list: HWND, rows: &[Row], selected: Option<u32>) {
    unsafe {
        SendMessageW(list, WM_SETREDRAW, 0, 0);
        super::kit::reset_list_hover(list);
        SendMessageW(list, LB_RESETCONTENT, 0, 0);
        let mut sel_index = -1isize;
        for (i, row) in rows.iter().enumerate() {
            SendMessageW(list, LB_ADDSTRING, 0, row.id as LPARAM);
            if Some(row.id) == selected {
                sel_index = i as isize;
            }
        }
        SendMessageW(list, LB_SETCURSEL, sel_index as WPARAM, 0);
        SendMessageW(list, WM_SETREDRAW, 1, 0);
        InvalidateRect(list, null(), 1);
    }
    show(list, !rows.is_empty() && !super::page::is_open());
}

/// Rebuilds the view from the app state and updates every control.
pub fn refresh() {
    let Some((ctl, selected, sort, show_all)) = st(|s| (s.ctl, s.selected, s.sort, s.show_all)) else {
        return;
    };
    let query = crate::win::get_text(ctl.search);
    let (view, selected) = app::with(|a| {
        let visible = build_view(a, None, sort, show_all, &query);
        let sel = selected.filter(|id| visible.rows.iter().any(|r| r.id == *id)).or_else(|| visible.rows.first().map(|r| r.id));
        (build_view(a, sel, sort, show_all, &query), sel)
    });
    let banner = !view.all_enabled || view.emergency_error.is_some();
    let key = (banner, view.profile.clone(), view.conn_short.clone());
    let Some((label, cooling, relayout, tint, was_banner)) = st(|s| {
        s.selected = selected;
        s.view = view.clone();
        let tint = s.flash.filter(|(id, _)| Some(*id) == selected).map(|(_, ok)| ok);
        let was = s.laid_out.as_ref().map(|k| k.0);
        (run_label(&view, s.flash, selected), s.cooling, s.laid_out.as_ref() != Some(&key), tint, was)
    }) else {
        return;
    };

    if relayout {
        slide_banner(&ctl, was_banner, banner);
        let status = header_layout(&ctl, &view.profile, &view.conn_short);
        st(|s| {
            s.status = status;
            s.laid_out = Some(key);
        });
    }
    fill_list(ctl.list, &view.rows, selected);
    let has_sel = selected.is_some();
    enable(ctl.edit, has_sel);
    enable(ctl.more, has_sel);
    crate::win::set_text(ctl.run, &label);
    kit::set_tint(ctl.run, tint);
    enable(ctl.run, has_sel && !cooling);
    set_checked(ctl.switch, view.all_enabled);
    crate::win::set_text(ctl.profile, &view.profile);
    let sort_label = SORT_KEYS.iter().find(|(k, _)| *k == sort).map(|(_, key)| sort_short(key)).unwrap_or_default();
    crate::win::set_text(ctl.sort, &sort_label);
    invalidate(ctl.hwnd);
}

/// Called right after a binding was started from the window: locks the run
/// button for the cooldown.
pub fn after_run(id: u32) {
    let cooldown_ms = app::with(|a| a.config.binding(id).map(|b| a.config.cooldown_for(b)).unwrap_or(0));
    st(|s| {
        if s.selected == Some(id) && cooldown_ms >= MIN_LOCK_MS {
            s.cooling = true;
            unsafe { SetTimer(s.ctl.hwnd, TIMER_COOLDOWN, cooldown_ms, None) };
        }
    });
    refresh();
}

/// Briefly shows success or failure on the run button and the detail line,
/// and flashes the binding's row (only while the window is on screen).
pub fn flash(id: u32, ok: bool) {
    let Some((ctl, clicked)) = st(|s| {
        s.flash = Some((id, ok));
        unsafe { SetTimer(s.ctl.hwnd, TIMER_FLASH, FLASH_MS, None) };
        (s.ctl, s.run_armed == Some(id) && s.selected == Some(id))
    }) else {
        return;
    };
    refresh();
    if is_visible(ctl.hwnd) && unsafe { IsIconic(ctl.hwnd) } == 0 {
        let (on, off) = if ok { (kit::ROW_OK, kit::ROW_ERR) } else { (kit::ROW_ERR, kit::ROW_OK) };
        anim::cancel(ctl.list, kit::row_slot(off, id));
        anim::start(ctl.list, kit::row_slot(on, id), 1.0, 0.0, kit::RUN_FLASH_MS);
        if clicked {
            kit::fade_tint(ctl.run, true);
        }
    }
}

/// End of the result flash: a clicked run's tint fades back out.
fn end_flash() {
    if let Some((run, Some(_))) = st(|s| {
        s.flash = None;
        (s.ctl.run, s.run_armed.take())
    }) {
        kit::fade_tint(run, false);
    }
}

// ---------- layout ----------

/// Places the header controls right to left (switch, profile) and returns the
/// area left for the Discord status.
fn header_layout(c: &Controls, profile: &str, conn: &str) -> (i32, i32) {
    let f = fonts();
    let cy = header_h() / 2;
    let switch_x = view_w() - PAD - SWITCH_W;
    place(c.switch, switch_x, cy - SWITCH_H / 2, SWITCH_W, SWITCH_H);
    let title_right = PAD + measure_text(c.hwnd, "NoKey", f.title) + star_space();
    let status_w = DOT_SPACE + measure_text(c.hwnd, conn, f.small);
    let room = switch_x - HEADER_GAP - (title_right + HEADER_GAP) - status_w - HEADER_GAP;
    let want = measure_text(c.hwnd, profile, f.normal) + DROPDOWN_PAD;
    let profile_w = want.min(PROFILE_MAX_W).min(room).max(PROFILE_MIN_W);
    let profile_x = switch_x - HEADER_GAP - profile_w;
    place(c.profile, profile_x, cy - BTN_H / 2, profile_w, BTN_H);
    (title_right + HEADER_GAP, profile_x - HEADER_GAP)
}

/// Width of the sort dropdown: its widest label in the current language.
fn sort_width(hwnd: HWND) -> i32 {
    let widest = SORT_KEYS.iter().map(|(_, key)| measure_text(hwnd, &sort_short(key), fonts().normal)).max().unwrap_or(0);
    (widest + DROPDOWN_PAD).clamp(SORT_MIN_W, SORT_MAX_W)
}

/// Search row and list; they move down while the banner shows. Moved in one
/// DeferWindowPos batch so a sliding banner does not flicker.
fn body_layout(c: &Controls, banner_h: i32, sort_w: i32) {
    let top = search_top(banner_h);
    let field_w = view_w() - 2 * PAD - sort_w - GAP;
    let list_y = list_top(banner_h);
    // The field frame (edit_frames) adds 10 x 8 around the edit control.
    // Rows are inset 4 inside the list, so the selection lines up with PAD.
    let moves = [
        (c.search, PAD + 10, top + 8, field_w - 20, 20),
        (c.sort, view_w() - PAD - sort_w, top + 2, sort_w, BTN_H),
        (c.list, PAD - 4, list_y, view_w() - 2 * (PAD - 4), list_bottom() - list_y),
    ];
    unsafe {
        let mut dwp = BeginDeferWindowPos(moves.len() as i32);
        for (h, x, y, w, hh) in moves {
            if !dwp.is_null() {
                dwp = DeferWindowPos(dwp, h, null_mut(), px(x), px(y), px(w), px(hh), SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }
        if !dwp.is_null() {
            EndDeferWindowPos(dwp);
        }
    }
    st(|s| {
        s.banner_laid = banner_h;
        s.sort_w = sort_w;
    });
}

/// Banner shown/hidden changed (or first layout): slide after a mouse click
/// on the switch, snap otherwise (hotkey, keyboard, emergency error).
fn slide_banner(c: &Controls, was: Option<bool>, now: bool) {
    let armed = st(|s| s.banner_armed).unwrap_or(false);
    match was {
        Some(was) if was != now && armed => {
            let from = anim::value(c.hwnd, SLOT_BANNER, banner_target(was) as f32);
            anim::animate(c.hwnd, SLOT_BANNER, from, banner_target(now) as f32, kit::BANNER_MS);
        }
        Some(was) if was == now => {}
        _ => anim::cancel(c.hwnd, SLOT_BANNER),
    }
    body_layout(c, banner_h(c.hwnd, now), sort_width(c.hwnd));
}

/// Current banner height (animated while sliding).
fn banner_h(hwnd: HWND, shown: bool) -> i32 {
    anim::value(hwnd, SLOT_BANNER, banner_target(shown) as f32).round() as i32
}

/// Animation frame of the main window: follows a sliding banner.
fn on_anim_frame(hwnd: HWND) {
    let Some((ctl, shown, laid, sort_w)) = st(|s| (s.ctl, s.laid_out.as_ref().is_some_and(|k| k.0), s.banner_laid, s.sort_w)) else {
        return;
    };
    let h = banner_h(hwnd, shown);
    if h != laid {
        body_layout(&ctl, h, sort_w);
    }
}

/// Bottom bar: New, Edit, More on the left; Run and Settings on the right.
/// Widths come from the labels, so long translations are never cut.
fn bar_layout(c: &Controls) {
    let f = fonts();
    let y = height() - BAR_H + (BAR_H - BTN_H) / 2;
    let text_w = |s: &str| measure_text(c.hwnd, s, f.bold) + TEXT_BTN_PAD;
    let new_w = text_w(&t("main.new"));
    let edit_w = text_w(&t("main.edit"));
    let mut more_w = measure_text(c.hwnd, &t("main.more"), f.normal) + DROPDOWN_PAD;
    let run_w = run_labels().iter().map(|l| text_w(l)).max().unwrap_or(0);
    let settings_x = view_w() - PAD - BTN_H;
    let edit_x = PAD + new_w + GAP;
    let more_x = edit_x + edit_w + GAP;
    let mut run_x = settings_x - GAP - run_w;
    if more_x + more_w + GAP > run_x {
        // Not enough room for the labelled dropdown: fall back to the icon.
        set_kind(c.more, BTN_ICON);
        crate::win::set_text(c.more, "⋯");
        more_w = BTN_H;
    } else {
        // Room again after the window grew.
        set_kind(c.more, BTN_DROPDOWN);
        crate::win::set_text(c.more, &t("main.more"));
    }
    run_x = run_x.max(more_x + more_w + GAP);
    place(c.new, PAD, y, new_w, BTN_H);
    place(c.edit, edit_x, y, edit_w, BTN_H);
    place(c.more, more_x, y, more_w, BTN_H);
    place(c.run, run_x, y, (settings_x - GAP - run_x).max(BTN_H), BTN_H);
    place(c.settings, settings_x, y, BTN_H, BTN_H);
}

fn on_create(hwnd: HWND) {
    let b = |id, text: &str, kind| button(hwnd, id, text, kind);
    // Fields are evaluated in order, and creation order is the Tab order:
    // keep it the visual order (header, search row, list, action bar).
    let ctl = Controls {
        hwnd,
        profile: b(ID_PROFILE, "", BTN_DROPDOWN),
        switch: b(ID_SWITCH, "", BTN_SWITCH),
        search: edit(hwnd, ID_SEARCH, 0),
        sort: b(ID_SORT, "", BTN_DROPDOWN),
        list: listbox(hwnd, ID_LIST, 0),
        new: b(ID_NEW, &t("main.new"), BTN_PRIMARY),
        edit: b(ID_EDIT, &t("main.edit"), BTN_SECONDARY),
        more: b(ID_MORE, &t("main.more"), BTN_DROPDOWN),
        run: b(ID_RUN, &run_text(&t("main.run")), BTN_SECONDARY),
        settings: b(ID_SETTINGS, "⚙", BTN_ICON),
    };
    set_cue(ctl.search, &t("main.search"));
    kit::track_list_hover(ctl.list);
    bar_layout(&ctl);
    let sort_w = sort_width(hwnd);
    body_layout(&ctl, 0, sort_w);

    add_tooltip(hwnd, ctl.switch, &t("tip.disable_all"));
    add_tooltip(hwnd, ctl.profile, &t("tip.profile"));
    add_tooltip(hwnd, ctl.search, &t("tip.search"));
    add_tooltip(hwnd, ctl.sort, &t("tip.sort"));
    add_tooltip(hwnd, ctl.more, &t("tip.more"));
    add_tooltip(hwnd, ctl.settings, &t("tip.settings"));

    let (sort, show_all, selected) = KEEP.with(|k| *k.borrow());
    ST.with(|s| {
        *s.borrow_mut() = Some(State {
            ctl,
            selected,
            view: View::default(),
            flash: None,
            cooling: false,
            sort,
            show_all,
            told_about_tray: false,
            laid_out: None,
            status: (PAD, view_w() - PAD),
            banner_msg: String::new(),
            banner_armed: false,
            banner_laid: 0,
            sort_w,
            run_armed: None,
        })
    });
}

// ---------- painting ----------

fn on_paint(hwnd: HWND) {
    if super::page::is_open() {
        // Only the background can show, beside a page that is sliding in.
        super::paint(hwnd, |hdc, rc| super::fill(hdc, rc, palette().bg));
        return;
    }
    let Some((v, search, flash, status, mut msg)) =
        st(|s| (s.view.clone(), s.ctl.search, s.flash, s.status, s.banner_msg.clone()))
    else {
        return;
    };
    let shown = paint::banner_text(&v);
    if let Some(text) = &shown {
        msg = text.clone();
        st(|s| s.banner_msg = msg.clone());
    }
    let banner = paint::Banner { height: banner_h(hwnd, shown.is_some()), text: msg };
    // Each pulse frame repaints only the Discord status area of the header.
    let dot_area = r(status.0, 0, status.1 - status.0, header_h());
    let dot_pulse = if v.checking { anim::pulse(hwnd, SLOT_DOT, kit::DOT_PULSE_MS, 0.0, Some(dot_area)) } else { 0.0 };
    paint(hwnd, |hdc, _| paint::window(hwnd, hdc, &v, search, flash, status, &banner, dot_pulse));
}

fn draw_row(dis: &DRAWITEMSTRUCT) {
    if dis.itemID == u32::MAX {
        return;
    }
    if let Some(row) = st(|s| s.view.rows.get(dis.itemID as usize).cloned()).flatten() {
        row::draw(dis, &row);
    }
}

// ---------- commands ----------

fn selected_binding() -> Option<Binding> {
    let id = st(|s| s.selected).flatten()?;
    app::with(|a| a.config.binding(id).cloned())
}

fn hide_to_tray(hwnd: HWND) {
    unsafe { ShowWindow(hwnd, SW_HIDE) };
    let first = st(|s| !std::mem::replace(&mut s.told_about_tray, true)).unwrap_or(false);
    let (notify, host) = app::with(|a| (a.config.notifications, a.host));
    if first && notify {
        crate::win::tray::notify(host, "NoKey", &t("notify.still_running"), false);
    }
}

fn select_at_cursor(list: HWND) {
    let mut pt = POINT { x: 0, y: 0 };
    unsafe {
        GetCursorPos(&mut pt);
        ScreenToClient(list, &mut pt);
    }
    let packed = ((pt.y as u32 & 0xFFFF) << 16 | (pt.x as u32 & 0xFFFF)) as LPARAM;
    let hit = unsafe { SendMessageW(list, LB_ITEMFROMPOINT, 0, packed) } as u32;
    if hit >> 16 == 0 {
        let id = unsafe { SendMessageW(list, LB_GETITEMDATA, (hit & 0xFFFF) as WPARAM, 0) } as u32;
        select(id);
    }
}

fn on_list_selchange(list: HWND) {
    let idx = unsafe { SendMessageW(list, LB_GETCURSEL, 0, 0) };
    if idx < 0 {
        return;
    }
    let data = unsafe { SendMessageW(list, LB_GETITEMDATA, idx as WPARAM, 0) } as u32;
    st(|s| {
        s.selected = Some(data);
        s.cooling = false;
        unsafe { KillTimer(s.ctl.hwnd, TIMER_COOLDOWN) };
    });
    refresh();
}

fn on_command(hwnd: HWND, id: i32, code: u32) {
    let Some(c) = st(|s| s.ctl) else { return };
    match id {
        ID_LIST if code == LBN_SELCHANGE => on_list_selchange(c.list),
        ID_LIST if code == LBN_DBLCLK => on_command(hwnd, ID_EDIT, 0),
        ID_SEARCH if code == EN_CHANGE => refresh(),
        ID_SEARCH if code == EN_SETFOCUS || code == EN_KILLFOCUS => invalidate(hwnd),
        ID_NEW => editor::open(hwnd, None),
        ID_EDIT => {
            if let Some(b) = selected_binding() {
                editor::open(hwnd, Some(b));
            }
        }
        ID_MORE => more_menu(hwnd, c.more),
        ID_RUN => {
            if let Some(b) = selected_binding() {
                st(|s| s.run_armed = anim::by_pointer().then_some(b.id));
                app::run_binding(b.id);
            }
        }
        ID_SETTINGS => settings::open(hwnd),
        ID_SWITCH => {
            st(|s| s.banner_armed = anim::by_pointer());
            app::toggle_all();
            st(|s| s.banner_armed = false);
        }
        ID_PROFILE => profile_menu(hwnd, c.profile),
        ID_SORT => sort_menu(hwnd, c.sort),
        _ => {}
    }
}

/// The binding list sits on `bg` (ctl_color would give list boxes `surface`).
fn list_color(wparam: WPARAM) -> LRESULT {
    let hdc = wparam as HDC;
    let p = palette();
    unsafe {
        SetTextColor(hdc, p.text);
        SetBkColor(hdc, p.bg);
    }
    theme::bg_brush() as LRESULT
}

/// WM_TIMER: animation frames, the end of a result flash, the run cooldown.
fn on_timer(hwnd: HWND, id: usize) {
    if anim::on_timer(hwnd, id) {
        on_anim_frame(hwnd);
        return;
    }
    unsafe { KillTimer(hwnd, id) };
    match id {
        TIMER_FLASH => end_flash(),
        TIMER_COOLDOWN => {
            st(|s| s.cooling = false);
        }
        _ => {}
    }
    refresh();
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_CTLCOLORLISTBOX {
        return list_color(wparam);
    }
    if let Some(r) = ctl_color(msg, wparam) {
        return r;
    }
    match msg {
        WM_CREATE => {
            on_create(hwnd);
            0
        }
        WM_PAINT => {
            on_paint(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_GETMINMAXINFO => {
            let info = unsafe { &mut *(lparam as *mut MINMAXINFO) };
            let (w, h) = super::window_size(super::page::MIN_W, super::page::MIN_H);
            info.ptMinTrackSize.x = w;
            info.ptMinTrackSize.y = h;
            0
        }
        WM_SIZE => {
            if wparam as u32 != SIZE_MINIMIZED && st(|_| ()).is_some() {
                super::page::resized((lparam & 0xFFFF) as i32, ((lparam >> 16) & 0xFFFF) as i32);
            }
            0
        }
        WM_MEASUREITEM => {
            let mis = unsafe { &mut *(lparam as *mut MEASUREITEMSTRUCT) };
            mis.itemHeight = px(ROW_H) as u32;
            1
        }
        WM_DRAWITEM => {
            let dis = unsafe { &*(lparam as *const DRAWITEMSTRUCT) };
            if dis.CtlType == ODT_LISTBOX {
                draw_row(dis);
            } else {
                draw_button(dis);
            }
            1
        }
        WM_COMMAND => {
            let (id, code) = command_parts(wparam);
            on_command(hwnd, id, code);
            0
        }
        WM_CONTEXTMENU => {
            let list = st(|s| s.ctl.list).unwrap_or(null_mut());
            if wparam as HWND == list {
                select_at_cursor(list);
                more_menu(hwnd, list);
            }
            0
        }
        WM_TIMER => {
            on_timer(hwnd, wparam);
            0
        }
        WM_NCDESTROY => {
            anim::forget(hwnd);
            def(hwnd, msg, wparam, lparam)
        }
        WM_SYSCOMMAND if (wparam & 0xFFF0) as u32 == SC_MINIMIZE => {
            if app::with(|a| a.config.minimize_to_tray) {
                hide_to_tray(hwnd);
                0
            } else {
                def(hwnd, msg, wparam, lparam)
            }
        }
        WM_CLOSE => {
            if app::with(|a| a.config.minimize_to_tray) {
                hide_to_tray(hwnd);
            } else if super::page::go_home() {
                // False: the user keeps an edited binding open.
                app::exit();
            }
            0
        }
        _ => def(hwnd, msg, wparam, lparam),
    }
}
