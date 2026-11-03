//! Minimal Discord REST client (API v10). Blocking: call it from a worker
//! thread, never from the UI thread.

use super::mentions::message_body;
use super::models::{is_text_channel, Channel, ErrorBody, Guild, User};
use crate::net::http::{self, Response};
use std::fmt;
use std::time::Duration;

const API: &str = "https://discord.com/api/v10";
const USER_AGENT: &str = concat!("DiscordBot (https://github.com/Maestoo-of-69/NoKey, ", env!("CARGO_PKG_VERSION"), ")");
/// A 429 means the message was NOT created, so one short wait-and-retry
/// still produces exactly one message.
const MAX_AUTO_RETRY_SECS: f64 = 2.0;

#[derive(Clone, Debug, PartialEq)]
pub enum ApiError {
    InvalidToken,
    NoAccess,
    MissingPermissions,
    UnknownChannel,
    UnknownUser,
    UnknownGuild,
    /// Needs the Server Members privileged intent (member list only).
    MembersIntent,
    RateLimited(f64),
    Network(String),
    Http(u16, String),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::InvalidToken => write!(f, "Bot token is invalid or was reset. Connect again in Settings."),
            ApiError::NoAccess => write!(f, "The bot can't see this channel. Check the channel's permissions for the bot."),
            ApiError::MissingPermissions => write!(f, "The bot lacks the Send Messages permission in this channel."),
            ApiError::UnknownChannel => write!(f, "Channel not found. Check the Channel ID."),
            ApiError::UnknownUser => write!(f, "User not found. Check the User ID."),
            ApiError::UnknownGuild => write!(f, "Server not found, or the bot is not a member of it."),
            ApiError::MembersIntent => {
                write!(f, "Listing members needs the Server Members intent. Enter the User ID manually instead.")
            }
            ApiError::RateLimited(s) => write!(f, "Discord rate limit reached. Try again in {s:.1} s."),
            ApiError::Network(e) => write!(f, "{e}"),
            ApiError::Http(code, msg) => write!(f, "Discord error {code}: {msg}"),
        }
    }
}

const MAX_WAIT_SECS: f64 = 3600.0;

/// Converts a wait time from Discord into a Duration. Header and JSON
/// values are untrusted: NaN, infinity or negatives become 0, huge values
/// are capped (Duration::from_secs_f64 would panic on them).
pub fn wait_duration(secs: f64) -> Duration {
    if secs.is_finite() && secs > 0.0 { Duration::from_secs_f64(secs.min(MAX_WAIT_SECS)) } else { Duration::ZERO }
}

/// Maps a non-success response to a readable error.
pub fn map_error(status: u16, body: &[u8]) -> ApiError {
    let err: ErrorBody = serde_json::from_slice(body).unwrap_or_default();
    let msg = err.message.clone().unwrap_or_else(|| "unexpected response".into());
    match (status, err.code) {
        (401, _) => ApiError::InvalidToken,
        (403, Some(50001)) => ApiError::NoAccess,
        (403, Some(50013)) => ApiError::MissingPermissions,
        (403, _) => ApiError::MissingPermissions,
        (404, Some(10003)) => ApiError::UnknownChannel,
        (404, Some(10013)) => ApiError::UnknownUser,
        (404, Some(10004)) => ApiError::UnknownGuild,
        (429, _) => ApiError::RateLimited(wait_duration(err.retry_after.unwrap_or(1.0)).as_secs_f64().max(0.1)),
        (s, _) if s >= 500 => ApiError::Http(s, "Discord is having problems. Try again later.".into()),
        (s, _) => ApiError::Http(s, msg),
    }
}

pub struct Client {
    token: String,
}

/// Rate-limit hint from a successful response: seconds until the bucket
/// refills when no requests are left.
pub fn bucket_exhausted(resp: &Response) -> Option<f64> {
    let remaining: u32 = resp.header("x-ratelimit-remaining")?.parse().ok()?;
    if remaining > 0 {
        return None;
    }
    let secs: f64 = resp.header("x-ratelimit-reset-after")?.parse().ok()?;
    Some(wait_duration(secs).as_secs_f64())
}

impl Client {
    pub fn new(token: &str) -> Self {
        Self { token: token.trim().to_string() }
    }

