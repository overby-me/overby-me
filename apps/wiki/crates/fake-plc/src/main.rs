//! `fake-plc [--port N]`: see the library.

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let port = match (args.next().as_deref(), args.next()) {
        (Some("--port"), Some(port)) => port.parse().expect("a port"),
        (None, _) => 2582,
        _ => {
            eprintln!("usage: fake-plc [--port N]");
            std::process::exit(2);
        }
    };
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .expect("a port");
    println!("http://{}", listener.local_addr().expect("an address"));
    axum::serve(listener, fake_plc::router())
        .await
        .expect("serving");
}
