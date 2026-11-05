use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};
use base64::Engine;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use wapit::{Settings, build};

#[derive(Clone)]
struct Received {
    body: String,
    signature: String,
    json: Value,
}

struct Harness {
    graph: String,
    ui: String,
    received: Arc<Mutex<Vec<Received>>>,
    client: reqwest::Client,
    config: TempDir,
    _data: TempDir,
}

async fn serve(router: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    format!("http://{address}")
}

fn template() -> Value {
    json!({
        "name": "order_ready",
        "language": "en_US",
        "status": "APPROVED",
        "category": "UTILITY",
        "components": [
            { "type": "BODY", "text": "Order at {{1}} for {{2}}, code {{3}}" },
            { "type": "BUTTONS", "buttons": [
                { "type": "URL", "text": "Track", "url": "https://shop.example/t/{{1}}" },
                { "type": "URL", "text": "Help", "url": "https://shop.example/help/{{1}}" },
            ] },
        ],
    })
}

async fn harness() -> Harness {
    let received = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&received);
    let hook_router = Router::new().route(
        "/{*path}",
        axum::routing::get(
            |axum::extract::Query(query): axum::extract::Query<
                std::collections::HashMap<String, String>,
            >| async move {
                if query.get("hub.mode").map(String::as_str) == Some("subscribe")
                    && query.get("hub.verify_token").map(String::as_str) == Some("verify-a")
                {
                    (
                        StatusCode::OK,
                        query.get("hub.challenge").cloned().unwrap_or_default(),
                    )
                } else {
                    (StatusCode::FORBIDDEN, String::new())
                }
            },
        )
        .post(move |headers: HeaderMap, body: Bytes| {
            let sink = Arc::clone(&sink);
            async move {
                let body = String::from_utf8(body.to_vec()).unwrap();
                sink.lock().unwrap().push(Received {
                    signature: headers["x-hub-signature-256"].to_str().unwrap().to_owned(),
                    json: serde_json::from_str(&body).unwrap(),
                    body,
                });
                (StatusCode::OK, "{\"ok\":true}")
            }
        }),
    );
    let receiver_url = serve(hook_router).await;

    let config = TempDir::new().unwrap();
    std::fs::create_dir(config.path().join("templates")).unwrap();
    std::fs::write(
        config.path().join("apps.json"),
        json!({ "apps": [
            {
                "id": "a", "name": "App A", "phone_number_id": "100", "display_phone_number": "+1 555 0100",
                "waba_id": "200", "access_token": "token-a", "app_secret": "secret-a",
                "webhook_url": format!("{receiver_url}/hook"), "auto_statuses": [], "verify_token": "verify-a",
            },
            {
                "id": "b", "name": "App B", "phone_number_id": "101", "waba_id": "201", "access_token": "token-b",
                "app_secret": "secret-b", "webhook_url": format!("{receiver_url}/hook-b"), "auto_statuses": [],
            },
        ] })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        config.path().join("templates/a.json"),
        json!([template(), {
            "name": "reordered", "language": "en_US", "status": "APPROVED", "category": "UTILITY",
            "components": [{ "type": "BODY", "text": "Unit {{2}} of {{1}}" }],
        }])
        .to_string(),
    )
    .unwrap();

    let data = TempDir::new().unwrap();
    let graph_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let graph_url = format!("http://{}", graph_listener.local_addr().unwrap());
    let servers = build(&Settings {
        config_dir: config.path().to_owned(),
        data_dir: data.path().to_owned(),
        public_api_url: graph_url.clone(),
        public_ui_url: "http://127.0.0.1:8026".into(),
        loopback_host: None,
        extra_ca_file: None,
        allowed_hosts: Vec::new(),
        auto_status_delay: Duration::from_millis(10),
    })
    .unwrap();

    let graph = servers.graph;
    tokio::spawn(async move { axum::serve(graph_listener, graph).await.unwrap() });
    let ui = serve(servers.ui).await;

    Harness {
        graph: graph_url,
        ui,
        received,
        client: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap(),
        config,
        _data: data,
    }
}

fn template_payload(body: &[&str], buttons: &[&str], name: &str, language: &str) -> Value {
    let mut components = vec![
        json!({ "type": "body", "parameters": body.iter().map(|text| json!({ "type": "text", "text": text })).collect::<Vec<_>>() }),
    ];

    for (index, text) in buttons.iter().enumerate() {
        components.push(json!({ "type": "button", "sub_type": "url", "index": index.to_string(), "parameters": [{ "type": "text", "text": text }] }));
    }

    json!({
        "messaging_product": "whatsapp",
        "recipient_type": "individual",
        "to": "15550000000",
        "type": "template",
        "template": { "name": name, "language": { "code": language }, "components": components },
    })
}

fn valid_template() -> Value {
    template_payload(
        &["Sun Plaza", "Mary", "A1B2"],
        &["tok123", "desk"],
        "order_ready",
        "en_US",
    )
}

impl Harness {
    async fn post(&self, path: &str, body: &Value, token: &str) -> (StatusCode, Value) {
        let response = self
            .client
            .post(format!("{}{path}", self.graph))
            .bearer_auth(token)
            .json(body)
            .send()
            .await
            .unwrap();
        let status = response.status();

        (status, response.json().await.unwrap())
    }

