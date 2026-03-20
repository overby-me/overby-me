//! Flatpak build commands for creating and exporting applications.
//!
//! Implements the `flatpak build-*` workflow:
//! 1. `build-init` — initialize a build directory with runtime/SDK
//! 2. `build` — run a command inside the build sandbox
//! 3. `build-finish` — finalize metadata and set permissions
//! 4. `build-export` — export a build to an OSTree repository
//! 5. `build-bundle` / `build-import-bundle` — single-file bundles
//! 6. `build-sign` — GPG sign commits
//! 7. `build-update-repo` — regenerate summary file
//! 8. `build-commit-from` — create new commit from existing ref

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::installation::{Installation, Ref, RefKind};
use crate::metadata::Metadata;
use crate::sandbox;

/// Initialize a build directory.
///
/// Creates the directory structure and metadata file needed for
/// `flatpak build` to work.
pub fn build_init(
    dir: &Path,
    sdk: &str,
    runtime: &str,
    runtime_version: &str,
    app_id: &str,
    extension_tag: Option<&str>,
) -> Result<(), String> {
    let _ = fs::create_dir_all(dir);
    let files_dir = dir.join("files");
    let _ = fs::create_dir_all(&files_dir);
    let _ = fs::create_dir_all(files_dir.join("bin"));
    let _ = fs::create_dir_all(files_dir.join("lib"));
    let _ = fs::create_dir_all(files_dir.join("share"));

    let var_dir = dir.join("var");
    let _ = fs::create_dir_all(&var_dir);
    let _ = fs::create_dir_all(var_dir.join("tmp"));
    let _ = fs::create_dir_all(var_dir.join("lib"));
    let _ = fs::create_dir_all(var_dir.join("run"));

    // Create metadata file.
    let arch = std::env::consts::ARCH;
    let mut content = String::new();

    if let Some(tag) = extension_tag {
        content.push_str("[Runtime]\n");
        content.push_str(&format!("name={app_id}\n"));
        content.push_str(&format!("runtime={runtime}/{arch}/{runtime_version}\n"));
        content.push_str(&format!("sdk={sdk}/{arch}/{runtime_version}\n"));
        content.push_str(&format!(
            "\n[ExtensionOf]\nref=runtime/{tag}/{arch}/{runtime_version}\n"
        ));
    } else {
        content.push_str("[Application]\n");
        content.push_str(&format!("name={app_id}\n"));
        content.push_str(&format!("runtime={runtime}/{arch}/{runtime_version}\n"));
        content.push_str(&format!("sdk={sdk}/{arch}/{runtime_version}\n"));
    }

    fs::write(dir.join("metadata"), &content).map_err(|e| format!("write metadata: {e}"))?;

    Ok(())
}

