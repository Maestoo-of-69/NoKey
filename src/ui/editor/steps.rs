//! Actions tab of the binding editor: an ordered list of steps, the fields
//! of the selected step, the delay between steps and "stop on failure".

use super::super::theme::palette;
use super::super::*;
use super::layout::{labels, muted, muted_centered, place_field, Measure, BTN_EXTRA, DROP_EXTRA, TOGGLE_EXTRA};
use super::type_text::{self, TypeCtl};
use super::{ed, layout as editor_layout, CAP, content_w, FRAME_H, PAD, TOP, view_w};
use crate::app::{self, api_error_text, TaskResult};
use crate::config::{timing, Action, ActionKind, Binding, MAX_ACTIONS, STEP_DELAY_RANGE};
use crate::discord::api::ApiError;
use crate::discord::mentions::is_snowflake;
use crate::discord::models::{Channel, Guild, User};
use crate::i18n::{self, t, tf};
use crate::win::{get_text, set_text, shell};
use windows_sys::Win32::Foundation::{HWND, LPARAM, WPARAM};

const ID_STEPS: i32 = 360;
const ID_ADD: i32 = 361;
const ID_REMOVE: i32 = 362;
const ID_UP: i32 = 363;
const ID_DOWN: i32 = 364;
const ID_FIELD: i32 = 370; // 370..373
const ID_FIELD_BTN: i32 = 380; // 380..383
const ID_DELAY: i32 = 390;
const ID_STOP: i32 = 391;

// Actions tab rows (96-DPI units). The list frame holds three rows.
pub(super) const ROW_H: i32 = 28;
const LIST_Y: i32 = TOP + CAP;
const LIST_H: i32 = 3 * ROW_H + 8;
const BUTTONS_Y: i32 = LIST_Y + LIST_H + 4;
const DELAY_Y: i32 = BUTTONS_Y + 32 + 12;
const STEP_Y: i32 = DELAY_Y + FRAME_H + CAP;
pub(super) const FIELDS_Y: i32 = STEP_Y + CAP;
const DELAY_W: i32 = 80;
/// Field labels sit in a measured column on the left, wrapped past the cap.
pub(super) const LABEL_W: (i32, i32) = (72, 160);
const LINE: i32 = 16;

pub(in crate::ui) struct Steps {
    pub actions: Vec<Action>,
    cur: Option<usize>,
    /// True while the fields are being filled from a step: their change
    /// notifications must not be saved back half-filled.
    loading: bool,
    list: HWND,
    remove_btn: HWND,
    up_btn: HWND,
    down_btn: HWND,
    add_btn: HWND,
    fields: [HWND; 3],
    field_btns: [HWND; 3],
    /// Controls of a Type text step (shown instead of `fields`).
    typing: TypeCtl,
    delay: HWND,
    t_stop: HWND,
    delay_label_w: i32,
    field_btn_w: i32,
    geo: FieldGeo,
}

impl Steps {
    pub fn edits(&self) -> Vec<HWND> {
        let mut v = self.fields.to_vec();
        v.push(self.delay);
        v.push(self.typing.text);
        v
    }
}

/// Where the selected step's fields are (depends only on the action kind).
#[derive(Clone, Copy, Default)]
struct FieldGeo {
    label_w: i32,
    rows: [i32; 3],
    hint_h: [i32; 3],
}

fn st<R>(f: impl FnOnce(&mut Steps) -> R) -> Option<R> {
    ed(|e| f(&mut e.steps))
}

// ---------- step fields ----------

struct FieldSpec {
    label: &'static str,
    button: Option<&'static str>,
    cue: &'static str,
}

