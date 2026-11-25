//! Action engine: runs a binding's actions in order, on a worker thread.
//! Only the action types in `config::Action` exist — there is no way to
//! run arbitrary commands.

pub mod typing;

use crate::config::{internet_shortcut_url, is_shortcut_url_allowed, is_url_shortcut, Action, ActionKind};
use crate::discord::api::{ApiError, Client};
use crate::win::shell::{self, ShellFail};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use typing::{Erase, Stop, TypeFail, TypingJob};

#[derive(Clone, Debug, PartialEq)]
pub enum StepError {
    /// Offline validation failed (localization key).
    Invalid(&'static str),
    NoToken,
    Discord(ApiError),
    Shell(ShellFail),
    Typing(TypeFail),
}

#[derive(Clone, Debug)]
pub struct StepOutcome {
    pub index: usize,
    pub kind: ActionKind,
    pub target: String,
    pub result: Result<(), StepError>,
}

#[derive(Clone, Debug, Default)]
pub struct RunReport {
    pub outcomes: Vec<StepOutcome>,
    /// Steps not run because an earlier one failed.
    pub skipped: usize,
    /// Seconds until Discord accepts requests again, when it said so.
    pub discord_wait: Option<f64>,
    /// The user stopped the run (Esc, the shortcut again, the emergency
    /// switch, exit). Not an error; the remaining steps were skipped.
    pub cancelled: bool,
}

impl RunReport {
    pub fn ok(&self) -> bool {
        !self.cancelled && self.skipped == 0 && self.outcomes.iter().all(|o| o.result.is_ok())
    }

    pub fn first_error(&self) -> Option<&StepOutcome> {
        self.outcomes.iter().find(|o| o.result.is_err())
    }
}

/// Types the text on this (worker) thread. Ok(false) = cancelled.
fn type_text(text: &str, cpm: u32, human: bool, erase: Erase, cancel: &AtomicBool) -> Result<bool, StepError> {
    let _job = TypingJob::try_begin().map_err(StepError::Typing)?;
    let chars = typing::plan::prepare(text);
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
    let delays = typing::plan::plan(&chars, cpm, human, seed);
    let mut keyboard = crate::win::typer::WinKeyboard::new(cancel);
    match typing::type_chars(&mut keyboard, &chars, &delays, erase) {
        Ok(()) => Ok(true),
        Err(Stop::Cancelled) => Ok(false),
        Err(Stop::Failed(f)) => Err(StepError::Typing(f)),
    }
}

/// Sleeps `ms` in short slices; false when the run was cancelled meanwhile.
fn pause(ms: u32, cancel: &AtomicBool) -> bool {
    let end = std::time::Instant::now() + Duration::from_millis(ms as u64);
    while !cancel.load(Ordering::Acquire) {
        let left = end.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return true;
        }
        std::thread::sleep(left.min(Duration::from_millis(typing::SLICE_MS as u64)));
    }
    false
}

/// One step's result, Discord's wait, and whether the user cancelled it.
type StepRun = (Result<(), StepError>, Option<f64>, bool);

/// What step `index` may erase. Only a first step that asks for it: after
/// an earlier step (or its pause) the focus may be somewhere else, and
/// Backspace would delete the user's own text there.
fn erase_for(index: usize, action: &Action, fired: Erase) -> Erase {
    match action {
        Action::TypeText { erase_trigger: true, .. } if index == 0 => fired,
        _ => Erase::default(),
    }
}

fn run_step(action: &Action, erase: Erase, client: Option<&Client>, cancel: &AtomicBool) -> StepRun {
    if let (Action::TypeText { text, cpm, human, .. }, Ok(())) = (action, action.validate()) {
        return match type_text(text, *cpm, *human, erase, cancel) {
            Ok(done) => (Ok(()), None, !done),
            Err(e) => (Err(e), None, false),
        };
    }
    let (result, wait) = run_one(action, client);
    (result, wait, false)
}

