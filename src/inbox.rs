use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use rand::RngExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;

use crate::config::{App, ConfigStore};
use crate::graph_error::GraphError;
use crate::model::{
    Button, Direction, Media, MediaRef, Message, Rendered, TemplateRef, Webhook, digits, now_ms,
};
use crate::store::{Store, StoreError};
use crate::validate::{DEFAULT_MESSAGE_TYPE, OutboundContext, validate_outbound};
use crate::webhooks::{StatusUpdate, WebhookSender, envelope, inbound_value, status_value};

pub struct FailureReason {
    pub code: i64,
    pub title: &'static str,
    pub details: &'static str,
}

pub const FAILURE_REASONS: &[FailureReason] = &[
    FailureReason {
        code: 131_026,
        title: "Message undeliverable",
        details: "Message could not be delivered: the recipient may not be a WhatsApp user or has not accepted the latest terms.",
    },
    FailureReason {
        code: 131_047,
        title: "Re-engagement message",
        details: "More than 24 hours have passed since the recipient last replied to the sender number.",
    },
    FailureReason {
        code: 131_049,
        title: "This message was not delivered to maintain healthy ecosystem engagement.",
        details: "Meta chose not to deliver this marketing message.",
    },
    FailureReason {
        code: 131_050,
        title: "Unable to deliver the message. This recipient has chosen to stop receiving marketing messages on WhatsApp from your business",
        details: "The user stopped marketing messages.",
    },
    FailureReason {
        code: 131_000,
        title: "Something went wrong",
        details: "Message failed to send due to an unknown error.",
    },
];

const MEDIA_KINDS: &[&str] = &["image", "document", "audio", "video", "sticker"];

#[derive(Debug, thiserror::Error)]
pub enum InboxError {
    #[error(transparent)]
    Graph(#[from] GraphError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("{0}")]
    NotFound(Problem),
    #[error("{0}")]
    Invalid(Problem),
}

/// An inbox failure with a stable `code` the UI translates, the English `message` for logs and API clients,
/// and an optional `detail` such as the field or value that caused it.
#[derive(Debug)]
pub struct Problem {
    pub code: &'static str,
    pub message: String,
    pub detail: Option<String>,
}

impl Problem {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
        }
    }

    #[must_use]
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

