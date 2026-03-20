//! XDG Desktop Portal integration.
//!
//! Uses `gdbus` or `busctl` subprocess calls to communicate with the
//! document portal and permission store D-Bus services.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

// ---------------------------------------------------------------------------
// D-Bus helper
// ---------------------------------------------------------------------------

/// Call a D-Bus method via gdbus and return stdout.
fn gdbus_call(
    bus: &str,
    dest: &str,
    object: &str,
    interface: &str,
    method: &str,
    args: &str,
) -> Result<String, String> {
    let bus_flag = match bus {
        "session" => "--session",
        "system" => "--system",
        _ => "--session",
    };

    // Try gdbus first.
    let result = Command::new("gdbus")
        .args([
            "call",
            bus_flag,
            "--dest",
            dest,
            "--object-path",
            object,
            "--method",
            &format!("{interface}.{method}"),
        ])
        .arg(args)
        .output();

    match result {
        Ok(output) if output.status.success() => {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        }
        Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
        Err(_) => {
            // Fall back to busctl.
            let result = Command::new("busctl")
                .args(["--user", "call", dest, object, interface, method, args])
                .output();

            match result {
                Ok(output) if output.status.success() => {
                    Ok(String::from_utf8_lossy(&output.stdout).to_string())
                }
                Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
                Err(e) => Err(format!("no D-Bus client available (gdbus/busctl): {e}")),
            }
        }
    }
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
pub fn export_document(path: &str, app_ids: &[String]) -> Result<String, String> {
    // Open the file and get its fd, then call AddFull on the portal.
    // Since we can't easily pass fds via gdbus, use the filesystem path approach.
    let abs_path = fs::canonicalize(path).map_err(|e| format!("resolve path: {e}"))?;

    // Try calling the portal via gdbus.
    let apps_str = if app_ids.is_empty() {
        "[]".to_string()
    } else {
        format!(
            "[{}]",
            app_ids
                .iter()
                .map(|a| format!("'{a}'"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };

    let result = gdbus_call(
        "session",
        DOC_PORTAL_DEST,
        DOC_PORTAL_PATH,
        DOC_PORTAL_IFACE,
        "Add",
        &format!("'{}' {apps_str} 0", abs_path.display()),
    )?;

    // Parse the returned document ID from the GVariant response.
    let doc_id = result
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .trim_matches('\'')
        .trim_matches(',')
        .to_string();

    Ok(doc_id)
}

/// Unexport a document from the portal.
pub fn unexport_document(doc_id: &str) -> Result<(), String> {
    gdbus_call(
        "session",
        DOC_PORTAL_DEST,
        DOC_PORTAL_PATH,
        DOC_PORTAL_IFACE,
        "Delete",
        &format!("'{doc_id}'"),
    )?;
    Ok(())
}

/// Get info about a document.
pub fn document_info(doc_id: &str) -> Result<DocumentInfo, String> {
    let result = gdbus_call(
        "session",
        DOC_PORTAL_DEST,
        DOC_PORTAL_PATH,
        DOC_PORTAL_IFACE,
        "Info",
        &format!("'{doc_id}'"),
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
    let result = gdbus_call(
        "session",
        PERM_STORE_DEST,
        PERM_STORE_PATH,
        PERM_STORE_IFACE,
        "List",
        &format!("'{table_name}'"),
    );

    match result {
        Ok(output) => parse_permission_list(&output, table_name),
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
    let perms_str = format!(
        "[{}]",
        permissions
            .iter()
            .map(|p| format!("'{p}'"))
            .collect::<Vec<_>>()
            .join(", ")
    );

    gdbus_call(
        "session",
        PERM_STORE_DEST,
        PERM_STORE_PATH,
        PERM_STORE_IFACE,
        "SetPermission",
        &format!("'{table}' true '{id}' '{app_id}' {perms_str}"),
    )?;
    Ok(())
}

/// Remove a permission entry.
pub fn remove_permission(table: &str, id: &str) -> Result<(), String> {
    gdbus_call(
        "session",
        PERM_STORE_DEST,
        PERM_STORE_PATH,
        PERM_STORE_IFACE,
        "Delete",
        &format!("'{table}' '{id}'"),
    )?;
    Ok(())
}

/// Reset all permissions for an app.
pub fn reset_permissions(app_id: &str) -> Result<(), String> {
    // List all tables and remove entries for this app.
    // This is a best-effort approach since the API doesn't have a bulk reset.
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

fn parse_permission_list(output: &str, table: &str) -> Vec<PermissionEntry> {
    // The output is a GVariant representation. Do basic parsing.
    let mut entries = Vec::new();

    // Simple heuristic: look for patterns like 'id': {'app_id': ['perm', ...]}
    // This is fragile but works for the common case.
    for line in output.lines() {
        let line = line.trim();
        if line.contains("':") {
            // Try to extract ID and app permissions.
            if let Some((id_part, rest)) = line.split_once("':") {
                let id = id_part.trim().trim_matches('\'').trim_matches('{').trim();
                if let Some((app_part, _)) = rest.split_once("':") {
                    let app_id = app_part.trim().trim_matches('\'').trim_matches('{').trim();
                    entries.push(PermissionEntry {
                        table: table.to_string(),
                        id: id.to_string(),
                        app_id: app_id.to_string(),
                        permissions: Vec::new(), // Detailed parsing is complex.
                    });
                }
            }
        }
    }

    entries
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
