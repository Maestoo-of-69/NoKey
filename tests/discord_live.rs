//! Tests against the real Discord API. Ignored by default (they need the
//! network, and the second one needs your own bot).
//!
//! Run the safe one (an invalid token must be rejected with 401):
//!     cargo test --test discord_live -- --ignored invalid_token
//!
//! Send one real mention with your own bot (PowerShell):
//!     $env:NOKEY_TEST_TOKEN = "<bot token>"        # never commit this
//!     $env:NOKEY_TEST_CHANNEL = "<channel id>"
//!     $env:NOKEY_TEST_USER = "<user id>"
//!     cargo test --test discord_live -- --ignored real_mention
//!     Remove-Item Env:NOKEY_TEST_TOKEN

use nokey::discord::api::{ApiError, Client};

#[test]
#[ignore = "needs network"]
fn invalid_token_is_rejected_by_discord() {
    let client = Client::new("NOKEY.invalid.token");
    match client.current_user() {
        Err(ApiError::InvalidToken) => {}
        other => panic!("expected InvalidToken, got {:?}", other.map(|u| u.username)),
    }
}

#[test]
#[ignore = "needs your own Discord bot"]
fn real_mention_is_delivered() {
    let token = std::env::var("NOKEY_TEST_TOKEN").expect("set NOKEY_TEST_TOKEN");
    let channel = std::env::var("NOKEY_TEST_CHANNEL").expect("set NOKEY_TEST_CHANNEL");
    let user = std::env::var("NOKEY_TEST_USER").expect("set NOKEY_TEST_USER");
    let client = Client::new(&token);
    let bot = client.current_user().unwrap_or_else(|e| panic!("token check failed: {e}"));
    println!("bot: {}", bot.display_name());
    client.send_mention(&channel, &user).unwrap_or_else(|e| panic!("send failed: {e}"));
}
