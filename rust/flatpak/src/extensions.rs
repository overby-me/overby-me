//! Extension resolution and mounting for Flatpak sandboxes.
//!
//! Parses `[Extension <name>]` groups from runtime and app metadata, resolves
//! them against installed extensions, and generates the bwrap mount arguments.

use std::path::PathBuf;

use crate::installation::{Installation, Ref, RefKind};
use crate::metadata::Metadata;

/// A resolved extension ready to mount.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ResolvedExtension {
    /// Extension ID (e.g., "org.freedesktop.Platform.GL.default").
    pub id: String,
    /// Target mount directory inside the sandbox (relative to /usr or /app).
    pub directory: String,
    /// Path to the extension's files on disk.
    pub files_path: PathBuf,
    /// Whether to add a library path entry.
    pub add_ld_path: Option<String>,
    /// Directories to merge.
    pub merge_dirs: Option<String>,
    /// Whether this extension has subdirectories.
    pub subdirectories: bool,
}

/// An extension declaration from metadata.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct ExtensionDecl {
    /// Full extension point name (e.g., "org.freedesktop.Platform.GL").
    name: String,
    /// Mount directory inside the sandbox.
    directory: String,
    /// Library path to add.
    add_ld_path: Option<String>,
    /// Directories to merge.
    merge_dirs: Option<String>,
    /// Whether subdirectories are mounted individually.
    subdirectories: bool,
    /// Specific version to look for.
    version: Option<String>,
    /// Multiple versions to search.
    versions: Option<String>,
    /// Don't auto-download.
    no_autodownload: bool,
}

/// Parse extension declarations from metadata.
fn parse_extensions(metadata: &Metadata) -> Vec<ExtensionDecl> {
    let mut extensions = Vec::new();

    for (group, entries) in &metadata.groups {
        let name = if let Some(n) = group.strip_prefix("Extension ") {
            n.to_string()
        } else {
            continue;
        };

        let directory = entries.get("directory").cloned().unwrap_or_default();
        if directory.is_empty() {
            continue;
        }

        extensions.push(ExtensionDecl {
            name,
            directory,
            add_ld_path: entries.get("add-ld-path").cloned(),
            merge_dirs: entries.get("merge-dirs").cloned(),
            subdirectories: entries.get("subdirectories").is_some_and(|v| v == "true"),
            version: entries.get("version").cloned(),
            versions: entries.get("versions").cloned(),
            no_autodownload: entries.get("no-autodownload").is_some_and(|v| v == "true"),
        });
    }

    extensions
}

