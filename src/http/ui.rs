use std::convert::Infallible;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine;
use futures_util::{Stream, StreamExt, stream};
use regex::Regex;
use rust_embed::RustEmbed;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_stream::wrappers::BroadcastStream;

use super::content_disposition;
use crate::inbox::{Delivered, FAILURE_REASONS, InboundMedia, Inbox, InboxError, Problem};
use crate::model::digits;
use crate::validate::SERVICE_WINDOW_MS;

#[derive(RustEmbed)]
#[folder = "public/"]
struct Assets;

static PHONE_LIKE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[\d+\s()-]+$").expect("valid regex"));
static BSUID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Z]{2}\.[A-Za-z0-9]+$").expect("valid regex"));

const BODY_LIMIT: usize = 32 * 1024 * 1024;
const PAGE_POLICY: &str = "default-src 'self'; img-src 'self' data: blob: http: https:; style-src 'self' 'unsafe-inline'; \
    font-src 'self'; script-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'";

#[derive(Clone)]
pub struct UiState {
    pub inbox: Arc<Inbox>,
    pub public_api_url: String,
}

/// An inbox API error: `{ "error": message, "code": code, "detail": detail }`. The UI shows its own translation
/// of `code` and falls back to `error` for codes it does not know.
pub struct ApiError {
    status: StatusCode,
    problem: Problem,
}

impl ApiError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            problem: Problem::new(code, message),
        }
    }

    #[must_use]
    fn detail(mut self, detail: impl Into<String>) -> Self {
        self.problem.detail = Some(detail.into());
        self
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = json!({ "error": self.problem.message, "code": self.problem.code });

        if let Some(detail) = self.problem.detail {
            body["detail"] = json!(detail);
        }

        (self.status, axum::Json(body)).into_response()
    }
}

/// Parses a JSON body into `T`, answering with an `invalid_body` `ApiError` instead of axum's plain-text rejection.
fn parse_body<T: serde::de::DeserializeOwned>(bytes: &Bytes) -> Result<T, ApiError> {
    serde_json::from_slice(bytes).map_err(|error| {
        ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_body",
            format!("The request body is not valid: {error}"),
        )
        .detail(error.to_string())
    })
}

impl From<InboxError> for ApiError {
    fn from(error: InboxError) -> Self {
        match error {
            InboxError::NotFound(problem) => Self {
                status: StatusCode::NOT_FOUND,
                problem,
            },
            InboxError::Invalid(problem) => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                problem,
            },
            InboxError::Graph(error) => Self::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "graph_error",
                error.message.clone(),
            ),
            InboxError::Store(error) => {
                tracing::error!("inbox storage failed: {error}");
                Self::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_failed",
                    "wapit could not read or write its data directory",
                )
            }
        }
    }
}

impl From<crate::store::StoreError> for ApiError {
    fn from(error: crate::store::StoreError) -> Self {
        InboxError::from(error).into()
    }
}

type ApiResult = Result<Response, ApiError>;

pub fn router(state: UiState) -> Router {
    let api = Router::new()
        .route("/events", get(events))
        .route("/state", get(state_summary))
        .route(
            "/conversations/{app}/{contact}",
            get(conversation).delete(delete_conversation),
        )
        .route(
            "/conversations/{app}/{contact}/messages",
            post(send_as_user),
        )
        .route("/messages", axum::routing::delete(delete_all))
        .route("/messages/{id}/tap", post(tap))
        .route("/messages/{id}/status", post(fire_status))
        .route("/webhooks/{id}", get(webhook))
        .route("/webhooks/{id}/redeliver", post(redeliver))
        .route("/apps/{id}/verify-webhook", post(verify_webhook))
        .fallback(|| async {
            ApiError::new(StatusCode::NOT_FOUND, "route_not_found", "Route not found")
        })
        .layer(middleware::from_fn(same_origin_writes));

    Router::new()
        .nest("/api", api)
        .route("/media/{id}", get(media))
        .fallback(static_asset)
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .with_state(state)
}

/// Writes must come from the inbox page itself: a JSON body (which a cross-site form cannot send)
/// and, when the browser reports it, a same-origin fetch.
async fn same_origin_writes(request: Request, next: Next) -> Response {
    if request.method() != Method::GET {
        let headers = request.headers();
        let fetch_site = headers
            .get("sec-fetch-site")
            .and_then(|value| value.to_str().ok());
        let content_type = headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        let cross_site = fetch_site.is_some_and(|site| site != "same-origin" && site != "none");
        let not_json =
            request.method() != Method::DELETE && !content_type.starts_with("application/json");

        if cross_site || not_json {
            return ApiError::new(
                StatusCode::FORBIDDEN,
                "not_same_origin",
                "The wapit inbox only accepts JSON requests from its own page",
            )
            .into_response();
        }
    }

    next.run(request).await
}

