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

    while let Some(url) = next.take() {
        let response = client
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|error| format!("Meta did not answer: {error}"))?;
        let status = response.status();
        let page: Page = response
            .json()
            .await
            .map_err(|error| format!("Meta returned an unreadable page: {error}"))?;

        if !status.is_success() {
            let message = page
                .error
                .and_then(|error| error.message)
                .unwrap_or_else(|| status.to_string());
            return Err(format!("Meta refused to list the templates: {message}"));
        }

        templates.extend(page.data);
        next = page.paging.and_then(|paging| paging.next);
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
    use super::first_page;

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
}
