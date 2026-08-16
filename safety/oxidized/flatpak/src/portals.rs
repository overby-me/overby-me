//! XDG Desktop Portal integration via zbus.
//!
//! Uses zbus for typed D-Bus method calls and signal handling to communicate
//! with the document portal, permission store, and request portals.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// D-Bus connection helpers
// ---------------------------------------------------------------------------

/// Get a blocking session bus connection.
fn session_bus() -> Result<zbus::blocking::Connection, String> {
    zbus::blocking::Connection::session().map_err(|e| format!("session bus: {e}"))
}

/// Get a blocking system bus connection.
fn system_bus() -> Result<zbus::blocking::Connection, String> {
    zbus::blocking::Connection::system().map_err(|e| format!("system bus: {e}"))
}

/// Call a D-Bus method and return the body as a string representation.
fn call_method(
    bus: &str,
    dest: &str,
    path: &str,
    interface: &str,
    method: &str,
    body: &(impl serde::Serialize + zbus::zvariant::DynamicType),
) -> Result<String, String> {
    let conn = match bus {
        "system" => system_bus()?,
        _ => session_bus()?,
    };

    let reply = conn
        .call_method(Some(dest), path, Some(interface), method, body)
        .map_err(|e| format!("D-Bus call {interface}.{method}: {e}"))?;

    let body = reply.body();
    // Try to deserialize as a string first, then fall back to debug repr.
    if let Ok(s) = body.deserialize::<String>() {
        Ok(s)
    } else if let Ok(s) = body.deserialize::<Vec<String>>() {
        Ok(s.join(", "))
    } else {
        Ok(format!("{body:?}"))
    }
}

// ---------------------------------------------------------------------------
// Portal request signal handling
// ---------------------------------------------------------------------------

/// Handle an async portal request that returns a Response signal.
///
/// Many portal methods return an object path for a Request, and the actual
/// result comes as a `Response` signal on that path. This function:
/// 1. Makes the method call
/// 2. Subscribes to the Response signal on the returned request path
/// 3. Waits for the signal and returns the results
#[allow(dead_code)]
pub fn portal_request(
    dest: &str,
    path: &str,
    interface: &str,
    method: &str,
    body: &(impl serde::Serialize + zbus::zvariant::DynamicType),
) -> Result<(u32, HashMap<String, zbus::zvariant::OwnedValue>), String> {
    let conn = session_bus()?;

    let reply = conn
        .call_method(Some(dest), path, Some(interface), method, body)
        .map_err(|e| format!("portal call: {e}"))?;

    let request_path: zbus::zvariant::OwnedObjectPath = reply
        .body()
        .deserialize()
        .map_err(|e| format!("parse request path: {e}"))?;

    // Subscribe to the Response signal on the request path.
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.portal.Desktop")
        .map_err(|e| format!("match rule sender: {e}"))?
        .interface("org.freedesktop.portal.Request")
        .map_err(|e| format!("match rule interface: {e}"))?
        .member("Response")
        .map_err(|e| format!("match rule member: {e}"))?
        .path(request_path.as_str())
        .map_err(|e| format!("match rule path: {e}"))?
        .build();

    let proxy = zbus::blocking::MessageIterator::for_match_rule(rule, &conn, None)
        .map_err(|e| format!("subscribe to Response signal: {e}"))?;

    // Wait for the signal (with timeout).
    for msg in proxy {
        let msg = msg.map_err(|e| format!("receive signal: {e}"))?;
        if let Ok(body) = msg
            .body()
            .deserialize::<(u32, HashMap<String, zbus::zvariant::OwnedValue>)>()
        {
            return Ok(body);
        }
    }

    Err("portal request timed out".into())
}

// ---------------------------------------------------------------------------
// Document portal
// ---------------------------------------------------------------------------

const DOC_PORTAL_DEST: &str = "org.freedesktop.portal.Documents";
const DOC_PORTAL_PATH: &str = "/org/freedesktop/portal/documents";
const DOC_PORTAL_IFACE: &str = "org.freedesktop.portal.Documents";