fn spec(kind: ActionKind) -> [Option<FieldSpec>; 3] {
    let f = |label, button, cue| Some(FieldSpec { label, button, cue });
    match kind {
        ActionKind::DiscordPing => [
            f("field.server_id", Some("field.load"), "hint.server_cue"),
            f("field.channel_id", Some("field.load"), "hint.channel_cue"),
            f("field.user_id", Some("field.check"), "hint.user_cue"),
        ],
        ActionKind::OpenApp => {
            [f("field.app", Some("field.browse"), "hint.app_cue"), f("field.args", None, "hint.args_cue"), None]
        }
        ActionKind::OpenFile => [f("field.file", Some("field.browse"), "hint.file_cue"), None, None],
        ActionKind::OpenFolder => [f("field.folder", Some("field.browse"), "hint.folder_cue"), None, None],
        ActionKind::OpenUrl => [f("field.url", None, "hint.url_cue"), None, None],
        ActionKind::TypeText => [None, None, None], // own controls: type_text.rs
    }
}

fn values(a: &Action) -> [String; 3] {
    match a {
        Action::DiscordPing { guild_id, channel_id, user_id, .. } => {
            [guild_id.clone(), channel_id.clone(), user_id.clone()]
        }
        Action::OpenApp { path, args } => [path.clone(), args.clone(), String::new()],
        Action::OpenFile { path } | Action::OpenFolder { path } => [path.clone(), String::new(), String::new()],
        Action::OpenUrl { url } => [url.clone(), String::new(), String::new()],
        Action::TypeText { .. } => Default::default(),
    }
}

/// Builds the action from the field texts, keeping looked-up names only
/// while their id is unchanged.
fn from_fields(old: &Action, v: &[String; 3]) -> Action {
    match old {
        Action::DiscordPing { guild_id, guild_name, channel_id, channel_name, user_id, user_name } => {
            let keep = |old_id: &String, name: &String, new_id: &String| {
                if old_id == new_id { name.clone() } else { String::new() }
            };
            Action::DiscordPing {
                guild_id: v[0].clone(),
                guild_name: keep(guild_id, guild_name, &v[0]),
                channel_id: v[1].clone(),
                channel_name: keep(channel_id, channel_name, &v[1]),
                user_id: v[2].clone(),
                user_name: keep(user_id, user_name, &v[2]),
            }
        }
        Action::OpenApp { .. } => Action::OpenApp { path: v[0].clone(), args: v[1].clone() },
        Action::OpenFile { .. } => Action::OpenFile { path: v[0].clone() },
        Action::OpenFolder { .. } => Action::OpenFolder { path: v[0].clone() },
        Action::OpenUrl { .. } => Action::OpenUrl { url: v[0].clone() },
        Action::TypeText { .. } => old.clone(),
    }
}

fn hint(a: &Action, i: usize) -> String {
    match (a, i) {
        (Action::DiscordPing { guild_name, .. }, 0) if !guild_name.is_empty() => guild_name.clone(),
        (Action::DiscordPing { channel_name, .. }, 1) if !channel_name.is_empty() => format!("#{channel_name}"),
        (Action::DiscordPing { user_name, .. }, 2) if !user_name.is_empty() => format!("@{user_name}"),
        (Action::DiscordPing { .. }, 2) => t("hint.developer_mode"),
        (Action::OpenApp { .. }, 0) => t("hint.app"),
        (Action::OpenUrl { .. }, 0) => t("hint.url"),
        _ => String::new(),
    }
}

/// Label column, frame rows and hint heights for a kind's fields. Hints get
/// at least one line (looked-up names appear there later).
fn field_geometry(kind: ActionKind) -> FieldGeo {
    let m = Measure::window(super::hwnd());
    let small = fonts().small;
    let specs = spec(kind);
    let names: Vec<String> = specs.iter().flatten().map(|sp| t(sp.label)).collect();
    let label_w = (m.widest(&names, small) + 4).clamp(LABEL_W.0, LABEL_W.1);
    let hint_w = view_w() - PAD - (PAD + label_w + 12);
    let empty = kind.empty();
    // Three fields (Discord) end at 556 with two-line hints; the action bar starts at 564.
    let max_lines = if names.len() == 3 { 2 } else { 3 };
    let mut geo = FieldGeo { label_w, ..FieldGeo::default() };
    let mut y = FIELDS_Y;
    for (j, sp) in specs.iter().enumerate() {
        if sp.is_none() {
            continue;
        }
        geo.rows[j] = y;
        geo.hint_h[j] = m.height(&hint(&empty, j), small, hint_w).clamp(LINE, max_lines * LINE);
        y += FRAME_H + 4 + geo.hint_h[j] + 8;
    }
    geo
}

