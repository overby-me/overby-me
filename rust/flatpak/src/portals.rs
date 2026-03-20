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
    let doc_dir = documents_dir();
    if !doc_dir.exists() {
        return Vec::new();
    }

    let mut docs = Vec::new();
    if let Ok(entries) = fs::read_dir(&doc_dir) {
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

    docs
}

/// Export a file to the document portal.
pub fn export_document(path: &str, _app_ids: &[String]) -> Result<String, String> {
    let abs_path = fs::canonicalize(path).map_err(|e| format!("resolve path: {e}"))?;

    // Open the file to get an fd, then call AddFull.
    let file = std::fs::File::open(&abs_path).map_err(|e| format!("open: {e}"))?;
    let fd = zbus::zvariant::OwnedFd::from(std::os::fd::OwnedFd::from(file));

    let conn = session_bus()?;
    let reply = conn
        .call_method(
            Some(DOC_PORTAL_DEST),
            DOC_PORTAL_PATH,
            Some(DOC_PORTAL_IFACE),
            "Add",
            &(fd, true, &[] as &[&str]),
        )
        .map_err(|e| format!("document Add: {e}"))?;

    let doc_id: String = reply
        .body()
        .deserialize()
        .map_err(|e| format!("parse doc id: {e}"))?;

    Ok(doc_id)
}

/// Unexport a document from the portal.
pub fn unexport_document(doc_id: &str) -> Result<(), String> {
    call_method(
        "session",
        DOC_PORTAL_DEST,
        DOC_PORTAL_PATH,
        DOC_PORTAL_IFACE,
        "Delete",
        &(doc_id,),
    )?;
    Ok(())
}

/// Get info about a document.
pub fn document_info(doc_id: &str) -> Result<DocumentInfo, String> {
    let result = call_method(
        "session",
        DOC_PORTAL_DEST,
        DOC_PORTAL_PATH,
        DOC_PORTAL_IFACE,
        "Info",
        &(doc_id,),
    )?;

    Ok(DocumentInfo {
        id: doc_id.to_string(),
        path: PathBuf::from(result.trim()),
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

    let conn = match session_bus() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let reply = conn.call_method(
        Some(PERM_STORE_DEST),
        PERM_STORE_PATH,
        Some(PERM_STORE_IFACE),
        "List",
        &(table_name,),
    );

    match reply {
        Ok(msg) => {
            if let Ok(ids) = msg.body().deserialize::<Vec<String>>() {
                ids.iter()
                    .map(|id| PermissionEntry {
                        table: table_name.to_string(),
                        id: id.clone(),
                        app_id: String::new(),
                        permissions: Vec::new(),
                    })
                    .collect()
            } else {
                Vec::new()
            }
        }
        Err(_) => Vec::new(),
    }
}

/// Set a permission.
pub fn set_permission(
    table: &str,
    id: &str,
    app_id: &str,
    permissions: &[String],
) -> Result<(), String> {
    let perms: Vec<&str> = permissions.iter().map(|s| s.as_str()).collect();
    call_method(
        "session",
        PERM_STORE_DEST,
        PERM_STORE_PATH,
        PERM_STORE_IFACE,
        "SetPermission",
        &(table, true, id, app_id, perms.as_slice()),
    )?;
    Ok(())
}

/// Remove a permission entry.
pub fn remove_permission(table: &str, id: &str) -> Result<(), String> {
    call_method(
        "session",
        PERM_STORE_DEST,
        PERM_STORE_PATH,
        PERM_STORE_IFACE,
        "Delete",
        &(table, id),
    )?;
    Ok(())
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
    Ok(())
}

/// Show permissions for an app.
pub fn show_permissions(app_id: &str) -> Vec<PermissionEntry> {
    let mut all = Vec::new();
    for table in &["flatpak", "notifications", "devices", "background"] {
        let perms = list_permissions(Some(table));
        for p in perms {
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