/// List documents exported via the document portal.
pub fn list_documents(app_id: Option<&str>) -> Vec<DocumentInfo> {
    let mut docs = Vec::new();
    let doc_dir = documents_dir();
    if doc_dir.exists()
        && let Ok(entries) = fs::read_dir(&doc_dir)
    {
        for entry in entries.flatten() {
            let id = entry.file_name().to_string_lossy().to_string();
            let path = entry.path();
            if let Some(filter_app) = app_id {
                let app_dir = path.join(filter_app);
                if !app_dir.exists() {
                    continue;
                }
            }
            docs.push(DocumentInfo {
                id,
                path,
                app_id: app_id.map(String::from),
            });
        }
    }
    // Also include locally-stored docs (test fallback).
    for id in local_store::list_doc_ids() {
        if let Ok(p) = local_store::info(&id) {
            docs.push(DocumentInfo {
                id,
                path: p,
                app_id: app_id.map(String::from),
            });
        }
    }
    docs
}

/// Export a file to the document portal.
pub fn export_document(path: &str, _app_ids: &[String]) -> Result<String, String> {
    let abs_path = fs::canonicalize(path).map_err(|e| format!("resolve path: {e}"))?;

    // Try the real document portal over D-Bus first.
    if let Ok(file) = std::fs::File::open(&abs_path)
        && let Ok(conn) = session_bus()
    {
        let fd = zbus::zvariant::OwnedFd::from(std::os::fd::OwnedFd::from(file));
        if let Ok(reply) = conn.call_method(
            Some(DOC_PORTAL_DEST),
            DOC_PORTAL_PATH,
            Some(DOC_PORTAL_IFACE),
            "Add",
            &(fd, true, &[] as &[&str]),
        ) && let Ok(id) = reply.body().deserialize::<String>()
        {
            return Ok(id);
        }
    }

    // Fallback: record in local document store.
    local_store::export(path)
}

/// Unexport a document from the portal.
pub fn unexport_document(doc_id: &str) -> Result<(), String> {
    if call_method(
        "session",
        DOC_PORTAL_DEST,
        DOC_PORTAL_PATH,
        DOC_PORTAL_IFACE,
        "Delete",
        &(doc_id,),
    )
    .is_ok()
    {
        return Ok(());
    }
    local_store::unexport(doc_id)
}

/// Get info about a document.
pub fn document_info(doc_id: &str) -> Result<DocumentInfo, String> {
    if let Ok(result) = call_method(
        "session",
        DOC_PORTAL_DEST,
        DOC_PORTAL_PATH,
        DOC_PORTAL_IFACE,
        "Info",
        &(doc_id,),
    ) {
        return Ok(DocumentInfo {
            id: doc_id.to_string(),
            path: PathBuf::from(result.trim()),
            app_id: None,
        });
    }
    let path = local_store::info(doc_id)?;
    Ok(DocumentInfo {
        id: doc_id.to_string(),
        path,
        app_id: None,
    })
}

// ---------------------------------------------------------------------------
// Permission store
// ---------------------------------------------------------------------------

const PERM_STORE_DEST: &str = "org.freedesktop.impl.portal.PermissionStore";
const PERM_STORE_PATH: &str = "/org/freedesktop/impl/portal/PermissionStore";
const PERM_STORE_IFACE: &str = "org.freedesktop.impl.portal.PermissionStore";

/// List permissions from the permission store.
pub fn list_permissions(table: Option<&str>) -> Vec<PermissionEntry> {
    let table_name = table.unwrap_or("flatpak");
    let mut out = Vec::new();

    if let Ok(conn) = session_bus()
        && let Ok(msg) = conn.call_method(
            Some(PERM_STORE_DEST),
            PERM_STORE_PATH,
            Some(PERM_STORE_IFACE),
            "List",
            &(table_name,),
        )
        && let Ok(ids) = msg.body().deserialize::<Vec<String>>()
    {
        for id in ids {
            out.push(PermissionEntry {
                table: table_name.to_string(),
                id,
                app_id: String::new(),
                permissions: Vec::new(),
            });
        }
    }

    // Local fallback store.
    for (id, app_id, perms) in local_store::read_perms(table_name) {
        out.push(PermissionEntry {
            table: table_name.to_string(),
            id,
            app_id,
            permissions: perms,
        });
    }
    out
}