fn run_one(action: &Action, client: Option<&Client>) -> (Result<(), StepError>, Option<f64>) {
    if let Err(key) = action.validate() {
        return (Err(StepError::Invalid(key)), None);
    }
    match action {
        Action::DiscordPing { channel_id, user_id, .. } => {
            let Some(client) = client else { return (Err(StepError::NoToken), None) };
            match client.send_mention(channel_id, user_id) {
                Ok(wait) => (Ok(()), wait),
                Err(ApiError::RateLimited(w)) => (Err(StepError::Discord(ApiError::RateLimited(w))), Some(w)),
                Err(e) => (Err(StepError::Discord(e)), None),
            }
        }
        Action::OpenApp { path, .. } | Action::OpenFile { path } if is_url_shortcut(path) => {
            (open_internet_shortcut(path.trim()), None)
        }
        Action::OpenApp { path, args } => (shell::open_existing_file(path, args).map_err(StepError::Shell), None),
        Action::OpenFile { path } => (shell::open_existing_file(path, "").map_err(StepError::Shell), None),
        Action::OpenFolder { path } => (shell::open_folder(path).map_err(StepError::Shell), None),
        Action::OpenUrl { url } => (shell::open(url.trim(), "").map_err(StepError::Shell), None),
        Action::TypeText { .. } => unreachable!("typed by run_step"),
    }
}

/// Opens the link inside a .url file (e.g. `steam://rungameid/570`) rather
/// than the file itself, so exactly the checked URL is opened.
fn open_internet_shortcut(path: &str) -> Result<(), StepError> {
    let bytes = std::fs::read(path).map_err(|_| StepError::Shell(ShellFail::FileNotFound))?;
    match internet_shortcut_url(&String::from_utf8_lossy(&bytes)) {
        Some(url) if is_shortcut_url_allowed(&url) => shell::open(&url, "").map_err(StepError::Shell),
        _ => Err(StepError::Invalid("err.url_shortcut")),
    }
}

