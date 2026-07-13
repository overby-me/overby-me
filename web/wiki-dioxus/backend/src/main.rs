//! Serverless Container entrypoint: serve the `handle` request handler over HTTP.
//!
//! Scaleway Serverless Containers inject `$PORT` and route inbound 80/443 to it, so
//! we bind `0.0.0.0:$PORT` and send every request to [`wiki_backend::handle`], which
//! does its own path routing. This replaces the old Scaleway Functions runtime that
//! called `handle` per request (and which never worked on the rust196 runtime).

use axum::extract::Request;
use axum::response::Response;
use std::convert::Infallible;
use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    // Route every request to `handle` (it matches paths internally). A tower
    // service, rather than an axum `Handler`, since `handle` takes the raw request.
    let app = axum::Router::new().fallback_service(tower::service_fn(|req: Request| async move {
        Ok::<Response, Infallible>(wiki_backend::handle(req).await)
    }));
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("failed to bind {addr}: {e}");
            std::process::exit(1);
        }
    };
    println!("wiki-backend listening on {addr}");
    if let Err(e) = axum::serve(listener, app).await {
        eprintln!("http server error: {e}");
        std::process::exit(1);
    }
}
