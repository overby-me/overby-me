//! Native host of the WebXR compositor.
//!
//! Two loops on two threads: a smithay/calloop Wayland compositor (comp),
//! and a tokio HTTP server that hands the dx-built bundle to browsers and
//! speaks the wire protocol with them over /ws (session).

mod comp;
mod session;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::routing::get;
use smithay::reexports::calloop;
use tower_http::services::{ServeDir, ServeFile};

use crate::session::Hub;

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let (events_tx, events_rx) = calloop::channel::channel();
    let hub = Arc::new(Hub::new(events_tx));

    let comp_hub = Arc::clone(&hub);
    std::thread::Builder::new()
        .name("wayland".into())
        .spawn(move || comp::run(comp_hub, events_rx))?;

    tokio::runtime::Runtime::new()?.block_on(serve_http(hub))
}

async fn serve_http(hub: Arc<Hub>) -> Result<(), Box<dyn std::error::Error>> {
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
