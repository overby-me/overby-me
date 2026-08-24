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

    let tls_wanted = std::env::var("WEBXR_COMPOSITOR_TLS").is_ok_and(|v| v == "1");
    let insecure = std::env::var("WEBXR_COMPOSITOR_INSECURE").is_ok_and(|v| v == "1");
    let secure = tls_wanted || !listen.ip().is_loopback();
    if secure && insecure {
        tracing::warn!("serving beyond loopback WITHOUT TLS or tokens, as requested");
    }
    let secure = secure && !insecure;
    if secure {
        hub.set_token(access_token()?);
    }

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
        .with_state(Arc::clone(&hub));

    if secure {
        let config = tls_config().await?;
        let token = hub.token().unwrap_or_default();
        tracing::info!(
            url = %format!("https://{listen}/?token={token}"),
            protocol = webxr_compositor_protocol::VERSION,
            "serving the frontend over TLS"
        );
        axum_server::bind_rustls(listen, config)
            .serve(app.into_make_service())
            .await?;
    } else {
        let listener = tokio::net::TcpListener::bind(listen).await?;
        tracing::info!(
            addr = %listen,
            protocol = webxr_compositor_protocol::VERSION,
            "serving the frontend"
        );
        axum::serve(listener, app).await?;
    }
    Ok(())
}

/// The bearer every WebSocket connect must present in secure mode.
fn access_token() -> Result<String, Box<dyn std::error::Error>> {
    if let Ok(token) = std::env::var("WEBXR_COMPOSITOR_TOKEN") {
        return Ok(token);
    }
    use std::io::Read;
    let mut bytes = [0_u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Certificates from WEBXR_COMPOSITOR_CERT/KEY, else a fresh self-signed
/// one; browsers will warn, which is the honest state of a first contact.
async fn tls_config() -> Result<axum_server::tls_rustls::RustlsConfig, Box<dyn std::error::Error>>
{
    if let (Ok(cert), Ok(key)) = (
        std::env::var("WEBXR_COMPOSITOR_CERT"),
        std::env::var("WEBXR_COMPOSITOR_KEY"),
    ) {
        return Ok(axum_server::tls_rustls::RustlsConfig::from_pem_file(cert, key).await?);
    }
    let hostname = std::env::var("HOSTNAME").unwrap_or_else(|_| "webxr-compositor".to_owned());
    let names = vec!["localhost".to_owned(), "127.0.0.1".to_owned(), hostname];
    let certified = rcgen::generate_simple_self_signed(names)?;
    Ok(axum_server::tls_rustls::RustlsConfig::from_pem(
        certified.cert.pem().into_bytes(),
        certified.signing_key.serialize_pem().into_bytes(),
    )
    .await?)
}
