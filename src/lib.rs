pub mod config;
pub mod graph_error;
pub mod http;
pub mod inbox;
pub mod model;
pub mod store;
pub mod sync;
pub mod templates;
pub mod validate;
pub mod webhooks;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;

use crate::config::ConfigStore;
use crate::http::{AllowedHosts, graph, guard_host, host_of, ui};
use crate::inbox::Inbox;
use crate::store::Store;
use crate::webhooks::WebhookSender;

pub struct Settings {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub public_api_url: String,
    pub public_ui_url: String,
    pub loopback_host: Option<String>,
    pub extra_ca_file: Option<String>,
    pub allowed_hosts: Vec<String>,
    pub auto_status_delay: Duration,
}

pub struct Servers {
    pub inbox: Arc<Inbox>,
    pub graph: Router,
    pub ui: Router,
}

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error(transparent)]
    Store(#[from] store::StoreError),
    #[error(transparent)]
    Client(#[from] webhooks::ClientError),
}

/// Uses `ring` for TLS, which builds on every target without a C toolchain. Safe to call more than once.
pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

pub fn build(settings: &Settings) -> Result<Servers, StartError> {
    install_crypto_provider();

    let store = Arc::new(Store::open(&settings.data_dir)?);
    let webhooks = WebhookSender::new(
        Arc::clone(&store),
        settings.loopback_host.as_deref(),
        settings.extra_ca_file.as_deref(),
    )?;
    let inbox = Arc::new(Inbox::new(
        ConfigStore::new(&settings.config_dir),
        store,
        webhooks,
        settings.auto_status_delay,
    ));

    let hosts = AllowedHosts::new(
        [&settings.public_api_url, &settings.public_ui_url]
            .into_iter()
            .filter_map(|url| host_of(url))
            .chain(settings.allowed_hosts.iter().cloned()),
    );

    let graph = graph::router(graph::GraphState {
        inbox: Arc::clone(&inbox),
        public_api_url: settings.public_api_url.clone(),
        public_ui_url: settings.public_ui_url.clone(),
    })
    .layer(axum::middleware::from_fn_with_state(
        hosts.clone(),
        guard_host,
    ));

    let ui = ui::router(ui::UiState {
        inbox: Arc::clone(&inbox),
        public_api_url: settings.public_api_url.clone(),
    })
    .layer(axum::middleware::from_fn_with_state(hosts, guard_host));

    Ok(Servers { inbox, graph, ui })
}