/// Runs the actions one after another. Blocking: call from a worker thread.
/// Each Discord step sends exactly one message and is never retried after a
/// failure (except the single short 429 wait inside `send_mention`).
/// `cancel` stops the run between steps and inside a Type text step.
/// `fired` is what the shortcut that started the run typed (default: none).
pub fn run(
    actions: &[Action],
    delay_ms: u32,
    stop_on_failure: bool,
    token: Option<&str>,
    fired: Erase,
    cancel: &AtomicBool,
) -> RunReport {
    let client = token.map(Client::new);
    let mut report = RunReport::default();
    for (index, action) in actions.iter().enumerate() {
        let waited = index == 0 || delay_ms == 0 || pause(delay_ms, cancel);
        if !waited || cancel.load(Ordering::Acquire) {
            report.cancelled = true;
            report.skipped = actions.len() - index;
            break;
        }
        let erase = erase_for(index, action, fired);
        let (result, wait, cancelled) = run_step(action, erase, client.as_ref(), cancel);
        if wait.is_some() {
            report.discord_wait = wait;
        }
        let failed = result.is_err();
        report.outcomes.push(StepOutcome { index, kind: action.kind(), target: action.target(), result });
        if cancelled || (failed && stop_on_failure) {
            report.cancelled = cancelled;
            report.skipped = actions.len() - index - 1;
            break;
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run nobody cancels.
    static GO: AtomicBool = AtomicBool::new(false);

    /// `super::run` for a run with nothing to erase.
    fn run(actions: &[Action], delay_ms: u32, stop: bool, token: Option<&str>, cancel: &AtomicBool) -> RunReport {
        super::run(actions, delay_ms, stop, token, Erase::default(), cancel)
    }

    #[test]
    fn only_a_first_typing_step_that_asks_for_it_erases() {
        let typing = |erase_trigger| Action::TypeText { text: "x".into(), cpm: 300, human: false, erase_trigger };
        let fired = Erase { count: 3, window: 9 };
        assert_eq!(erase_for(0, &typing(true), fired), fired);
        assert_eq!(erase_for(0, &typing(false), fired), Erase::default());
        assert_eq!(erase_for(1, &typing(true), fired), Erase::default(), "focus may have moved");
        assert_eq!(erase_for(0, &bad_url(), fired), Erase::default());
    }

    fn bad_url() -> Action {
        Action::OpenUrl { url: "ftp://x".into() }
    }

    fn missing_folder() -> Action {
        Action::OpenFolder { path: r"C:\nokey-test\definitely\missing".into() }
    }

    #[test]
    fn stops_on_first_failure() {
        let r = run(&[bad_url(), missing_folder()], 0, true, None, &GO);
        assert_eq!(r.outcomes.len(), 1);
        assert_eq!(r.skipped, 1);
        assert_eq!(r.outcomes[0].result, Err(StepError::Invalid("err.url")));
        assert!(!r.ok());
    }

    #[test]
    fn continues_when_asked() {
        let r = run(&[bad_url(), missing_folder()], 0, false, None, &GO);
        assert_eq!(r.outcomes.len(), 2);
        assert_eq!(r.skipped, 0);
        assert_eq!(r.outcomes[1].result, Err(StepError::Shell(ShellFail::FolderNotFound)));
    }

    #[test]
    fn discord_without_token_fails_cleanly() {
        let ping = Action::DiscordPing {
            guild_id: String::new(),
            guild_name: String::new(),
            channel_id: "222222222222222222".into(),
            channel_name: String::new(),
            user_id: "333333333333333333".into(),
            user_name: String::new(),
        };
        let r = run(&[ping], 0, true, None, &GO);
        assert_eq!(r.outcomes[0].result, Err(StepError::NoToken));
    }

    #[test]
    fn url_shortcut_to_a_local_file_is_refused() {
        let dir = std::env::temp_dir().join(format!("nokey-url-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("evil.url");
        std::fs::write(&path, "[InternetShortcut]\r\nURL=file:///C:/Windows/System32/cmd.exe\r\n").unwrap();
        let app = Action::OpenApp { path: path.to_string_lossy().into_owned(), args: String::new() };
        let r = run(&[app], 0, true, None, &GO);
        assert_eq!(r.outcomes[0].result, Err(StepError::Invalid("err.url_shortcut")));
        let file = Action::OpenFile { path: path.to_string_lossy().into_owned() };
        let r = run(&[file], 0, true, None, &GO);
        assert_eq!(r.outcomes[0].result, Err(StepError::Invalid("err.url_shortcut")), "Open File checks the link too");
        let missing = Action::OpenApp { path: dir.join("missing.url").to_string_lossy().into_owned(), args: String::new() };
        let r = run(&[missing], 0, true, None, &GO);
        assert_eq!(r.outcomes[0].result, Err(StepError::Shell(ShellFail::FileNotFound)));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn waits_between_steps() {
        let start = std::time::Instant::now();
        run(&[bad_url(), bad_url(), bad_url()], 50, false, None, &GO);
        assert!(start.elapsed() >= Duration::from_millis(100), "two pauses of 50 ms");
    }

    #[test]
    fn a_cancelled_run_skips_the_rest_quietly() {
        let cancel = AtomicBool::new(true);
        let r = run(&[bad_url(), bad_url()], 0, false, None, &cancel);
        assert!(r.cancelled);
        assert_eq!((r.outcomes.len(), r.skipped), (0, 2));
        assert!(!r.ok());
        assert!(r.first_error().is_none(), "a cancel is not an error");
    }

    #[test]
    fn cancel_cuts_the_pause_between_steps() {
        let cancel = AtomicBool::new(false);
        let start = std::time::Instant::now();
        std::thread::scope(|s| {
            s.spawn(|| {
                std::thread::sleep(Duration::from_millis(60));
                cancel.store(true, Ordering::Release);
            });
            let r = run(&[bad_url(), bad_url()], 10_000, false, None, &cancel);
            assert!(r.cancelled);
            assert_eq!((r.outcomes.len(), r.skipped), (1, 1));
        });
        assert!(start.elapsed() < Duration::from_secs(2), "the 10 s pause was cut short");
    }

    #[test]
    fn invalid_type_text_fails_before_touching_the_keyboard() {
        let empty = Action::TypeText { text: "   ".into(), cpm: 300, human: true, erase_trigger: true };
        let r = run(&[empty], 0, true, None, &GO);
        assert_eq!(r.outcomes[0].result, Err(StepError::Invalid("err.type_text_empty")));
        let slow = Action::TypeText { text: "x".into(), cpm: 5, human: false, erase_trigger: true };
        let r = run(&[slow], 0, true, None, &GO);
        assert_eq!(r.outcomes[0].result, Err(StepError::Invalid("err.typing_speed")));
    }
}