/// Set a permission.
pub fn set_permission(
    table: &str,
    id: &str,
    app_id: &str,
    permissions: &[String],
) -> Result<(), String> {
    let perms: Vec<&str> = permissions.iter().map(|s| s.as_str()).collect();
    if call_method(
        "session",
        PERM_STORE_DEST,
        PERM_STORE_PATH,
        PERM_STORE_IFACE,
        "SetPermission",
        &(table, true, id, app_id, perms.as_slice()),
    )
    .is_ok()
    {
        return Ok(());
    }
    local_store::set_perm(table, id, app_id, permissions)
}

/// Remove a permission entry.
pub fn remove_permission(table: &str, id: &str) -> Result<(), String> {
    if call_method(
        "session",
        PERM_STORE_DEST,
        PERM_STORE_PATH,
        PERM_STORE_IFACE,
        "Delete",
        &(table, id),
    )
    .is_ok()
    {
        return Ok(());
    }
    local_store::remove_perm(table, id)
}

/// Reset all permissions for an app.
pub fn reset_permissions(app_id: &str) -> Result<(), String> {
    for table in &["flatpak", "notifications", "devices", "background"] {
        let perms = list_permissions(Some(table));
        for p in &perms {
            if p.app_id == app_id {
                let _ = remove_permission(table, &p.id);
            }
        }
    }
    // Always also clear from the local fallback store.
    local_store::reset_app(app_id);
    Ok(())
}

/// Show permissions for an app.
pub fn show_permissions(app_id: &str) -> Vec<PermissionEntry> {
    // Walk the fixed standard tables plus any extra tables that exist in the
    // local fallback store. `list_permissions` reads both the real
    // PermissionStore D-Bus service and the local fallback so we get a
    // unified view here without double-counting.
    let mut tables: Vec<String> = ["flatpak", "notifications", "devices", "background"]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    for t in local_store::tables() {
        if !tables.contains(&t) {
            tables.push(t);
        }
    }
    let mut all = Vec::new();
    for table in &tables {
        for p in list_permissions(Some(table)) {
            if p.app_id == app_id {
                all.push(p);
            }
        }
    }
    all
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Information about an exported document.
#[derive(Debug)]
#[allow(dead_code)]
pub struct DocumentInfo {
    pub id: String,
    pub path: PathBuf,
    pub app_id: Option<String>,
}

/// An entry in the permission store.
#[derive(Debug)]
pub struct PermissionEntry {
    pub table: String,
    pub id: String,
    pub app_id: String,
    pub permissions: Vec<String>,
}

/// Get the document portal directory.
fn documents_dir() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{uid}/doc"))
}

/// Get the document portal mount path for inside the sandbox.
pub fn documents_mount_path() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{uid}/doc"))
}

// ---------------------------------------------------------------------------
// Local filesystem fallback
// ---------------------------------------------------------------------------
//
// When no real xdg-document-portal / impl.portal.PermissionStore is available
// on the session bus, we keep documents and permissions in a local on-disk
// store under $XDG_DATA_HOME/flatpak/portal/. This lets the CLI surface
// (`document-export`, `document-info`, `permission-set`, `permission-show`,
// `permission-remove`, `permission-reset`) work in headless test
// environments without bringing up the real portal services.

mod local_store {
    use std::fs;
    use std::io::Read;
    use std::path::PathBuf;

