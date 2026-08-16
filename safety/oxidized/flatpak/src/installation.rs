//! Flatpak installation directory management.
//!
//! Handles the directory layout for both system-wide (/var/lib/flatpak/) and
//! per-user (~/.local/share/flatpak/) installations.

use std::fs;
use std::path::PathBuf;

use crate::metadata::Metadata;

/// A Flatpak installation (system or user).
#[derive(Debug, Clone)]
pub struct Installation {
    pub path: PathBuf,
    pub is_user: bool,
}

/// Reference to an installed app or runtime.
#[derive(Debug, Clone)]
pub struct Ref {
    pub kind: RefKind,
    pub id: String,
    pub arch: String,
    pub branch: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    App,
    Runtime,
}

/// Information about a deployed app/runtime.
#[derive(Debug, Clone)]
pub struct DeployedRef {
    pub ref_: Ref,
    pub path: PathBuf,
    pub metadata: Metadata,
    pub installation: Installation,
}

impl Ref {
    /// Parse a ref string like "app/org.example.App/x86_64/stable".
    pub fn parse(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split('/').collect();
        if parts.len() == 4 {
            let kind = match parts[0] {
                "app" => RefKind::App,
                "runtime" => RefKind::Runtime,
                _ => return None,
            };
            Some(Ref {
                kind,
                id: parts[1].to_string(),
                arch: parts[2].to_string(),
                branch: parts[3].to_string(),
            })
        } else if parts.len() == 3 {
            // "org.freedesktop.Platform/x86_64/23.08" — assume runtime.
            Some(Ref {
                kind: RefKind::Runtime,
                id: parts[0].to_string(),
                arch: parts[1].to_string(),
                branch: parts[2].to_string(),
            })
        } else {
            None
        }
    }

    pub fn format_ref(&self) -> String {
        let kind = match self.kind {
            RefKind::App => "app",
            RefKind::Runtime => "runtime",
        };
        format!("{}/{}/{}/{}", kind, self.id, self.arch, self.branch)
    }

    pub fn kind_dir(&self) -> &str {
        match self.kind {
            RefKind::App => "app",
            RefKind::Runtime => "runtime",
        }
    }
}

impl std::fmt::Display for Ref {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}/{}", self.id, self.arch, self.branch)
    }
}

impl Installation {
    /// System-wide installation at /var/lib/flatpak.
    pub fn system() -> Self {
        Installation {
            path: PathBuf::from("/var/lib/flatpak"),
            is_user: false,
        }
    }

    /// Per-user installation.
    pub fn user() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        Installation {
            path: PathBuf::from(format!("{home}/.local/share/flatpak")),
            is_user: true,
        }
    }

    /// Get both installations (user first, then system).
    pub fn all() -> Vec<Self> {
        vec![Self::user(), Self::system()]
    }

    /// Path to the deploy directory for a ref.
    pub fn deploy_path(&self, ref_: &Ref) -> PathBuf {
        self.path
            .join(ref_.kind_dir())
            .join(&ref_.id)
            .join(&ref_.arch)
            .join(&ref_.branch)
            .join("active")
    }

    /// Path to the files directory for a ref.
    pub fn files_path(&self, ref_: &Ref) -> PathBuf {
        self.deploy_path(ref_).join("files")
    }

    /// Path to the metadata file for a ref.
    pub fn metadata_path(&self, ref_: &Ref) -> PathBuf {
        self.deploy_path(ref_).join("metadata")
    }

    /// Path to the export directory for a ref.
    #[allow(dead_code)]
    pub fn export_path(&self, ref_: &Ref) -> PathBuf {
        self.deploy_path(ref_).join("export")
    }

    /// Path to the overrides directory.
    pub fn overrides_dir(&self) -> PathBuf {
        self.path.join("overrides")
    }

    /// Path to the override file for an app.
    pub fn override_path(&self, app_id: &str) -> PathBuf {
        self.overrides_dir().join(app_id)
    }

    /// Load overrides for an app (merging global + per-app).
    pub fn load_overrides(&self, app_id: &str) -> Option<Metadata> {
        let global_path = self.overrides_dir().join("global");
        let app_path = self.override_path(app_id);

        let mut merged = Metadata::default();
        if global_path.exists()
            && let Ok(m) = Metadata::from_file(&global_path)
        {
            merged = m;
        }
        if app_path.exists()
            && let Ok(m) = Metadata::from_file(&app_path)
        {
            // Merge app overrides on top of global.
            for (group, entries) in m.groups {
                let target = merged.groups.entry(group).or_default();
                for (k, v) in entries {
                    target.insert(k, v);
                }
            }
        }
        if merged.groups.is_empty() {
            None
        } else {
            Some(merged)
        }
    }

    /// Path to the remotes config file.
    #[allow(dead_code)]
    pub fn remotes_dir(&self) -> PathBuf {
        self.path.join("repo").join("config")
    }

    /// List all installed refs.
    pub fn list_refs(&self) -> Vec<DeployedRef> {
        let mut refs = Vec::new();
        for kind in &["app", "runtime"] {
            let kind_dir = self.path.join(kind);
            if !kind_dir.exists() {
                continue;
            }
            let entries = match fs::read_dir(&kind_dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let id = entry.file_name().to_string_lossy().to_string();
                let id_dir = entry.path();
                for arch_entry in fs::read_dir(&id_dir).into_iter().flatten().flatten() {
                    let arch = arch_entry.file_name().to_string_lossy().to_string();
                    let arch_dir = arch_entry.path();
                    for branch_entry in fs::read_dir(&arch_dir).into_iter().flatten().flatten() {
                        let branch = branch_entry.file_name().to_string_lossy().to_string();
                        let ref_ = Ref {
                            kind: if *kind == "app" {
                                RefKind::App
                            } else {
                                RefKind::Runtime
                            },
                            id: id.clone(),
                            arch: arch.clone(),
                            branch: branch.clone(),
                        };
                        let metadata_path = self.metadata_path(&ref_);
                        if metadata_path.exists()
                            && let Ok(metadata) = Metadata::from_file(&metadata_path)
                        {
                            refs.push(DeployedRef {
                                ref_: ref_.clone(),
                                path: self.deploy_path(&ref_),
                                metadata,
                                installation: self.clone(),
                            });
                        }
                    }
                }
            }
        }
        refs
    }

    /// Find a deployed ref by app ID (searches all arches/branches, returns most recent).
    pub fn find_ref(&self, app_id: &str) -> Option<DeployedRef> {
        self.list_refs().into_iter().find(|r| r.ref_.id == app_id)
    }

    /// Find a deployed ref matching a full or partial ref string.
    pub fn find_ref_by_string(&self, s: &str) -> Option<DeployedRef> {
        // Try exact match first.
        if let Some(ref_) = Ref::parse(s) {
            let meta_path = self.metadata_path(&ref_);
            if meta_path.exists()
                && let Ok(metadata) = Metadata::from_file(&meta_path)
            {
                return Some(DeployedRef {
                    path: self.deploy_path(&ref_),
                    ref_,
                    metadata,
                    installation: self.clone(),
                });
            }
        }
        // Fall back to searching by app ID.
        self.find_ref(s)
    }

    /// Get the per-app data directory (~/.var/app/<id>).
    pub fn app_data_dir(app_id: &str) -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        PathBuf::from(format!("{home}/.var/app/{app_id}"))
    }

    /// Ensure the per-app data directories exist.
    pub fn ensure_app_data_dirs(app_id: &str) -> PathBuf {
        let base = Self::app_data_dir(app_id);
        let _ = fs::create_dir_all(base.join("data"));
        let _ = fs::create_dir_all(base.join("config"));
        let _ = fs::create_dir_all(base.join("cache/tmp"));
        let _ = fs::create_dir_all(base.join(".local/state"));
        base
    }
}