// ---------- creation ----------

/// Add / Remove / Up / Down under the list, each sized to its label.
fn create_list_buttons(hwnd: HWND, m: &Measure) -> [HWND; 4] {
    let f = fonts();
    let mut x = PAD;
    let items = [
        (ID_ADD, "editor.add_action", BTN_DROPDOWN), (ID_REMOVE, "editor.remove", BTN_SECONDARY),
        (ID_UP, "editor.up", BTN_SECONDARY), (ID_DOWN, "editor.down", BTN_SECONDARY),
    ];
    items.map(|(id, key, kind)| {
        let label = t(key);
        let (font, extra) = if kind == BTN_DROPDOWN { (f.normal, DROP_EXTRA) } else { (f.bold, BTN_EXTRA) };
        let w = (m.width(&label, font) + extra).max(64);
        let btn = button(hwnd, id, &label, kind);
        place(btn, x, BUTTONS_Y, w, 32);
        x += w + 8;
        btn
    })
}

/// "Pause between actions" label + field, then "stop on failure". Returns
/// the field, the checkbox and the label width.
fn create_delay_row(hwnd: HWND, m: &Measure) -> (HWND, HWND, i32) {
    let f = fonts();
    let label_w = m.width(&t("editor.step_delay"), f.small) + 4;
    let delay = edit(hwnd, ID_DELAY, 0);
    let delay_x = PAD + label_w + 12;
    place_field(delay, delay_x, DELAY_Y, DELAY_W);
    let stop_label = t("editor.stop_on_failure");
    let t_stop = button(hwnd, ID_STOP, &stop_label, BTN_TOGGLE);
    let stop_x = delay_x + DELAY_W + 24;
    let stop_w = (m.width(&stop_label, f.normal) + TOGGLE_EXTRA).min(view_w() - PAD - stop_x);
    place(t_stop, stop_x, DELAY_Y + 4, stop_w, 28);
    (delay, t_stop, label_w)
}

impl Steps {
    /// The step list; it stretches with the window, like `stop_toggle`
    /// (the rest of the tab is re-placed by `layout`).
    pub(super) fn list(&self) -> HWND {
        self.list
    }

    pub(super) fn stop_toggle(&self) -> HWND {
        self.t_stop
    }
}

pub(super) fn create(hwnd: HWND) -> (Steps, Vec<HWND>) {
    let m = Measure::window(hwnd);
    let list = listbox(hwnd, ID_STEPS, 0);
    place(list, PAD + 4, LIST_Y + 4, content_w() - 8, LIST_H - 8);
    kit::track_list_hover(list);
    let [add_btn, remove_btn, up_btn, down_btn] = create_list_buttons(hwnd, &m);
    let (delay, t_stop, delay_label_w) = create_delay_row(hwnd, &m);
    let field_labels = labels(&["field.load", "field.check", "field.browse", "field.loading"]);
    let field_btn_w = (m.widest(&field_labels, fonts().bold) + BTN_EXTRA).max(88);
    // Each field before its button, so Tab goes field -> button -> next field.
    let pairs: [(HWND, HWND); 3] = std::array::from_fn(|i| {
        let field = edit(hwnd, ID_FIELD + i as i32, 0);
        (field, button(hwnd, ID_FIELD_BTN + i as i32, "", BTN_SECONDARY))
    });
    let (fields, field_btns) = (pairs.map(|p| p.0), pairs.map(|p| p.1));
    let typing = type_text::create(hwnd);
    add_tooltip(hwnd, delay, &t("tip.step_delay"));
    add_tooltip(hwnd, t_stop, &t("tip.stop_on_failure"));
    add_tooltip(hwnd, add_btn, &t("tip.add_action"));
    let mut group = vec![list, add_btn, remove_btn, up_btn, down_btn, delay, t_stop];
    group.extend(fields);
    group.extend(field_btns);
    group.extend(typing.all());
    let steps = Steps {
        actions: Vec::new(),
        cur: None,
        loading: false,
        list,
        remove_btn,
        up_btn,
        down_btn,
        add_btn,
        fields,
        field_btns,
        typing,
        delay,
        t_stop,
        delay_label_w,
        field_btn_w,
        geo: FieldGeo::default(),
    };
    (steps, group)
}