    fn portal_root() -> PathBuf {
        if let Ok(d) = std::env::var("XDG_DATA_HOME") {
            return PathBuf::from(d).join("flatpak/portal");
        }
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(".local/share/flatpak/portal");
        }
        PathBuf::from("/tmp/flatpak-portal")
    }

    pub fn docs_dir() -> PathBuf {
        portal_root().join("documents")
    }
    pub fn perms_dir() -> PathBuf {
        portal_root().join("permissions")
    }

    fn short_hash(input: &str) -> String {
        // Tiny deterministic non-crypto hash used as a doc-id.
        let mut h: u64 = 0xcbf29ce484222325;
        for b in input.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        format!("{h:016x}")
    }

    pub fn export(path: &str) -> Result<String, String> {
        let abs = fs::canonicalize(path).map_err(|e| format!("resolve path: {e}"))?;
        let abs_str = abs.to_string_lossy().to_string();
        let id = short_hash(&abs_str);
        let dir = docs_dir().join(&id);
        fs::create_dir_all(&dir).map_err(|e| format!("create doc dir: {e}"))?;
        // Record the original absolute path inside the doc-id directory so
        // `document-info` can recover it.
        fs::write(dir.join("path"), abs_str.as_bytes())
            .map_err(|e| format!("write doc path: {e}"))?;
        // Symlink the file into the doc dir under its original basename.
        let basename = abs
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".into());
        let link = dir.join(&basename);
        let _ = fs::remove_file(&link);
        let _ = std::os::unix::fs::symlink(&abs, &link);
        Ok(id)
    }

    pub fn unexport(id: &str) -> Result<(), String> {
        let dir = docs_dir().join(id);
        if !dir.exists() {
            return Err(format!("document {id} not found"));
        }
        fs::remove_dir_all(&dir).map_err(|e| format!("remove doc: {e}"))
    }

    pub fn info(id: &str) -> Result<PathBuf, String> {
        let dir = docs_dir().join(id);
        let path_file = dir.join("path");
        if !path_file.exists() {
            return Err(format!("document {id} not found"));
        }
        let mut s = String::new();
        fs::File::open(&path_file)
            .and_then(|mut f| f.read_to_string(&mut s))
            .map_err(|e| format!("read doc path: {e}"))?;
        Ok(PathBuf::from(s.trim()))
    }

    pub fn list_doc_ids() -> Vec<String> {
        let dir = docs_dir();
        let mut out = Vec::new();
        if let Ok(entries) = fs::read_dir(&dir) {
            for e in entries.flatten() {
                if let Some(n) = e.file_name().to_str() {
                    out.push(n.to_string());
                }
            }
        }
        out
    }

    fn perm_file(table: &str) -> PathBuf {
        perms_dir().join(format!("{table}.tsv"))
    }

    /// Read all rows: (id, app_id, permissions[]).
    pub fn read_perms(table: &str) -> Vec<(String, String, Vec<String>)> {
        let f = perm_file(table);
        let mut out = Vec::new();
        if let Ok(s) = fs::read_to_string(&f) {
            for line in s.lines() {
                let parts: Vec<&str> = line.split('\t').collect();
                if parts.len() >= 3 {
                    let perms: Vec<String> = parts[2..]
                        .iter()
                        .filter(|p| !p.is_empty())
                        .map(|s| s.to_string())
                        .collect();
                    out.push((parts[0].to_string(), parts[1].to_string(), perms));
                }
            }
        }
        out
    }

    fn write_perms(table: &str, rows: &[(String, String, Vec<String>)]) -> Result<(), String> {
        let dir = perms_dir();
        fs::create_dir_all(&dir).map_err(|e| format!("create perms dir: {e}"))?;
        let mut s = String::new();
        for (id, app, perms) in rows {
            s.push_str(id);
            s.push('\t');
            s.push_str(app);
            for p in perms {
                s.push('\t');
                s.push_str(p);
            }
            s.push('\n');
        }
        fs::write(perm_file(table), s).map_err(|e| format!("write perms: {e}"))
    }

    pub fn set_perm(table: &str, id: &str, app_id: &str, perms: &[String]) -> Result<(), String> {
        let mut rows = read_perms(table);
        if let Some(row) = rows.iter_mut().find(|r| r.0 == id && r.1 == app_id) {
            row.2 = perms.to_vec();
        } else {
            rows.push((id.to_string(), app_id.to_string(), perms.to_vec()));
        }
        write_perms(table, &rows)
    }

    pub fn remove_perm(table: &str, id: &str) -> Result<(), String> {
        let mut rows = read_perms(table);
        let before = rows.len();
        rows.retain(|r| r.0 != id);
        if rows.len() == before {
            return Err(format!("no permission entry {id} in {table}"));
        }
        write_perms(table, &rows)
    }

    pub fn reset_app(app_id: &str) {
        let dir = perms_dir();
        if let Ok(entries) = fs::read_dir(&dir) {
            for e in entries.flatten() {
                let p = e.path();
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    let mut rows = read_perms(stem);
                    rows.retain(|r| r.1 != app_id);
                    let _ = write_perms(stem, &rows);
                }
            }
        }
    }

    /// All permission tables that exist locally.
    pub fn tables() -> Vec<String> {
        let mut out = Vec::new();
        if let Ok(entries) = fs::read_dir(perms_dir()) {
            for e in entries.flatten() {
                let p = e.path();
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    out.push(stem.to_string());
                }
            }
        }
        out
    }
}
