//! Action engine: runs a binding's actions in order, on a worker thread.
//! Only the action types in `config::Action` exist — there is no way to
//! run arbitrary commands.

use crate::config::{Action, ActionKind};
use crate::discord::api::{ApiError, Client};
use crate::win::shell::{self, ShellFail};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub enum StepError {
    /// Offline validation failed (localization key).
    Invalid(&'static str),
    NoToken,
    Discord(ApiError),
    Shell(ShellFail),
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
}

impl RunReport {
    pub fn ok(&self) -> bool {
        self.skipped == 0 && self.outcomes.iter().all(|o| o.result.is_ok())
    }

    pub fn first_error(&self) -> Option<&StepOutcome> {
        self.outcomes.iter().find(|o| o.result.is_err())
    }
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
        Action::OpenApp { path, args } => (shell::open_existing_file(path, args).map_err(StepError::Shell), None),
        Action::OpenFile { path } => (shell::open_existing_file(path, "").map_err(StepError::Shell), None),
        Action::OpenFolder { path } => (shell::open_folder(path).map_err(StepError::Shell), None),
        Action::OpenUrl { url } => (shell::open(url.trim(), "").map_err(StepError::Shell), None),
    }
}

/// Runs the actions one after another. Blocking: call from a worker thread.
/// Each Discord step sends exactly one message and is never retried after a
/// failure (except the single short 429 wait inside `send_mention`).
pub fn run(actions: &[Action], delay_ms: u32, stop_on_failure: bool, token: Option<&str>) -> RunReport {
    let client = token.map(Client::new);
    let mut report = RunReport::default();
    for (index, action) in actions.iter().enumerate() {
        if index > 0 && delay_ms > 0 {
            std::thread::sleep(Duration::from_millis(delay_ms as u64));
        }
        let (result, wait) = run_one(action, client.as_ref());
        if wait.is_some() {
            report.discord_wait = wait;
        }
        let failed = result.is_err();
        report.outcomes.push(StepOutcome { index, kind: action.kind(), target: action.target(), result });
        if failed && stop_on_failure {
            report.skipped = actions.len() - index - 1;
            break;
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bad_url() -> Action {
        Action::OpenUrl { url: "ftp://x".into() }
    }

    fn missing_folder() -> Action {
        Action::OpenFolder { path: r"C:\nokey-test\definitely\missing".into() }
    }

    #[test]
    fn stops_on_first_failure() {
        let r = run(&[bad_url(), missing_folder()], 0, true, None);
        assert_eq!(r.outcomes.len(), 1);
        assert_eq!(r.skipped, 1);
        assert_eq!(r.outcomes[0].result, Err(StepError::Invalid("err.url")));
        assert!(!r.ok());
    }

    #[test]
    fn continues_when_asked() {
        let r = run(&[bad_url(), missing_folder()], 0, false, None);
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
        let r = run(&[ping], 0, true, None);
        assert_eq!(r.outcomes[0].result, Err(StepError::NoToken));
    }

    #[test]
    fn waits_between_steps() {
        let start = std::time::Instant::now();
        run(&[bad_url(), bad_url(), bad_url()], 50, false, None);
        assert!(start.elapsed() >= Duration::from_millis(100), "two pauses of 50 ms");
    }
}
