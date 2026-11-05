use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, FromRequest, Multipart, Query, Request, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use regex::Regex;
use serde_json::{Value, json};

use super::{BODY_LIMIT, content_disposition, media_response};
use crate::config::App;
use crate::graph_error::GraphError;
use crate::inbox::{Inbox, InboxError};
use crate::templates;

static VERSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^v\d+\.\d+$").expect("valid regex"));

#[derive(Clone)]
pub struct GraphState {
    pub inbox: Arc<Inbox>,
    pub public_ui_url: String,
}

pub fn router(state: GraphState) -> Router {
    Router::new()
        .fallback(handle)
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .with_state(state)
}

fn bearer(headers: &HeaderMap, query: &HashMap<String, String>) -> String {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value
                .split_once(' ')
                .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
                .map(|(_, token)| token)
        })
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .or_else(|| query.get("access_token").map(String::as_str))
        .unwrap_or_default()
        .to_owned()
}

fn json(status: StatusCode, body: Value) -> Response {
    (status, axum::Json(body)).into_response()
}

async fn handle(State(state): State<GraphState>, request: Request) -> Response {
    let method = request.method().clone();
    let uri = request.uri().clone();

    match route(&state, request).await {
        Ok(response) => response,
        Err(InboxError::Graph(error)) => error.into_response(),
        Err(error) => {
            tracing::error!("graph request {method} {uri} failed: {error}");
            GraphError::unknown().into_response()
        }
    }
}

fn path_segments(path: &str) -> Vec<String> {
    let mut segments: Vec<String> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
        .collect();

    if segments
        .first()
        .is_some_and(|segment| VERSION.is_match(segment))
    {
        segments.remove(0);
    }

    segments
}

async fn route(state: &GraphState, request: Request) -> Result<Response, InboxError> {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let query: HashMap<String, String> = Query::try_from_uri(request.uri())
        .map(|Query(query)| query)
        .unwrap_or_default();
    let headers = request.headers().clone();
    let token = bearer(&headers, &query);

    if method == Method::GET && path == "/" && !state.public_ui_url.is_empty() && token.is_empty() {
        let wants_html = headers
            .get(header::ACCEPT)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|accept| accept.contains("text/html"));

        if wants_html {
            return Ok((
                StatusCode::FOUND,
                [(header::LOCATION, format!("{}/", state.public_ui_url))],
            )
                .into_response());
        }
    }

    if method == Method::GET && path == "/health" {
        return Ok(json(StatusCode::OK, json!({ "ok": true })));
    }

    if from_a_web_page(&method, &headers) {
        return Err(GraphError::new(
            100,
            "(#100) wapit's Graph API only accepts requests from servers, not from web pages",
        )
        .status(StatusCode::FORBIDDEN)
        .into());
    }

    let app = authenticate(&state.inbox, &token)?;

    if method == Method::GET && path.trim_end_matches('/') == "/whatsapp_business/attachments" {
        return download_media(
            &state.inbox,
            &app,
            query.get("mid").map_or("", String::as_str),
        )
        .await;
    }

    let segments = path_segments(&path);
    let Some(id) = segments.first().cloned() else {
        return Err(GraphError::new(
            100,
            format!(
                "Unsupported request - method type: {}",
                method.as_str().to_lowercase()
            ),
        )
        .kind("GraphMethodException")
        .into());
    };
    let edge = segments.get(1).map(String::as_str);

    if method == Method::POST && edge == Some("media") && segments.len() == 2 {
        if app.phone_number_id != id {
            return Err(GraphError::unsupported("POST", &id).into());
        }

        let multipart = Multipart::from_request(request, &())
            .await
            .map_err(|rejection| GraphError::invalid_parameter(rejection.body_text()))?;
        return upload_media(&state.inbox, &app, multipart).await;
    }

    let body = axum::body::to_bytes(request.into_body(), BODY_LIMIT)
        .await
        .map_err(|_| {
            GraphError::new(100, "(#100) Invalid parameter")
                .details(format!(
                    "Request body is larger than {} MB",
                    BODY_LIMIT / 1024 / 1024
                ))
                .status(StatusCode::PAYLOAD_TOO_LARGE)
        })?;
    let templates_of_app =
        edge == Some("message_templates") && segments.len() == 2 && app.waba_id == id;

    match (&method, edge, segments.len()) {
        (&Method::POST, Some("messages"), 2) => post_message(&state.inbox, &app, &id, &body),
        (&Method::GET, None, 1) => get_object(state, &app, &id, &query),
        (&Method::POST, None, 1) => {
            state
                .inbox
                .edit_template(&app, &id, &template_input(&body)?)?;
            Ok(json(StatusCode::OK, json!({ "success": true })))
        }
        (&Method::DELETE, None, 1) => {
            state.inbox.delete_media(&app, &id).await?;
            Ok(json(StatusCode::OK, json!({ "success": true })))
        }
        (&Method::GET, _, _) if templates_of_app => Ok(json(
            StatusCode::OK,
            state
                .inbox
                .list_templates(&app, &templates::ListQuery::parse(&query)?),
        )),
        (&Method::POST, _, _) if templates_of_app => Ok(json(
            StatusCode::OK,
            state.inbox.create_template(&app, &template_input(&body)?)?,
        )),
        (&Method::DELETE, _, _) if templates_of_app => {
            state.inbox.delete_template(&app, &query)?;
            Ok(json(StatusCode::OK, json!({ "success": true })))
        }
        _ => Err(GraphError::unsupported(method.as_str(), &id).into()),
    }
}

