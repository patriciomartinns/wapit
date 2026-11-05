use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;
use wapit::config::ConfigStore;
use wapit::{Settings, build, sync};

#[derive(Parser)]
#[command(
    version,
    about = "A local WhatsApp Cloud API mock and inbox for development"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    #[arg(
        long,
        env = "WAPIT_CONFIG_DIR",
        default_value = "config",
        global = true
    )]
    config_dir: PathBuf,

    #[command(flatten)]
    serve: Serve,
}

#[derive(Subcommand)]
enum Command {
    /// Run the Graph API mock and the inbox (the default)
    Serve,
    /// Copy an app's templates from its real `WhatsApp` Business Account (read-only)
    Sync { app: String },
    /// Exit successfully when the Graph API mock answers on its port
    Healthcheck,
}

#[derive(clap::Args)]
struct Serve {
    #[arg(long, env = "WAPIT_DATA_DIR", default_value = "data")]
    data_dir: PathBuf,
    #[arg(long, env = "WAPIT_HOST", default_value = "127.0.0.1")]
    host: String,
    #[arg(long, env = "WAPIT_API_PORT", default_value_t = 8027)]
    api_port: u16,
    #[arg(long, env = "WAPIT_UI_PORT", default_value_t = 8026)]
    ui_port: u16,
    /// Base URL apps use to reach the API, used in media download links
    #[arg(long, env = "WAPIT_PUBLIC_API_URL")]
    public_api_url: Option<String>,
    /// Inbox URL a browser opening the API root is redirected to
    #[arg(long, env = "WAPIT_PUBLIC_UI_URL")]
    public_ui_url: Option<String>,
    /// Host to send webhooks to when an app's webhook host resolves to loopback, e.g. host.docker.internal
    #[arg(long, env = "WAPIT_LOOPBACK_HOST")]
    loopback_host: Option<String>,
    /// PEM bundle of extra certificate authorities trusted for webhook delivery, e.g. a local dev CA
    #[arg(long, env = "WAPIT_CA_CERTS")]
    ca_certs: Option<String>,
    /// Extra host names the servers answer to, comma separated
    #[arg(long, env = "WAPIT_ALLOWED_HOSTS", value_delimiter = ',')]
    allowed_hosts: Vec<String>,
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_env("WAPIT_LOG").unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    wapit::install_crypto_provider();

    let result = match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => serve_forever(cli.config_dir, cli.serve).await,
        Command::Sync { app } => sync::run(&ConfigStore::new(&cli.config_dir), &app)
            .await
            .map(|summary| println!("{summary}")),
        Command::Healthcheck => healthcheck(cli.serve.api_port).await,
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

async fn serve_forever(config_dir: PathBuf, serve: Serve) -> Result<(), String> {
    let public_api_url = trim_url(
        &serve
            .public_api_url
            .unwrap_or_else(|| format!("http://127.0.0.1:{}", serve.api_port)),
    );
    let public_ui_url = trim_url(
        &serve
            .public_ui_url
            .unwrap_or_else(|| format!("http://127.0.0.1:{}", serve.ui_port)),
    );
    let settings = Settings {
        config_dir,
        data_dir: serve.data_dir,
        public_api_url: public_api_url.clone(),
        public_ui_url,
        loopback_host: serve.loopback_host,
        // Containers set up for the first, Node-based release pass the dev CA as NODE_EXTRA_CA_CERTS.
        extra_ca_file: serve.ca_certs.or_else(|| {
            std::env::var("NODE_EXTRA_CA_CERTS")
                .ok()
                .filter(|path| !path.is_empty())
        }),
        allowed_hosts: serve.allowed_hosts,
        auto_status_delay: Duration::from_millis(1500),
    };
    let servers = build(&settings).map_err(|error| error.to_string())?;

    match servers.inbox.startup_report() {
        Err(error) => tracing::error!("{error}"),
        Ok(apps) => {
            for (app, templates) in apps {
                tracing::info!(
                    "app {}: phone_number_id {}, {templates} templates, webhook {}",
                    app.id,
                    app.phone_number_id,
                    app.webhook_url
                );
            }
        }
    }

    let graph = bind(&serve.host, serve.api_port).await?;
    let ui = bind(&serve.host, serve.ui_port).await?;
    tracing::info!(
        "Graph API mock on {} (public {public_api_url})",
        local(&graph)
    );
    tracing::info!("Inbox on http://{}", local(&ui));

    let inbox = std::sync::Arc::clone(&servers.inbox);
    tokio::spawn(async move {
        shutdown().await;
        inbox.stop();
    });

    let graph = axum::serve(graph, servers.graph).with_graceful_shutdown(servers.inbox.stopped());
    let ui = axum::serve(ui, servers.ui).with_graceful_shutdown(servers.inbox.stopped());

    tokio::try_join!(async { graph.await }, async { ui.await })
        .map_err(|error| error.to_string())?;

    Ok(())
}

fn trim_url(url: &str) -> String {
    url.trim_end_matches('/').to_owned()
}

async fn bind(host: &str, port: u16) -> Result<TcpListener, String> {
    TcpListener::bind((host, port))
        .await
        .map_err(|error| format!("cannot listen on {host}:{port}: {error}"))
}

fn local(listener: &TcpListener) -> String {
    listener
        .local_addr()
        .map_or_else(|_| "?".into(), |address: SocketAddr| address.to_string())
}

async fn shutdown() {
    let interrupt = tokio::signal::ctrl_c();

    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("signal handler");
        tokio::select! {
            _ = interrupt => {}
            _ = terminate.recv() => {}
        }
    }

    #[cfg(not(unix))]
    let _ = interrupt.await;
}

async fn healthcheck(api_port: u16) -> Result<(), String> {
    let url = format!("http://127.0.0.1:{api_port}/health");
    let response = reqwest::Client::new()
        .get(&url)
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("{url} answered {}", response.status()))
    }
}
