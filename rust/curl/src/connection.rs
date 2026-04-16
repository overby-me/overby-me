use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;

use crate::options::Options;
use crate::tls::{make_tls_config, InsecureVerifier};
use crate::url::ParsedUrl;

pub(crate) enum Connection {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Read for Connection {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Connection::Plain(s) => s.read(buf),
            Connection::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Connection {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Connection::Plain(s) => s.write(buf),
            Connection::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Connection::Plain(s) => s.flush(),
            Connection::Tls(s) => s.flush(),
        }
    }
}

pub(crate) fn connect(url: &ParsedUrl, opts: &Options) -> Result<Connection, String> {
    let addr = format!("{}:{}", url.host, url.port);

    let tcp = if let Some(timeout) = opts.connect_timeout {
        let addrs: Vec<_> = std::net::ToSocketAddrs::to_socket_addrs(&addr)
            .map_err(|e| format!("DNS resolution failed for {}: {e}", url.host))?
            .collect();
        let mut last_err = String::from("no addresses resolved");
        let mut stream = None;
        for a in addrs {
            match TcpStream::connect_timeout(&a, timeout) {
                Ok(s) => {
                    stream = Some(s);
                    break;
                }
                Err(e) => last_err = e.to_string(),
            }
        }
        stream.ok_or(last_err)?
    } else {
        TcpStream::connect(&addr).map_err(|e| format!("connection failed to {addr}: {e}"))?
    };

    if let Some(timeout) = opts.max_time {
        let _ = tcp.set_read_timeout(Some(timeout));
        let _ = tcp.set_write_timeout(Some(timeout));
    }

    if url.scheme == "https" {
        let tls_config = if opts.insecure {
            let mut config = rustls::ClientConfig::builder()
                .with_root_certificates(rustls::RootCertStore::empty())
                .with_no_client_auth();
            config
                .dangerous()
                .set_certificate_verifier(Arc::new(InsecureVerifier));
            Arc::new(config)
        } else {
            make_tls_config(opts)?
        };

        let server_name = rustls::pki_types::ServerName::try_from(url.host.as_str())
            .map_err(|e| format!("invalid server name '{}': {e}", url.host))?
            .to_owned();
        let conn = rustls::ClientConnection::new(tls_config, server_name)
            .map_err(|e| format!("TLS handshake failed: {e}"))?;
        let stream = rustls::StreamOwned::new(conn, tcp);
        Ok(Connection::Tls(Box::new(stream)))
    } else {
        Ok(Connection::Plain(tcp))
    }
}
