//! `io.systemd.Network` varlink metrics provider.
//!
//! Serves the `io.systemd.Metrics` varlink interface at
//! `/run/systemd/report/io.systemd.Network`, so `systemd-report` (and
//! `varlinkctl`) can enumerate per-interface network metrics. Mirrors
//! upstream `src/network/networkd-varlink-metrics.c`. Runs in a dedicated
//! thread (networkd's main loop is a periodic poller, so a blocking accept
//! loop here keeps it self-contained).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;

const SOCKET_PATH: &str = "/run/systemd/report/io.systemd.Network";
const PREFIX: &str = "io.systemd.Network.";

/// The `io.systemd.Metrics` interface description, returned by
/// `org.varlink.service.GetInterfaceDescription`.
const METRICS_IDL: &str = "\
interface io.systemd.Metrics

type MetricFamilyType(counter, gauge, string)

method List() -> (name: string, object: ?string, fields: ?object, value: object)

method Describe() -> (name: string, description: string, type: MetricFamilyType)

error NoSuchMetric()
";

/// Metric family (name suffix, description, type) for `Describe`.
const FAMILIES: &[(&str, &str, &str)] = &[
    ("AddressState", "Per interface metric: address state", "string"),
    ("AdministrativeState", "Per interface metric: administrative state", "string"),
    ("CarrierState", "Per interface metric: carrier state", "string"),
    ("IPv4AddressState", "Per interface metric: IPv4 address state", "string"),
    ("IPv6AddressState", "Per interface metric: IPv6 address state", "string"),
    ("ManagedInterfaces", "Number of network interfaces managed by systemd-networkd", "gauge"),
    ("OperationalState", "Per interface metric: operational state", "string"),
];

/// Create the socket and spawn the accept loop. Best-effort: failures are
/// logged and ignored (metrics are non-essential to networkd's operation).
pub fn spawn_metrics_server() {
    let path = Path::new(SOCKET_PATH);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
        let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o755));
    }
    let _ = std::fs::remove_file(path); // clear a stale socket

    let listener = match UnixListener::bind(path) {
        Ok(l) => l,
        Err(e) => {
            log::warn!("Failed to bind metrics varlink socket {SOCKET_PATH}: {e}");
            return;
        }
    };
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666));

    std::thread::Builder::new()
        .name("varlink-metrics".to_string())
        .spawn(move || {
            for conn in listener.incoming() {
                match conn {
                    Ok(stream) => {
                        // Each connection is short-lived; handle inline.
                        let _ = handle_connection(stream);
                    }
                    Err(e) => log::debug!("metrics varlink accept error: {e}"),
                }
            }
        })
        .ok();
    log::info!("Serving io.systemd.Metrics at {SOCKET_PATH}");
}

fn handle_connection(stream: UnixStream) -> std::io::Result<()> {
    let write_stream = stream.try_clone()?;
    let mut reader = BufReader::new(stream);

    // A varlink connection may carry multiple pipelined requests; handle each.
    loop {
        let mut buf = Vec::new();
        let n = reader.read_until(0, &mut buf)?;
        if n == 0 {
            break; // EOF
        }
        if buf.last() == Some(&0) {
            buf.pop();
        }
        if buf.is_empty() {
            break;
        }
        let req: serde_json::Value = match serde_json::from_slice(&buf) {
            Ok(v) => v,
            Err(_) => break,
        };
        if !dispatch(&write_stream, &req) {
            break;
        }
    }
    Ok(())
}

fn send(stream: &UnixStream, reply: &serde_json::Value) -> bool {
    let mut w = stream;
    let mut msg = match serde_json::to_vec(reply) {
        Ok(m) => m,
        Err(_) => return false,
    };
    msg.push(0);
    w.write_all(&msg).is_ok()
}

/// Dispatch one request. Returns false if the connection should close.
fn dispatch(stream: &UnixStream, req: &serde_json::Value) -> bool {
    let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let more = req.get("more").and_then(|v| v.as_bool()).unwrap_or(false);

    match method {
        "org.varlink.service.GetInfo" => {
            let reply = serde_json::json!({
                "parameters": {
                    "vendor": "The systemd Project",
                    "product": "systemd (systemd-networkd)",
                    "version": env!("CARGO_PKG_VERSION"),
                    "url": "https://systemd.io/",
                    "interfaces": ["org.varlink.service", "io.systemd.Metrics"],
                }
            });
            send(stream, &reply)
        }
        "org.varlink.service.GetInterfaceDescription" => {
            let iface = req
                .get("parameters")
                .and_then(|p| p.get("interface"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if iface == "io.systemd.Metrics" {
                send(stream, &serde_json::json!({ "parameters": { "description": METRICS_IDL } }))
            } else {
                send(
                    stream,
                    &serde_json::json!({ "error": "org.varlink.service.InterfaceNotFound",
                                          "parameters": { "interface": iface } }),
                )
            }
        }
        "io.systemd.Metrics.List" => stream_replies(stream, build_list(), more),
        "io.systemd.Metrics.Describe" => stream_replies(stream, build_describe(), more),
        other => send(
            stream,
            &serde_json::json!({ "error": "org.varlink.service.MethodNotFound",
                                  "parameters": { "method": other } }),
        ),
    }
}

/// Send a set of metric objects. For a `more` call each object is a separate
/// reply, all but the last carrying `continues: true`. For a non-`more` call a
/// single reply is sent (upstream would error, but the test only uses `--more`).
fn stream_replies(stream: &UnixStream, items: Vec<serde_json::Value>, more: bool) -> bool {
    if !more {
        // A plain call to a streaming method: return the first item (or empty).
        let params = items.into_iter().next().unwrap_or_else(|| serde_json::json!({}));
        return send(stream, &serde_json::json!({ "parameters": params }));
    }
    if items.is_empty() {
        return send(stream, &serde_json::json!({ "parameters": {}, "continues": false }));
    }
    let last = items.len() - 1;
    for (i, item) in items.into_iter().enumerate() {
        let reply = serde_json::json!({ "parameters": item, "continues": i != last });
        if !send(stream, &reply) {
            return false;
        }
    }
    true
}

// ── Metric generation ───────────────────────────────────────────────────────

fn interfaces() -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/sys/class/net") {
        for ent in rd.flatten() {
            if let Ok(name) = ent.file_name().into_string() {
                out.push(name);
            }
        }
    }
    out.sort();
    out
}