#[allow(clippy::unnecessary_wraps)]
fn ok(value: impl serde::Serialize) -> ApiResult {
    Ok(axum::Json(value).into_response())
}

fn normalize_contact(contact: &str) -> String {
    if PHONE_LIKE.is_match(contact) {
        digits(contact)
    } else {
        contact.to_owned()
    }
}

fn require_app(inbox: &Inbox, app: &str) -> Result<(), ApiError> {
    inbox.config.app(app).map(|_| ()).ok_or_else(|| {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "app_not_found",
            format!("App {app} does not exist"),
        )
        .detail(app)
    })
}

async fn events(
    State(state): State<UiState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let changes = BroadcastStream::new(state.inbox.subscribe()).map(|change| {
        let change = change.unwrap_or_else(|_| json!({ "app_id": "*", "contact": null }));
        Ok(Event::default().data(change.to_string()))
    });
    let first = stream::once(async {
        Ok(Event::default()
            .retry(Duration::from_secs(2))
            .comment("connected"))
    });

    Sse::new(first.chain(changes).take_until(state.inbox.stopped())).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(25))
            .text("ping"),
    )
}

async fn state_summary(State(state): State<UiState>) -> ApiResult {
    let inbox = &state.inbox;
    let apps: Vec<Value> = inbox
        .config
        .apps()
        .iter()
        .map(|app| {
            json!({
                "id": app.id,
                "name": app.name,
                "phone_number_id": app.phone_number_id,
                "display_phone_number": app.display_phone_number,
                "waba_id": app.waba_id,
                "webhook_url": app.webhook_url,
                "templates": inbox.config.templates_for(&app.id).len(),
                "can_verify": app.verify_token.is_some(),
            })
        })
        .collect();
    let conversations: Vec<Value> = inbox
        .store
        .conversations()?
        .into_iter()
        .map(|(conversation, last)| {
            let mut value = json!(conversation);
            value["last"] = last.map_or(Value::Null, |last| {
                let body = last
                    .rendered
                    .body
                    .clone()
                    .or_else(|| last.rendered.caption.clone())
                    .or_else(|| last.rendered.template.as_ref().map(|template| template.name.clone()))
                    .unwrap_or_default();

                json!({ "direction": last.direction, "kind": last.rendered.kind, "body": body, "status": last.status })
            });
            value
        })
        .collect();
    let failure_reasons: Vec<Value> = FAILURE_REASONS
        .iter()
        .map(|reason| json!({ "code": reason.code, "title": reason.title }))
        .collect();

    ok(json!({
        "api_url": state.public_api_url,
        "apps": apps,
        "conversations": conversations,
        "failure_reasons": failure_reasons,
        "config_error": inbox.config.error(),
        "using_default_app": inbox.config.using_fallback(),
    }))
}

async fn conversation(
    State(state): State<UiState>,
    Path((app, contact)): Path<(String, String)>,
) -> ApiResult {
    let inbox = &state.inbox;
    require_app(inbox, &app)?;

    let contact = normalize_contact(&contact);
    let messages = inbox.store.conversation(&app, &contact)?;
    let ids: Vec<String> = messages.iter().map(|message| message.id.clone()).collect();
    let statuses = inbox.store.statuses(&ids)?;
    let webhooks = inbox.store.webhooks_for_messages(&ids)?;
    let last_inbound = inbox.store.last_inbound_at(&app, &contact)?;

    let messages: Vec<Value> = messages
        .into_iter()
        .map(|message| {
            let mut value = json!(message);
            value["statuses"] = json!(
                statuses
                    .iter()
                    .filter(|status| status.message_id == message.id)
                    .collect::<Vec<_>>()
            );
            value["webhooks"] = webhooks
                .iter()
                .filter(|webhook| webhook.message_id.as_deref() == Some(message.id.as_str()))
                .map(|webhook| {
                    let mut summary = json!(webhook);
                    summary.as_object_mut().map(|object| object.remove("body"));
                    summary
                })
                .collect();
            value
        })
        .collect();

    ok(json!({
        "app_id": app,
        "contact": contact,
        "name": inbox.store.contact_name(&app, &contact)?,
        "window_open_until": last_inbound.map(|at| at + SERVICE_WINDOW_MS),
        "messages": messages,
    }))
}

async fn delete_conversation(
    State(state): State<UiState>,
    Path((app, contact)): Path<(String, String)>,
) -> ApiResult {
    require_app(&state.inbox, &app)?;

    let contact = normalize_contact(&contact);
    state.inbox.store.delete_conversation(&app, &contact)?;
    state.inbox.changed(&app, Some(&contact), json!({}));

    ok(json!({ "ok": true }))
}

async fn delete_all(State(state): State<UiState>) -> ApiResult {
    state.inbox.store.delete_all()?;
    state.inbox.changed("*", None, json!({}));

    ok(json!({ "ok": true }))
}