/// Resolve extensions against installed refs.
///
/// For each extension declaration, search installations for matching
/// installed extensions and return the resolved set.
pub fn resolve_extensions(
    runtime_metadata: &Metadata,
    app_metadata: Option<&Metadata>,
    installations: &[Installation],
    runtime_ref: &Ref,
) -> Vec<ResolvedExtension> {
    let mut resolved = Vec::new();

    // Parse extensions from both runtime and app metadata.
    let mut decls = parse_extensions(runtime_metadata);
    if let Some(app_meta) = app_metadata {
        decls.extend(parse_extensions(app_meta));
    }

    for decl in &decls {
        // Determine which branches/versions to search.
        let branches = extension_branches(decl, runtime_ref);

        // Search for the extension in installed refs.
        for inst in installations {
            for branch in &branches {
                let ext_ref = Ref {
                    kind: RefKind::Runtime,
                    id: decl.name.clone(),
                    arch: runtime_ref.arch.clone(),
                    branch: branch.clone(),
                };

                let files_path = inst.files_path(&ext_ref);
                if files_path.exists() {
                    resolved.push(ResolvedExtension {
                        id: decl.name.clone(),
                        directory: decl.directory.clone(),
                        files_path,
                        add_ld_path: decl.add_ld_path.clone(),
                        merge_dirs: decl.merge_dirs.clone(),
                        subdirectories: decl.subdirectories,
                    });
                    break; // Found, don't search more branches.
                }

                // If subdirectories mode, look for extensions that start with this name.
                if decl.subdirectories {
                    let installed = inst.list_refs();
                    for deployed in &installed {
                        if deployed.ref_.id.starts_with(&decl.name)
                            && deployed.ref_.arch == runtime_ref.arch
                        {
                            let sub_files = inst.files_path(&deployed.ref_);
                            if sub_files.exists() {
                                // Subdirectory name is the part after the extension point name.
                                let sub_name = deployed
                                    .ref_
                                    .id
                                    .strip_prefix(&format!("{}.", decl.name))
                                    .unwrap_or(&deployed.ref_.id);
                                let sub_dir = format!("{}/{}", decl.directory, sub_name);

                                resolved.push(ResolvedExtension {
                                    id: deployed.ref_.id.clone(),
                                    directory: sub_dir,
                                    files_path: sub_files,
                                    add_ld_path: decl.add_ld_path.clone(),
                                    merge_dirs: decl.merge_dirs.clone(),
                                    subdirectories: false,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    resolved
}

/// Determine which branch names to search for an extension.
fn extension_branches(decl: &ExtensionDecl, runtime_ref: &Ref) -> Vec<String> {
    let mut branches = Vec::new();

    if let Some(ref v) = decl.version {
        branches.push(v.clone());
    }
    if let Some(ref vs) = decl.versions {
        for v in vs.split(';') {
            let v = v.trim();
            if !v.is_empty() && !branches.contains(&v.to_string()) {
                branches.push(v.to_string());
            }
        }
    }

    // Always try the runtime's branch as a fallback.
    if !branches.contains(&runtime_ref.branch) {
        branches.push(runtime_ref.branch.clone());
    }

    branches
}

/// Generate bwrap mount arguments for resolved extensions.
///
/// Returns (bwrap_args, ld_library_paths).
pub fn extension_mount_args(
    extensions: &[ResolvedExtension],
    is_app: bool,
) -> (Vec<String>, Vec<String>) {
    let mut args = Vec::new();
    let mut ld_paths = Vec::new();

    let base = if is_app { "/app" } else { "/usr" };

    // Collect merge-dirs: group extensions by their merge target directory.
    let mut merge_groups: std::collections::HashMap<String, Vec<&ResolvedExtension>> =
        std::collections::HashMap::new();

    for ext in extensions {
        if let Some(ref merge_dirs) = ext.merge_dirs {
            for merge_dir in merge_dirs
                .split(';')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
            {
                merge_groups
                    .entry(merge_dir.to_string())
                    .or_default()
                    .push(ext);
            }
        }
    }

    for ext in extensions {
        let dest = format!("{base}/{}", ext.directory);
        let src = ext.files_path.to_string_lossy();

        args.push("--ro-bind".to_string());
        args.push(src.to_string());
        args.push(dest.clone());

        // Add LD library path if specified.
        if let Some(ref ld_path) = ext.add_ld_path {
            let full_path = format!("{dest}/{ld_path}");
            ld_paths.push(full_path);
        }
    }

    // For merge-dirs, create temporary merged directories and bind-mount them.
    // This creates a tmpfs-backed directory containing symlinks to each
    // extension's files for the specified subdirectories.
    for (merge_dir, exts) in &merge_groups {
        let merged_tmp = format!(
            "/tmp/.flatpak-merge-{}-{}",
            merge_dir.replace('/', "_"),
            std::process::id()
        );
        let _ = std::fs::create_dir_all(&merged_tmp);

        for ext in exts {
            let src_dir = ext.files_path.join(merge_dir);
            if src_dir.exists()
                && let Ok(entries) = std::fs::read_dir(&src_dir)
            {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let dest_link = PathBuf::from(&merged_tmp).join(&name);
                    if !dest_link.exists() {
                        let _ = std::os::unix::fs::symlink(entry.path(), &dest_link);
                    }
                }
            }
        }

        let dest = format!("{base}/{merge_dir}");
        args.push("--ro-bind".to_string());
        args.push(merged_tmp);
        args.push(dest);
    }

    (args, ld_paths)
}

/// Regenerate ld.so.cache for extensions that add library paths.
///
/// Runs ldconfig inside a minimal bwrap sandbox to generate the cache,
/// then returns the path to the generated cache file for bind-mounting.
pub fn regenerate_ld_cache(
    runtime_files: &std::path::Path,
    ld_paths: &[String],
) -> Option<PathBuf> {
    if ld_paths.is_empty() {
        return None;
    }

    let ldconfig = runtime_files.join("bin/ldconfig");
    if !ldconfig.exists() {
        // Try /usr/sbin/ldconfig.
        let alt = runtime_files.join("sbin/ldconfig");
        if !alt.exists() {
            return None;
        }
    }

    let cache_dir = PathBuf::from(format!("/tmp/.flatpak-ldcache-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&cache_dir);
    let cache_file = cache_dir.join("ld.so.cache");

    // Write an ld.so.conf with the extension paths.
    let conf_file = cache_dir.join("ld.so.conf");
    let conf_content = ld_paths.join("\n") + "\n";
    let _ = std::fs::write(&conf_file, conf_content);

    // Run ldconfig via bwrap.
    let bwrap = crate::sandbox::find_bwrap();
    let status = std::process::Command::new(&bwrap)
        .args(["--ro-bind", &runtime_files.to_string_lossy(), "/usr"])
        .args(["--symlink", "usr/lib", "/lib"])
        .args(["--symlink", "usr/lib64", "/lib64"])
        .args(["--bind", &cache_dir.to_string_lossy(), "/tmp/ldcache"])
        .args(["--proc", "/proc"])
        .args(["--dev", "/dev"])
        .arg("--")
        .arg("/usr/sbin/ldconfig")
        .args(["-f", "/tmp/ldcache/ld.so.conf"])
        .args(["-C", "/tmp/ldcache/ld.so.cache"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    match status {
        Ok(s) if s.success() && cache_file.exists() => Some(cache_file),
        _ => None,
    }
}