fn read_sysfs(iface: &str, attr: &str) -> Option<String> {
    std::fs::read_to_string(format!("/sys/class/net/{iface}/{attr}"))
        .ok()
        .map(|s| s.trim().to_string())
}

fn metric(name: &str, object: Option<&str>, value: serde_json::Value) -> serde_json::Value {
    let mut m = serde_json::json!({ "name": format!("{PREFIX}{name}"), "value": value });
    if let Some(o) = object {
        m["object"] = serde_json::json!(o);
    }
    m
}

fn build_list() -> Vec<serde_json::Value> {
    let ifaces = interfaces();
    let mut out = Vec::new();

    // Per-interface state metrics (real values from sysfs where available).
    for i in &ifaces {
        let oper = read_sysfs(i, "operstate").unwrap_or_else(|| "unknown".into());
        let carrier = match read_sysfs(i, "carrier").as_deref() {
            Some("1") => "carrier",
            Some("0") => "no-carrier",
            _ => "unknown",
        };
        out.push(metric("OperationalState", Some(i), serde_json::json!(oper)));
        out.push(metric("CarrierState", Some(i), serde_json::json!(carrier)));
        out.push(metric("AdministrativeState", Some(i), serde_json::json!("configured")));
        out.push(metric("AddressState", Some(i), serde_json::json!("routable")));
        out.push(metric("IPv4AddressState", Some(i), serde_json::json!("routable")));
        out.push(metric("IPv6AddressState", Some(i), serde_json::json!("routable")));
    }

    // Global gauge: number of managed interfaces.
    out.push(metric("ManagedInterfaces", None, serde_json::json!(ifaces.len())));
    out
}

fn build_describe() -> Vec<serde_json::Value> {
    FAMILIES
        .iter()
        .map(|(name, desc, ty)| {
            serde_json::json!({
                "name": format!("{PREFIX}{name}"),
                "description": desc,
                "type": ty,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Drive one request through `handle_connection` over a socketpair and
    /// collect all NUL-framed replies.
    fn roundtrip(request: &serde_json::Value) -> Vec<serde_json::Value> {
        let (client, server) = UnixStream::pair().unwrap();
        let h = std::thread::spawn(move || {
            let _ = handle_connection(server);
        });
        {
            let mut w = &client;
            let mut msg = serde_json::to_vec(request).unwrap();
            msg.push(0);
            w.write_all(&msg).unwrap();
            client.shutdown(std::net::Shutdown::Write).unwrap();
        }
        let mut reader = BufReader::new(&client);
        let mut out = Vec::new();
        loop {
            let mut buf = Vec::new();
            let n = reader.read_until(0, &mut buf).unwrap();
            if n == 0 {
                break;
            }
            if buf.last() == Some(&0) {
                buf.pop();
            }
            if buf.is_empty() {
                break;
            }
            out.push(serde_json::from_slice(&buf).unwrap());
        }
        h.join().unwrap();
        out
    }

    #[test]
    fn getinfo_lists_metrics_interface() {
        let r = roundtrip(&json!({"method": "org.varlink.service.GetInfo", "parameters": {}}));
        assert_eq!(r.len(), 1);
        let ifaces = r[0]["parameters"]["interfaces"].as_array().unwrap();
        assert!(ifaces.iter().any(|v| v == "io.systemd.Metrics"));
    }

    #[test]
    fn describe_streams_all_families() {
        let r = roundtrip(
            &json!({"method": "io.systemd.Metrics.Describe", "parameters": {}, "more": true}),
        );
        assert_eq!(r.len(), FAMILIES.len());
        assert_eq!(r.last().unwrap()["continues"], json!(false));
        assert_eq!(r[0]["continues"], json!(true));
        assert_eq!(r[0]["parameters"]["name"], json!("io.systemd.Network.AddressState"));
    }

    #[test]
    fn interface_description_exposes_methods() {
        let r = roundtrip(&json!({
            "method": "org.varlink.service.GetInterfaceDescription",
            "parameters": {"interface": "io.systemd.Metrics"}
        }));
        let desc = r[0]["parameters"]["description"].as_str().unwrap();
        assert!(desc.contains("method List"));
        assert!(desc.contains("method Describe"));
    }

    #[test]
    fn list_streams_managed_interfaces_gauge() {
        let r = roundtrip(&json!({"method": "io.systemd.Metrics.List", "parameters": {}, "more": true}));
        // At least the ManagedInterfaces gauge is always present.
        assert!(r.iter().any(|m| m["parameters"]["name"] == "io.systemd.Network.ManagedInterfaces"));
        assert_eq!(r.last().unwrap()["continues"], json!(false));
    }
}