impl std::fmt::Display for Problem {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

pub struct InboundMedia {
    pub kind: String,
    pub data: Vec<u8>,
    pub mime_type: String,
    pub filename: Option<String>,
    pub caption: Option<String>,
}

pub use crate::webhooks::Verification;

pub struct Delivered {
    pub message: Message,
    pub webhook: Webhook,
}

pub struct Inbox {
    pub config: ConfigStore,
    pub store: Arc<Store>,
    webhooks: WebhookSender,
    events: broadcast::Sender<Value>,
    stopping: tokio::sync::watch::Sender<bool>,
    auto_status_delay: Duration,
}

pub fn new_message_id() -> String {
    let bytes: [u8; 30] = rand::rng().random();

    format!(
        "wamid.HBgM{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

fn new_media_id() -> String {
    let mut rng = rand::rng();
    let first = rng.random_range(1..10);
    let rest: String = (0..15)
        .map(|_| char::from(b'0' + rng.random_range(0..10u8)))
        .collect();

    format!("{first}{rest}")
}

async fn write_media_file(path: &std::path::Path, data: &[u8]) -> Result<(), StoreError> {
    if let Err(error) = tokio::fs::write(path, data).await {
        let _ = tokio::fs::remove_file(path).await;
        return Err(error.into());
    }

    Ok(())
}

/// The reason a simulated `failed` status carries. Without a code it is 131026, the generic undeliverable error.
fn failure_reason(code: Option<&str>) -> Result<&'static FailureReason, InboxError> {
    let Some(code) = code.map(str::trim).filter(|code| !code.is_empty()) else {
        return Ok(&FAILURE_REASONS[0]);
    };

    code.parse::<i64>()
        .ok()
        .and_then(|code| FAILURE_REASONS.iter().find(|reason| reason.code == code))
        .ok_or_else(|| {
            InboxError::Invalid(
                Problem::new(
                    "unknown_failure_code",
                    format!("wapit cannot simulate a failure with error code {code}"),
                )
                .detail(code),
            )
        })
}

impl Inbox {
    pub fn new(
        config: ConfigStore,
        store: Arc<Store>,
        webhooks: WebhookSender,
        auto_status_delay: Duration,
    ) -> Self {
        let (events, _) = broadcast::channel(256);
        let (stopping, _) = tokio::sync::watch::channel(false);

        Self {
            config,
            store,
            webhooks,
            events,
            stopping,
            auto_status_delay,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.events.subscribe()
    }

    /// Ends the live event streams, so graceful shutdown is not held open by an inbox tab.
    pub fn stop(&self) {
        let _ = self.stopping.send(true);
    }

    pub fn stopped(&self) -> impl std::future::Future<Output = ()> + Send + use<> {
        let mut receiver = self.stopping.subscribe();

        async move {
            let _ = receiver.wait_for(|stopped| *stopped).await;
        }
    }

    pub fn changed(&self, app_id: &str, contact: Option<&str>, extra: Value) {
        let mut event = json!({ "app_id": app_id, "contact": contact });

        if let (Some(event), Value::Object(extra)) = (event.as_object_mut(), extra) {
            event.extend(extra);
        }

        let _ = self.events.send(event);
    }

    fn require_app(&self, app_id: &str) -> Result<Arc<App>, InboxError> {
        self.config.app(app_id).ok_or_else(|| {
            InboxError::NotFound(
                Problem::new(
                    "app_not_found",
                    format!("App {app_id} is not in config/apps.json"),
                )
                .detail(app_id),
            )
        })
    }

    pub fn send_outbound(
        self: &Arc<Self>,
        app: &Arc<App>,
        payload: Value,
    ) -> Result<(Message, String), InboxError> {
        let contact_hint = match payload.get("to").and_then(Value::as_str) {
            Some(to) => digits(to),
            None => payload
                .get("recipient")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_owned(),
        };
        let last_inbound_at = if contact_hint.is_empty() {
            None
        } else {
            self.store.last_inbound_at(&app.id, &contact_hint)?
        };
        let templates = self.config.templates_for(&app.id);
        let context = OutboundContext {
            templates: &templates,
            last_inbound_at,
            now: now_ms(),
        };

        let outbound = match validate_outbound(&payload, &context) {
            Ok(outbound) => outbound,
            Err(error) => {
                if !contact_hint.is_empty() {
                    self.record_rejected(app, &contact_hint, payload, &error)?;
                }

                return Err(error.into());
            }
        };

        let message = Message {
            id: new_message_id(),
            app_id: app.id.clone(),
            contact: outbound.contact,
            direction: Direction::Out,
            kind: outbound.kind,
            status: "accepted".into(),
            payload,
            rendered: outbound.rendered,
            issues: outbound.issues,
            error: None,
            context_id: outbound.context_id,
            created_at: now_ms(),
        };

        self.store.record_message(&message, None, None)?;
        self.changed(&app.id, Some(&message.contact), json!({}));
        self.schedule_auto_statuses(app, &message.id, outbound.outside_window);

        Ok((message, outbound.input))
    }

    fn record_rejected(
        &self,
        app: &App,
        contact: &str,
        payload: Value,
        error: &GraphError,
    ) -> Result<(), InboxError> {
        let kind = payload
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or(DEFAULT_MESSAGE_TYPE)
            .to_owned();
        let template = payload.get("template").map(|template| TemplateRef {
            name: template
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            language: template
                .pointer("/language/code")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            category: None,
        });
        let random: [u8; 12] = rand::rng().random();

        self.store.record_message(
            &Message {
                id: format!("rejected.{}", hex::encode(random)),
                app_id: app.id.clone(),
                contact: contact.to_owned(),
                direction: Direction::Out,
                kind: kind.clone(),
                status: "rejected".into(),
                payload,
                rendered: Rendered {
                    template,
                    ..Rendered::of_kind(kind)
                },
                issues: Vec::new(),
                error: Some(error.record()),
                context_id: None,
                created_at: now_ms(),
            },
            None,
            None,
        )?;
        self.changed(&app.id, Some(contact), json!({}));

        Ok(())
    }

    fn schedule_auto_statuses(self: &Arc<Self>, app: &App, message_id: &str, outside_window: bool) {
        let statuses: Vec<String> = if outside_window {
            vec!["failed".into()]
        } else {
            app.auto_statuses
                .iter()
                .filter(|status| matches!(status.as_str(), "sent" | "delivered" | "read"))
                .cloned()
                .collect()
        };

        if statuses.is_empty() {
            return;
        }

        let inbox = Arc::clone(self);
        let message_id = message_id.to_owned();

        tokio::spawn(async move {
            for status in statuses {
                tokio::time::sleep(inbox.auto_status_delay).await;
                let code = (status == "failed").then_some("131047");

                match inbox.fire_status(&message_id, &status, code).await {
                    Ok(_) => {}
                    Err(InboxError::NotFound(_)) => return,
                    Err(error) => {
                        tracing::error!(
                            "automatic {status} status for {message_id} failed: {error}"
                        );
                        return;
                    }
                }
            }
        });
    }

    pub async fn fire_status(
        &self,
        message_id: &str,
        status: &str,
        failure_code: Option<&str>,
    ) -> Result<Webhook, InboxError> {
        let message = self
            .store
            .message(message_id)?
            .filter(|message| message.direction == Direction::Out && message.status != "rejected")
            .ok_or_else(|| {
                InboxError::NotFound(Problem::new("message_not_found", "Sent message not found"))
            })?;
        let app = self.require_app(&message.app_id)?;
        let reason = if status == "failed" {
            Some(failure_reason(failure_code)?)
        } else {
            None
        };
        let error = reason.map(|reason| {
            json!({
                "code": reason.code,
                "title": reason.title,
                "message": reason.title,
                "error_data": { "details": reason.details },
                "href": "https://developers.facebook.com/docs/whatsapp/cloud-api/support/error-codes/",
            })
        });
        let at = now_ms();

        if !self
            .store
            .record_status(&message.id, status, error.as_ref(), at)?
        {
            return Err(InboxError::NotFound(Problem::new(
                "message_not_found",
                "Sent message not found",
            )));
        }

        self.changed(&app.id, Some(&message.contact), json!({}));

        let category = if message.kind == "template" {
            message
                .rendered
                .template
                .as_ref()
                .and_then(|template| template.category.as_deref())
                .unwrap_or("utility")
                .to_lowercase()
        } else {
            "service".to_owned()
        };
        let value = status_value(&StatusUpdate {
            message_id: &message.id,
            status,
            recipient: &message.contact,
            timestamp_ms: at,
            category: &category,
            error: error.as_ref(),
        });

        let webhook = self
            .webhooks
            .send(
                &app,
                &format!("status:{status}"),
                Some(&message.id),
                &envelope(&app, value),
            )
            .await?;
        self.changed(&app.id, None, json!({ "webhook": webhook.id }));

        Ok(webhook)
    }

    pub fn mark_inbound_read(
        &self,
        app: &App,
        message_id: &str,
        typing: bool,
    ) -> Result<(), InboxError> {
        let message = self
            .store
            .message(message_id)?
            .filter(|message| message.app_id == app.id && message.direction == Direction::In)
            .ok_or_else(|| {
                GraphError::invalid_parameter(format!(
                    "message_id {message_id} is not a valid inbound message for this phone number"
                ))
            })?;

        self.store
            .set_inbound_status(&message.id, if typing { "read+typing" } else { "read" })?;
        self.changed(&app.id, Some(&message.contact), json!({ "typing": typing }));

        Ok(())
    }

    fn base_inbound(contact: &str, kind: &str) -> serde_json::Map<String, Value> {
        let mut message = serde_json::Map::new();
        message.insert("from".into(), json!(contact));
        message.insert("id".into(), json!(new_message_id()));
        message.insert("timestamp".into(), json!((now_ms() / 1000).to_string()));
        message.insert("type".into(), json!(kind));
        message
    }

    #[allow(clippy::too_many_arguments)]
    async fn deliver_inbound(
        &self,
        app: &App,
        contact: &str,
        name: Option<&str>,
        message: serde_json::Map<String, Value>,
        rendered: Rendered,
        context_id: Option<String>,
        media: Option<Media>,
    ) -> Result<Delivered, InboxError> {
        let contact_name = match name {
            Some(name) => Some(name.to_owned()),
            None => match self.store.contact_name(&app.id, contact) {
                Ok(name) => name,
                Err(error) => {
                    if let Some(media) = &media {
                        let _ = std::fs::remove_file(&media.path);
                    }
                    return Err(error.into());
                }
            },
        };
        let message = Value::Object(message);
        let row = Message {
            id: message["id"].as_str().unwrap_or_default().to_owned(),
            app_id: app.id.clone(),
            contact: contact.to_owned(),
            direction: Direction::In,
            kind: message["type"].as_str().unwrap_or_default().to_owned(),
            status: "received".into(),
            payload: message.clone(),
            rendered,
            issues: Vec::new(),
            error: None,
            context_id,
            created_at: now_ms(),
        };

        self.store
            .record_message(&row, contact_name.as_deref(), media.as_ref())?;

        self.changed(&app.id, Some(contact), json!({}));

        let value = inbound_value(contact, contact_name.as_deref(), &message);
        let webhook = self
            .webhooks
            .send(
                app,
                &format!("message:{}", row.kind),
                Some(&row.id),
                &envelope(app, value),
            )
            .await?;
        self.changed(&app.id, None, json!({ "webhook": webhook.id }));

        Ok(Delivered {
            message: row,
            webhook,
        })
    }

    pub async fn inbound_text(
        &self,
        app_id: &str,
        contact: &str,
        name: Option<&str>,
        text: &str,
    ) -> Result<Delivered, InboxError> {
        let app = self.require_app(app_id)?;
        let mut message = Self::base_inbound(contact, "text");
        message.insert("text".into(), json!({ "body": text }));
        let rendered = Rendered {
            body: Some(text.to_owned()),
            ..Rendered::of_kind("text")
        };

        self.deliver_inbound(&app, contact, name, message, rendered, None, None)
            .await
    }

    pub async fn inbound_media(
        &self,
        app_id: &str,
        contact: &str,
        name: Option<&str>,
        media: InboundMedia,
    ) -> Result<Delivered, InboxError> {
        if !MEDIA_KINDS.contains(&media.kind.as_str()) {
            return Err(InboxError::Invalid(
                Problem::new(
                    "unsupported_media_kind",
                    format!("Unsupported media kind {}", media.kind),
                )
                .detail(media.kind.clone()),
            ));
        }

        let app = self.require_app(app_id)?;
        let id = new_media_id();
        let path = self.store.media_dir().join(&id);
        let sha256 = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(&media.data));

        write_media_file(&path, &media.data).await?;

        let row = Media {
            id: id.clone(),
            app_id: app.id.clone(),
            mime_type: media.mime_type.clone(),
            filename: media.filename.clone(),
            sha256: sha256.clone(),
            file_size: i64::try_from(media.data.len()).unwrap_or(i64::MAX),
            path: path.to_string_lossy().into_owned(),
            created_at: now_ms(),
        };

        let mut object = serde_json::Map::new();
        object.insert("mime_type".into(), json!(media.mime_type));
        object.insert("sha256".into(), json!(sha256));
        object.insert("id".into(), json!(id));

        if media.kind == "document"
            && let Some(filename) = &media.filename
        {
            object.insert("filename".into(), json!(filename));
        }

        if let Some(caption) = media.caption.as_ref().filter(|caption| !caption.is_empty())
            && matches!(media.kind.as_str(), "image" | "document" | "video")
        {
            object.insert("caption".into(), json!(caption));
        }

        if media.kind == "audio" {
            object.insert("voice".into(), json!(false));
        }

        let mut message = Self::base_inbound(contact, &media.kind);
        message.insert(media.kind.clone(), Value::Object(object));
        let rendered = Rendered {
            caption: media
                .caption
                .filter(|_| matches!(media.kind.as_str(), "image" | "document" | "video")),
            media: Some(MediaRef {
                id: Some(id),
                mime_type: Some(media.mime_type),
                filename: media.filename,
                link: None,
            }),
            ..Rendered::of_kind(media.kind)
        };

        self.deliver_inbound(&app, contact, name, message, rendered, None, Some(row))
            .await
    }

    pub async fn tap_button(
        &self,
        message_id: &str,
        index: usize,
    ) -> Result<Delivered, InboxError> {
        let original = self
            .store
            .message(message_id)?
            .filter(|message| message.direction == Direction::Out)
            .ok_or_else(|| {
                InboxError::NotFound(Problem::new("message_not_found", "Sent message not found"))
            })?;
        let button = original
            .rendered
            .buttons
            .as_ref()
            .and_then(|buttons| buttons.get(index))
            .filter(|button| button.is_tappable())
            .cloned()
            .ok_or_else(|| {
                InboxError::Invalid(Problem::new(
                    "not_tappable",
                    "This button does not send a reply to the app",
                ))
            })?;
        let app = self.require_app(&original.app_id)?;
        let quick_reply = button.kind == "quick_reply";

        let mut message = serde_json::Map::new();
        message.insert(
            "context".into(),
            json!({ "from": digits(&app.display_phone_number), "id": original.id }),
        );
        message.extend(Self::base_inbound(
            &original.contact,
            if quick_reply { "button" } else { "interactive" },
        ));

        if quick_reply {
            message.insert("button".into(), json!({ "payload": button.payload.as_deref().unwrap_or(&button.text), "text": button.text }));
        } else if button.kind == "reply" {
            message.insert("interactive".into(), json!({ "type": "button_reply", "button_reply": { "id": button.id, "title": button.text } }));
        } else {
            message.insert("interactive".into(), json!({ "type": "list_reply", "list_reply": { "id": button.id, "title": button.text } }));
        }

        let rendered = Rendered {
            body: Some(button.text.clone()),
            buttons: Some(vec![Button {
                kind: button.kind.clone(),
                text: button.text.clone(),
                id: button.id.clone().or_else(|| button.payload.clone()),
                url: None,
                payload: None,
            }]),
            ..Rendered::of_kind("button_reply")
        };

        self.deliver_inbound(
            &app,
            &original.contact,
            None,
            message,
            rendered,
            Some(original.id.clone()),
            None,
        )
        .await
    }

    /// Stores a file the app uploads with `POST /{phone_number_id}/media`, as Meta does before a media message.
    pub async fn upload_media(
        &self,
        app: &App,
        mime_type: &str,
        filename: Option<String>,
        data: &[u8],
    ) -> Result<Media, InboxError> {
        let id = new_media_id();
        let path = self.store.media_dir().join(&id);

        write_media_file(&path, data).await?;

        let media = Media {
            id,
            app_id: app.id.clone(),
            mime_type: mime_type.to_owned(),
            filename,
            sha256: base64::engine::general_purpose::STANDARD.encode(Sha256::digest(data)),
            file_size: i64::try_from(data.len()).unwrap_or(i64::MAX),
            path: path.to_string_lossy().into_owned(),
            created_at: now_ms(),
        };
        self.store.record_upload(&media)?;

        Ok(media)
    }

    pub async fn delete_media(&self, app: &App, media_id: &str) -> Result<(), InboxError> {
        let owned = self
            .store
            .media(media_id)?
            .is_some_and(|media| media.app_id == app.id);
        let path = if owned {
            self.store.delete_media(media_id)?
        } else {
            None
        };

        let Some(path) = path else {
            return Err(GraphError::unsupported("DELETE", media_id).into());
        };

        if let Err(error) = tokio::fs::remove_file(&path).await
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!("cannot remove media file {path}: {error}");
        }

        Ok(())
    }

    /// Sends a recorded webhook again with the same body, like Meta's retries after a failed delivery. The body is
    /// signed again with the app's current secret, so a redelivery after a secret change verifies against the new one.
    pub async fn redeliver(&self, webhook_id: i64) -> Result<Webhook, InboxError> {
        let original = self.store.webhook(webhook_id)?.ok_or_else(|| {
            InboxError::NotFound(Problem::new("webhook_not_found", "Webhook not found"))
        })?;
        let app = self.require_app(&original.app_id)?;
        let webhook = self.webhooks.resend(&app, &original).await?;
        self.changed(&app.id, None, json!({ "webhook": webhook.id }));

        Ok(webhook)
    }

    /// Runs Meta's subscription handshake against the app: a GET with `hub.mode`, `hub.verify_token` and
    /// `hub.challenge`, which passes when the app answers 200 with the challenge as the body.
    pub async fn verify_webhook(&self, app_id: &str) -> Result<Verification, InboxError> {
        let app = self.require_app(app_id)?;
        let token = app.verify_token.clone().ok_or_else(|| {
            InboxError::Invalid(
                Problem::new(
                    "no_verify_token",
                    format!("App {app_id} has no verify_token in apps.json"),
                )
                .detail(app_id),
            )
        })?;
        let challenge: u32 = rand::rng().random_range(100_000_000..1_000_000_000);

        Ok(self
            .webhooks
            .verify(&app, &token, &challenge.to_string())
            .await)
    }
}
