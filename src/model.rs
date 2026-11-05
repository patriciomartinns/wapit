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

/// Where a message stands. Outbound messages move from `accepted` through Meta's statuses, or are `rejected`
/// at send time; inbound ones are `received` and then read by the app, with or without a typing indicator.
/// `Other` keeps a status written by an older release readable instead of failing the row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MessageStatus {
    Accepted,
    Sent,
    Delivered,
    Read,
    Failed,
    Rejected,
    Received,
    ReadTyping,
    Other(String),
}

impl MessageStatus {
    pub const FLAGGED: &[Self] = &[Self::Rejected, Self::Failed];

    pub fn parse(raw: &str) -> Self {
        match raw {
            "accepted" => Self::Accepted,
            "sent" => Self::Sent,
            "delivered" => Self::Delivered,
            "read" => Self::Read,
            "failed" => Self::Failed,
            "rejected" => Self::Rejected,
            "received" => Self::Received,
            "read+typing" => Self::ReadTyping,
            other => Self::Other(other.to_owned()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Accepted => "accepted",
            Self::Sent => "sent",
            Self::Delivered => "delivered",
            Self::Read => "read",
            Self::Failed => "failed",
            Self::Rejected => "rejected",
            Self::Received => "received",
            Self::ReadTyping => "read+typing",
            Self::Other(other) => other,
        }
    }

    /// Delivery order of outbound statuses. A status only replaces the current one when it is further along,
    /// so a late automatic `delivered` never overwrites a `read` fired by hand.
    pub fn rank(&self) -> u8 {
        match self {
            Self::Sent => 1,
            Self::Delivered => 2,
            Self::Read => 3,
            Self::Failed => 4,
            _ => 0,
        }
    }

    /// Whether the inbox counts the message as a problem in its conversation.
    pub fn flagged(&self) -> bool {
        Self::FLAGGED.contains(self)
    }

    /// Statuses the inbox can fire for an outbound message.
    pub fn simulatable(&self) -> bool {
        matches!(
            self,
            Self::Sent | Self::Delivered | Self::Read | Self::Failed
        )
    }

    /// Statuses an app may list in `auto_statuses`, fired in order after each accepted send.
    pub fn automatic(&self) -> bool {
        self.simulatable() && *self != Self::Failed
    }
}

impl std::fmt::Display for MessageStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for MessageStatus {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for MessageStatus {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::parse(&String::deserialize(deserializer)?))
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
    pub status: MessageStatus,
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
    pub status: MessageStatus,
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

#[cfg(test)]
mod tests {
    use super::MessageStatus;

    #[test]
    fn statuses_round_trip_and_unknown_ones_survive() {
        for raw in [
            "accepted",
            "sent",
            "delivered",
            "read",
            "failed",
            "rejected",
            "received",
            "read+typing",
            "queued",
        ] {
            assert_eq!(MessageStatus::parse(raw).as_str(), raw);
        }

        assert_eq!(
            MessageStatus::parse("queued"),
            MessageStatus::Other("queued".into())
        );
    }

    #[test]
    fn only_later_statuses_rank_higher() {
        let order = ["sent", "delivered", "read", "failed"].map(MessageStatus::parse);

        assert!(order.windows(2).all(|pair| pair[0].rank() < pair[1].rank()));
        assert_eq!(MessageStatus::Accepted.rank(), 0);
        assert!(MessageStatus::Failed.flagged() && MessageStatus::Rejected.flagged());
        assert!(!MessageStatus::Read.flagged());
        assert!(!MessageStatus::Failed.automatic() && MessageStatus::Failed.simulatable());
    }
}