async fn upload_media(
    inbox: &Inbox,
    app: &App,
    mut multipart: Multipart,
) -> Result<Response, InboxError> {
    let mut file: Option<(Option<String>, Option<String>, Bytes)> = None;
    let mut declared_type: Option<String> = None;
    let mut product: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| GraphError::invalid_parameter(error.body_text()))?
    {
        match field.name() {
            Some("file") => {
                let filename = field.file_name().map(str::to_owned);
                let content_type = field.content_type().map(str::to_owned);
                let data = field
                    .bytes()
                    .await
                    .map_err(|error| GraphError::invalid_parameter(error.body_text()))?;
                file = Some((filename, content_type, data));
            }
            Some("type") => declared_type = field.text().await.ok(),
            Some("messaging_product") => product = field.text().await.ok(),
            _ => {}
        }
    }

    if product.as_deref() != Some("whatsapp") {
        return Err(GraphError::required("messaging_product").into());
    }

    let (filename, content_type, data) = file.ok_or_else(|| GraphError::required("file"))?;
    let mime_type = declared_type
        .or(content_type)
        .filter(|mime| !mime.is_empty())
        .ok_or_else(|| GraphError::required("type"))?;
    let media = inbox.upload_media(app, &mime_type, filename, &data).await?;

    Ok(json(StatusCode::OK, json!({ "id": media.id })))
}

fn template_input(body: &Bytes) -> Result<Value, GraphError> {
    parse_body(body).map(|value| if value.is_object() { value } else { json!({}) })
}

/// Server-side SDKs send neither `Origin` nor a cross-site `Sec-Fetch-Site`. A browser does, even for no-cors
/// `text/plain` posts a page could otherwise aim at this port with the public default token.
fn from_a_web_page(method: &Method, headers: &HeaderMap) -> bool {
    if method == Method::GET || method == Method::HEAD {
        return false;
    }

    let fetch_site = headers
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok());

    headers.contains_key(header::ORIGIN)
        || matches!(fetch_site, Some("cross-site" | "same-site" | "same-origin"))
}