pub(super) fn load(b: &Binding) {
    let comma = i18n::language().decimal_comma();
    let Some((delay, t_stop)) = st(|s| {
        s.actions = b.actions.clone();
        (s.delay, s.t_stop)
    }) else {
        return;
    };
    set_text(delay, &timing::format_seconds(b.step_delay_ms, comma));
    set_checked(t_stop, b.stop_on_failure);
    reload_list(Some(0));
}

fn reload_list(select_index: Option<usize>) {
    let Some((list, count)) = st(|s| (s.list, s.actions.len())) else { return };
    unsafe {
        super::kit::reset_list_hover(list);
        SendMessageW(list, LB_RESETCONTENT, 0, 0);
        for i in 0..count {
            SendMessageW(list, LB_ADDSTRING, 0, i as LPARAM);
        }
    }
    let sel = select_index.filter(|&i| i < count).or(if count > 0 { Some(0) } else { None });
    st(|s| s.cur = sel);
    unsafe { SendMessageW(list, LB_SETCURSEL, sel.unwrap_or(usize::MAX) as WPARAM, 0) };
    load_fields();
}

/// Puts the selected step into the fields.
fn load_fields() {
    let Some((action, fields, typing)) =
        st(|s| (s.cur.and_then(|i| s.actions.get(i).cloned()), s.fields, s.typing))
    else {
        return;
    };
    let vals = action.as_ref().map(values).unwrap_or_default();
    st(|s| s.loading = true);
    for (f, v) in fields.iter().zip(vals) {
        set_text(*f, &v);
    }
    if let Some(a) = action.as_ref().filter(|a| a.kind() == ActionKind::TypeText) {
        type_text::load(&typing, a);
    }
    st(|s| s.loading = false);
    editor_layout();
}

/// Saves the field texts into the selected step.
pub(super) fn commit() {
    let Some((cur, fields, typing, kind)) =
        st(|s| (s.cur, s.fields, s.typing, s.cur.and_then(|i| s.actions.get(i)).map(Action::kind)))
    else {
        return;
    };
    let Some(i) = cur else { return };
    if kind == Some(ActionKind::TypeText) {
        let action = type_text::read(&typing);
        st(|s| s.actions[i] = action);
        return;
    }
    let vals: [String; 3] = fields.map(|f| get_text(f).trim().to_string());
    st(|s| {
        if let Some(old) = s.actions.get(i).cloned() {
            s.actions[i] = from_fields(&old, &vals);
        }
    });
}

pub(super) fn select(i: usize) {
    commit();
    reload_list(Some(i));
}

pub(super) fn read_options() -> Result<(u32, bool), String> {
    let Some((delay, t_stop)) = st(|s| (s.delay, s.t_stop)) else { return Ok((0, true)) };
    let ms = super::seconds(&get_text(delay), STEP_DELAY_RANGE, &t("editor.step_delay"))?;
    Ok((ms, is_checked(t_stop)))
}

/// Shows, places and labels the selected step's fields and buttons.
fn place_fields(kind: Option<ActionKind>, fields: [HWND; 3], btns: [HWND; 3], btn_w: i32) {
    let specs = kind.map(spec).unwrap_or([None, None, None]);
    let geo = kind.map(field_geometry).unwrap_or_default();
    st(|s| s.geo = geo);
    let fx = PAD + geo.label_w + 12;
    for (i, sp) in specs.iter().enumerate() {
        let Some(sp) = sp else {
            show(fields[i], false);
            show(btns[i], false);
            continue;
        };
        show(fields[i], true);
        set_cue(fields[i], &t(sp.cue));
        let right = if sp.button.is_some() { view_w() - PAD - btn_w - 8 } else { view_w() - PAD };
        place_field(fields[i], fx, geo.rows[i], right - fx);
        match sp.button {
            Some(label) => {
                set_text(btns[i], &t(label));
                place(btns[i], view_w() - PAD - btn_w, geo.rows[i] + 2, btn_w, 32);
                show(btns[i], true);
            }
            None => show(btns[i], false),
        }
    }
}