/// Run a build command inside the build directory.
///
/// Sets up a sandbox with the SDK mounted as /usr and the build directory's
/// files/ as /app, then executes the given command.
pub fn build_run(
    dir: &Path,
    command: &[String],
    runtime_env: bool,
    installations: &[Installation],
) -> Result<i32, String> {
    let metadata_path = dir.join("metadata");
    let metadata = Metadata::from_file(&metadata_path)?;

    // Find the SDK (used as /usr during build).
    let sdk_ref_str = metadata
        .get("Application", "sdk")
        .or_else(|| metadata.get("Runtime", "sdk"))
        .ok_or("no sdk specified in metadata")?;

    let sdk_ref = Ref::parse(sdk_ref_str).ok_or("could not parse SDK ref")?;

    let mut sdk_deployed = None;
    for inst in installations {
        if let Some(d) = inst.find_ref_by_string(&sdk_ref.to_string()) {
            sdk_deployed = Some(d);
            break;
        }
    }
    let sdk_deployed = sdk_deployed
        .ok_or_else(|| format!("SDK {} is not installed. Install it first.", sdk_ref))?;

    let sdk_files = sdk_deployed.installation.files_path(&sdk_deployed.ref_);
    let app_files = dir.join("files");
    let var_dir = dir.join("var");

    let bwrap_path = sandbox::find_bwrap();

    let mut cmd = Command::new(&bwrap_path);

    // Namespace setup — less restrictive than runtime sandbox.
    cmd.args(["--unshare-pid", "--die-with-parent"]);

    // Mount SDK as /usr.
    cmd.args(["--ro-bind", &sdk_files.to_string_lossy(), "/usr"]);

    // Mount app files as /app (writable for builds).
    if app_files.exists() {
        cmd.args(["--bind", &app_files.to_string_lossy(), "/app"]);
    }

    // Mount build var.
    if var_dir.exists() {
        cmd.args(["--bind", &var_dir.to_string_lossy(), "/var"]);
    }

    // Basic filesystem.
    cmd.args(["--proc", "/proc"]);
    cmd.args(["--dev", "/dev"]);
    cmd.args(["--tmpfs", "/tmp"]);

    // Usr-merged symlinks.
    for name in &["bin", "sbin", "lib", "lib32", "lib64"] {
        cmd.args(["--symlink", &format!("usr/{name}"), &format!("/{name}")]);
    }

    // Host /etc files for build tools.
    for name in &[
        "resolv.conf",
        "hosts",
        "localtime",
        "timezone",
        "passwd",
        "group",
    ] {
        let path = format!("/etc/{name}");
        if Path::new(&path).exists() {
            cmd.args(["--ro-bind", &path, &path]);
        }
    }

    // Allow network access for builds (package downloads).
    // No --unshare-net.

    // Environment.
    cmd.args(["--setenv", "PATH", "/app/bin:/usr/bin:/usr/lib/sdk/bin"]);
    cmd.args(["--setenv", "XDG_DATA_DIRS", "/app/share:/usr/share"]);
    cmd.args([
        "--setenv",
        "PKG_CONFIG_PATH",
        "/app/lib/pkgconfig:/app/share/pkgconfig:/usr/lib/pkgconfig:/usr/share/pkgconfig",
    ]);
    cmd.args(["--setenv", "FLATPAK_BUILDER", "1"]);

    if runtime_env {
        cmd.args(["--setenv", "FLATPAK_BUILD_RUNTIME", "1"]);
    }

    // The build command.
    cmd.arg("--");
    for a in command {
        cmd.arg(a);
    }

    let status = cmd
        .status()
        .map_err(|e| format!("failed to execute bwrap: {e}"))?;

    Ok(status.code().unwrap_or(1))
}

/// Finalize a build directory for export.
///
/// Sets the command, permissions, and other metadata based on the provided
/// options.
pub fn build_finish(
    dir: &Path,
    command: Option<&str>,
    permissions: &[(String, String)],
) -> Result<(), String> {
    let metadata_path = dir.join("metadata");
    let mut metadata = Metadata::from_file(&metadata_path)?;

    // Set the command.
    if let Some(cmd) = command {
        let app_group = metadata
            .groups
            .entry("Application".to_string())
            .or_default();
        app_group.insert("command".to_string(), cmd.to_string());
    }

    // Apply permissions to [Context].
    if !permissions.is_empty() {
        let ctx = metadata.groups.entry("Context".to_string()).or_default();
        for (key, val) in permissions {
            let existing = ctx.entry(key.clone()).or_default();
            if !existing.is_empty() {
                existing.push(';');
            }
            existing.push_str(val);
        }
    }

    // Write updated metadata.
    fs::write(&metadata_path, metadata.serialize()).map_err(|e| format!("write metadata: {e}"))?;

    // Create export directory structure.
    let export_dir = dir.join("export");
    let files_dir = dir.join("files");

    // Copy desktop files from files/share/applications to export/share/applications.
    let desktop_src = files_dir.join("share/applications");
    if desktop_src.exists() {
        let desktop_dest = export_dir.join("share/applications");
        let _ = fs::create_dir_all(&desktop_dest);
        copy_dir_contents(&desktop_src, &desktop_dest);
    }

    // Copy icons.
    let icons_src = files_dir.join("share/icons");
    if icons_src.exists() {
        let icons_dest = export_dir.join("share/icons");
        let _ = fs::create_dir_all(&icons_dest);
        copy_dir_contents(&icons_src, &icons_dest);
    }

    // Copy appdata/metainfo.
    for subdir in &["share/metainfo", "share/appdata"] {
        let src = files_dir.join(subdir);
        if src.exists() {
            let dest = export_dir.join(subdir);
            let _ = fs::create_dir_all(&dest);
            copy_dir_contents(&src, &dest);
        }
    }

    // Copy D-Bus service files.
    let dbus_src = files_dir.join("share/dbus-1/services");
    if dbus_src.exists() {
        let dbus_dest = export_dir.join("share/dbus-1/services");
        let _ = fs::create_dir_all(&dbus_dest);
        copy_dir_contents(&dbus_src, &dbus_dest);
    }

    Ok(())
}