    async fn ui_get(&self, path: &str) -> Value {
        self.client
            .get(format!("{}{path}", self.ui))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    async fn ui_post(&self, path: &str, body: &Value) -> reqwest::Response {
        self.client
            .post(format!("{}{path}", self.ui))
            .json(body)
            .send()
            .await
            .unwrap()
    }

    async fn webhook_after(&self, before: usize) -> Received {
        for _ in 0..300 {
            if let Some(received) = self.received.lock().unwrap().get(before).cloned() {
                return received;
            }

            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        panic!("no webhook arrived");
    }

    fn received_count(&self) -> usize {
        self.received.lock().unwrap().len()
    }
}

fn value_of(received: &Received) -> &Value {
    &received.json["entry"][0]["changes"][0]["value"]
}

fn encode(path: &str) -> String {
    percent_encoding::utf8_percent_encode(path, percent_encoding::NON_ALPHANUMERIC).to_string()
}

#[tokio::test]
async fn a_browser_opening_the_api_root_goes_to_the_inbox() {
    let harness = harness().await;
    let browser = harness
        .client
        .get(format!("{}/", harness.graph))
        .header("Accept", "text/html")
        .send()
        .await
        .unwrap();
    let client = harness
        .client
        .get(format!("{}/", harness.graph))
        .send()
        .await
        .unwrap();

    assert_eq!(browser.status(), StatusCode::FOUND);
    assert_eq!(browser.headers()["location"], "http://127.0.0.1:8026/");
    assert_eq!(client.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn requests_without_a_valid_token_are_rejected_like_meta() {
    let harness = harness().await;
    let (status, body) = harness
        .post("/v23.0/100/messages", &valid_template(), "wrong")
        .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], 190);
}

#[tokio::test]
async fn a_phone_number_id_of_another_app_is_unsupported() {
    let harness = harness().await;
    let (status, body) = harness
        .post("/v23.0/101/messages", &valid_template(), "token-a")
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], 100);
    assert_eq!(body["error"]["error_subcode"], 33);
}

#[tokio::test]
async fn a_valid_template_renders_variables_and_button_urls() {
    let harness = harness().await;
    let (status, body) = harness
        .post("/v26.0/100/messages", &valid_template(), "token-a")
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["contacts"][0]["wa_id"], "15550000000");
    assert!(
        body["messages"][0]["id"]
            .as_str()
            .unwrap()
            .starts_with("wamid.")
    );
    assert_eq!(body["messages"][0]["message_status"], "accepted");

    let conversation = harness.ui_get("/api/conversations/a/15550000000").await;
    let message = conversation["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["id"] == body["messages"][0]["id"])
        .unwrap();
    let variables: Vec<&str> = message["rendered"]["body_parts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|part| part["variable"] == true)
        .map(|part| part["text"].as_str().unwrap())
        .collect();
    let urls: Vec<&str> = message["rendered"]["buttons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|button| button["url"].as_str().unwrap())
        .collect();

    assert_eq!(
        message["rendered"]["body"],
        "Order at Sun Plaza for Mary, code A1B2"
    );
    assert_eq!(variables, ["Sun Plaza", "Mary", "A1B2"]);
    assert_eq!(
        urls,
        [
            "https://shop.example/t/tok123",
            "https://shop.example/help/desk"
        ]
    );
}

#[tokio::test]
async fn a_body_parameter_count_mismatch_is_132000_and_flags_the_conversation() {
    let harness = harness().await;
    let payload = template_payload(
        &["Sun Plaza", "Mary"],
        &["tok123", "desk"],
        "order_ready",
        "en_US",
    );
    let (status, body) = harness
        .post("/v26.0/100/messages", &payload, "token-a")
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], 132_000);
    assert_eq!(
        body["error"]["error_data"]["details"],
        "body: number of localizable_params (2) does not match the expected number of params (3)"
    );

    let state = harness.ui_get("/api/state").await;
    let conversation = state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|conversation| conversation["app_id"] == "a")
        .unwrap();
    assert!(conversation["flagged"].as_i64().unwrap() >= 1);

    let rejected = &harness.ui_get("/api/conversations/a/15550000000").await["messages"][0];
    assert_eq!(
        rejected["error"]["mismatch"],
        json!({ "location": "body", "sent": 2, "expected": 3 })
    );
}

#[tokio::test]
async fn a_missing_url_button_parameter_is_132000() {
    let harness = harness().await;
    let payload = template_payload(
        &["Sun Plaza", "Mary", "A1B2"],
        &["tok123"],
        "order_ready",
        "en_US",
    );
    let (_, body) = harness
        .post("/v26.0/100/messages", &payload, "token-a")
        .await;

    assert_eq!(body["error"]["code"], 132_000);
    assert!(
        body["error"]["error_data"]["details"]
            .as_str()
            .unwrap()
            .contains("button at index 1")
    );
}

#[tokio::test]
async fn unknown_template_names_and_languages_are_132001() {
    let harness = harness().await;
    let unknown_name = template_payload(&["a", "b", "c"], &["x", "y"], "nope", "en_US");
    let unknown_language = template_payload(&["a", "b", "c"], &["x", "y"], "order_ready", "pt_BR");

    assert_eq!(
        harness
            .post("/v26.0/100/messages", &unknown_name, "token-a")
            .await
            .1["error"]["code"],
        132_001
    );
    assert_eq!(
        harness
            .post("/v26.0/100/messages", &unknown_language, "token-a")
            .await
            .1["error"]["code"],
        132_001
    );
}

#[tokio::test]
async fn new_lines_inside_body_parameters_are_132018() {
    let harness = harness().await;
    let payload = template_payload(
        &["Sun\nPlaza", "Mary", "A1B2"],
        &["tok123", "desk"],
        "order_ready",
        "en_US",
    );

    assert_eq!(
        harness
            .post("/v26.0/100/messages", &payload, "token-a")
            .await
            .1["error"]["code"],
        132_018
    );
}

#[tokio::test]
async fn interactive_reply_button_titles_are_limited_to_20_characters() {
    let harness = harness().await;
    let payload = json!({
        "messaging_product": "whatsapp", "to": "15550000001", "type": "interactive",
        "interactive": { "type": "button", "body": { "text": "Confirm?" }, "action": { "buttons": [{ "type": "reply", "reply": { "id": "yes", "title": "A title that is far too long" } }] } },
    });

    assert_eq!(
        harness
            .post("/v23.0/100/messages", &payload, "token-a")
            .await
            .1["error"]["code"],
        131_009
    );
}

#[tokio::test]
async fn the_waba_pricing_query_gets_an_empty_well_formed_result() {
    let harness = harness().await;
    let response = harness
        .client
        .get(format!(
            "{}/v26.0/200?fields=currency,pricing_analytics.start(1).end(2)",
            harness.graph
        ))
        .bearer_auth("token-a")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["currency"], "BRL");
    assert_eq!(
        body["pricing_analytics"]["data"][0]["data_points"],
        json!([])
    );
}

