//! Flatpak metadata file parser.
//!
//! Parses the INI-format metadata files used by Flatpak for application and
//! runtime configuration. Handles groups like `[Application]`, `[Context]`,
//! `[Environment]`, `[Session Bus Policy]`, and extension groups.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Parsed metadata file containing all groups and their key-value pairs.
#[derive(Debug, Clone, Default)]
pub struct Metadata {
    pub groups: HashMap<String, HashMap<String, String>>,
}

impl Metadata {
    pub fn from_file(path: &Path) -> Result<Self, String> {
        let content = fs::read_to_string(path)
            .map_err(|e| format!("read metadata {}: {e}", path.display()))?;
        Self::parse(&content)
    }

    pub fn parse(content: &str) -> Result<Self, String> {
        let mut groups: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut current_group: Option<String> = None;

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                let group = line[1..line.len() - 1].to_string();
                groups.entry(group.clone()).or_default();
                current_group = Some(group);
            } else if let Some(ref group) = current_group
                && let Some((key, val)) = line.split_once('=')
            {
                groups
                    .get_mut(group)
                    .unwrap()
                    .insert(key.trim().to_string(), val.trim().to_string());
            }
        }

        Ok(Metadata { groups })
    }

    pub fn get(&self, group: &str, key: &str) -> Option<&str> {
        self.groups.get(group)?.get(key).map(|s| s.as_str())
    }

    /// Get the application or runtime name.
    pub fn app_name(&self) -> Option<&str> {
        self.get("Application", "name")
            .or_else(|| self.get("Runtime", "name"))
    }

    /// Get the runtime ref string (e.g., "org.freedesktop.Platform/x86_64/23.08").
    pub fn runtime(&self) -> Option<&str> {
        self.get("Application", "runtime")
            .or_else(|| self.get("Application", "sdk"))
    }

    /// Get the command to run.
    pub fn command(&self) -> Option<&str> {
        self.get("Application", "command")
    }

    /// Check if this is an application (vs. a runtime).
    pub fn is_app(&self) -> bool {
        self.groups.contains_key("Application")
    }

    /// Get context permissions (shared, sockets, devices, features, filesystems).
    pub fn context(&self) -> ContextPermissions {
        let ctx = self.groups.get("Context");
        ContextPermissions {
            shared: parse_semicolon_list(ctx.and_then(|g| g.get("shared"))),
            sockets: parse_semicolon_list(ctx.and_then(|g| g.get("sockets"))),
            devices: parse_semicolon_list(ctx.and_then(|g| g.get("devices"))),
            features: parse_semicolon_list(ctx.and_then(|g| g.get("features"))),
            filesystems: parse_semicolon_list(ctx.and_then(|g| g.get("filesystems"))),
            persistent: parse_semicolon_list(ctx.and_then(|g| g.get("persistent"))),
            unset_environment: parse_semicolon_list(ctx.and_then(|g| g.get("unset-environment"))),
        }
    }

    /// Get environment variables from [Environment] group.
    pub fn environment(&self) -> HashMap<String, String> {
        self.groups.get("Environment").cloned().unwrap_or_default()
    }

    /// Get session bus policies.
    #[allow(dead_code)]
    pub fn session_bus_policy(&self) -> HashMap<String, String> {
        self.groups
            .get("Session Bus Policy")
            .cloned()
            .unwrap_or_default()
    }

    /// Get system bus policies.
    #[allow(dead_code)]
    pub fn system_bus_policy(&self) -> HashMap<String, String> {
        self.groups
            .get("System Bus Policy")
            .cloned()
            .unwrap_or_default()
    }

    /// Serialize back to INI format.
    pub fn serialize(&self) -> String {
        let mut out = String::new();
        for (group, entries) in &self.groups {
            out.push_str(&format!("[{group}]\n"));
            for (key, val) in entries {
                out.push_str(&format!("{key}={val}\n"));
            }
            out.push('\n');
        }
        out
    }
}

/// Context permissions extracted from [Context] group.
#[derive(Debug, Clone, Default)]
pub struct ContextPermissions {
    pub shared: Vec<String>,
    pub sockets: Vec<String>,
    pub devices: Vec<String>,
    pub features: Vec<String>,
    pub filesystems: Vec<String>,
    pub persistent: Vec<String>,
    pub unset_environment: Vec<String>,
}

impl ContextPermissions {
    pub fn has_shared(&self, name: &str) -> bool {
        self.shared.iter().any(|s| s == name)
    }
    pub fn has_socket(&self, name: &str) -> bool {
        self.sockets.iter().any(|s| s == name)
    }
    pub fn has_device(&self, name: &str) -> bool {
        self.devices.iter().any(|s| s == name)
    }
    #[allow(dead_code)]
    pub fn has_feature(&self, name: &str) -> bool {
        self.features.iter().any(|s| s == name)
    }

    /// Merge another set of permissions on top of this one.
    /// Items prefixed with `!` revoke the permission.
    pub fn merge(&mut self, other: &ContextPermissions) {
        merge_list(&mut self.shared, &other.shared);
        merge_list(&mut self.sockets, &other.sockets);
        merge_list(&mut self.devices, &other.devices);
        merge_list(&mut self.features, &other.features);
        merge_list(&mut self.filesystems, &other.filesystems);
        merge_list(&mut self.persistent, &other.persistent);
        merge_list(&mut self.unset_environment, &other.unset_environment);
    }
}

fn merge_list(base: &mut Vec<String>, additions: &[String]) {
    for item in additions {
        if let Some(stripped) = item.strip_prefix('!') {
            base.retain(|s| s != stripped);
        } else if !base.contains(item) {
            base.push(item.clone());
        }
    }
}

fn parse_semicolon_list(val: Option<&String>) -> Vec<String> {
    match val {
        Some(s) => s
            .split(';')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_metadata() {
        let content = "\
[Application]
name=org.example.App
runtime=org.freedesktop.Platform/x86_64/23.08
command=myapp

[Context]
shared=network;ipc
sockets=x11;wayland;pulseaudio
devices=dri
filesystems=home;/tmp

[Environment]
MY_VAR=hello
";
        let meta = Metadata::parse(content).unwrap();
        assert_eq!(meta.app_name(), Some("org.example.App"));
        assert_eq!(meta.command(), Some("myapp"));
        let ctx = meta.context();
        assert!(ctx.has_shared("network"));
        assert!(ctx.has_socket("wayland"));
        assert!(ctx.has_device("dri"));
        assert_eq!(meta.environment().get("MY_VAR").unwrap(), "hello");
    }

    #[test]
    fn merge_permissions() {
        let mut base = ContextPermissions {
            shared: vec!["network".into(), "ipc".into()],
            ..Default::default()
        };
        let overlay = ContextPermissions {
            shared: vec!["!ipc".into()],
            ..Default::default()
        };
        base.merge(&overlay);
        assert!(base.has_shared("network"));
        assert!(!base.has_shared("ipc"));
    }
}