fn authenticate(inbox: &Inbox, token: &str) -> Result<Arc<App>, GraphError> {
    let app = if token.is_empty() {
        None
    } else {
        inbox.app_by_token(token)
    };

    app.ok_or_else(|| {
        let message = if token.is_empty() {
            "An active access token must be used to query information about the current user."
        } else {
            "Invalid OAuth access token - Cannot parse access token"
        };

        GraphError::new(190, message).status(StatusCode::UNAUTHORIZED)
    })
}

fn post_message(
    inbox: &Arc<Inbox>,
    app: &Arc<App>,
    phone_number_id: &str,
    body: &Bytes,
) -> Result<Response, InboxError> {
    if app.phone_number_id != phone_number_id {
        return Err(GraphError::unsupported("POST", phone_number_id).into());
    }

    let payload = parse_body(body)?;

    if payload.get("status").and_then(Value::as_str) == Some("read") {
        let message_id = payload
            .get("message_id")
            .and_then(Value::as_str)
            .ok_or_else(|| GraphError::required("message_id"))?;
        let typing = payload
            .get("typing_indicator")
            .is_some_and(Value::is_object);

        inbox.mark_inbound_read(app, message_id, typing)?;

        return Ok(json(StatusCode::OK, json!({ "success": true })));
    }

    let (message, input) = inbox.send_outbound(app, payload)?;
    let mut sent = json!({ "id": message.id });

    if message.kind == "template" {
        sent["message_status"] = json!("accepted");
    }

    Ok(json(
        StatusCode::OK,
        json!({ "messaging_product": "whatsapp", "contacts": [{ "input": input, "wa_id": message.contact }], "messages": [sent] }),
    ))
}

fn parse_body(body: &Bytes) -> Result<Value, GraphError> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(json!({}));
    }

    match serde_json::from_slice::<Value>(body) {
        Ok(value @ Value::Object(_)) => Ok(value),
        Ok(_) => Ok(json!({})),
        Err(_) => Err(GraphError::invalid_parameter(
            "Request body is not valid JSON",
        )),
    }
}

fn get_object(
    state: &GraphState,
    app: &App,
    id: &str,
    query: &HashMap<String, String>,
) -> Result<Response, InboxError> {
    if let Some(media) = state.inbox.media(id, Some(app))? {
        return Ok(json(
            StatusCode::OK,
            json!({
                "messaging_product": "whatsapp",
                "url": format!("{}&source=getMedia", state.inbox.media_url(&media.id)),
                "mime_type": media.mime_type,
                "sha256": media.sha256,
                "file_size": media.file_size,
                "id": media.id,
            }),
        ));
    }

    if id == app.waba_id {
        let fields = query.get("fields").map_or("", String::as_str);
        let mut object = json!({ "id": id });

        if fields.is_empty() || fields.contains("currency") {
            object["currency"] = json!("BRL");
        }

        if fields.contains("pricing_analytics") {
            object["pricing_analytics"] = json!({ "data": [{ "data_points": [] }] });
        }

        if fields.is_empty() || fields.contains("name") {
            object["name"] = json!(format!("{} (wapit)", app.name));
        }

        return Ok(json(StatusCode::OK, object));
    }

    if id == app.phone_number_id {
        return Ok(json(
            StatusCode::OK,
            json!({
                "id": id,
                "display_phone_number": app.display_phone_number,
                "verified_name": app.name,
                "quality_rating": "GREEN",
                "code_verification_status": "VERIFIED",
                "platform_type": "CLOUD_API",
            }),
        ));
    }

    Err(GraphError::unsupported("GET", id).into())
}

async fn download_media(inbox: &Inbox, app: &App, media_id: &str) -> Result<Response, InboxError> {
    let Some((media, bytes)) = inbox.media_file(media_id, Some(app)).await? else {
        return Ok((StatusCode::NOT_FOUND, "Not found").into_response());
    };

    Ok(media_response(
        bytes,
        &media.mime_type,
        content_disposition("attachment", media.filename.as_deref().unwrap_or(&media.id)),
    ))
}