/// Export a build directory to a local OSTree repository.
///
/// This creates a simple file-based "repository" that can be used with
/// `flatpak install` from a local path.
pub fn build_export(
    repo_path: &Path,
    dir: &Path,
    branch: Option<&str>,
    subject: Option<&str>,
) -> Result<String, String> {
    let metadata_path = dir.join("metadata");
    let metadata = Metadata::from_file(&metadata_path)?;

    let app_name = metadata
        .app_name()
        .ok_or("no name in metadata")?
        .to_string();
    let kind = if metadata.is_app() {
        RefKind::App
    } else {
        RefKind::Runtime
    };
    let arch = std::env::consts::ARCH;
    let branch = branch.unwrap_or("stable");

    let ref_ = Ref {
        kind,
        id: app_name.clone(),
        arch: arch.to_string(),
        branch: branch.to_string(),
    };

    // Create the repository structure.
    let ref_dir = repo_path
        .join(ref_.kind_dir())
        .join(&ref_.id)
        .join(&ref_.arch)
        .join(&ref_.branch)
        .join("active");
    let _ = fs::create_dir_all(&ref_dir);

    // Copy metadata.
    let _ = fs::copy(dir.join("metadata"), ref_dir.join("metadata"));

    // Copy files.
    let src_files = dir.join("files");
    let dest_files = ref_dir.join("files");
    if src_files.exists() {
        copy_dir_recursive(&src_files, &dest_files);
    }

    // Copy export.
    let src_export = dir.join("export");
    let dest_export = ref_dir.join("export");
    if src_export.exists() {
        copy_dir_recursive(&src_export, &dest_export);
    }

    let ref_str = ref_.format_ref();
    let subject = subject.unwrap_or("Export");

    // Create OSTree objects for the export.
    let ostree_repo = repo_path.join("repo");
    let _ = fs::create_dir_all(&ostree_repo);
    if src_files.exists() {
        match crate::ostree::create_dirtree_from_dir(&src_files, &ostree_repo) {
            Ok((dirtree_cksum, dirmeta_cksum)) => {
                match crate::ostree::create_commit(
                    &ostree_repo,
                    &dirtree_cksum,
                    &dirmeta_cksum,
                    subject,
                    None,
                ) {
                    Ok(commit_cksum) => {
                        crate::ostree::write_ref(&ostree_repo, &ref_str, &commit_cksum);
                        eprintln!("Exported {ref_str}: {subject} (commit {commit_cksum})");
                    }
                    Err(e) => {
                        eprintln!("warning: OSTree commit creation failed: {e}");
                        eprintln!("Exported {ref_str}: {subject} (file copy only)");
                    }
                }
            }
            Err(e) => {
                eprintln!("warning: OSTree tree creation failed: {e}");
                eprintln!("Exported {ref_str}: {subject} (file copy only)");
            }
        }
    } else {
        eprintln!("Exported {ref_str}: {subject}");
    }

    Ok(ref_str)
}

/// Create a single-file bundle from a ref in a repository.
pub fn build_bundle(repo_path: &Path, bundle_path: &Path, ref_name: &str) -> Result<(), String> {
    let ref_ = Ref::parse(ref_name).ok_or("could not parse ref")?;
    let deploy_path = repo_path
        .join(ref_.kind_dir())
        .join(&ref_.id)
        .join(&ref_.arch)
        .join(&ref_.branch)
        .join("active");

    if !deploy_path.exists() {
        return Err(format!("ref {ref_name} not found in repo"));
    }

    // Create a simple tar-like bundle: a directory with metadata and files.
    // For a real implementation, this would be a proper Flatpak bundle format.
    // For now, use a tar archive.
    let status = Command::new("tar")
        .args([
            "czf",
            &bundle_path.to_string_lossy(),
            "-C",
            &deploy_path.to_string_lossy(),
            ".",
        ])
        .status()
        .map_err(|e| format!("tar: {e}"))?;

    if !status.success() {
        return Err("tar failed".into());
    }

    eprintln!(
        "Created bundle: {} from {}",
        bundle_path.display(),
        ref_name
    );
    Ok(())
}

