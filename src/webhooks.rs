use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hmac::{KeyInit, Mac};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use serde_json::{Value, json};
use sha2::Sha256;

use crate::clock::Clock;
use crate::config::App;
use crate::model::{MessageStatus, Webhook, digits};
use crate::store::{Store, StoreError};

const TIMEOUT: Duration = Duration::from_secs(20);
const RESPONSE_LIMIT: usize = 4000;

pub fn sign(body: &str, secret: &str) -> String {
    let mut mac = hmac::Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts keys of any length");
    mac.update(body.as_bytes());

    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

pub fn envelope(app: &App, value: Value) -> Value {
    let mut change = json!({
        "messaging_product": "whatsapp",
        "metadata": { "display_phone_number": digits(&app.display_phone_number), "phone_number_id": app.phone_number_id },
    });

    if let (Some(target), Value::Object(extra)) = (change.as_object_mut(), value) {
        target.extend(extra);
    }

    json!({
        "object": "whatsapp_business_account",
        "entry": [{ "id": app.waba_id, "changes": [{ "value": change, "field": "messages" }] }],
    })
}

pub fn inbound_value(contact: &str, name: Option<&str>, message: &Value) -> Value {
    json!({
        "contacts": [{ "profile": { "name": name.unwrap_or(contact) }, "wa_id": contact }],
        "messages": [message],
    })
}

pub struct StatusUpdate<'a> {
    pub message_id: &'a str,
    pub status: &'a MessageStatus,
    pub recipient: &'a str,
    pub timestamp_ms: i64,
    pub category: &'a str,
    pub error: Option<&'a Value>,
}

pub fn status_value(update: &StatusUpdate<'_>) -> Value {
    let mut status = json!({
        "id": update.message_id,
        "status": update.status.as_str(),
        "timestamp": (update.timestamp_ms / 1000).to_string(),
        "recipient_id": update.recipient,
    });

    if *update.status != MessageStatus::Failed {
        let service = update.category == "service";
        status["pricing"] = json!({
            "billable": !service,
            "pricing_model": "PMP",
            "category": update.category,
            "type": if service { "free_customer_service" } else { "regular" },
        });
    }

    if let Some(error) = update.error {
        status["errors"] = json!([error]);
    }

    json!({ "statuses": [status] })
}

fn is_loopback(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_unspecified(),
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
        }
    }
}

fn is_local_name(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host == "localhost"
        || host
            .rsplit('.')
            .next()
            .is_some_and(|label| label == "test" || label == "localhost")
}

/// Inside a container, `*.test` and `localhost` resolve to the container itself. This resolver sends any host
/// that points home to `loopback_host` (usually `host.docker.internal`), where the developer's apps listen.
struct LoopbackAware {
    loopback_host: String,
}

impl Resolve for LoopbackAware {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        let loopback_host = self.loopback_host.clone();