pub(super) fn layout() {
    let Some((cur, kind, count, fields, btns, ctrls, btn_w, typing)) = st(|s| {
        (
            s.cur,
            s.cur.and_then(|i| s.actions.get(i)).map(|a| a.kind()),
            s.actions.len(),
            s.fields,
            s.field_btns,
            [s.remove_btn, s.up_btn, s.down_btn, s.add_btn],
            s.field_btn_w,
            s.typing,
        )
    }) else {
        return;
    };
    let [remove, up, down, add] = ctrls;
    place_fields(kind, fields, btns, btn_w);
    type_text::place_controls(&typing, kind == Some(ActionKind::TypeText));
    enable(remove, count > 1);
    enable(up, cur.is_some_and(|i| i > 0));
    enable(down, cur.is_some_and(|i| i + 1 < count));
    enable(add, count < MAX_ACTIONS);
}

pub(super) fn paint(hdc: HDC) {
    let Some((cur, action, geo, delay_w, typing)) =
        st(|s| (s.cur, s.cur.and_then(|i| s.actions.get(i).cloned()), s.geo, s.delay_label_w, s.typing))
    else {
        return;
    };
    let p = palette();
    caption(hdc, &t("editor.actions_caption"), PAD, TOP, content_w());
    round_rect(hdc, &r(PAD, LIST_Y, content_w(), LIST_H), p.border, Some(p.surface), 8);
    let delay_rc = r(PAD, DELAY_Y, delay_w, FRAME_H);
    text(hdc, &t("editor.step_delay"), &delay_rc, fonts().small, p.muted, DT_SINGLELINE | DT_VCENTER);
    let (Some(i), Some(a)) = (cur, &action) else { return };
    let title = tf("editor.step_title", &[("n", &(i + 1).to_string()), ("kind", &t(a.kind().key()))]);
    caption(hdc, &title, PAD, STEP_Y, content_w());
    if a.kind() == ActionKind::TypeText {
        type_text::paint(hdc, &typing);
        return;
    }
    let fx = PAD + geo.label_w + 12;
    for (j, sp) in spec(a.kind()).iter().enumerate() {
        if let Some(sp) = sp {
            muted_centered(hdc, &t(sp.label), &r(PAD, geo.rows[j], geo.label_w, FRAME_H));
            muted(hdc, &hint(a, j), &r(fx, geo.rows[j] + FRAME_H + 4, view_w() - PAD - fx, geo.hint_h[j]));
        }
    }
}

pub(in crate::ui) fn draw_row(dis: &DRAWITEMSTRUCT) {
    if dis.itemID == u32::MAX {
        return;
    }
    let Some(action) = st(|s| s.actions.get(dis.itemID as usize).cloned()).flatten() else { return };
    let (p, f) = (palette(), fonts());
    buffered_item(dis.hDC, &dis.rcItem, |hdc, rc| {
        let rc = *rc;
        let selected = dis.itemState & ODS_SELECTED != 0;
        // Item data is the step index, so the effects key on the index.
        let fx = kit::RowFx::read(dis.hwndItem, dis.itemID, dis.itemID);
        fill(hdc, &rc, p.surface);
        if let Some(c) = kit::row_fill(p.surface, selected, &fx) {
            let inner = RECT { left: rc.left + px(2), top: rc.top + px(1), right: rc.right - px(2), bottom: rc.bottom - px(1) };
            round_rect(hdc, &inner, c, Some(c), 6);
        }
        let flags = DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS;
        let (x, right) = (rc.left + px(10), rc.right - px(8));
        text(hdc, &format!("{}.", dis.itemID + 1), &RECT { left: x, right: x + px(20), ..rc }, f.small, p.muted, flags);
        let kind = t(action.kind().key());
        let kx = x + px(22);
        text(hdc, &kind, &RECT { left: kx, right, ..rc }, f.normal, p.text, flags);
        let tx = kx + text_width(hdc, &kind, f.normal) + px(12);
        if tx < right {
            let target = app::action_detail(&action);
            let target = if target.is_empty() { t("editor.not_set") } else { target };
            text(hdc, &target, &RECT { left: tx, right, ..rc }, f.small, p.muted, flags);
        }
    });
}

