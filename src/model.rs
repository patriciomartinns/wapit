use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Out,
    In,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Out => "out",
            Self::In => "in",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rendered {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_parts: Option<Vec<BodyPart>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<TemplateRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buttons: Option<Vec<Button>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<MediaRef>,
}

impl Rendered {
    pub fn of_kind(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BodyPart {
    pub text: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub variable: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemplateRef {
    pub name: String,
    pub language: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Button {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<String>,
}

impl Button {
    pub fn is_tappable(&self) -> bool {
        matches!(self.kind.as_str(), "reply" | "list_reply" | "quick_reply")
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MediaRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
}

/// A problem the inbox shows next to an accepted message. The UI owns the wording, keyed by `code`.
/// `Legacy` carries the sentence the first, Node-based release stored, which has no code.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "code")]
pub enum Issue {
    NoLocalTemplates,
    OutsideServiceWindow,
    Legacy { text: String },
}

impl Issue {
    /// Reads an `issues` column written by any release: objects with a `code`, or the first release's sentences.
    pub fn parse_all(raw: &str) -> Vec<Self> {
        let values: Vec<serde_json::Value> = serde_json::from_str(raw).unwrap_or_default();

        values
            .into_iter()
            .filter_map(|value| match value {
                serde_json::Value::String(text) if text.contains("janela de 24h") => {
                    Some(Self::OutsideServiceWindow)
                }
                serde_json::Value::String(text) if text.starts_with("Nenhum template local") => {
                    Some(Self::NoLocalTemplates)
                }
                serde_json::Value::String(text) => Some(Self::Legacy { text }),
                other => serde_json::from_value(other).ok(),
            })
            .collect()
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Message {
    pub id: String,
    pub app_id: String,
    pub contact: String,
    pub direction: Direction,
    #[serde(rename = "type")]
    pub kind: String,
    pub status: String,
    pub payload: Value,
    pub rendered: Rendered,
    pub issues: Vec<Issue>,
    pub error: Option<Value>,
    pub context_id: Option<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct StatusEvent {
    pub id: i64,
    pub message_id: String,
    pub status: String,
    pub error: Option<Value>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Media {
    pub id: String,
    pub app_id: String,
    pub mime_type: String,
    pub filename: Option<String>,
    pub sha256: String,
    pub file_size: i64,
    pub path: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Webhook {
    pub id: i64,
    pub app_id: String,
    pub url: String,
    pub event: String,
    pub message_id: Option<String>,
    pub body: String,
    pub status_code: Option<i64>,
    pub response: Option<String>,
    pub error: Option<String>,
    pub duration_ms: i64,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Conversation {
    pub app_id: String,
    pub contact: String,
    pub name: Option<String>,
    pub last_at: i64,
    pub total: i64,
    pub flagged: i64,
    pub last_id: String,
}

pub fn now_ms() -> i64 {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();

    i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
}

pub fn digits(value: &str) -> String {
    value.chars().filter(char::is_ascii_digit).collect()
}
