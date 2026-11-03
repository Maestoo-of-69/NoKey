//! Builds the message body for a user mention.

/// Discord IDs ("snowflakes") are 64-bit integers written as 17–20 digits.
pub fn is_snowflake(s: &str) -> bool {
    (17..=20).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit()) && s.parse::<u64>().is_ok()
}

pub fn mention_content(user_id: &str) -> String {
    format!("<@{user_id}>")
}

/// JSON for `POST /channels/{id}/messages`. With an `allowed_mentions`
/// object that lists only `users`, Discord pings exactly those users and
/// never @everyone, @here or roles. `parse` is left out on purpose: the API
/// docs say it is mutually exclusive with `users` (combining them is a
/// validation error). https://docs.discord.com/developers/resources/message
pub fn message_body(user_id: &str) -> String {
    serde_json::json!({
        "content": mention_content(user_id),
        "allowed_mentions": { "users": [user_id] }
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "123456789012345678";

    #[test]
    fn formats_mention() {
        assert_eq!(mention_content(ID), "<@123456789012345678>");
    }

    #[test]
    fn allowed_mentions_only_target_user() {
        let v: serde_json::Value = serde_json::from_str(&message_body(ID)).unwrap();
        assert_eq!(v["content"], "<@123456789012345678>");
        assert_eq!(v["allowed_mentions"], serde_json::json!({ "users": [ID] }));
        assert!(!message_body(ID).contains("everyone"));
    }

    #[test]
    fn snowflake_validation() {
        assert!(is_snowflake(ID));
        assert!(!is_snowflake("12345"));
        assert!(!is_snowflake("12345678901234567a"));
        assert!(!is_snowflake("99999999999999999999")); // > u64::MAX
        assert!(!is_snowflake(""));
    }
}