/// Import a bundle file into a local installation.
pub fn build_import_bundle(
    installations: &[Installation],
    bundle_path: &Path,
) -> Result<String, String> {
    let inst = &installations[0];

    // Extract to a temp dir first to read metadata.
    let temp_dir = PathBuf::from(format!("/tmp/.flatpak-import-{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);

    let status = Command::new("tar")
        .args([
            "xzf",
            &bundle_path.to_string_lossy(),
            "-C",
            &temp_dir.to_string_lossy(),
        ])
        .status()
        .map_err(|e| format!("tar: {e}"))?;

    if !status.success() {
        let _ = fs::remove_dir_all(&temp_dir);
        return Err("failed to extract bundle".into());
    }

    let metadata = Metadata::from_file(&temp_dir.join("metadata"))?;
    let app_name = metadata
        .app_name()
        .ok_or("no name in metadata")?
        .to_string();
    let kind = if metadata.is_app() {
        RefKind::App
    } else {
        RefKind::Runtime
    };
    let arch = std::env::consts::ARCH;

    let ref_ = Ref {
        kind,
        id: app_name.clone(),
        arch: arch.to_string(),
        branch: "stable".to_string(),
    };

    let deploy_path = inst.deploy_path(&ref_);
    let _ = fs::create_dir_all(&deploy_path);

    // Move extracted files to deploy path.
    copy_dir_recursive(&temp_dir, &deploy_path);
    let _ = fs::remove_dir_all(&temp_dir);

    let ref_str = ref_.format_ref();
    eprintln!("Imported: {ref_str}");
    Ok(ref_str)
}

/// Update the summary file in a repository (stub).
pub fn build_update_repo(repo_path: &Path) -> Result<(), String> {
    // Walk the repo and create a summary of all refs.
    let summary_path = repo_path.join("summary");
    let mut refs = Vec::new();

    for kind in &["app", "runtime"] {
        let kind_dir = repo_path.join(kind);
        if !kind_dir.exists() {
            continue;
        }
        for id_entry in fs::read_dir(&kind_dir).into_iter().flatten().flatten() {
            let id = id_entry.file_name().to_string_lossy().to_string();
            for arch_entry in fs::read_dir(id_entry.path())
                .into_iter()
                .flatten()
                .flatten()
            {
                let arch = arch_entry.file_name().to_string_lossy().to_string();
                for branch_entry in fs::read_dir(arch_entry.path())
                    .into_iter()
                    .flatten()
                    .flatten()
                {
                    let branch = branch_entry.file_name().to_string_lossy().to_string();
                    refs.push(format!("{kind}/{id}/{arch}/{branch}"));
                }
            }
        }
    }

    let mut content = String::from("# Flatpak repo summary\n");
    for r in &refs {
        content.push_str(&format!("{r}\n"));
    }
    fs::write(&summary_path, &content).map_err(|e| format!("write summary: {e}"))?;

    eprintln!("Updated summary: {} refs", refs.len());
    Ok(())
}

/// Create a new commit from an existing ref's content.
pub fn build_commit_from(repo_path: &Path, src_ref: &str, dest_ref: &str) -> Result<(), String> {
    // Find the source ref's files.
    let src_parsed = Ref::parse(src_ref).ok_or("could not parse source ref")?;
    let src_files = repo_path
        .join(src_parsed.kind_dir())
        .join(&src_parsed.id)
        .join(&src_parsed.arch)
        .join(&src_parsed.branch)
        .join("active")
        .join("files");

    if !src_files.exists() {
        return Err(format!("source ref {src_ref} not found"));
    }

    let ostree_repo = repo_path.join("repo");
    let _ = fs::create_dir_all(&ostree_repo);

    let (dirtree_cksum, dirmeta_cksum) =
        crate::ostree::create_dirtree_from_dir(&src_files, &ostree_repo)?;
    let commit_cksum = crate::ostree::create_commit(
        &ostree_repo,
        &dirtree_cksum,
        &dirmeta_cksum,
        &format!("Commit from {src_ref}"),
        None,
    )?;
    crate::ostree::write_ref(&ostree_repo, dest_ref, &commit_cksum);

    // Also copy the deployment for the new ref.
    let dest_parsed = Ref::parse(dest_ref).ok_or("could not parse dest ref")?;
    let dest_dir = repo_path
        .join(dest_parsed.kind_dir())
        .join(&dest_parsed.id)
        .join(&dest_parsed.arch)
        .join(&dest_parsed.branch)
        .join("active");
    let _ = fs::create_dir_all(&dest_dir);
    let src_deploy = repo_path
        .join(src_parsed.kind_dir())
        .join(&src_parsed.id)
        .join(&src_parsed.arch)
        .join(&src_parsed.branch)
        .join("active");
    copy_dir_recursive(&src_deploy, &dest_dir);

    eprintln!("Created {dest_ref} from {src_ref} (commit {commit_cksum})");
    Ok(())
}

/// Sign a ref in a repository using GPG.
pub fn build_sign(repo_path: &Path, ref_name: &str, key_id: &str) -> Result<(), String> {
    // Find the commit checksum for this ref.
    let ostree_repo = repo_path.join("repo");
    let ref_path = ostree_repo.join("refs").join("heads").join(ref_name);

    let commit_checksum = if ref_path.exists() {
        fs::read_to_string(&ref_path)
            .map_err(|e| format!("read ref: {e}"))?
            .trim()
            .to_string()
    } else {
        return Err(format!("ref {ref_name} not found in repo"));
    };

    // Read the commit object.
    let commit_path = ostree_repo
        .join("objects")
        .join(&commit_checksum[..2])
        .join(format!("{}.commit", &commit_checksum[2..]));

    if !commit_path.exists() {
        return Err(format!("commit object {} not found", commit_checksum));
    }

    let commit_data = fs::read(&commit_path).map_err(|e| format!("read commit: {e}"))?;

    // Sign with GPG.
    let sig_path = format!("/tmp/.flatpak-sign-{}", std::process::id());
    let data_path = format!("/tmp/.flatpak-signdata-{}", std::process::id());
    let _ = fs::write(&data_path, &commit_data);

    let status = Command::new("gpg")
        .args([
            "--detach-sign",
            "--armor",
            "-u",
            key_id,
            "-o",
            &sig_path,
            &data_path,
        ])
        .status()
        .map_err(|e| format!("gpg: {e}"))?;

    let _ = fs::remove_file(&data_path);

    if !status.success() {
        let _ = fs::remove_file(&sig_path);
        return Err("GPG signing failed".into());
    }

    // Store the signature as a .commitmeta object.
    let sig_data = fs::read(&sig_path).map_err(|e| format!("read signature: {e}"))?;
    let _ = fs::remove_file(&sig_path);

    let sig_obj_dir = ostree_repo.join("objects").join(&commit_checksum[..2]);
    let sig_obj_path = sig_obj_dir.join(format!("{}.commitmeta", &commit_checksum[2..]));
    let _ = fs::create_dir_all(&sig_obj_dir);
    fs::write(&sig_obj_path, &sig_data).map_err(|e| format!("write signature: {e}"))?;

    eprintln!("Signed {ref_name} (commit {commit_checksum}) with key {key_id}");
    Ok(())
}

/// Show repository information.
pub fn repo_info(repo_path: &Path) -> Result<(), String> {
    if !repo_path.exists() {
        return Err(format!("repo not found: {}", repo_path.display()));
    }

    println!("Repository: {}", repo_path.display());

    // Count refs.
    let mut app_count = 0;
    let mut runtime_count = 0;

    for kind in &["app", "runtime"] {
        let kind_dir = repo_path.join(kind);
        if !kind_dir.exists() {
            continue;
        }
        for id_entry in fs::read_dir(&kind_dir).into_iter().flatten().flatten() {
            for arch_entry in fs::read_dir(id_entry.path())
                .into_iter()
                .flatten()
                .flatten()
            {
                for _branch_entry in fs::read_dir(arch_entry.path())
                    .into_iter()
                    .flatten()
                    .flatten()
                {
                    if *kind == "app" {
                        app_count += 1;
                    } else {
                        runtime_count += 1;
                    }
                }
            }
        }
    }

    println!("  Apps: {app_count}");
    println!("  Runtimes: {runtime_count}");

    // Check for summary.
    let summary = repo_path.join("summary");
    if summary.exists() {
        println!("  Summary: present");
    } else {
        println!("  Summary: missing");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn copy_dir_contents(src: &Path, dest: &Path) {
    if let Ok(entries) = fs::read_dir(src) {
        for entry in entries.flatten() {
            let src_path = entry.path();
            let dest_path = dest.join(entry.file_name());
            if src_path.is_dir() {
                let _ = fs::create_dir_all(&dest_path);
                copy_dir_contents(&src_path, &dest_path);
            } else {
                let _ = fs::copy(&src_path, &dest_path);
            }
        }
    }
}

fn copy_dir_recursive(src: &Path, dest: &Path) {
    let _ = fs::create_dir_all(dest);
    if let Ok(entries) = fs::read_dir(src) {
        for entry in entries.flatten() {
            let src_path = entry.path();
            let dest_path = dest.join(entry.file_name());
            if src_path.is_dir() {
                copy_dir_recursive(&src_path, &dest_path);
            } else {
                let _ = fs::copy(&src_path, &dest_path);
                if let Ok(meta) = fs::metadata(&src_path) {
                    let _ = fs::set_permissions(&dest_path, meta.permissions());
                }
            }
        }
    }
}
