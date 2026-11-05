pub mod graph;
pub mod ui;

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};

/// Hosts the two servers answer to. Anything else is refused, so a page that rebinds its domain to 127.0.0.1
/// cannot read the inbox or post signed webhooks into the developer's apps.
#[derive(Clone, Debug)]
pub struct AllowedHosts(Arc<Vec<String>>);

impl AllowedHosts {
    pub fn new(extra: impl IntoIterator<Item = String>) -> Self {
        let mut hosts: Vec<String> = [
            "127.0.0.1",
            "localhost",
            "[::1]",
            "::1",
            "host.docker.internal",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        hosts.extend(
            extra
                .into_iter()
                .map(|host| host.to_lowercase())
                .filter(|host| !host.is_empty()),
        );

        Self(Arc::new(hosts))
    }

    fn allows(&self, authority: &str) -> bool {
        let host = strip_port(&authority.to_lowercase());

        host.ends_with(".localhost") || self.0.contains(&host)
    }
}

fn strip_port(authority: &str) -> String {
    if let Some(rest) = authority.strip_prefix('[') {
        return rest
            .split(']')
            .next()
            .map(|host| format!("[{host}]"))
            .unwrap_or_default();
    }

    authority
        .rsplit_once(':')
        .filter(|(_, port)| port.chars().all(|char| char.is_ascii_digit()))
        .map_or_else(|| authority.to_owned(), |(host, _)| host.to_owned())
}

pub fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next()?;

    Some(strip_port(authority)).filter(|host| !host.is_empty())
}

pub async fn guard_host(
    State(hosts): State<AllowedHosts>,
    request: Request,
    next: Next,
) -> Response {
    let authority = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);

    match authority {
        Some(authority) if !hosts.allows(&authority) => (
            StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({ "error": format!("Host {authority} is not allowed. Add it to WAPIT_ALLOWED_HOSTS to reach wapit through it.") })),
        )
            .into_response(),
        _ => next.run(request).await,
    }
}

pub fn content_disposition(kind: &str, filename: &str) -> HeaderValue {
    let clean: String = filename
        .chars()
        .filter(|char| !matches!(char, '\r' | '\n' | '"'))
        .collect();
    let ascii: String = clean
        .chars()
        .map(|char| {
            if (' '..='~').contains(&char) {
                char
            } else {
                '_'
            }
        })
        .collect();
    let encoded = utf8_percent_encode(&clean, NON_ALPHANUMERIC);

    HeaderValue::from_str(&format!(
        "{kind}; filename=\"{ascii}\"; filename*=UTF-8''{encoded}"
    ))
    .unwrap_or_else(|_| HeaderValue::from_static("attachment"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_loopback_hosts_with_ports() {
        let hosts = AllowedHosts::new(["wapit.example".to_owned()]);

        assert!(hosts.allows("127.0.0.1:8026"));
        assert!(hosts.allows("localhost"));
        assert!(hosts.allows("[::1]:8027"));
        assert!(hosts.allows("inbox.localhost:8026"));
        assert!(hosts.allows("WAPIT.example:443"));
        assert!(!hosts.allows("attacker.example:8026"));
        assert!(!hosts.allows("127.0.0.1.attacker.example"));
    }

    #[test]
    fn reads_the_host_of_a_url() {
        assert_eq!(
            host_of("http://127.0.0.1:8027").as_deref(),
            Some("127.0.0.1")
        );
        assert_eq!(
            host_of("https://inbox.example/path").as_deref(),
            Some("inbox.example")
        );
        assert_eq!(host_of("http://[::1]:8026/").as_deref(), Some("[::1]"));
    }
}