#[tokio::test]
async fn a_user_text_reaches_the_webhook_signed() {
    let harness = harness().await;
    let before = harness.received_count();
    let response = harness
        .ui_post(
            "/api/conversations/a/15550000002/messages",
            &json!({ "kind": "text", "text": "Hi", "name": "Mary" }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);

    let webhook = harness.webhook_after(before).await;
    let value = value_of(&webhook);

    assert_eq!(
        webhook.signature,
        wapit::webhooks::sign(&webhook.body, "secret-a")
    );
    assert_eq!(webhook.json["object"], "whatsapp_business_account");
    assert_eq!(value["metadata"]["phone_number_id"], "100");
    assert_eq!(value["contacts"][0]["profile"]["name"], "Mary");
    assert_eq!(value["messages"][0]["text"]["body"], "Hi");
}

#[tokio::test]
async fn a_reply_button_tap_reaches_the_webhook_with_its_id_and_context() {
    let harness = harness().await;
    let payload = json!({
        "messaging_product": "whatsapp", "to": "15550000002", "type": "interactive",
        "interactive": { "type": "button", "body": { "text": "Confirm?" }, "action": { "buttons": [{ "type": "reply", "reply": { "id": "confirm:1", "title": "Confirm" } }] } },
    });
    let message_id = harness
        .post("/v23.0/100/messages", &payload, "token-a")
        .await
        .1["messages"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let before = harness.received_count();

    harness
        .ui_post(
            &format!("/api/messages/{}/tap", encode(&message_id)),
            &json!({ "index": 0 }),
        )
        .await;
    let webhook = harness.webhook_after(before).await;
    let message = &value_of(&webhook)["messages"][0];

    assert_eq!(message["type"], "interactive");
    assert_eq!(message["interactive"]["button_reply"]["id"], "confirm:1");
    assert_eq!(message["context"]["id"], message_id.as_str());
}

#[tokio::test]
async fn a_user_image_can_be_fetched_through_the_media_endpoints() {
    let harness = harness().await;
    let before = harness.received_count();
    let bytes = b"fake-png-bytes";

    harness
        .ui_post(
            "/api/conversations/a/15550000002/messages",
            &json!({ "kind": "image", "mime_type": "image/png", "filename": "receipt.png", "caption": "Receipt", "data": base64::engine::general_purpose::STANDARD.encode(bytes) }),
        )
        .await;
    let webhook = harness.webhook_after(before).await;
    let image = value_of(&webhook)["messages"][0]["image"].clone();
    let meta: Value = harness
        .client
        .get(format!(
            "{}/v23.0/{}",
            harness.graph,
            image["id"].as_str().unwrap()
        ))
        .bearer_auth("token-a")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let url = meta["url"].as_str().unwrap();

    assert_eq!(image["caption"], "Receipt");
    assert_eq!(meta["file_size"], bytes.len());
    assert_eq!(
        harness.client.get(url).send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        harness
            .client
            .get(format!(
                "{}/v23.0/{}",
                harness.graph,
                image["id"].as_str().unwrap()
            ))
            .bearer_auth("token-b")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );

    let download = harness
        .client
        .get(url)
        .bearer_auth("token-a")
        .send()
        .await
        .unwrap();
    assert_eq!(download.bytes().await.unwrap().as_ref(), bytes);
}

#[tokio::test]
async fn a_read_status_from_the_inbox_carries_the_outbound_id_and_pricing() {
    let harness = harness().await;
    let message_id = harness
        .post("/v26.0/100/messages", &valid_template(), "token-a")
        .await
        .1["messages"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let before = harness.received_count();

    harness
        .ui_post(
            &format!("/api/messages/{}/status", encode(&message_id)),
            &json!({ "status": "read" }),
        )
        .await;
    let status = value_of(&harness.webhook_after(before).await)["statuses"][0].clone();

    assert_eq!(status["id"], message_id.as_str());
    assert_eq!(status["status"], "read");
    assert_eq!(status["recipient_id"], "15550000000");
    assert!(
        status["timestamp"]
            .as_str()
            .unwrap()
            .chars()
            .all(|char| char.is_ascii_digit())
    );
    assert_eq!(status["pricing"]["category"], "utility");
}

#[tokio::test]
async fn a_failed_status_carries_a_meta_error() {
    let harness = harness().await;
    let message_id = harness
        .post("/v26.0/100/messages", &valid_template(), "token-a")
        .await
        .1["messages"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let before = harness.received_count();

    harness
        .ui_post(
            &format!("/api/messages/{}/status", encode(&message_id)),
            &json!({ "status": "failed", "error_code": "131047" }),
        )
        .await;
    let status = value_of(&harness.webhook_after(before).await)["statuses"][0].clone();

    assert_eq!(status["status"], "failed");
    assert_eq!(status["errors"][0]["code"], 131_047);
}

#[tokio::test]
async fn a_failure_with_an_unknown_error_code_is_refused() {
    let harness = harness().await;
    let message_id = harness
        .post("/v26.0/100/messages", &valid_template(), "token-a")
        .await
        .1["messages"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let before = harness.received_count();

    let response = harness
        .ui_post(
            &format!("/api/messages/{}/status", encode(&message_id)),
            &json!({ "status": "failed", "error_code": "999999" }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["code"], "unknown_failure_code");
    assert_eq!(body["detail"], "999999");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(harness.received_count(), before);
}

#[tokio::test]
async fn a_rejected_send_without_a_type_is_recorded_as_text() {
    let harness = harness().await;
    let payload =
        json!({ "messaging_product": "whatsapp", "to": "15550000010", "text": { "body": "" } });
    let (status, _) = harness
        .post("/v26.0/100/messages", &payload, "token-a")
        .await;
    let rejected = &harness.ui_get("/api/conversations/a/15550000010").await["messages"][0];

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(rejected["status"], "rejected");
    assert_eq!(rejected["type"], "text");
    assert_eq!(rejected["rendered"]["kind"], "text");
}

#[tokio::test]
async fn a_late_status_never_moves_a_message_backwards() {
    let harness = harness().await;
    let message_id = harness
        .post("/v26.0/100/messages", &valid_template(), "token-a")
        .await
        .1["messages"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    for status in ["read", "delivered"] {
        harness
            .ui_post(
                &format!("/api/messages/{}/status", encode(&message_id)),
                &json!({ "status": status }),
            )
            .await;
    }

    let conversation = harness.ui_get("/api/conversations/a/15550000000").await;
    assert_eq!(conversation["messages"][0]["status"], "read");
    assert_eq!(
        conversation["messages"][0]["statuses"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn the_app_marking_an_inbound_message_read_is_accepted() {
    let harness = harness().await;
    let inbound: Value = harness
        .ui_post(
            "/api/conversations/a/15550000003/messages",
            &json!({ "kind": "text", "text": "Hello" }),
        )
        .await
        .json()
        .await
        .unwrap();
    let payload = json!({ "messaging_product": "whatsapp", "status": "read", "message_id": inbound["message"]["id"], "typing_indicator": { "type": "text" } });

    assert_eq!(
        harness
            .post("/v23.0/100/messages", &payload, "token-a")
            .await
            .1,
        json!({ "success": true })
    );
}

#[tokio::test]
async fn free_form_outside_the_service_window_gets_a_failed_131047_status() {
    let harness = harness().await;
    let before = harness.received_count();
    let payload = json!({ "messaging_product": "whatsapp", "to": "15550000009", "type": "text", "text": { "body": "Hi" } });
    let message_id = harness
        .post("/v23.0/100/messages", &payload, "token-a")
        .await
        .1["messages"][0]["id"]
        .clone();
    let status = value_of(&harness.webhook_after(before).await)["statuses"][0].clone();

    assert_eq!(status["id"], message_id);
    assert_eq!(status["status"], "failed");
    assert_eq!(status["errors"][0]["code"], 131_047);

    let conversation = harness.ui_get("/api/conversations/a/15550000009").await;
    assert_eq!(
        conversation["messages"][0]["issues"],
        json!([{ "code": "outside_service_window" }])
    );
}

#[tokio::test]
async fn a_bsuid_conversation_opens_by_its_raw_id() {
    let harness = harness().await;
    let payload = json!({
        "messaging_product": "whatsapp", "recipient": "BR.abc123", "type": "template",
        "template": { "name": "reordered", "language": { "code": "en_US" }, "components": [{ "type": "body", "parameters": [{ "type": "text", "text": "Mary" }, { "type": "text", "text": "101" }] }] },
    });
    harness
        .post("/v23.0/100/messages", &payload, "token-a")
        .await;
    let conversation = harness.ui_get("/api/conversations/a/BR.abc123").await;

    assert_eq!(conversation["messages"].as_array().unwrap().len(), 1);
    assert_eq!(
        conversation["messages"][0]["rendered"]["body"],
        "Unit 101 of Mary"
    );
}

#[tokio::test]
async fn a_malformed_path_is_rejected_without_crashing() {
    let harness = harness().await;

    assert_eq!(
        harness
            .client
            .get(format!("{}/api/conversations/a/%E0%A4%A", harness.ui))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        harness
            .client
            .get(format!("{}/api/state", harness.ui))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn cross_site_and_non_json_writes_are_refused() {
    let harness = harness().await;
    let path = format!("{}/api/conversations/a/15550000002/messages", harness.ui);
    let body = json!({ "kind": "text", "text": "forged" }).to_string();

    let plain = harness
        .client
        .post(&path)
        .header("Content-Type", "text/plain")
        .body(body.clone())
        .send()
        .await
        .unwrap();
    let cross = harness
        .client
        .post(&path)
        .header("Content-Type", "application/json")
        .header("Sec-Fetch-Site", "cross-site")
        .body(body)
        .send()
        .await
        .unwrap();

    assert_eq!(plain.status(), StatusCode::FORBIDDEN);
    assert_eq!(cross.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn an_unparsable_request_target_gets_a_400_on_both_ports() {
    let harness = harness().await;

    for base in [&harness.ui, &harness.graph] {
        let mut socket = tokio::net::TcpStream::connect(base.trim_start_matches("http://"))
            .await
            .unwrap();
        socket
            .write_all(b"GET http://[ HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .await
            .unwrap();
        let mut reply = String::new();
        let _ =
            tokio::time::timeout(Duration::from_secs(2), socket.read_to_string(&mut reply)).await;

        assert!(reply.starts_with("HTTP/1.1 400"), "{reply}");
    }

    assert_eq!(
        harness
            .client
            .get(format!("{}/health", harness.graph))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn media_with_non_latin_file_names_downloads() {
    let harness = harness().await;
    let inbound: Value = harness
        .ui_post(
            "/api/conversations/a/BR.13491208655302741918/messages",
            &json!({ "kind": "document", "mime_type": "application/pdf", "filename": "relatório – final.pdf", "data": base64::engine::general_purpose::STANDARD.encode(b"pdf") }),
        )
        .await
        .json()
        .await
        .unwrap();
    let media_id = inbound["message"]["payload"]["document"]["id"]
        .as_str()
        .unwrap();
    let meta: Value = harness
        .client
        .get(format!("{}/v23.0/{media_id}", harness.graph))
        .bearer_auth("token-a")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let download = harness
        .client
        .get(meta["url"].as_str().unwrap())
        .bearer_auth("token-a")
        .send()
        .await
        .unwrap();

    assert_eq!(download.status(), StatusCode::OK);
    assert!(
        download.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .contains("filename*=UTF-8''relat%C3%B3rio")
    );
}

#[tokio::test]
async fn a_foreign_host_header_is_refused_on_both_ports() {
    let harness = harness().await;

    for url in [
        format!("{}/api/state", harness.ui),
        format!("{}/health", harness.graph),
    ] {
        let response = harness
            .client
            .get(url)
            .header("Host", "attacker.example")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}

#[tokio::test]
async fn an_uploaded_svg_downloads_instead_of_rendering() {
    let harness = harness().await;
    let svg = b"<svg xmlns='http://www.w3.org/2000/svg'><script>alert(1)</script></svg>";
    let inbound: Value = harness
        .ui_post(
            "/api/conversations/a/15550000004/messages",
            &json!({ "kind": "image", "mime_type": "image/svg+xml", "data": base64::engine::general_purpose::STANDARD.encode(svg) }),
        )
        .await
        .json()
        .await
        .unwrap();
    let response = harness
        .client
        .get(format!(
            "{}/media/{}",
            harness.ui,
            inbound["message"]["payload"]["image"]["id"]
                .as_str()
                .unwrap()
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.headers()["content-type"],
        "application/octet-stream"
    );
    assert!(
        response.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .starts_with("attachment")
    );
    assert!(
        response.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("sandbox")
    );
}

#[tokio::test]
async fn deleting_a_conversation_removes_its_media_files() {
    let harness = harness().await;
    let inbound: Value = harness
        .ui_post(
            "/api/conversations/a/15550000005/messages",
            &json!({ "kind": "image", "mime_type": "image/png", "data": base64::engine::general_purpose::STANDARD.encode(b"png") }),
        )
        .await
        .json()
        .await
        .unwrap();
    let media_url = format!(
        "{}/media/{}",
        harness.ui,
        inbound["message"]["payload"]["image"]["id"]
            .as_str()
            .unwrap()
    );

    assert_eq!(
        harness
            .client
            .get(&media_url)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    harness
        .client
        .delete(format!("{}/api/conversations/a/15550000005", harness.ui))
        .send()
        .await
        .unwrap();
    assert_eq!(
        harness
            .client
            .get(&media_url)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_broken_apps_file_keeps_the_last_good_config() {
    let harness = harness().await;
    assert_eq!(
        harness
            .post("/v26.0/100/messages", &valid_template(), "token-a")
            .await
            .0,
        StatusCode::OK
    );

    tokio::time::sleep(Duration::from_millis(20)).await;
    std::fs::write(harness.config.path().join("apps.json"), "{ \"apps\": [").unwrap();

    assert_eq!(
        harness
            .post("/v26.0/100/messages", &valid_template(), "token-a")
            .await
            .0,
        StatusCode::OK
    );
    assert!(
        harness.ui_get("/api/state").await["config_error"]
            .as_str()
            .unwrap()
            .contains("apps.json")
    );
}

#[tokio::test]
async fn a_receiver_that_hangs_up_mid_response_still_records_the_webhook() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0u8; 4096];
            let _ = socket.read(&mut buffer).await;
            let _ = socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\npartial")
                .await;
            drop(socket);
        }
    });

    let harness = harness().await;
    let apps = std::fs::read_to_string(harness.config.path().join("apps.json"))
        .unwrap()
        .replace(&value_of_hook(&harness), &format!("http://{address}/hook"));
    tokio::time::sleep(Duration::from_millis(20)).await;
    std::fs::write(harness.config.path().join("apps.json"), apps).unwrap();

    let response = tokio::time::timeout(
        Duration::from_secs(10),
        harness.ui_post(
            "/api/conversations/a/15550000006/messages",
            &json!({ "kind": "text", "text": "Hi" }),
        ),
    )
    .await
    .expect("the inbox answers instead of hanging");
    let body: Value = response.json().await.unwrap();

    assert_eq!(body["webhook"]["status_code"], 200);
    assert!(body["webhook"]["error"].is_string());
}

fn value_of_hook(harness: &Harness) -> String {
    let apps: Value = serde_json::from_str(
        &std::fs::read_to_string(harness.config.path().join("apps.json")).unwrap(),
    )
    .unwrap();

    apps["apps"][0]["webhook_url"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn an_uploaded_media_file_can_be_sent_fetched_and_deleted() {
    let harness = harness().await;
    let form = reqwest::multipart::Form::new()
        .text("messaging_product", "whatsapp")
        .text("type", "image/png")
        .part(
            "file",
            reqwest::multipart::Part::bytes(b"png-bytes".to_vec())
                .file_name("logo.png")
                .mime_str("image/png")
                .unwrap(),
        );
    let upload: Value = harness
        .client
        .post(format!("{}/v23.0/100/media", harness.graph))
        .bearer_auth("token-a")
        .multipart(form)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let media_id = upload["id"].as_str().unwrap().to_owned();

    let meta: Value = harness
        .client
        .get(format!("{}/v23.0/{media_id}", harness.graph))
        .bearer_auth("token-a")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(meta["mime_type"], "image/png");
    assert_eq!(meta["file_size"], 9);

    let deleted = harness
        .client
        .delete(format!("{}/v23.0/{media_id}", harness.graph))
        .bearer_auth("token-a")
        .send()
        .await
        .unwrap();
    assert_eq!(
        deleted.json::<Value>().await.unwrap(),
        json!({ "success": true })
    );
    assert_eq!(
        harness
            .client
            .get(format!("{}/v23.0/{media_id}", harness.graph))
            .bearer_auth("token-a")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn an_upload_for_another_apps_phone_number_is_unsupported() {
    let harness = harness().await;
    let form = reqwest::multipart::Form::new()
        .text("messaging_product", "whatsapp")
        .part(
            "file",
            reqwest::multipart::Part::bytes(b"x".to_vec())
                .mime_str("image/png")
                .unwrap(),
        );
    let response = harness
        .client
        .post(format!("{}/v23.0/101/media", harness.graph))
        .bearer_auth("token-a")
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.json::<Value>().await.unwrap()["error"]["error_subcode"],
        33
    );
}

#[tokio::test]
async fn templates_can_be_created_used_edited_and_deleted() {
    let harness = harness().await;
    let created = harness
        .post(
            "/v23.0/200/message_templates",
            &json!({ "name": "welcome", "language": "en_US", "category": "utility", "components": [{ "type": "BODY", "text": "Welcome, {{1}}" }] }),
            "token-a",
        )
        .await;

    assert_eq!(created.0, StatusCode::OK);
    assert_eq!(created.1["status"], "APPROVED");
    assert_eq!(created.1["category"], "UTILITY");

    let duplicate = harness
        .post("/v23.0/200/message_templates", &json!({ "name": "welcome", "language": "en_US", "category": "UTILITY", "components": [{ "type": "BODY", "text": "Hi" }] }), "token-a")
        .await;
    assert_eq!(duplicate.1["error"]["error_subcode"], 2_388_024);

    let send = json!({
        "messaging_product": "whatsapp", "to": "15550000007", "type": "template",
        "template": { "name": "welcome", "language": { "code": "en_US" }, "components": [{ "type": "body", "parameters": [{ "type": "text", "text": "Mary" }] }] },
    });
    assert_eq!(
        harness
            .post("/v23.0/100/messages", &send, "token-a")
            .await
            .0,
        StatusCode::OK
    );

    let id = created.1["id"].as_str().unwrap();
    let edited = harness
        .post(
            &format!("/v23.0/{id}"),
            &json!({ "components": [{ "type": "BODY", "text": "Hello {{1}}, welcome" }] }),
            "token-a",
        )
        .await;
    assert_eq!(edited.1, json!({ "success": true }));

    let listed: Value = harness
        .client
        .get(format!(
            "{}/v23.0/200/message_templates?name=welcome",
            harness.graph
        ))
        .bearer_auth("token-a")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        listed["data"][0]["components"][0]["text"],
        "Hello {{1}}, welcome"
    );

    let deleted = harness
        .client
        .delete(format!(
            "{}/v23.0/200/message_templates?name=welcome",
            harness.graph
        ))
        .bearer_auth("token-a")
        .send()
        .await
        .unwrap();
    assert_eq!(
        deleted.json::<Value>().await.unwrap(),
        json!({ "success": true })
    );
    assert_eq!(
        harness
            .post("/v23.0/100/messages", &send, "token-a")
            .await
            .1["error"]["code"],
        132_001
    );
}

#[tokio::test]
async fn a_template_without_a_body_is_rejected() {
    let harness = harness().await;
    let result = harness
        .post("/v23.0/200/message_templates", &json!({ "name": "empty", "language": "en_US", "category": "UTILITY", "components": [{ "type": "FOOTER", "text": "Bye" }] }), "token-a")
        .await;

    assert_eq!(result.0, StatusCode::BAD_REQUEST);
    assert_eq!(result.1["error"]["code"], 100);
}

#[tokio::test]
async fn a_webhook_can_be_redelivered_with_the_same_signed_body() {
    let harness = harness().await;
    let before = harness.received_count();
    let sent: Value = harness
        .ui_post(
            "/api/conversations/a/15550000008/messages",
            &json!({ "kind": "text", "text": "Again" }),
        )
        .await
        .json()
        .await
        .unwrap();
    let first = harness.webhook_after(before).await;

    harness
        .ui_post(
            &format!("/api/webhooks/{}/redeliver", sent["webhook"]["id"]),
            &json!({}),
        )
        .await;
    let second = harness.webhook_after(before + 1).await;

    assert_eq!(first.body, second.body);
    assert_eq!(first.signature, second.signature);
}

#[tokio::test]
async fn the_webhook_verification_handshake_checks_the_challenge() {
    let harness = harness().await;
    let passed: Value = harness
        .ui_post("/api/apps/a/verify-webhook", &json!({}))
        .await
        .json()
        .await
        .unwrap();
    let missing = harness
        .ui_post("/api/apps/b/verify-webhook", &json!({}))
        .await;

    assert_eq!(passed["ok"], true);
    assert_eq!(passed["body"], passed["challenge"]);
    assert_eq!(missing.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn without_apps_json_a_default_app_answers() {
    let config = TempDir::new().unwrap();
    let data = TempDir::new().unwrap();
    let servers = build(&Settings {
        config_dir: config.path().to_owned(),
        data_dir: data.path().to_owned(),
        public_api_url: "http://127.0.0.1:8027".into(),
        public_ui_url: "http://127.0.0.1:8026".into(),
        loopback_host: None,
        extra_ca_file: None,
        allowed_hosts: Vec::new(),
        auto_status_delay: Duration::from_millis(10),
    })
    .unwrap();
    let graph = serve(servers.graph).await;
    let ui = serve(servers.ui).await;
    let client = reqwest::Client::new();

    let phone: Value = client
        .get(format!("{graph}/v23.0/100000000000001"))
        .bearer_auth("wapit-local-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let state: Value = client
        .get(format!("{ui}/api/state"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(phone["platform_type"], "CLOUD_API");
    assert_eq!(state["using_default_app"], true);
    assert_eq!(state["apps"][0]["id"], "default");
}

#[tokio::test]
async fn the_shipped_example_config_accepts_its_example_template() {
    let config = TempDir::new().unwrap();
    let data = TempDir::new().unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config");
    std::fs::create_dir(config.path().join("templates")).unwrap();
    std::fs::copy(
        root.join("apps.example.json"),
        config.path().join("apps.json"),
    )
    .unwrap();
    std::fs::copy(
        root.join("templates/example.json"),
        config.path().join("templates/example.json"),
    )
    .unwrap();

    let servers = build(&Settings {
        config_dir: config.path().to_owned(),
        data_dir: data.path().to_owned(),
        public_api_url: "http://127.0.0.1:8027".into(),
        public_ui_url: String::new(),
        loopback_host: None,
        extra_ca_file: None,
        allowed_hosts: Vec::new(),
        auto_status_delay: Duration::from_secs(60),
    })
    .unwrap();
    let graph = serve(servers.graph).await;
    let payload = json!({
        "messaging_product": "whatsapp", "to": "15550000000", "type": "template",
        "template": { "name": "order_ready", "language": { "code": "en_US" }, "components": [
            { "type": "header", "parameters": [{ "type": "text", "text": "A-1001" }] },
            { "type": "body", "parameters": [{ "type": "text", "text": "Mary" }, { "type": "text", "text": "Main Street" }, { "type": "text", "text": "4821" }] },
            { "type": "button", "sub_type": "url", "index": "0", "parameters": [{ "type": "text", "text": "A-1001" }] },
        ] },
    });
    let response = reqwest::Client::new()
        .post(format!("{graph}/v23.0/100000000000002/messages"))
        .bearer_auth("wapit-example-token")
        .json(&payload)
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "{}",
        response.text().await.unwrap()
    );
}

#[tokio::test]
async fn template_fields_wapit_does_not_model_survive_a_rewrite() {
    let harness = harness().await;
    let file = harness.config.path().join("templates/a.json");
    let mut templates: Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    templates.as_array_mut().unwrap().push(json!({
        "name": "carousel_offer", "language": "en_US", "status": "APPROVED", "category": "MARKETING", "sub_category": "CUSTOM",
        "components": [
            { "type": "BODY", "text": "Offers" },
            { "type": "CAROUSEL", "cards": [{ "components": [{ "type": "HEADER", "format": "IMAGE" }] }] },
            { "type": "BUTTONS", "buttons": [{ "type": "OTP", "otp_type": "COPY_CODE" }] },
        ],
    }));
    tokio::time::sleep(Duration::from_millis(20)).await;
    std::fs::write(&file, templates.to_string()).unwrap();

    harness
        .post("/v23.0/200/message_templates", &json!({ "name": "another", "language": "en_US", "category": "UTILITY", "components": [{ "type": "BODY", "text": "Hi" }] }), "token-a")
        .await;
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    let carousel = saved
        .as_array()
        .unwrap()
        .iter()
        .find(|template| template["name"] == "carousel_offer")
        .unwrap();

    assert_eq!(carousel["sub_category"], "CUSTOM");
    assert_eq!(
        carousel["components"][1]["cards"][0]["components"][0]["format"],
        "IMAGE"
    );
    assert_eq!(
        carousel["components"][2]["buttons"][0]["otp_type"],
        "COPY_CODE"
    );
}

fn new_template(name: &str) -> Value {
    json!({ "name": name, "language": "en_US", "category": "UTILITY", "components": [{ "type": "BODY", "text": "Hi" }] })
}

fn saved_template_names(harness: &Harness) -> Vec<String> {
    let saved: Value = serde_json::from_str(
        &std::fs::read_to_string(harness.config.path().join("templates/a.json")).unwrap(),
    )
    .unwrap();

    saved
        .as_array()
        .unwrap()
        .iter()
        .map(|template| template["name"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn template_creates_with_a_broken_apps_file_keep_each_other() {
    let harness = harness().await;
    harness.ui_get("/api/state").await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    std::fs::write(harness.config.path().join("apps.json"), "{ \"apps\": [").unwrap();

    for name in ["first_new", "second_new"] {
        let (status, _) = harness
            .post(
                "/v23.0/200/message_templates",
                &new_template(name),
                "token-a",
            )
            .await;
        assert_eq!(status, StatusCode::OK);
    }

    assert_eq!(
        saved_template_names(&harness),
        ["order_ready", "reordered", "first_new", "second_new"]
    );

    let listed: Value = harness
        .client
        .get(format!("{}/v23.0/200/message_templates", harness.graph))
        .bearer_auth("token-a")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listed["data"].as_array().unwrap().len(), 4);
}

#[tokio::test]
async fn a_template_create_never_overwrites_a_broken_templates_file() {
    let harness = harness().await;
    let file = harness.config.path().join("templates/a.json");
    let half_done = "[{ \"name\": \"draft\", ";
    harness.ui_get("/api/state").await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    std::fs::write(&file, half_done).unwrap();

    let (status, body) = harness
        .post(
            "/v23.0/200/message_templates",
            &new_template("blocked"),
            "token-a",
        )
        .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(body["error"]["error_data"]["details"].is_string());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), half_done);
}

async fn next_change(events: &mut reqwest::Response) -> Value {
    let mut buffer = String::new();

    loop {
        let chunk = tokio::time::timeout(Duration::from_secs(3), events.chunk())
            .await
            .expect("a change event arrives")
            .unwrap()
            .expect("the event stream stays open");
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        if let Some(line) = buffer.lines().find_map(|line| line.strip_prefix("data:")) {
            return serde_json::from_str(line.trim()).unwrap();
        }
    }
}

#[tokio::test]
async fn template_writes_tell_the_inbox_to_refresh() {
    let harness = harness().await;
    let mut events = harness
        .client
        .get(format!("{}/api/events", harness.ui))
        .send()
        .await
        .unwrap();
    let (status, created) = harness
        .post(
            "/v23.0/200/message_templates",
            &new_template("announced"),
            "token-a",
        )
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        next_change(&mut events).await,
        json!({ "app_id": "a", "contact": null })
    );

    harness
        .post(
            &format!("/v23.0/{}", created["id"].as_str().unwrap()),
            &json!({ "components": [{ "type": "BODY", "text": "Hello" }] }),
            "token-a",
        )
        .await;
    assert_eq!(next_change(&mut events).await["app_id"], "a");

    harness
        .client
        .delete(format!(
            "{}/v23.0/200/message_templates?name=announced",
            harness.graph
        ))
        .bearer_auth("token-a")
        .send()
        .await
        .unwrap();
    assert_eq!(next_change(&mut events).await["app_id"], "a");
}

#[tokio::test]
async fn concurrent_template_creates_all_land() {
    let harness = Arc::new(harness().await);
    let creates = (0..15).map(|index| {
        let harness = Arc::clone(&harness);
        tokio::spawn(async move {
            harness
                .post(
                    "/v23.0/200/message_templates",
                    &json!({ "name": format!("parallel_{index}"), "language": "en_US", "category": "UTILITY", "components": [{ "type": "BODY", "text": "Hi" }] }),
                    "token-a",
                )
                .await
                .0
        })
    });

    for create in creates.collect::<Vec<_>>() {
        assert_eq!(create.await.unwrap(), StatusCode::OK);
    }

    let listed: Value = harness
        .client
        .get(format!("{}/v23.0/200/message_templates", harness.graph))
        .bearer_auth("token-a")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let parallel = listed["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|template| template["name"].as_str().unwrap().starts_with("parallel_"))
        .count();

    assert_eq!(parallel, 15);
}

#[tokio::test]
async fn a_web_page_cannot_post_to_the_graph_api() {
    let harness = harness().await;
    let response = harness
        .client
        .post(format!(
            "{}/v23.0/100/messages?access_token=token-a",
            harness.graph
        ))
        .header("Origin", "https://attacker.example")
        .header("Content-Type", "text/plain")
        .body(valid_template().to_string())
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        harness.ui_get("/api/conversations/a/15550000000").await["messages"],
        json!([])
    );
}

#[tokio::test]
async fn inbox_errors_carry_a_code_the_ui_translates() {
    let harness = harness().await;
    let missing_app = harness
        .ui_post(
            "/api/conversations/nope/15550000000/messages",
            &json!({ "text": "Hi" }),
        )
        .await;
    let bad_body = harness
        .ui_post("/api/messages/x/tap", &json!({ "index": "first" }))
        .await;

    assert_eq!(
        missing_app.json::<Value>().await.unwrap()["code"],
        "app_not_found"
    );
    let bad_body: Value = bad_body.json().await.unwrap();
    assert_eq!(bad_body["code"], "invalid_body");
}

#[tokio::test]
async fn an_uploaded_file_downloads_as_a_sandboxed_attachment() {
    let harness = harness().await;
    let form = reqwest::multipart::Form::new()
        .text("messaging_product", "whatsapp")
        .text("type", "text/html")
        .part(
            "file",
            reqwest::multipart::Part::bytes(b"<script>alert(1)</script>".to_vec()),
        );
    let upload: Value = harness
        .client
        .post(format!("{}/v23.0/100/media", harness.graph))
        .bearer_auth("token-a")
        .multipart(form)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let meta: Value = harness
        .client
        .get(format!(
            "{}/v23.0/{}",
            harness.graph,
            upload["id"].as_str().unwrap()
        ))
        .bearer_auth("token-a")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let download = harness
        .client
        .get(meta["url"].as_str().unwrap())
        .bearer_auth("token-a")
        .send()
        .await
        .unwrap();

    assert!(
        download.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .starts_with("attachment")
    );
    assert!(
        download.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("sandbox")
    );
}

#[tokio::test]
async fn clear_all_removes_uploaded_media_no_message_uses() {
    let harness = harness().await;
    let form = reqwest::multipart::Form::new()
        .text("messaging_product", "whatsapp")
        .part(
            "file",
            reqwest::multipart::Part::bytes(b"png".to_vec())
                .mime_str("image/png")
                .unwrap(),
        );
    let upload: Value = harness
        .client
        .post(format!("{}/v23.0/100/media", harness.graph))
        .bearer_auth("token-a")
        .multipart(form)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let media = format!("{}/v23.0/{}", harness.graph, upload["id"].as_str().unwrap());

    harness
        .client
        .delete(format!("{}/api/messages", harness.ui))
        .send()
        .await
        .unwrap();

    assert_eq!(
        harness
            .client
            .get(&media)
            .bearer_auth("token-a")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn a_database_from_the_node_release_is_adopted() {
    let data = TempDir::new().unwrap();
    let media_file = data.path().join("media").join("1234567890123456");
    std::fs::create_dir_all(media_file.parent().unwrap()).unwrap();
    std::fs::write(&media_file, b"old-png").unwrap();

    let db = rusqlite::Connection::open(data.path().join("wapit.sqlite")).unwrap();
    db.execute_batch(&format!(
        r#"
        CREATE TABLE messages (id TEXT PRIMARY KEY, app_id TEXT NOT NULL, contact TEXT NOT NULL, direction TEXT NOT NULL, type TEXT NOT NULL,
            status TEXT NOT NULL, payload TEXT NOT NULL, rendered TEXT NOT NULL, issues TEXT NOT NULL DEFAULT '[]', error TEXT, context_id TEXT, created_at INTEGER NOT NULL);
        CREATE TABLE statuses (id INTEGER PRIMARY KEY AUTOINCREMENT, message_id TEXT NOT NULL, status TEXT NOT NULL, error TEXT, created_at INTEGER NOT NULL);
        CREATE TABLE contacts (app_id TEXT NOT NULL, wa_id TEXT NOT NULL, name TEXT, PRIMARY KEY (app_id, wa_id));
        CREATE TABLE media (id TEXT PRIMARY KEY, app_id TEXT NOT NULL, mime_type TEXT NOT NULL, filename TEXT, sha256 TEXT NOT NULL, file_size INTEGER NOT NULL, path TEXT NOT NULL, created_at INTEGER NOT NULL);
        CREATE TABLE webhooks (id INTEGER PRIMARY KEY AUTOINCREMENT, app_id TEXT NOT NULL, url TEXT NOT NULL, event TEXT NOT NULL, message_id TEXT, body TEXT NOT NULL,
            status_code INTEGER, response TEXT, error TEXT, duration_ms INTEGER NOT NULL, created_at INTEGER NOT NULL);
        INSERT INTO messages VALUES ('wamid.out', 'a', '15550000000', 'out', 'text', 'failed', '{{}}', '{{"kind":"text","body":"Hi"}}',
            '["Fora da janela de 24h: a Meta aceita o envio e depois manda o status failed com o erro 131047, como o wapit fez."]', NULL, NULL, 1);
        INSERT INTO messages VALUES ('wamid.in', 'a', '15550000000', 'in', 'image', 'received', '{{"type":"image","image":{{"id":"1234567890123456"}}}}',
            '{{"kind":"image","media":{{"id":"1234567890123456"}}}}', '[]', NULL, NULL, 2);
        INSERT INTO media VALUES ('1234567890123456', 'a', 'image/png', NULL, 'x', 7, '{}', 2);
        "#,
        media_file.display()
    ))
    .unwrap();
    drop(db);

    let harness = harness().await;
    let servers = build(&Settings {
        config_dir: harness.config.path().to_owned(),
        data_dir: data.path().to_owned(),
        public_api_url: "http://127.0.0.1:8027".into(),
        public_ui_url: String::new(),
        loopback_host: None,
        extra_ca_file: None,
        allowed_hosts: Vec::new(),
        auto_status_delay: Duration::from_secs(60),
    })
    .unwrap();
    let ui = serve(servers.ui).await;
    let conversation: Value = harness
        .client
        .get(format!("{ui}/api/conversations/a/15550000000"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(
        conversation["messages"][0]["issues"],
        json!([{ "code": "outside_service_window" }])
    );
    assert_eq!(
        harness
            .client
            .get(format!("{ui}/media/1234567890123456"))
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap()
            .as_ref(),
        b"old-png"
    );

    harness
        .client
        .delete(format!("{ui}/api/conversations/a/15550000000"))
        .send()
        .await
        .unwrap();
    assert!(!media_file.exists());
}
