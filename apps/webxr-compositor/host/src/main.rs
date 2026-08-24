//! Native host of the WebXR compositor.
//!
//! Serves the dx-built frontend bundle over HTTP and speaks the wire
//! protocol with browsers over /ws. The Wayland socket and the surface
//! pipeline land next.

mod session;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::routing::get;
use tokio::sync::mpsc;
use tower_http::services::{ServeDir, ServeFile};

use crate::session::Hub;

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let web_root = PathBuf::from(env_or(
        "WEBXR_COMPOSITOR_WEB_ROOT",
        "target/dx/webxr-compositor/release/web/public",
    ));
    let listen: SocketAddr = env_or("WEBXR_COMPOSITOR_LISTEN", "127.0.0.1:8370").parse()?;

    if !web_root.join("index.html").is_file() {
        tracing::warn!(
            web_root = %web_root.display(),
            "no index.html under the web root; build the frontend first (just build)"
        );
    }

    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let hub = Arc::new(Hub::new(events_tx));

    // Stands in for the compositor loop until the Wayland side exists.
    tokio::spawn(async move {
        while let Some((client, event)) = events_rx.recv().await {
            tracing::info!(client, ?event, "browser event");
        }
    });

    // The same deep-link behaviour as dx serve: unknown paths get the SPA.
    let spa = ServeDir::new(&web_root).fallback(ServeFile::new(web_root.join("index.html")));
    let app = Router::new()
        .route("/ws", get(session::ws_handler))
        .fallback_service(spa)
        .with_state(hub);

    let listener = tokio::net::TcpListener::bind(listen).await?;
    tracing::info!(
        addr = %listen,
        protocol = webxr_compositor_protocol::VERSION,
        "serving the frontend"
    );
    axum::serve(listener, app).await?;
    Ok(())
}
