use serde::Serialize;

use crate::model::{Conversation, Direction, Message, StatusEvent, Webhook};

/// A change the inbox page reacts to. `app_id` is `*` when everything may have changed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Change {
    pub app_id: String,
    pub contact: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub typing: Option<bool>,
}

impl Change {
    pub fn of(app_id: &str, contact: Option<&str>) -> Self {
        Self {
            app_id: app_id.to_owned(),
            contact: contact.map(str::to_owned),
            webhook: None,
            typing: None,
        }
    }

    pub fn everything() -> Self {
        Self::of("*", None)
    }
}

#[derive(Debug, Serialize)]
pub struct StateSummary {
    pub apps: Vec<AppSummary>,
    pub conversations: Vec<ConversationSummary>,
    pub failure_reasons: Vec<FailureReasonSummary>,
    pub config_error: Option<String>,
    pub using_default_app: bool,
}

#[derive(Debug, Serialize)]
pub struct AppSummary {
    pub id: String,
    pub name: String,
    pub phone_number_id: String,
    pub display_phone_number: String,
    pub waba_id: String,
    pub webhook_url: String,
    pub templates: usize,
    pub can_verify: bool,
}

#[derive(Debug, Serialize)]
pub struct ConversationSummary {
    #[serde(flatten)]
    pub conversation: Conversation,
    pub last: Option<LastMessage>,
}

#[derive(Debug, Serialize)]
pub struct LastMessage {
    pub direction: Direction,
    pub kind: String,
    pub body: String,
    pub status: String,
}

impl LastMessage {
    pub fn of(message: &Message) -> Self {
        let rendered = &message.rendered;
        let body = rendered
            .body
            .clone()
            .or_else(|| rendered.caption.clone())
            .or_else(|| {
                rendered
                    .template
                    .as_ref()
                    .map(|template| template.name.clone())
            })
            .unwrap_or_default();

        Self {
            direction: message.direction,
            kind: rendered.kind.clone(),
            body,
            status: message.status.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct FailureReasonSummary {
    pub code: i64,
    pub title: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ConversationView {
    pub app_id: String,
    pub contact: String,
    pub name: Option<String>,
    pub window_open_until: Option<i64>,
    pub messages: Vec<MessageView>,
}

#[derive(Debug, Serialize)]
pub struct MessageView {
    #[serde(flatten)]
    pub message: Message,
    pub statuses: Vec<StatusEvent>,
    pub webhooks: Vec<WebhookSummary>,
}

/// A webhook delivery without its body, which the inbox loads on demand.
#[derive(Debug, Serialize)]
pub struct WebhookSummary {
    pub id: i64,
    pub app_id: String,
    pub url: String,
    pub event: String,
    pub message_id: Option<String>,
    pub status_code: Option<i64>,
    pub response: Option<String>,
    pub error: Option<String>,
    pub duration_ms: i64,
    pub created_at: i64,
}

impl From<Webhook> for WebhookSummary {
    fn from(webhook: Webhook) -> Self {
        Self {
            id: webhook.id,
            app_id: webhook.app_id,
            url: webhook.url,
            event: webhook.event,
            message_id: webhook.message_id,
            status_code: webhook.status_code,
            response: webhook.response,
            error: webhook.error,
            duration_ms: webhook.duration_ms,
            created_at: webhook.created_at,
        }
    }
}
