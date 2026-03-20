//! XDG Desktop Portal integration stubs.
//!
//! Provides interfaces for the document portal, permission store, and
//! related portal services. Currently stubbed — full implementation requires
//! D-Bus client support for the portal APIs.

use std::fs;
use std::path::PathBuf;

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

            // If filtering by app_id, check if this doc belongs to the app.
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

/// Export a file to the document portal (stub).
pub fn export_document(_path: &str, _app_ids: &[String]) -> Result<String, String> {
    Err("Document portal export requires D-Bus portal API (not yet implemented)".into())
}

/// Unexport a document from the portal (stub).
pub fn unexport_document(_doc_id: &str) -> Result<(), String> {
    Err("Document portal unexport requires D-Bus portal API (not yet implemented)".into())
}

/// Get info about a document (stub).
pub fn document_info(_doc_id: &str) -> Result<DocumentInfo, String> {
    Err("Document portal info requires D-Bus portal API (not yet implemented)".into())
}

/// List permissions from the permission store (stub).
pub fn list_permissions(_table: Option<&str>) -> Vec<PermissionEntry> {
    // The permission store is a D-Bus API at org.freedesktop.impl.portal.PermissionStore.
    Vec::new()
}

/// Set a permission (stub).
pub fn set_permission(
    _table: &str,
    _id: &str,
    _app_id: &str,
    _permissions: &[String],
) -> Result<(), String> {
    Err("Permission store requires D-Bus portal API (not yet implemented)".into())
}

/// Remove a permission (stub).
pub fn remove_permission(_table: &str, _id: &str) -> Result<(), String> {
    Err("Permission store requires D-Bus portal API (not yet implemented)".into())
}

/// Reset all permissions for an app (stub).
pub fn reset_permissions(_app_id: &str) -> Result<(), String> {
    Err("Permission store requires D-Bus portal API (not yet implemented)".into())
}

/// Show permissions for an app (stub).
pub fn show_permissions(_app_id: &str) -> Vec<PermissionEntry> {
    Vec::new()
}

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