    fn call(&self, method: &str, path: &str, body: Option<&str>) -> Result<Response, ApiError> {
        let auth = format!("Bot {}", self.token);
        let mut headers = vec![("Authorization", auth.as_str()), ("User-Agent", USER_AGENT)];
        if body.is_some() {
            headers.push(("Content-Type", "application/json"));
        }
        let url = format!("{API}{path}");
        let resp = http::request(method, &url, &headers, body.map(str::as_bytes)).map_err(ApiError::Network)?;
        if (200..300).contains(&resp.status) { Ok(resp) } else { Err(map_error(resp.status, &resp.body)) }
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        let resp = self.call("GET", path, None)?;
        serde_json::from_slice(&resp.body).map_err(|e| ApiError::Http(resp.status, format!("bad JSON: {e}")))
    }

    /// Validates the token and returns the bot user.
    pub fn current_user(&self) -> Result<User, ApiError> {
        self.get_json("/users/@me")
    }

    /// Servers the bot has joined (no privileged intent needed).
    pub fn guilds(&self) -> Result<Vec<Guild>, ApiError> {
        self.get_json("/users/@me/guilds")
    }

    /// Text channels of a server, in display order.
    pub fn text_channels(&self, guild_id: &str) -> Result<Vec<Channel>, ApiError> {
        let mut channels: Vec<Channel> = self.get_json(&format!("/guilds/{guild_id}/channels"))?;
        channels.retain(|c| is_text_channel(c.kind));
        channels.sort_by_key(|c| c.position.unwrap_or(0));
        Ok(channels)
    }

    pub fn user(&self, user_id: &str) -> Result<User, ApiError> {
        self.get_json(&format!("/users/{user_id}"))
    }

    pub fn channel(&self, channel_id: &str) -> Result<Channel, ApiError> {
        self.get_json(&format!("/channels/{channel_id}"))
    }

    /// Sends exactly one mention message. Returns the bucket cooldown, if
    /// Discord says no further requests are allowed for a while.
    pub fn send_mention(&self, channel_id: &str, user_id: &str) -> Result<Option<f64>, ApiError> {
        let body = message_body(user_id);
        let path = format!("/channels/{channel_id}/messages");
        let result = match self.call("POST", &path, Some(&body)) {
            Ok(resp) => Ok(bucket_exhausted(&resp)),
            Err(ApiError::RateLimited(wait)) if wait <= MAX_AUTO_RETRY_SECS => {
                std::thread::sleep(wait_duration(wait));
                self.call("POST", &path, Some(&body)).map(|r| bucket_exhausted(&r))
            }
            Err(e) => Err(e),
        };
        // A timeout while reading the reply doesn't prove the message failed,
        // and NoKey never retries it (that could send it twice).
        result.map_err(|e| match e {
            ApiError::Network(msg) => ApiError::Network(format!("{msg}. The message may or may not have been sent.")),
            other => other,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_common_errors() {
        assert_eq!(map_error(401, br#"{"message":"401: Unauthorized","code":0}"#), ApiError::InvalidToken);
        assert_eq!(map_error(403, br#"{"message":"Missing Access","code":50001}"#), ApiError::NoAccess);
        assert_eq!(map_error(403, br#"{"message":"Missing Permissions","code":50013}"#), ApiError::MissingPermissions);
        assert_eq!(map_error(404, br#"{"message":"Unknown Channel","code":10003}"#), ApiError::UnknownChannel);
        assert_eq!(
            map_error(429, br#"{"message":"You are being rate limited.","retry_after":0.75,"global":false}"#),
            ApiError::RateLimited(0.75)
        );
        assert!(matches!(map_error(502, b"<html>"), ApiError::Http(502, _)));
        assert!(matches!(map_error(400, br#"{"message":"Cannot send an empty message","code":50006}"#), ApiError::Http(400, m) if m.contains("empty")));
    }

    #[test]
    fn untrusted_wait_times_never_panic() {
        assert_eq!(wait_duration(f64::NAN), Duration::ZERO);
        assert_eq!(wait_duration(f64::INFINITY), Duration::ZERO);
        assert_eq!(wait_duration(-5.0), Duration::ZERO);
        assert_eq!(wait_duration(1e300), Duration::from_secs(3600));
        assert_eq!(wait_duration(0.5), Duration::from_millis(500));
        assert_eq!(map_error(429, br#"{"retry_after":1e300}"#), ApiError::RateLimited(3600.0));
    }

    #[test]
    fn error_messages_never_contain_token() {
        let c = Client::new("secret.token.value");
        let shown = format!("{}", ApiError::InvalidToken);
        assert!(!shown.contains(&c.token));
    }
}