/// Edit notifications from the step fields. Returns true when handled.
pub(super) fn on_edit_change(id: i32, code: u32) -> bool {
    if !(ID_FIELD..ID_FIELD + 3).contains(&id) && id != type_text::ID_TEXT {
        return false;
    }
    if code == EN_CHANGE
        && id == type_text::ID_TEXT
        && let Some(typing) = st(|s| s.typing)
    {
        type_text::on_text_change(&typing);
    }
    if code == EN_CHANGE && !st(|s| s.loading).unwrap_or(false) {
        commit();
        refresh_row();
    }
    invalidate(super::hwnd());
    true
}

/// Repaints the selected row of the step list (its summary changed).
fn refresh_row() {
    if let Some((list, Some(i))) = st(|s| (s.list, s.cur)) {
        let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        unsafe {
            SendMessageW(list, LB_GETITEMRECT, i, &mut rc as *mut RECT as LPARAM);
            InvalidateRect(list, &rc, 0);
        }
    }
}

fn lookup_started(i: usize, started: Result<(), String>) {
    match started {
        Ok(()) => {
            if let Some(btn) = st(|s| s.field_btns[i]) {
                set_text(btn, &t("field.loading"));
                enable(btn, false);
            }
        }
        Err(err) => alert(super::hwnd(), &err),
    }
}

fn lookup_done(i: usize) -> Option<(HWND, HWND)> {
    let btn = st(|s| s.field_btns[i])?;
    enable(btn, true);
    editor_layout();
    Some((super::hwnd(), btn))
}

/// Applies a looked-up id and name to the selected Discord step.
fn set_discord(index: usize, id: &str, name: &str) {
    let Some(field) = st(|s| s.fields[index]) else { return };
    set_text(field, id);
    commit();
    st(|s| {
        if let Some(Action::DiscordPing { guild_name, channel_name, user_name, .. }) =
            s.cur.and_then(|i| s.actions.get_mut(i))
        {
            let slot = match index {
                0 => guild_name,
                1 => channel_name,
                _ => user_name,
            };
            *slot = name.to_string();
        }
    });
    invalidate(super::hwnd());
}

pub(super) fn on_guilds(result: Result<Vec<Guild>, ApiError>) {
    let Some((hwnd, btn)) = lookup_done(0) else { return };
    match result {
        Ok(list) if list.is_empty() => alert(hwnd, &t("discord.no_servers")),
        Ok(list) => {
            let items: Vec<String> = list.iter().map(|g| g.name.clone()).collect();
            if let Some(i) = popup_menu(hwnd, btn, &items, &[]) {
                set_discord(0, &list[i].id, &list[i].name);
            }
        }
        Err(e) => alert(hwnd, &api_error_text(&e)),
    }
}

pub(super) fn on_channels(result: Result<Vec<Channel>, ApiError>) {
    let Some((hwnd, btn)) = lookup_done(1) else { return };
    match result {
        Ok(list) if list.is_empty() => alert(hwnd, &t("discord.no_channels")),
        Ok(list) => {
            let items: Vec<String> = list.iter().map(|c| format!("#{}", c.name.clone().unwrap_or_default())).collect();
            if let Some(i) = popup_menu(hwnd, btn, &items, &[]) {
                set_discord(1, &list[i].id, &list[i].name.clone().unwrap_or_default());
            }
        }
        Err(e) => alert(hwnd, &api_error_text(&e)),
    }
}

pub(super) fn on_user(result: Result<User, ApiError>) {
    let Some((hwnd, _)) = lookup_done(2) else { return };
    match result {
        Ok(user) => set_discord(2, &user.id, user.display_name()),
        Err(e) => alert(hwnd, &api_error_text(&e)),
    }
}

