use std::time::Duration;

use serde::Deserialize;

use crate::config::{ConfigStore, Template};
use crate::templates;

const REAL_GRAPH: &str = "https://graph.facebook.com";
const PLACEHOLDER_WABA: &str = "REAL_WABA_ID";

#[derive(Deserialize)]
struct Page {
    #[serde(default)]
    data: Vec<Template>,
    #[serde(default)]
    paging: Option<Paging>,
    #[serde(default)]
    error: Option<MetaError>,
}

#[derive(Deserialize)]
struct Paging {
    next: Option<String>,
    cursors: Option<Cursors>,
}

#[derive(Deserialize)]
struct Cursors {
    after: Option<String>,
}

impl Paging {
    /// Meta's `paging.next` URL can carry the access token in its query string, so wapit never requests it or
    /// prints it. It only reads the `after` cursor and builds the next page on the real Graph host itself.
    fn after(self) -> Result<Option<String>, String> {
        if self.next.is_none() {
            return Ok(None);
        }
        self.cursors
            .and_then(|cursors| cursors.after)
            .filter(|after| !after.is_empty())
            .map(Some)
            .ok_or_else(|| {
                "Meta sent a next page without an after cursor; the local file was kept".to_string()
            })
    }
}

#[derive(Deserialize)]
struct MetaError {
    message: Option<String>,
}

/// Fields of Meta's message template node that wapit keeps. `id` matters most: with it, `POST /{template_id}` and
/// `hsm_id` deletes use the same ids as the real account. Files synced without it still get a stable local id.
const FIELDS: &str = "id,name,language,status,category,sub_category,previous_category,parameter_format,components,quality_score,rejected_reason";

fn first_page(version: &str, waba_id: &str) -> String {
    format!("{REAL_GRAPH}/{version}/{waba_id}/message_templates?fields={FIELDS}&limit=100")
}

fn page_after(version: &str, waba_id: &str, after: &str) -> String {
    let after = percent_encoding::utf8_percent_encode(after, percent_encoding::NON_ALPHANUMERIC);
    format!("{}&after={after}", first_page(version, waba_id))
}

/// Copies the app's templates from its real `WhatsApp` Business Account into `templates/<app>.json`.
/// It only reads from Meta, and only when the developer runs `wapit sync`.
pub async fn run(config: &ConfigStore, app_id: &str) -> Result<String, String> {
    if let Some(error) = config.error() {
        return Err(error);
    }

    let app = config.app(app_id).ok_or_else(|| {
        format!(
            "App {app_id} is not in {}/apps.json",
            config.dir().display()
        )
    })?;
    let sync = app
        .sync
        .as_ref()
        .filter(|sync| !sync.waba_id.is_empty() && sync.waba_id != PLACEHOLDER_WABA)
        .ok_or_else(|| {
            format!("App {app_id} needs sync.waba_id in apps.json, set to the real WABA id")
        })?;
    let token_env = sync.token_env.as_deref().unwrap_or("WAPIT_SYNC_TOKEN");
    let token = std::env::var(token_env)
        .ok()
        .filter(|token| !token.is_empty())
        .ok_or_else(|| format!("Set {token_env} to a token that can read the real WABA"))?;
    let version = sync.graph_version.as_deref().unwrap_or("v26.0");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| error.to_string())?;
    let mut next = Some(first_page(version, &sync.waba_id));
    let mut templates: Vec<Template> = Vec::new();
    let mut cursors = std::collections::HashSet::new();

    while let Some(url) = next.take() {
        let response = client
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|error| format!("Meta did not answer: {}", error.without_url()))?;
        let status = response.status();
        let body = response
            .bytes()
            .await
            .map_err(|error| format!("Meta did not answer: {}", error.without_url()))?;

        if !status.is_success() {
            let message = serde_json::from_slice::<Page>(&body)
                .ok()
                .and_then(|page| page.error)
                .and_then(|error| error.message)
                .unwrap_or_else(|| status.to_string());
            return Err(format!("Meta refused to list the templates: {message}"));
        }

        let page: Page = serde_json::from_slice(&body)
            .map_err(|error| format!("Meta returned an unreadable page: {error}"))?;
        templates.extend(page.data);

        if let Some(after) = page.paging.map(Paging::after).transpose()?.flatten() {
            if !cursors.insert(after.clone()) {
                return Err(
                    "Meta sent the same page cursor twice; the local file was kept".to_string(),
                );
            }
            next = Some(page_after(version, &sync.waba_id, &after));
        }
    }

    if templates.is_empty() {
        return Err(format!(
            "Meta returned no templates for WABA {}; the local file was kept",
            sync.waba_id
        ));
    }

    let file = config.templates_file(&app.id);

    if file.exists() {
        std::fs::copy(&file, file.with_extension("json.bak")).map_err(|error| error.to_string())?;
    }

    let count = templates.len();
    let approved_count = templates
        .iter()
        .filter(|template| template.status == "APPROVED")
        .count();
    templates::replace_all(config, &app.id, templates)
        .map_err(|error| error.details.clone().unwrap_or_else(|| error.to_string()))?;

    Ok(format!(
        "{count} templates ({approved_count} approved) written to {}",
        file.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::{Paging, first_page, page_after};

    #[test]
    fn sync_asks_meta_for_the_template_ids() {
        let url = first_page("v26.0", "123");
        let fields = url
            .split_once("fields=")
            .and_then(|(_, rest)| rest.split('&').next())
            .unwrap();

        assert!(url.starts_with("https://graph.facebook.com/v26.0/123/message_templates?"));
        assert!(fields.split(',').any(|field| field == "id"));
        assert!(fields.split(',').any(|field| field == "sub_category"));
    }

    #[test]
    fn sync_builds_the_next_page_from_the_cursor_instead_of_meta_s_url() {
        let paging: Paging = serde_json::from_value(serde_json::json!({
            "cursors": { "before": "b", "after": "QVFI+x/y=" },
            "next": "https://graph.facebook.com/v26.0/123/message_templates?access_token=SECRET&after=QVFI"
        }))
        .unwrap();
        let url = page_after("v26.0", "123", &paging.after().unwrap().unwrap());

        assert!(url.starts_with("https://graph.facebook.com/v26.0/123/message_templates?"));
        assert!(url.ends_with("&after=QVFI%2Bx%2Fy%3D"));
        assert!(!url.contains("SECRET"));
    }

    #[test]
    fn sync_stops_when_meta_sends_no_next_page() {
        let last: Paging = serde_json::from_value(serde_json::json!({
            "cursors": { "before": "b", "after": "a" }
        }))
        .unwrap();

        assert_eq!(last.after(), Ok(None));
    }

    #[test]
    fn sync_refuses_a_next_page_it_cannot_follow_without_meta_s_url() {
        for paging in [
            serde_json::json!({ "next": "https://graph.facebook.com/next" }),
            serde_json::json!({ "next": "https://graph.facebook.com/next", "cursors": { "after": "" } }),
        ] {
            let paging: Paging = serde_json::from_value(paging).unwrap();

            assert!(paging.after().is_err());
        }
    }
}
