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
    /// List the messages a running wapit recorded, newest first
    Messages {
        /// Only this app's messages
        #[arg(long)]
        app: Option<String>,
        /// Only messages with this phone number or BSUID
        #[arg(long, value_parser = clap::builder::NonEmptyStringValueParser::new())]
        contact: Option<String>,
        /// Only rejected or failed sends and messages with an issue
        #[arg(long)]
        failed: bool,
        /// How many messages to print, up to 500
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u16).range(1..=500))]
        limit: u16,
        #[command(flatten)]
        read: Read,
    },
    /// Show one message with its request, error, statuses and webhooks
    Show {
        /// The message id, as `wapit messages` prints it
        #[arg(value_parser = clap::builder::NonEmptyStringValueParser::new())]
        id: String,
        #[command(flatten)]
        read: Read,
    },
}

#[derive(clap::Args)]
struct Read {
    /// Print the JSON the inbox API returns instead of text
    #[arg(long)]
    json: bool,
    /// The running inbox to read; defaults to the inbox port on 127.0.0.1
    #[arg(long, env = "WAPIT_INBOX_URL")]
    inbox_url: Option<String>,
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
        Command::Messages {
            app,
            contact,
            failed,
            limit,
            read,
        } => {
            let mut query = vec![("limit", limit.to_string())];
            query.extend(app.map(|app| ("app", app)));
            query.extend(contact.map(|contact| ("contact", contact)));
            if failed {
                query.push(("failed", "true".to_owned()));
            }
            let inbox = read.inbox(cli.serve.ui_port);
            inbox_get(&inbox, "/api/messages", &query)
                .await
                .map(|response| {
                    read.print(&response, |now| {
                        wapit::report::messages(&response, now, failed)
                    });
                })
        }
        Command::Show { id, read } => {
            let inbox = read.inbox(cli.serve.ui_port);
            let path = format!(
                "/api/messages/{}",
                percent_encoding::utf8_percent_encode(&id, percent_encoding::NON_ALPHANUMERIC)
            );
            inbox_get(&inbox, &path, &[])
                .await
                .map(|response| read.print(&response, |now| wapit::report::message(&response, now)))
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

impl Read {
    fn inbox(&self, ui_port: u16) -> String {
        trim_url(
            &self
                .inbox_url
                .clone()
                .unwrap_or_else(|| format!("http://127.0.0.1:{ui_port}")),
        )
    }

    fn print(&self, response: &serde_json::Value, text: impl FnOnce(i64) -> String) {
        if self.json {
            println!(
                "{}",
                serde_json::to_string_pretty(response).unwrap_or_default()
            );
        } else {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| {
                    i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
                });
            print!("{}", text(now));
        }
    }
}

/// Reads the inbox API of a running wapit. Errors name the address so a wrong port or a stopped container is obvious.
async fn inbox_get(
    inbox: &str,
    path: &str,
    query: &[(&str, String)],
) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|error| error.to_string())?;
    if !inbox.starts_with("http://") && !inbox.starts_with("https://") {
        return Err(format!(
            "The inbox URL {inbox} needs http:// or https://, for example http://127.0.0.1:8026"
        ));
    }

    let response = client
        .get(format!("{inbox}{path}"))
        .query(query)
        .send()
        .await
        .map_err(|_| {
            format!(
                "Could not reach the wapit inbox at {inbox}. Start wapit, or point --inbox-url or WAPIT_INBOX_URL at it."
            )
        })?;
    let status = response.status();

    if status == reqwest::StatusCode::METHOD_NOT_ALLOWED {
        return Err(older(inbox));
    }

    let body: serde_json::Value = response.json().await.map_err(|_| {
        format!("{inbox} answered {status} with something that is not the wapit inbox API")
    })?;

    if status.is_success() {
        Ok(body)
    } else if body["code"] == "route_not_found" {
        Err(older(inbox))
    } else {
        Err(body["error"].as_str().map_or_else(
            || format!("{inbox} answered {status} but does not look like the wapit inbox; check the inbox port"),
            str::to_owned,
        ))
    }
}

fn older(inbox: &str) -> String {
    format!(
        "The wapit at {inbox} is older than this command. Update it to {} or newer.",
        env!("CARGO_PKG_VERSION")
    )
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
        clock: std::sync::Arc::new(wapit::clock::SystemClock),
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
