use std::fmt::Write;

use serde_json::Value;

const SUMMARY_WIDTH: usize = 60;

/// The text `wapit messages` prints: one block per message, newest first, from `GET /api/messages`.
pub fn messages(response: &Value, now_ms: i64, flagged_only: bool) -> String {
    let messages = response["messages"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);

    if messages.is_empty() {
        return if flagged_only {
            "No rejected or failed messages.\n".to_owned()
        } else {
            "No messages.\n".to_owned()
        };
    }

    let mut out = String::new();

    for message in messages {
        let _ = writeln!(
            out,
            "{:<8}  {}  {}  {}  {}  {}",
            ago(now_ms, message["created_at"].as_i64().unwrap_or(now_ms)),
            text(&message["app_id"]),
            text(&message["contact"]),
            text(&message["direction"]),
            summary(message),
            outcome(message),
        );

        if let Some(reason) = reason(message) {
            let _ = writeln!(out, "{:<8}  {reason}", "");
        }
        for issue in issues(message) {
            let _ = writeln!(out, "{:<8}  issue: {issue}", "");
        }
        let _ = writeln!(out, "{:<8}  {}", "", text(&message["id"]));
    }

    out
}

/// The text `wapit show` prints for one message from `GET /api/messages/{id}`.
pub fn message(message: &Value, now_ms: i64) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{}  {} {}  {} {} {}  {}",
        outcome(message),
        text(&message["direction"]),
        summary(message),
        text(&message["app_id"]),
        if message["direction"] == "in" {
            "<-"
        } else {
            "->"
        },
        text(&message["contact"]),
        ago(now_ms, message["created_at"].as_i64().unwrap_or(now_ms)),
    );
    let _ = writeln!(out, "id {}", text(&message["id"]));

    if let Some(error) = failure(message) {
        let _ = writeln!(
            out,
            "error {}: {}",
            error["code"].as_i64().unwrap_or_default(),
            text(&error["message"])
        );
        if let Some(details) = details(error) {
            let _ = writeln!(out, "  {details}");
        }
    }
    for issue in issues(message) {
        let _ = writeln!(out, "issue: {issue}");
    }

    let payload = serde_json::to_string_pretty(&message["payload"]).unwrap_or_default();
    let _ = write!(out, "\nRequest\n{payload}\n");

    let statuses = message["statuses"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    let _ = writeln!(out, "\nStatuses");
    if statuses.is_empty() {
        let _ = writeln!(out, "  none");
    }
    for status in statuses {
        let mut line = format!(
            "  {:<9} {}",
            text(&status["status"]),
            ago(now_ms, status["created_at"].as_i64().unwrap_or(now_ms))
        );
        if let Some(code) = status["error"]["code"].as_i64() {
            let _ = write!(
                line,
                "  error {code}: {}",
                text(&status["error"]["message"])
            );
        }
        let _ = writeln!(out, "{line}");
        if let Some(details) = details(&status["error"]) {
            let _ = writeln!(out, "            {details}");
        }
    }

    let webhooks = message["webhooks"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    let _ = writeln!(out, "\nWebhooks");
    if webhooks.is_empty() {
        let _ = writeln!(out, "  none");
    }
    for webhook in webhooks {
        let result = match (webhook["status_code"].as_i64(), webhook["error"].as_str()) {
            (Some(code), _) => format!("{code} in {} ms", webhook["duration_ms"]),
            (None, Some(error)) => format!("not delivered: {}", clean(error)),
            (None, None) => "pending".to_owned(),
        };
        let _ = writeln!(
            out,
            "  #{}  {}  {result}  {}",
            webhook["id"],
            text(&webhook["event"]),
            text(&webhook["url"])
        );
        if let Some(response) = webhook["response"]
            .as_str()
            .map(str::trim)
            .filter(|response| !response.is_empty())
        {
            let _ = writeln!(
                out,
                "       response: {}",
                shorten(&clean(response.lines().next().unwrap_or_default()))
            );
        }
    }

    out
}

fn summary(message: &Value) -> String {
    let rendered = &message["rendered"];
    let kind = text(&message["type"]);
    let detail = rendered["template"]["name"]
        .as_str()
        .or_else(|| rendered["body"].as_str())
        .or_else(|| rendered["caption"].as_str())
        .map(|value| clean(value.lines().next().unwrap_or_default()))
        .unwrap_or_default();

    if detail.is_empty() {
        return kind;
    }

    format!("{kind} {}", shorten(&detail))
}

fn shorten(value: &str) -> String {
    if value.chars().count() <= SUMMARY_WIDTH {
        return value.to_owned();
    }

    let mut short: String = value.chars().take(SUMMARY_WIDTH).collect();
    short.push('…');
    short
}

fn outcome(message: &Value) -> String {
    let status = text(&message["status"]);

    match failure(message).and_then(|error| error["code"].as_i64()) {
        Some(code) => format!("{status} {code}"),
        None => status,
    }
}

/// Why a message failed: the error a rejected send was answered with, or else the error of its latest failed
/// status, such as 131047 when the service window was closed.
fn failure(message: &Value) -> Option<&Value> {
    if message["error"].is_object() {
        return Some(&message["error"]);
    }
    if message["status"] != "failed" {
        return None;
    }

    message["statuses"]
        .as_array()?
        .iter()
        .rev()
        .map(|status| &status["error"])
        .find(|error| error.is_object())
}

fn details(error: &Value) -> Option<String> {
    error["details"]
        .as_str()
        .or_else(|| error["error_data"]["details"].as_str())
        .map(clean)
}

fn reason(message: &Value) -> Option<String> {
    let error = failure(message)?;

    details(error).or_else(|| error["message"].as_str().map(clean))
}

fn issues(message: &Value) -> Vec<String> {
    message["issues"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .map(|issue| match issue["text"].as_str() {
            Some(legacy) => clean(legacy),
            None => text(&issue["code"]),
        })
        .collect()
}

fn ago(now_ms: i64, then_ms: i64) -> String {
    let seconds = (now_ms - then_ms).max(0) / 1000;

    match seconds {
        0..60 => "just now".to_owned(),
        60..3600 => format!("{}m ago", seconds / 60),
        3600..86_400 => format!("{}h ago", seconds / 3600),
        _ => format!("{}d ago", seconds / 86_400),
    }
}

fn text(value: &Value) -> String {
    match value {
        Value::String(value) => clean(value),
        Value::Null => String::new(),
        other => clean(&other.to_string()),
    }
}

/// Message text comes from apps and users, so control characters are escaped before they reach a terminal.
fn clean(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                character.escape_default().to_string()
            } else {
                character.to_string()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{message, messages};

    const NOW: i64 = 1_800_000_000_000;

    fn rejected() -> serde_json::Value {
        json!({
            "id": "rejected.fa5c",
            "app_id": "meupac",
            "contact": "5585987323267",
            "direction": "out",
            "type": "template",
            "status": "rejected",
            "payload": { "to": "5585987323267", "type": "template" },
            "rendered": { "kind": "template", "template": { "name": "resident_login_code", "language": "pt_BR" } },
            "issues": [],
            "error": {
                "code": 132_001,
                "message": "(#132001) Template name does not exist in the translation",
                "details": "template name (resident_login_code) does not exist in pt_BR"
            },
            "created_at": NOW - 180_000,
            "statuses": [],
            "webhooks": []
        })
    }

    #[test]
    fn a_rejected_send_shows_its_code_reason_and_id() {
        let out = messages(&json!({ "messages": [rejected()] }), NOW, true);

        assert_eq!(
            out,
            "3m ago    meupac  5585987323267  out  template resident_login_code  rejected 132001\n\
             \x20         template name (resident_login_code) does not exist in pt_BR\n\
             \x20         rejected.fa5c\n"
        );
    }

    #[test]
    fn an_empty_list_says_so() {
        assert_eq!(
            messages(&json!({ "messages": [] }), NOW, true),
            "No rejected or failed messages.\n"
        );
        assert_eq!(
            messages(&json!({ "messages": [] }), NOW, false),
            "No messages.\n"
        );
    }

    #[test]
    fn show_prints_the_error_request_statuses_and_webhooks() {
        let mut sent = rejected();
        sent["status"] = json!("delivered");
        sent["error"] = json!(null);
        sent["statuses"] = json!([{ "status": "sent", "created_at": NOW - 60_000, "error": null }]);
        sent["webhooks"] = json!([
            { "id": 7, "event": "statuses", "url": "http://localhost:8000/webhook", "status_code": 200, "duration_ms": 38, "error": null, "response": "{\"ok\":true}" },
            { "id": 8, "event": "statuses", "url": "http://localhost:8000/webhook", "status_code": null, "duration_ms": 3, "error": "connection refused" }
        ]);

        let out = message(&sent, NOW);

        assert!(out.starts_with("delivered  out template resident_login_code  meupac -> 5585987323267  3m ago\nid rejected.fa5c\n"));
        assert!(out.contains("\nRequest\n{\n  \"to\": \"5585987323267\""));
        assert!(out.contains("\nStatuses\n  sent      1m ago\n"));
        assert!(out.contains("  #7  statuses  200 in 38 ms  http://localhost:8000/webhook\n       response: {\"ok\":true}\n"));
        assert!(out.contains(
            "  #8  statuses  not delivered: connection refused  http://localhost:8000/webhook\n"
        ));
    }

    #[test]
    fn a_send_that_failed_later_shows_the_status_error() {
        let mut failed = rejected();
        failed["status"] = json!("failed");
        failed["error"] = json!(null);
        failed["statuses"] = json!([{
            "status": "failed",
            "created_at": NOW - 60_000,
            "error": { "code": 131_047, "message": "Re-engagement message", "error_data": { "details": "More than 24 hours have passed" } }
        }]);

        let list = messages(&json!({ "messages": [failed.clone()] }), NOW, true);
        let shown = message(&failed, NOW);

        assert!(list.contains("rejected.fa5c"));
        assert!(list.contains("  failed 131047\n"));
        assert!(list.contains("          More than 24 hours have passed\n"));
        assert!(
            shown.contains(
                "error 131047: Re-engagement message\n  More than 24 hours have passed\n"
            )
        );
        assert!(shown.contains("  failed    1m ago  error 131047: Re-engagement message\n            More than 24 hours have passed\n"));
    }

    #[test]
    fn terminal_control_characters_from_messages_are_escaped() {
        let mut inbound = rejected();
        inbound["error"] = json!(null);
        inbound["rendered"] = json!({ "kind": "text", "body": "hi\u{1b}[31m there" });

        let out = messages(&json!({ "messages": [inbound] }), NOW, false);

        assert!(out.contains("template hi\\u{1b}[31m there"));
        assert!(!out.contains('\u{1b}'));
    }
}