/// Remote repository configuration.
#[derive(Debug, Clone)]
pub struct Remote {
    pub name: String,
    pub url: String,
    pub title: Option<String>,
    pub default_branch: Option<String>,
}

/// Load remotes from the flatpak repo config or a simple remotes file.
pub fn load_remotes(installation: &Installation) -> Vec<Remote> {
    let mut remotes = Vec::new();

    // Try reading from a simple remotes config.
    let remotes_file = installation.path.join("remotes.conf");
    if let Ok(content) = fs::read_to_string(&remotes_file)
        && let Ok(meta) = Metadata::parse(&content)
    {
        for (group, entries) in &meta.groups {
            if let Some(name) = group
                .strip_prefix("remote \"")
                .and_then(|s| s.strip_suffix('"'))
            {
                remotes.push(Remote {
                    name: name.to_string(),
                    url: entries.get("url").cloned().unwrap_or_default(),
                    title: entries
                        .get("xa.title")
                        .cloned()
                        .or_else(|| entries.get("title").cloned()),
                    default_branch: entries.get("xa.default-branch").cloned(),
                });
            }
        }
    }

    // Also try the OSTree repo config.
    let ostree_config = installation.path.join("repo").join("config");
    if let Ok(content) = fs::read_to_string(&ostree_config)
        && let Ok(meta) = Metadata::parse(&content)
    {
        for (group, entries) in &meta.groups {
            if let Some(name) = group
                .strip_prefix("remote \"")
                .and_then(|s| s.strip_suffix('"'))
                && !remotes.iter().any(|r| r.name == name)
            {
                remotes.push(Remote {
                    name: name.to_string(),
                    url: entries.get("url").cloned().unwrap_or_default(),
                    title: entries.get("xa.title").cloned(),
                    default_branch: entries.get("xa.default-branch").cloned(),
                });
            }
        }
    }

    remotes
}

/// Save remotes to the installation config.
pub fn save_remotes(installation: &Installation, remotes: &[Remote]) -> Result<(), String> {
    let _ = fs::create_dir_all(&installation.path);
    let remotes_file = installation.path.join("remotes.conf");
    let mut content = String::new();
    for remote in remotes {
        content.push_str(&format!("[remote \"{}\"]\n", remote.name));
        content.push_str(&format!("url={}\n", remote.url));
        if let Some(ref title) = remote.title {
            content.push_str(&format!("xa.title={title}\n"));
        }
        if let Some(ref branch) = remote.default_branch {
            content.push_str(&format!("xa.default-branch={branch}\n"));
        }
        content.push('\n');
    }
    fs::write(&remotes_file, content).map_err(|e| format!("write remotes.conf: {e}"))
}