#[derive(Deserialize)]
struct UserMessage {
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    data: Option<String>,
    #[serde(default)]
    mime_type: Option<String>,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    caption: Option<String>,
}

fn present(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn required(value: Option<String>, field: &str) -> Result<String, ApiError> {
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "field_required",
                format!("Field {field} is required"),
            )
            .detail(field)
        })
}

#[allow(clippy::needless_pass_by_value)]
fn delivered(delivered: Delivered) -> ApiResult {
    ok(json!({ "message": delivered.message, "webhook": delivered.webhook }))
}

async fn send_as_user(
    State(state): State<UiState>,
    Path((app, contact)): Path<(String, String)>,
    body: Bytes,
) -> ApiResult {
    let body: UserMessage = parse_body(&body)?;
    require_app(&state.inbox, &app)?;

    let contact = normalize_contact(&contact);
    let phone_shaped = !contact.is_empty() && contact.chars().all(|char| char.is_ascii_digit());

    if (phone_shaped && !(8..=15).contains(&contact.len()))
        || (!phone_shaped && !BSUID.is_match(&contact))
    {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_contact",
            "Use a number with country and area code, digits only (15550000000), or a BSUID like BR.13491208655302741918",
        ));
    }

    let kind = present(body.kind).unwrap_or_else(|| "text".into());
    let name = present(body.name);

    if kind == "text" {
        let text = required(body.text, "text")?;
        return delivered(
            state
                .inbox
                .inbound_text(&app, &contact, name.as_deref(), &text)
                .await?,
        );
    }

    let data = base64::engine::general_purpose::STANDARD
        .decode(required(body.data, "data")?)
        .map_err(|_| {
            ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_base64",
                "Field data must be base64",
            )
        })?;
    let media = InboundMedia {
        kind,
        data,
        mime_type: required(body.mime_type, "mime_type")?,
        filename: present(body.filename),
        caption: present(body.caption),
    };

    delivered(
        state
            .inbox
            .inbound_media(&app, &contact, name.as_deref(), media)
            .await?,
    )
}

#[derive(Deserialize)]
struct Tap {
    index: usize,
}

async fn tap(State(state): State<UiState>, Path(id): Path<String>, body: Bytes) -> ApiResult {
    let body: Tap = parse_body(&body)?;
    delivered(state.inbox.tap_button(&id, body.index).await?)
}

#[derive(Deserialize)]
struct StatusRequest {
    status: String,
    #[serde(default)]
    error_code: Option<String>,
}

async fn fire_status(
    State(state): State<UiState>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult {
    let body: StatusRequest = parse_body(&body)?;
    if !matches!(
        body.status.as_str(),
        "sent" | "delivered" | "read" | "failed"
    ) {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_status",
            format!("Invalid status {}", body.status),
        )
        .detail(body.status));
    }

    ok(state
        .inbox
        .fire_status(&id, &body.status, body.error_code.as_deref())
        .await?)
}

async fn webhook(State(state): State<UiState>, Path(id): Path<i64>) -> ApiResult {
    let webhook = state.inbox.store.webhook(id)?.ok_or_else(|| {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "webhook_not_found",
            "Webhook not found",
        )
    })?;

    ok(webhook)
}

/// Raster images open inline so the inbox can show them. Everything else, SVG included, downloads under a sandbox
/// policy, so a file the user attached can never run script on the inbox origin.
async fn media(State(state): State<UiState>, Path(id): Path<String>) -> ApiResult {
    let Some(media) = state.inbox.store.media(&id)? else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let Ok(bytes) = tokio::fs::read(&media.path).await else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };

    let inline = media.mime_type.starts_with("image/") && !media.mime_type.contains("svg");
    let mut response = Response::new(Body::from(bytes));
    let headers = response.headers_mut();
    let content_type = if inline {
        media.mime_type.as_str()
    } else {
        "application/octet-stream"
    };
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type)
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("sandbox; default-src 'none'"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        if inline {
            HeaderValue::from_static("inline")
        } else {
            content_disposition("attachment", media.filename.as_deref().unwrap_or(&media.id))
        },
    );

    Ok(response)
}

async fn static_asset(method: Method, uri: Uri, _headers: HeaderMap) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }

    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    let Some(asset) = Assets::get(path) else {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    };

    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let mut response = Response::new(Body::from(asset.data.into_owned()));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(mime.as_ref())
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );

    if std::path::Path::new(path)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("html"))
    {
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(PAGE_POLICY),
        );
        headers.insert(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        );
    }

    response
}

async fn redeliver(State(state): State<UiState>, Path(id): Path<i64>) -> ApiResult {
    ok(state.inbox.redeliver(id).await?)
}

async fn verify_webhook(State(state): State<UiState>, Path(id): Path<String>) -> ApiResult {
    ok(state.inbox.verify_webhook(&id).await?)
}