        Box::pin(async move {
            let resolved = tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map(Iterator::collect::<Vec<SocketAddr>>);
            let points_home = match &resolved {
                Ok(addresses) => {
                    !addresses.is_empty()
                        && addresses.iter().all(|address| is_loopback(address.ip()))
                }
                Err(_) => is_local_name(&host),
            };

            let addresses = if points_home && host != loopback_host {
                tokio::net::lookup_host((loopback_host.as_str(), 0))
                    .await?
                    .collect()
            } else {
                resolved?
            };

            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("cannot read the CA bundle {path}: {source}")]
    CaFile {
        path: String,
        source: std::io::Error,
    },
    #[error("invalid CA bundle {path}: {source}")]
    CaPem {
        path: String,
        source: reqwest::Error,
    },
    #[error(transparent)]
    Build(#[from] reqwest::Error),
}

pub struct WebhookSender {
    client: reqwest::Client,
    store: Arc<Store>,
    clock: Arc<dyn Clock>,
}

#[derive(Debug, serde::Serialize)]
pub struct Verification {
    pub ok: bool,
    pub status_code: Option<u16>,
    pub body: Option<String>,
    pub challenge: String,
    pub error: Option<String>,
    pub duration_ms: i64,
}

pub struct Delivery {
    status: Option<i64>,
    response: Option<String>,
    error: Option<String>,
    duration_ms: i64,
}

impl WebhookSender {
    pub fn new(
        store: Arc<Store>,
        clock: Arc<dyn Clock>,
        loopback_host: Option<&str>,
        extra_ca_file: Option<&str>,
    ) -> Result<Self, ClientError> {
        let mut builder = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent("facebookexternalua");

        if let Some(path) = extra_ca_file {
            let pem = std::fs::read(path).map_err(|source| ClientError::CaFile {
                path: path.to_owned(),
                source,
            })?;
            let certs = reqwest::Certificate::from_pem_bundle(&pem).map_err(|source| {
                ClientError::CaPem {
                    path: path.to_owned(),
                    source,
                }
            })?;
            builder = builder.tls_certs_merge(certs);
        }

        if let Some(host) = loopback_host.filter(|host| !host.is_empty()) {
            builder = builder.dns_resolver(Arc::new(LoopbackAware {
                loopback_host: host.to_owned(),
            }));
        }

        Ok(Self {
            client: builder.build()?,
            store,
            clock,
        })
    }

    pub async fn send(
        &self,
        app: &App,
        event: &str,
        message_id: Option<&str>,
        payload: &Value,
    ) -> Result<Webhook, StoreError> {
        self.deliver(app, event, message_id, payload.to_string())
            .await
    }

    pub async fn resend(&self, app: &App, original: &Webhook) -> Result<Webhook, StoreError> {
        self.deliver(
            app,
            &original.event,
            original.message_id.as_deref(),
            original.body.clone(),
        )
        .await
    }

    pub async fn verify(&self, app: &App, token: &str, challenge: &str) -> Verification {
        let started = Instant::now();
        let result = self
            .client
            .get(&app.webhook_url)
            .query(&[
                ("hub.mode", "subscribe"),
                ("hub.verify_token", token),
                ("hub.challenge", challenge),
            ])
            .send()
            .await;
        let duration_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);

        match result {
            Ok(mut response) => {
                let status = response.status().as_u16();
                let body = read_capped(&mut response).await.unwrap_or_default();
                let ok = status == 200 && body.trim() == challenge;

                Verification {
                    ok,
                    status_code: Some(status),
                    body: Some(body),
                    challenge: challenge.to_owned(),
                    error: None,
                    duration_ms,
                }
            }
            Err(error) => Verification {
                ok: false,
                status_code: None,
                body: None,
                challenge: challenge.to_owned(),
                error: Some(describe(&error)),
                duration_ms,
            },
        }
    }

    async fn deliver(
        &self,
        app: &App,
        event: &str,
        message_id: Option<&str>,
        body: String,
    ) -> Result<Webhook, StoreError> {
        let delivery = self
            .post(&app.webhook_url, &body, &sign(&body, &app.app_secret))
            .await;

        let mut webhook = Webhook {
            id: 0,
            app_id: app.id.clone(),
            url: app.webhook_url.clone(),
            event: event.to_owned(),
            message_id: message_id.map(str::to_owned),
            body,
            status_code: delivery.status,
            response: delivery.response,
            error: delivery.error,
            duration_ms: delivery.duration_ms,
            created_at: self.clock.now_ms(),
        };
        webhook.id = self.store.insert_webhook(&webhook)?;

        Ok(webhook)
    }

    async fn post(&self, url: &str, body: &str, signature: &str) -> Delivery {
        let started = Instant::now();
        let elapsed = || i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);

        let result = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .header("X-Hub-Signature-256", signature)
            .body(body.to_owned())
            .send()
            .await;

        let mut response = match result {
            Ok(response) => response,
            Err(error) => {
                return Delivery {
                    status: None,
                    response: None,
                    error: Some(describe(&error)),
                    duration_ms: elapsed(),
                };
            }
        };

        let status = Some(i64::from(response.status().as_u16()));
        let text = match read_capped(&mut response).await {
            Ok(text) => text,
            Err(error) => {
                return Delivery {
                    status,
                    response: None,
                    error: Some(describe(&error)),
                    duration_ms: elapsed(),
                };
            }
        };

        Delivery {
            status,
            response: Some(text),
            error: None,
            duration_ms: elapsed(),
        }
    }
}

/// Reads at most `RESPONSE_LIMIT` bytes of a response, so an app that streams a huge body cannot exhaust memory.
async fn read_capped(response: &mut reqwest::Response) -> Result<String, reqwest::Error> {
    let mut bytes = Vec::new();

    while let Some(chunk) = response.chunk().await? {
        bytes.extend_from_slice(&chunk);

        if bytes.len() >= RESPONSE_LIMIT {
            break;
        }
    }

    Ok(String::from_utf8_lossy(&bytes)
        .chars()
        .take(RESPONSE_LIMIT)
        .collect())
}

fn describe(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        return format!("Timed out after {}s", TIMEOUT.as_secs());
    }

    let mut message = error.to_string();
    let mut source = std::error::Error::source(error);

    while let Some(inner) = source {
        message = format!("{message}: {inner}");
        source = inner.source();
    }

    message
}
