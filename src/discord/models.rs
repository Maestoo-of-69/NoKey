//! Discord API objects (only the fields NoKey uses).

use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct User {
    pub id: String,
    pub username: String,
    #[serde(default)]
    pub global_name: Option<String>,
}

impl User {
    pub fn display_name(&self) -> &str {
        self.global_name.as_deref().filter(|n| !n.is_empty()).unwrap_or(&self.username)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Guild {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Channel {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub kind: u8,
    #[serde(default)]
    pub position: Option<i32>,
}

/// Text (0) and announcement (5) channels accept normal messages.
pub fn is_text_channel(kind: u8) -> bool {
    kind == 0 || kind == 5
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ErrorBody {
    #[serde(default)]
    pub code: Option<u32>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub retry_after: Option<f64>,
}
