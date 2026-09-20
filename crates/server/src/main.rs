use std::net::SocketAddr;

use axum::{Json, Router, routing::get};
use hldr_core::Health;
use maud::{DOCTYPE, Markup, html};
use tokio::signal::unix::{SignalKind, signal};
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

const DEFAULT_ADDR: &str = "127.0.0.1:8080";

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_env("HLDR_LOG").unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let addr: SocketAddr = std::env::var("HLDR_ADDR")
        .unwrap_or_else(|_| DEFAULT_ADDR.to_owned())
        .parse()
        .expect("HLDR_ADDR must be a socket address");

    let app = Router::new()
        .route("/", get(index))
        .route("/healthz", get(healthz))
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind HLDR_ADDR");

    tracing::info!(%addr, version = hldr_core::VERSION, "hldr-server listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(terminate())
        .await
        .expect("server error");
}

async fn healthz() -> Json<Health> {
    Json(Health::ok())
}

async fn index() -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { "hvpaiva.dev" }
                style { (PLACEHOLDER_CSS) }
            }
            body {
                main {
                    p { span .prompt { "$ " } "curl hvpaiva.dev" }
                    p { "hldr " (hldr_core::VERSION) " — under construction." }
                }
            }
        }
    }
}

const PLACEHOLDER_CSS: &str = "\
:root { color-scheme: dark }
body { background: #0f0f0f; color: #cdd6f4; font: 300 1rem/1.8 ui-monospace, monospace; margin: 0 }
main { max-width: 900px; margin: 0 auto; padding: 4rem 1.5rem }
.prompt { color: #a6e3a1 }
";

async fn terminate() {
    let mut sigterm = signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");

    tokio::select! {
        _ = sigterm.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }

    tracing::info!("shutting down");
}