fn field_button(i: usize) {
    let hwnd = super::hwnd();
    let Some((kind, fields)) = st(|s| (s.cur.and_then(|c| s.actions.get(c)).map(|a| a.kind()), s.fields)) else {
        return;
    };
    let Some(kind) = kind else { return };
    match (kind, i) {
        (ActionKind::DiscordPing, 0) => lookup_started(0, app::discord_lookup(|c| TaskResult::Guilds(c.guilds()))),
        (ActionKind::DiscordPing, 1) => {
            let guild = get_text(fields[0]).trim().to_string();
            if !is_snowflake(&guild) {
                alert(hwnd, &t("err.guild_first"));
                return;
            }
            lookup_started(1, app::discord_lookup(move |c| TaskResult::Channels(c.text_channels(&guild))));
        }
        (ActionKind::DiscordPing, 2) => {
            let user = get_text(fields[2]).trim().to_string();
            if !is_snowflake(&user) {
                alert(hwnd, &t("err.user_id"));
                return;
            }
            lookup_started(2, app::discord_lookup(move |c| TaskResult::UserInfo(c.user(&user))));
        }
        (ActionKind::OpenApp, 0) => {
            if let Some(p) = shell::pick_file(hwnd, &[(&t("field.programs_filter"), "*.exe;*.lnk;*.url")]) {
                set_text(fields[0], &p);
            }
        }
        (ActionKind::OpenFile, 0) => {
            if let Some(p) = shell::pick_file(hwnd, &[(&t("field.all_files_filter"), "*.*")]) {
                set_text(fields[0], &p);
            }
        }
        (ActionKind::OpenFolder, 0) => {
            if let Some(p) = shell::pick_folder(hwnd) {
                set_text(fields[0], &p);
            }
        }
        _ => {}
    }
    commit();
}

pub(super) fn on_command(id: i32, code: u32) {
    let hwnd = super::hwnd();
    match id {
        ID_STEPS if code == LBN_SELCHANGE => {
            let Some(list) = st(|s| s.list) else { return };
            let idx = unsafe { SendMessageW(list, LB_GETCURSEL, 0, 0) };
            if idx >= 0 {
                commit();
                st(|s| s.cur = Some(idx as usize));
                load_fields();
            }
        }
        ID_ADD => {
            let Some(btn) = st(|s| s.add_btn) else { return };
            let items: Vec<String> = ActionKind::ALL.iter().map(|k| t(k.key())).collect();
            if let Some(i) = popup_menu(hwnd, btn, &items, &[]) {
                commit();
                let new_index = st(|s| {
                    s.actions.push(ActionKind::ALL[i].empty());
                    s.actions.len() - 1
                });
                reload_list(new_index);
            }
        }
        ID_REMOVE => {
            let next = st(|s| {
                let i = s.cur?;
                if s.actions.len() <= 1 {
                    return None;
                }
                s.actions.remove(i);
                Some(i.min(s.actions.len() - 1))
            })
            .flatten();
            if next.is_some() {
                reload_list(next);
            }
        }
        ID_UP | ID_DOWN => {
            commit();
            let moved = st(|s| {
                let i = s.cur?;
                let j = if id == ID_UP { i.checked_sub(1)? } else { i + 1 };
                if j >= s.actions.len() {
                    return None;
                }
                s.actions.swap(i, j);
                Some(j)
            })
            .flatten();
            if moved.is_some() {
                reload_list(moved);
            }
        }
        _ if (ID_FIELD_BTN..ID_FIELD_BTN + 3).contains(&id) => field_button((id - ID_FIELD_BTN) as usize),
        ID_STOP => {
            if let Some(btn) = st(|s| s.t_stop) {
                toggle(btn);
            }
        }
        type_text::ID_SPEED if code == kit::SLIDER_CHANGED => {
            commit();
            refresh_row();
            invalidate(hwnd);
        }
        type_text::ID_HUMAN | type_text::ID_ERASE => {
            if let Some(typing) = st(|s| s.typing) {
                toggle(if id == type_text::ID_ERASE { typing.erase } else { typing.human });
                commit();
            }
        }
        _ => {}
    }
}
