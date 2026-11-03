//! Actions tab of the binding editor: an ordered list of steps, the fields
//! of the selected step, the delay between steps and "stop on failure".

use super::super::theme::palette;
use super::super::*;
use super::{ed, layout as editor_layout, W};
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
const FIELD_Y: [i32; 3] = [276, 350, 424];

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
    delay: HWND,
    t_stop: HWND,
}

impl Steps {
    pub fn edits(&self) -> Vec<HWND> {
        let mut v = self.fields.to_vec();
        v.push(self.delay);
        v
    }
}

fn st<R>(f: impl FnOnce(&mut Steps) -> R) -> Option<R> {
    ed(|e| f(&mut e.steps))
}

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

pub(super) fn create(hwnd: HWND) -> (Steps, Vec<HWND>) {
    let b = |id, text: &str, kind, x, y, w, h| {
        let btn = button(hwnd, id, text, kind);
        place(btn, x, y, w, h);
        btn
    };
    let list = listbox(hwnd, ID_STEPS, 0);
    place(list, 24, 82, W - 48, 120);
    let add_btn = b(ID_ADD, &t("editor.add_action"), BTN_DROPDOWN, 24, 210, 150, 30);
    let remove_btn = b(ID_REMOVE, &t("editor.remove"), BTN_SECONDARY, 182, 210, 110, 30);
    let up_btn = b(ID_UP, &t("editor.up"), BTN_SECONDARY, 300, 210, 90, 30);
    let down_btn = b(ID_DOWN, &t("editor.down"), BTN_SECONDARY, 398, 210, 118, 30);
    let fields: [HWND; 3] = std::array::from_fn(|i| edit(hwnd, ID_FIELD + i as i32, 0));
    let field_btns: [HWND; 3] =
        std::array::from_fn(|i| b(ID_FIELD_BTN + i as i32, "", BTN_SECONDARY, W - 24 - 110, FIELD_Y[i] + 20, 110, 36));
    let delay = edit(hwnd, ID_DELAY, 0);
    place(delay, 34, 524, 90, 20);
    let t_stop = b(ID_STOP, &t("editor.stop_on_failure"), BTN_TOGGLE, 150, 520, W - 174, 26);
    add_tooltip(hwnd, delay, &t("tip.step_delay"));
    add_tooltip(hwnd, t_stop, &t("tip.stop_on_failure"));
    add_tooltip(hwnd, add_btn, &t("tip.add_action"));
    let mut group = vec![list, add_btn, remove_btn, up_btn, down_btn, delay, t_stop];
    group.extend(fields);
    group.extend(field_btns);
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
        delay,
        t_stop,
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
    let Some((action, fields)) = st(|s| (s.cur.and_then(|i| s.actions.get(i).cloned()), s.fields)) else { return };
    let vals = action.as_ref().map(values).unwrap_or_default();
    st(|s| s.loading = true);
    for (f, v) in fields.iter().zip(vals) {
        set_text(*f, &v);
    }
    st(|s| s.loading = false);
    editor_layout();
}

/// Saves the field texts into the selected step.
pub(super) fn commit() {
    let Some((cur, fields)) = st(|s| (s.cur, s.fields)) else { return };
    let Some(i) = cur else { return };
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

pub(super) fn layout() {
    let Some((cur, action, count, fields, btns, ctrls)) = st(|s| {
        (
            s.cur,
            s.cur.and_then(|i| s.actions.get(i).cloned()),
            s.actions.len(),
            s.fields,
            s.field_btns,
            [s.remove_btn, s.up_btn, s.down_btn, s.add_btn],
        )
    }) else {
        return;
    };
    let [remove, up, down, add] = ctrls;
    let specs = action.as_ref().map(|a| spec(a.kind())).unwrap_or([None, None, None]);
    for i in 0..3 {
        match &specs[i] {
            Some(sp) => {
                show(fields[i], true);
                set_cue(fields[i], &t(sp.cue));
                let w = if sp.button.is_some() { W - 68 - 120 } else { W - 68 };
                place(fields[i], 34, FIELD_Y[i] + 28, w, 20);
                match sp.button {
                    Some(label) => {
                        set_text(btns[i], &t(label));
                        show(btns[i], true);
                    }
                    None => show(btns[i], false),
                }
            }
            None => {
                show(fields[i], false);
                show(btns[i], false);
            }
        }
    }
    enable(remove, count > 1);
    enable(up, cur.is_some_and(|i| i > 0));
    enable(down, cur.is_some_and(|i| i + 1 < count));
    enable(add, count < MAX_ACTIONS);
}

pub(super) fn paint(hdc: HDC) {
    let Some((cur, action)) = st(|s| (s.cur, s.cur.and_then(|i| s.actions.get(i).cloned()))) else { return };
    let p = palette();
    let f = fonts();
    caption(hdc, &t("editor.actions_caption"), 24, 60, W - 48);
    if let (Some(i), Some(a)) = (cur, &action) {
        let title = tf("editor.step_title", &[("n", &(i + 1).to_string()), ("kind", &t(a.kind().key()))]);
        caption(hdc, &title, 24, 252, W - 48);
        for (j, sp) in spec(a.kind()).iter().enumerate() {
            if let Some(sp) = sp {
                caption(hdc, &t(sp.label), 24, FIELD_Y[j], W - 48);
                let h = hint(a, j);
                text(hdc, &h, &r(24, FIELD_Y[j] + 60, W - 48, 14), f.small, p.muted, DT_SINGLELINE | DT_END_ELLIPSIS);
            }
        }
    }
    text(hdc, &t("editor.step_delay"), &r(24, 498, 400, 16), f.small, p.muted, DT_SINGLELINE | DT_END_ELLIPSIS);
}

pub(in crate::ui) fn draw_row(dis: &DRAWITEMSTRUCT) {
    if dis.itemID == u32::MAX {
        return;
    }
    let Some(action) = st(|s| s.actions.get(dis.itemID as usize).cloned()).flatten() else { return };
    let p = palette();
    let rc = dis.rcItem;
    let selected = dis.itemState & ODS_SELECTED != 0;
    fill(dis.hDC, &rc, if selected { p.selected } else { p.surface });
    if selected {
        fill(dis.hDC, &RECT { right: rc.left + px(3), ..rc }, p.accent);
    }
    let target = action.target();
    let target = if target.is_empty() { t("editor.not_set") } else { target };
    let label = format!("{}.  {}  ·  {target}", dis.itemID + 1, t(action.kind().key()));
    let tr = RECT { left: rc.left + px(12), right: rc.right - px(8), ..rc };
    text(dis.hDC, &label, &tr, fonts().normal, p.text, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
}

/// Edit notifications from the step fields. Returns true when handled.
pub(super) fn on_edit_change(id: i32, code: u32) -> bool {
    if !(ID_FIELD..ID_FIELD + 3).contains(&id) {
        return false;
    }
    if code == EN_CHANGE && !st(|s| s.loading).unwrap_or(false) {
        commit();
        if let Some((list, Some(i))) = st(|s| (s.list, s.cur)) {
            let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            unsafe {
                SendMessageW(list, LB_GETITEMRECT, i, &mut rc as *mut RECT as LPARAM);
                InvalidateRect(list, &rc, 0);
            }
        }
    }
    invalidate(super::hwnd());
    true
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
            if let Some(p) = shell::pick_file(hwnd, &[(&t("field.programs_filter"), "*.exe;*.lnk")]) {
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
        _ => {}
    }
}
