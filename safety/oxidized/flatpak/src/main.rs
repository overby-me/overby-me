// rust-flatpak: A Flatpak-compatible application sandboxing and distribution tool.
//
// Implements the core Flatpak CLI for running, installing, listing, and
// managing sandboxed applications. Uses bwrap (rust-bubblewrap) for
// sandboxing.

mod build;
mod dbus_proxy;
mod deltas;
mod extensions;
mod gvariant;
mod installation;
mod instance;
mod metadata;
mod ostree;
mod portals;
mod sandbox;
mod seccomp;

use std::env;
use std::fs;
use std::path::Path;
use std::process;

use installation::{Installation, Ref, RefKind, Remote};

pub const FLATPAK_VERSION: &str = "0.1.0";

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        print_usage();
        process::exit(1);
    }

    // Parse global options.
    let mut user_mode = false;
    let mut system_mode = false;
    let mut verbose = false;
    let mut cmd_start = 0;

    for (i, arg) in args.iter().enumerate() {
        match arg.as_str() {
            "--user" | "-u" => user_mode = true,
            "--system" => system_mode = true,
            "--verbose" | "-v" => verbose = true,
            "--help" | "-h" => {
                print_usage();
                process::exit(0);
            }
            "--version" => {
                println!("Flatpak {FLATPAK_VERSION} (rust-flatpak)");
                process::exit(0);
            }
            _ if !arg.starts_with('-') => {
                cmd_start = i;
                break;
            }
            _ => {
                cmd_start = i;
                break;
            }
        }
        cmd_start = i + 1;
    }

    if cmd_start >= args.len() {
        print_usage();
        process::exit(1);
    }

    let installations = if user_mode {
        vec![Installation::user()]
    } else if system_mode {
        vec![Installation::system()]
    } else {
        Installation::all()
    };

    let command = &args[cmd_start];
    let cmd_args = &args[cmd_start + 1..];

    match command.as_str() {
        "run" => cmd_run(&installations, cmd_args, verbose),
        "list" => cmd_list(&installations, cmd_args),
        "info" => cmd_info(&installations, cmd_args),
        "install" => cmd_install(&installations, cmd_args),
        "uninstall" | "remove" => cmd_uninstall(&installations, cmd_args),
        "update" | "upgrade" => cmd_update(&installations, cmd_args),
        "override" => cmd_override(&installations, cmd_args),
        "remotes" | "remote-list" => cmd_remote_list(&installations, cmd_args),
        "remote-add" => cmd_remote_add(&installations, cmd_args),
        "remote-delete" => cmd_remote_delete(&installations, cmd_args),
        "remote-info" => cmd_remote_info(&installations, cmd_args),
        "remote-ls" => cmd_remote_ls(&installations, cmd_args),
        "ps" => cmd_ps(),
        "kill" => cmd_kill(cmd_args),
        "enter" => cmd_enter(cmd_args),
        "search" => cmd_search(cmd_args),
        "history" => cmd_history(&installations),
        "config" => cmd_config(&installations, cmd_args),
        "repair" => cmd_repair(&installations),
        "documents" | "document-list" => cmd_documents(cmd_args),
        "document-export" => cmd_document_export(cmd_args),
        "document-unexport" => cmd_document_unexport(cmd_args),
        "document-info" => cmd_document_info(cmd_args),
        "permissions" | "permission-list" => cmd_permissions(cmd_args),
        "permission-show" => cmd_permission_show(cmd_args),
        "permission-set" => cmd_permission_set(cmd_args),
        "permission-remove" => cmd_permission_remove(cmd_args),
        "permission-reset" => cmd_permission_reset(cmd_args),
        "make-current" => cmd_make_current(&installations, cmd_args),
        "mask" => cmd_mask(&installations, cmd_args),
        "pin" => cmd_pin(&installations, cmd_args),
        "build-init" => cmd_build_init(cmd_args),
        "build" => cmd_build(&installations, cmd_args),
        "build-finish" => cmd_build_finish(cmd_args),
        "build-export" => cmd_build_export(cmd_args),
        "build-bundle" => cmd_build_bundle(cmd_args),
        "build-import-bundle" => cmd_build_import_bundle(&installations, cmd_args),
        "build-sign" => cmd_build_sign(cmd_args),
        "build-update-repo" => cmd_build_update_repo(cmd_args),
        "build-commit-from" => cmd_build_commit_from(cmd_args),
        "repo" => cmd_repo(cmd_args),
        "create-usb" => cmd_create_usb(&installations, cmd_args),
        "complete" => cmd_complete(cmd_args),
        "help" => {
            print_usage();
            process::exit(0);
        }
        _ => {
            eprintln!("flatpak: unknown command '{command}'");
            eprintln!("Try 'flatpak --help' for more information.");
            process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Command: run
// ---------------------------------------------------------------------------

fn cmd_run(installations: &[Installation], args: &[String], verbose: bool) {
    let mut app_id: Option<String> = None;
    let mut command_override: Option<String> = None;
    let mut devel = false;
    let mut sandbox_mode = false;
    let mut extra_args: Vec<String> = Vec::new();
    let mut cap_ops: Vec<sandbox::CapOp> = Vec::new();
    let mut past_separator = false;

    let mut i = 0;
    while i < args.len() {
        if past_separator {
            extra_args.push(args[i].clone());
            i += 1;
            continue;
        }
        match args[i].as_str() {
            "--" => past_separator = true,
            "--command" => {
                i += 1;
                if i < args.len() {
                    command_override = Some(args[i].clone());
                }
            }
            s if s.starts_with("--command=") => {
                command_override = Some(s.strip_prefix("--command=").unwrap().to_string());
            }
            "--devel" | "-d" => devel = true,
            "--sandbox" => sandbox_mode = true,
            "--cap-add" => {
                i += 1;
                if i < args.len() {
                    cap_ops.push(sandbox::CapOp::Add(args[i].clone()));
                }
            }
            "--cap-drop" => {
                i += 1;
                if i < args.len() {
                    cap_ops.push(sandbox::CapOp::Drop(args[i].clone()));
                }
            }
            s if s.starts_with('-') => {
                // Pass through other options as extra args.
                extra_args.push(args[i].clone());
            }
            _ => {
                if app_id.is_none() {
                    app_id = Some(args[i].clone());
                } else {
                    extra_args.push(args[i].clone());
                }
            }
        }
        i += 1;
    }

    let app_id = app_id.unwrap_or_else(|| {
        eprintln!("flatpak run: no application specified");
        process::exit(1);
    });

    // Find the app in installations.
    let deployed = find_deployed(installations, &app_id);

    // Find the runtime.
    let runtime_deployed = if let Some(rt_ref_str) = deployed.metadata.runtime() {
        if let Some(rt_ref) = Ref::parse(rt_ref_str) {
            let mut found = None;
            for inst in installations {
                if let Some(d) = inst.find_ref_by_string(&rt_ref.to_string()) {
                    found = Some(d);
                    break;
                }
            }
            found
        } else {
            None
        }
    } else {
        None
    };

    if verbose {
        eprintln!(
            "flatpak: running {} ({})",
            app_id,
            deployed.ref_.format_ref()
        );
        if let Some(ref rt) = runtime_deployed {
            eprintln!("flatpak: using runtime {}", rt.ref_.format_ref());
        }
    }

    // Create instance tracking first (needed for proxy instance IDs).
    let instance_info = sandbox::get_flatpak_info(&deployed, runtime_deployed.as_ref());
    let instance_id = instance::create_instance(&instance_info).unwrap_or_default();

    let mut setup = sandbox::build_sandbox(
        &deployed,
        runtime_deployed.as_ref(),
        &extra_args,
        command_override.as_deref(),
        devel,
        sandbox_mode,
        &cap_ops,
        &instance_id,
    )
    .unwrap_or_else(|e| {
        eprintln!("flatpak run: {e}");
        instance::cleanup_instance(&instance_id);
        process::exit(1);
    });

    if verbose {
        eprintln!("flatpak: executing bwrap");
    }

    // Spawn bwrap (not wait) so we can read the info pipe.
    let mut child = setup.command.spawn().unwrap_or_else(|e| {
        eprintln!("flatpak run: failed to execute bwrap: {e}");
        instance::cleanup_instance(&instance_id);
        instance::cleanup_temp_files();
        process::exit(1);
    });

    // Read child PID from the info pipe.
    if let Some(read_fd) = setup.info_pipe_read {
        let mut buf = [0u8; 256];
        let n = unsafe { libc::read(read_fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        unsafe { libc::close(read_fd) };
        if n > 0 {
            let info_str = String::from_utf8_lossy(&buf[..n as usize]);
            // Parse {"child-pid": N}
            if let Some(pid_str) = info_str
                .split("\"child-pid\":")
                .nth(1)
                .and_then(|s| s.trim().strip_suffix('}'))
                .or_else(|| {
                    info_str
                        .split("\"child-pid\":")
                        .nth(1)
                        .and_then(|s| s.split('}').next())
                })
                && let Ok(pid) = pid_str.trim().parse::<u32>()
            {
                let _ = instance::write_pid(&instance_id, pid);
                if verbose {
                    eprintln!("flatpak: sandbox PID {pid}");
                }
            }
            // Also save bwrapinfo.
            let _ = instance::write_bwrap_info(&instance_id, &info_str);
        }
    }

    let status = child.wait().unwrap_or_else(|e| {
        eprintln!("flatpak run: wait failed: {e}");
        instance::cleanup_instance(&instance_id);
        instance::cleanup_temp_files();
        process::exit(1);
    });

    // Clean up instance, proxies, and temp files.
    // Proxies are cleaned up when `setup` is dropped.
    drop(setup);
    instance::cleanup_instance(&instance_id);
    instance::cleanup_temp_files();

    process::exit(status.code().unwrap_or(1));
}

// ---------------------------------------------------------------------------
// Command: list
// ---------------------------------------------------------------------------

fn cmd_list(installations: &[Installation], args: &[String]) {
    let show_runtime = args.contains(&"--runtime".to_string());
    let show_app = args.contains(&"--app".to_string()) || !show_runtime;
    let show_all = !args.contains(&"--app".to_string()) && !args.contains(&"--runtime".to_string());
    let columns = args.iter().any(|a| a.starts_with("--columns"));
    let arch_filter: Option<&str> = args.iter().find_map(|a| a.strip_prefix("--arch="));

    println!(
        "{:<40} {:<12} {:<12} Installation",
        "Name", "Branch", "Arch"
    );
    println!("{}", "-".repeat(80));

    for inst in installations {
        for deployed in inst.list_refs() {
            let is_app = deployed.ref_.kind == RefKind::App;
            if let Some(arch) = arch_filter
                && deployed.ref_.arch != arch
            {
                continue;
            }
            if show_all || (show_app && is_app) || (show_runtime && !is_app) {
                let inst_name = if inst.is_user { "user" } else { "system" };
                if columns {
                    println!(
                        "{:<40} {:<12} {:<12} {:<10} {}",
                        deployed.ref_.id,
                        deployed.ref_.branch,
                        deployed.ref_.arch,
                        inst_name,
                        deployed.ref_.format_ref()
                    );
                } else {
                    println!(
                        "{:<40} {:<12} {:<12} {}",
                        deployed.ref_.id, deployed.ref_.branch, deployed.ref_.arch, inst_name
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Command: info
// ---------------------------------------------------------------------------

fn cmd_info(installations: &[Installation], args: &[String]) {
    if args.is_empty() {
        eprintln!("flatpak info: no application specified");
        process::exit(1);
    }

    let show_metadata =
        args.contains(&"--show-metadata".to_string()) || args.contains(&"-m".to_string());
    let show_permissions = args.contains(&"--show-permissions".to_string());
    let show_commit =
        args.contains(&"--show-commit".to_string()) || args.contains(&"-c".to_string());
    let show_location =
        args.contains(&"--show-location".to_string()) || args.contains(&"-l".to_string());
    let show_runtime =
        args.contains(&"--show-runtime".to_string()) || args.contains(&"-r".to_string());
    let show_sdk = args.contains(&"--show-sdk".to_string());
    let show_extensions = args.contains(&"--show-extensions".to_string());
    let file_access_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--file-access="))
        .map(String::from)
        .or_else(|| {
            let mut it = args.iter();
            while let Some(a) = it.next() {
                if a == "--file-access" {
                    return it.next().cloned();
                }
            }
            None
        });
    let app_id = args.last().unwrap();

    let deployed = find_deployed(installations, app_id);

    if show_metadata {
        println!("{}", deployed.metadata.serialize());
        return;
    }

    // Single-value output flags — print just the value and return.
    if show_commit {
        // Look for a commit file or derive from deploy path.
        let commit_path = deployed.path.join("commit");
        if let Ok(c) = std::fs::read_to_string(&commit_path) {
            println!("{}", c.trim());
        } else {
            // No stored commit; print the deploy path basename as fallback.
            println!(
                "{}",
                deployed
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            );
        }
        return;
    }
    if show_location {
        println!("{}", deployed.path.display());
        return;
    }
    if show_runtime {
        if let Some(rt) = deployed.metadata.runtime() {
            println!("{rt}");
        } else {
            eprintln!("flatpak info: no runtime specified");
            process::exit(1);
        }
        return;
    }
    if show_sdk {
        if let Some(sdk) = deployed
            .metadata
            .get("Application", "sdk")
            .or_else(|| deployed.metadata.get("Runtime", "sdk"))
        {
            println!("{sdk}");
        } else if let Some(rt) = deployed.metadata.runtime() {
            // Fall back: derive SDK from runtime by replacing Platform with Sdk.
            println!("{}", rt.replace("Platform", "Sdk"));
        } else {
            eprintln!("flatpak info: no SDK specified");
            process::exit(1);
        }
        return;
    }

    if show_extensions {
        // List extension points from metadata [Extension ...] groups.
        let mut found = false;
        for group_name in deployed.metadata.groups.keys() {
            if let Some(ext_id) = group_name.strip_prefix("Extension ") {
                println!("{ext_id}");
                found = true;
            }
        }
        if !found {
            println!("No extensions");
        }
        return;
    }

    if let Some(ref path) = file_access_path {
        // Report the effective access level for a given path.
        let ctx = deployed.metadata.context();
        // Merge overrides if available.
        let inst = &deployed.installation;
        let override_path = inst.override_path(&deployed.ref_.id);
        let mut merged = ctx.clone();
        if let Ok(ovr) = metadata::Metadata::from_file(&override_path) {
            let ovr_ctx = ovr.context();
            merged.merge(&ovr_ctx);
        }

        // Check filesystems for the requested path.
        let mut access = "hidden";
        for fs_spec in &merged.filesystems {
            let (fs_path, ro) = if let Some(s) = fs_spec.strip_suffix(":ro") {
                (s, true)
            } else if let Some(s) = fs_spec.strip_suffix(":rw") {
                (s, false)
            } else if let Some(s) = fs_spec.strip_suffix(":create") {
                (s, false)
            } else {
                (fs_spec.as_str(), false)
            };
            // Negated entries.
            if let Some(neg) = fs_path.strip_prefix('!') {
                if path == neg || path.starts_with(&format!("{neg}/")) {
                    access = "hidden";
                }
                continue;
            }
            let resolved = match fs_path {
                "home" | "~" => "home",
                "host" => "/",
                "host-os" => "/usr",
                "host-etc" => "/etc",
                _ => fs_path,
            };
            if path == resolved || path.starts_with(&format!("{resolved}/")) || resolved == "/" {
                access = if ro { "read-only" } else { "read-write" };
            }
        }
        println!("{access}");
        return;
    }

    println!("  ID: {}", deployed.ref_.id);
    println!("  Ref: {}", deployed.ref_.format_ref());
    println!("  Arch: {}", deployed.ref_.arch);
    println!("  Branch: {}", deployed.ref_.branch);
    if let Some(cmd) = deployed.metadata.command() {
        println!("  Command: {cmd}");
    }
    if let Some(rt) = deployed.metadata.runtime() {
        println!("  Runtime: {rt}");
    }
    let inst_name = if deployed.installation.is_user {
        "user"
    } else {
        "system"
    };
    println!("  Installation: {inst_name}");
    println!("  Location: {}", deployed.path.display());

    if show_permissions {
        println!();
        let ctx = deployed.metadata.context();
        if !ctx.shared.is_empty() {
            println!("  Shared: {}", ctx.shared.join(", "));
        }
        if !ctx.sockets.is_empty() {
            println!("  Sockets: {}", ctx.sockets.join(", "));
        }
        if !ctx.devices.is_empty() {
            println!("  Devices: {}", ctx.devices.join(", "));
        }
        if !ctx.features.is_empty() {
            println!("  Features: {}", ctx.features.join(", "));
        }
        if !ctx.filesystems.is_empty() {
            println!("  Filesystems: {}", ctx.filesystems.join(", "));
        }
    }
}

// ---------------------------------------------------------------------------
// Command: install (from local directory)
// ---------------------------------------------------------------------------

fn cmd_install(installations: &[Installation], args: &[String]) {
    let mut source: Option<String> = None;
    let mut ref_str: Option<String> = None;
    let mut _reinstall = false;
    let mut subpaths: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "--reinstall" => _reinstall = true,
            "--subpath" => {
                i += 1;
                if i < args.len() {
                    subpaths.push(args[i].clone());
                }
            }
            s if s.starts_with("--subpath=") => {
                subpaths.push(s.strip_prefix("--subpath=").unwrap().to_string());
            }
            s if s.starts_with('-') => {}
            _ => {
                if source.is_none() {
                    source = Some(arg.clone());
                } else if ref_str.is_none() {
                    ref_str = Some(arg.clone());
                }
            }
        }
        i += 1;
    }

    // Determine what to install.
    // If source is a directory with a metadata file, install from local build.
    // If it looks like a remote name + ref, note it for future implementation.
    let source = source.unwrap_or_else(|| {
        eprintln!("flatpak install: usage: flatpak install <source-dir|remote> [REF]");
        process::exit(1);
    });

    let source_path = Path::new(&source);
    if source_path.is_dir()
        && (source_path.join("metadata").exists() || source_path.join("files").exists())
    {
        install_from_dir(installations, source_path, ref_str.as_deref(), &subpaths);
    } else if let Some(ref ref_name) = ref_str {
        // Install from a remote.
        install_from_remote(installations, &source, ref_name, &subpaths);
    } else {
        // Maybe `source` is a remote name and ref_str wasn't provided.
        // Try to find a remote with this name.
        let mut found_remote = false;
        for inst in installations {
            let remotes = installation::load_remotes(inst);
            if remotes.iter().any(|r| r.name == source) {
                found_remote = true;
                break;
            }
        }
        if found_remote {
            eprintln!("flatpak install: usage: flatpak install <remote> <ref>");
            eprintln!("Specify the ref to install (use 'flatpak remote-ls {source}' to list)");
        } else {
            eprintln!("flatpak install: '{}' is not a valid source", source);
            eprintln!("Expected a build directory, or: flatpak install <remote> <ref>");
        }
        process::exit(1);
    }
}

fn install_from_remote(
    installations: &[Installation],
    remote_name: &str,
    ref_name: &str,
    _subpaths: &[String],
) {
    let remote = find_remote(installations, remote_name);
    let inst = &installations[0]; // Install to first (user) installation.

    eprintln!("Looking for {ref_name} on {remote_name}...");

    // Parse the ref to determine kind, id, arch, branch.
    let parsed_ref = if let Some(r) = Ref::parse(ref_name) {
        r
    } else {
        // Search for a matching ref in the summary.
        let refs = ostree::fetch_summary(&remote.url).unwrap_or_else(|e| {
            eprintln!("flatpak install: {e}");
            process::exit(1);
        });

        // Try to find an app ref matching the name.
        let arch = std::env::consts::ARCH;

        let matching: Vec<_> = refs
            .iter()
            .filter(|r| r.name.contains(ref_name) && r.name.contains(arch))
            .collect();

        if matching.is_empty() {
            eprintln!("flatpak install: no matching ref for '{ref_name}'");
            process::exit(1);
        }
        if matching.len() > 1 {
            eprintln!("flatpak install: multiple matches for '{ref_name}':");
            for m in &matching {
                eprintln!("  {}", m.name);
            }
            eprintln!("Specify the full ref name.");
            process::exit(1);
        }

        Ref::parse(&matching[0].name).unwrap_or_else(|| {
            eprintln!(
                "flatpak install: could not parse ref '{}'",
                matching[0].name
            );
            process::exit(1);
        })
    };

    let deploy_path = inst.deploy_path(&parsed_ref);
    let _ = fs::create_dir_all(&deploy_path);

    let ref_str = parsed_ref.format_ref();
    eprintln!("Installing {ref_str}...");

    // Pull the ref using the OSTree client.
    let commit = ostree::pull_ref(&remote.url, &ref_str, &deploy_path, true).unwrap_or_else(|e| {
        eprintln!("flatpak install: pull failed: {e}");
        let _ = fs::remove_dir_all(&deploy_path);
        process::exit(1);
    });

    // Store the commit checksum so `info --show-commit` can retrieve it.
    let _ = fs::write(deploy_path.join("commit"), &commit);

    // The checkout puts files into deploy_path/files/. We also need the metadata
    // file. OSTree Flatpak repos store metadata as a file in the root tree.
    // If metadata file exists in the checkout, move it up.
    let checkout_metadata = deploy_path.join("files").join("metadata");
    let target_metadata = deploy_path.join("metadata");
    if checkout_metadata.exists() && !target_metadata.exists() {
        let _ = fs::rename(&checkout_metadata, &target_metadata);
    }

    // If no metadata was found in the repo, create a minimal one.
    if !target_metadata.exists() {
        let kind = if parsed_ref.kind == RefKind::App {
            "Application"
        } else {
            "Runtime"
        };
        let meta_content = format!(
            "[{kind}]\nname={}\nruntime=org.freedesktop.Platform/x86_64/23.08\n",
            parsed_ref.id
        );
        let _ = fs::write(&target_metadata, meta_content);
    }

    println!(
        "Installation complete: {} ({}/{})",
        parsed_ref.id, parsed_ref.arch, parsed_ref.branch
    );
    log_history(installations, "install", &parsed_ref.format_ref());
}

fn install_from_dir(
    installations: &[Installation],
    source: &Path,
    ref_override: Option<&str>,
    subpaths: &[String],
) {
    let metadata_path = source.join("metadata");
    let metadata = if metadata_path.exists() {
        metadata::Metadata::from_file(&metadata_path).unwrap_or_else(|e| {
            eprintln!("flatpak install: {e}");
            process::exit(1);
        })
    } else {
        eprintln!("flatpak install: no metadata file in {}", source.display());
        process::exit(1);
    };

    let app_name = metadata.app_name().unwrap_or_else(|| {
        eprintln!("flatpak install: metadata has no name");
        process::exit(1);
    });

    // Check required-flatpak version requirement.
    let required = metadata
        .get("Application", "required-flatpak")
        .or_else(|| metadata.get("Runtime", "required-flatpak"));
    if let Some(req) = required
        && version_less(FLATPAK_VERSION, req)
    {
        eprintln!(
            "flatpak install: this app needs Flatpak >= {req}, but you have {FLATPAK_VERSION}"
        );
        process::exit(1);
    }

    let kind = if metadata.is_app() {
        RefKind::App
    } else {
        RefKind::Runtime
    };

    let arch = std::env::consts::ARCH.to_string();
    let arch = match arch.as_str() {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        "x86" => "i386",
        a => a,
    };

    let branch = ref_override.unwrap_or("stable");

    let ref_ = Ref {
        kind,
        id: app_name.to_string(),
        arch: arch.to_string(),
        branch: branch.to_string(),
    };

    // Install to user installation.
    let inst = &installations[0];
    let deploy_path = inst.deploy_path(&ref_);
    let _ = fs::create_dir_all(&deploy_path);

    // Copy metadata.
    let _ = fs::copy(source.join("metadata"), deploy_path.join("metadata"));

    // Copy files directory (honoring --subpath filters).
    let src_files = source.join("files");
    let dest_files = deploy_path.join("files");
    if src_files.exists() {
        if subpaths.is_empty() {
            copy_dir_recursive(&src_files, &dest_files);
        } else {
            for sub in subpaths {
                let rel = sub.trim_start_matches('/');
                let src_sub = src_files.join(rel);
                let dest_sub = dest_files.join(rel);
                if src_sub.is_dir() {
                    copy_dir_recursive(&src_sub, &dest_sub);
                } else if src_sub.is_file() {
                    if let Some(parent) = dest_sub.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    let _ = fs::copy(&src_sub, &dest_sub);
                }
            }
            // Record subpaths so the installation knows it is partial.
            let _ = fs::write(deploy_path.join("subpaths"), subpaths.join("\n"));
        }
    }

    // Copy export directory.
    let src_export = source.join("export");
    let dest_export = deploy_path.join("export");
    if src_export.exists() {
        copy_dir_recursive(&src_export, &dest_export);
    }

    println!("Installation complete: {} ({}/{})", app_name, arch, branch);
    log_history(installations, "install", &ref_.format_ref());

    // Install desktop file and icons to export location.
    export_app(inst, &ref_, &deploy_path);
}

fn export_app(inst: &Installation, _ref: &installation::Ref, deploy_path: &Path) {
    let export_dir = deploy_path.join("export");
    if !export_dir.exists() {
        return;
    }

    // Link desktop files.
    let desktop_src = export_dir.join("share/applications");
    if desktop_src.exists() {
        let dest = inst.path.join("exports/share/applications");
        let _ = fs::create_dir_all(&dest);
        if let Ok(entries) = fs::read_dir(&desktop_src) {
            for entry in entries.flatten() {
                let src = entry.path();
                let dest_file = dest.join(entry.file_name());
                let _ = fs::copy(&src, &dest_file);
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
                // Preserve executable bit.
                if let Ok(meta) = fs::metadata(&src_path) {
                    let _ = fs::set_permissions(&dest_path, meta.permissions());
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Command: uninstall
// ---------------------------------------------------------------------------

fn cmd_uninstall(installations: &[Installation], args: &[String]) {
    if args.is_empty() {
        eprintln!("flatpak uninstall: no application specified");
        process::exit(1);
    }

    let delete_data = args.contains(&"--delete-data".to_string());
    let app_id = args.iter().find(|a| !a.starts_with('-')).unwrap();

    let deployed = find_deployed(installations, app_id);
    let ref_ = &deployed.ref_;
    let inst = &deployed.installation;

    // Remove the deployment.
    let deploy_path = inst.deploy_path(ref_);
    if deploy_path.exists() {
        let _ = fs::remove_dir_all(&deploy_path);
    }

    // Clean up empty parent dirs.
    let ref_dir = deploy_path
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let _ = fs::remove_dir(deploy_path.parent().unwrap()); // branch dir
    let _ = fs::remove_dir(ref_dir.join(&ref_.arch)); // arch dir
    let _ = fs::remove_dir(ref_dir); // id dir

    if delete_data {
        let data_dir = Installation::app_data_dir(&ref_.id);
        if data_dir.exists() {
            let _ = fs::remove_dir_all(&data_dir);
            println!("Removed app data: {}", data_dir.display());
        }
    }

    log_history(std::slice::from_ref(inst), "uninstall", &ref_.format_ref());
    println!("Uninstalled: {}", ref_.format_ref());
}

// ---------------------------------------------------------------------------
// Command: update (stub)
// ---------------------------------------------------------------------------

fn cmd_update(installations: &[Installation], args: &[String]) {
    // Parse --bundle=PATH for re-importing from a bundle file.
    let mut bundle_path: Option<String> = None;
    for arg in args {
        if let Some(p) = arg.strip_prefix("--bundle=") {
            bundle_path = Some(p.to_string());
        }
    }

    if let Some(bp) = bundle_path {
        let path = Path::new(&bp);
        if !path.exists() {
            eprintln!("flatpak update: bundle not found: {bp}");
            process::exit(1);
        }
        let ref_str = build::build_import_bundle(installations, path).unwrap_or_else(|e| {
            eprintln!("flatpak update: {e}");
            process::exit(1);
        });
        println!("Updated from bundle: {ref_str}");
        log_history(installations, "update", &ref_str);
        return;
    }

    let mut updated = 0;
    for inst in installations {
        let remotes = installation::load_remotes(inst);
        let deployed_refs = inst.list_refs();

        for deployed in &deployed_refs {
            // Find which remote has this ref.
            for remote in &remotes {
                let ref_str = deployed.ref_.format_ref();
                let summary = match ostree::fetch_summary(&remote.url) {
                    Ok(refs) => refs,
                    Err(_) => continue,
                };
                let Some(remote_ref) = summary.iter().find(|r| r.name == ref_str) else {
                    continue;
                };
                eprintln!("Checking {ref_str} on {}...", remote.name);

                // Compare commit checksums.
                let local_commit = std::fs::read_to_string(deployed.path.join("commit"))
                    .ok()
                    .map(|s| s.trim().to_string());
                if local_commit.as_deref() == Some(remote_ref.checksum.as_str()) {
                    eprintln!("  already up-to-date ({})", &remote_ref.checksum[..12]);
                    break;
                }

                eprintln!("  updating to commit {}...", &remote_ref.checksum[..12]);

                // Pull and overwrite the deployment.
                match ostree::pull_ref(&remote.url, &ref_str, &deployed.path, true) {
                    Ok(commit) => {
                        let _ = std::fs::write(deployed.path.join("commit"), &commit);
                        // Re-extract metadata from the new files/ if present.
                        let checkout_metadata = deployed.path.join("files").join("metadata");
                        let target_metadata = deployed.path.join("metadata");
                        if checkout_metadata.exists() {
                            let _ = std::fs::copy(&checkout_metadata, &target_metadata);
                        }
                        println!("Updated {ref_str} ({})", &commit[..12]);
                        log_history(installations, "update", &ref_str);
                        updated += 1;
                    }
                    Err(e) => {
                        eprintln!("  update failed: {e}");
                    }
                }
                break;
            }
        }
    }
    if updated == 0 {
        println!("Nothing to update.");
    }
}

// ---------------------------------------------------------------------------
// Command: override
// ---------------------------------------------------------------------------

fn cmd_override(installations: &[Installation], args: &[String]) {
    let mut app_id: Option<String> = None;
    let mut overrides: Vec<(&'static str, String)> = Vec::new();
    let mut do_reset = false;

    // Helper: extract the value for a flag in either `--flag=value` or `--flag value` form.
    fn val_of(arg: &str, name: &str, args: &[String], i: usize) -> Option<(String, usize)> {
        let prefix = format!("--{name}=");
        if let Some(v) = arg.strip_prefix(&prefix) {
            Some((v.to_string(), 0))
        } else if arg == format!("--{name}") {
            args.get(i + 1).map(|v| (v.clone(), 1))
        } else {
            None
        }
    }

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];

        // Handle valued flags (support both `--flag=value` and `--flag value`).
        let mut handled = false;
        for (flag, group, prefix) in [
            ("filesystem", "filesystems", ""),
            ("nofilesystem", "filesystems", "!"),
            ("share", "shared", ""),
            ("unshare", "shared", "!"),
            ("socket", "sockets", ""),
            ("nosocket", "sockets", "!"),
            ("device", "devices", ""),
            ("nodevice", "devices", "!"),
            ("allow", "features", ""),
            ("disallow", "features", "!"),
            ("persist", "persistent", ""),
            ("talk-name", "session-bus-talk", ""),
            ("no-talk-name", "session-bus-talk", "!"),
            ("own-name", "session-bus-own", ""),
            ("no-own-name", "session-bus-own", "!"),
            ("system-talk-name", "system-bus-talk", ""),
            ("no-system-talk-name", "system-bus-talk", "!"),
            ("system-own-name", "system-bus-own", ""),
            ("no-system-own-name", "system-bus-own", "!"),
            ("env", "env", ""),
        ] {
            if let Some((v, adv)) = val_of(arg, flag, args, i) {
                overrides.push((group, format!("{prefix}{v}")));
                i += adv;
                handled = true;
                break;
            }
        }
        if handled {
            i += 1;
            continue;
        }

        match arg.as_str() {
            "--reset" => do_reset = true,
            s if !s.starts_with('-') => app_id = Some(s.to_string()),
            _ => {}
        }
        i += 1;
    }

    if do_reset && let Some(ref id) = app_id {
        let inst = &installations[0];
        let path = inst.override_path(id);
        let _ = fs::remove_file(&path);
        println!("Reset overrides for {id}");
        return;
    }

    let app_id = app_id.unwrap_or_else(|| {
        eprintln!("flatpak override: no application specified");
        process::exit(1);
    });

    let inst = &installations[0];
    let override_path = inst.override_path(&app_id);
    let _ = fs::create_dir_all(inst.overrides_dir());

    // Load existing overrides.
    let mut meta = if override_path.exists() {
        metadata::Metadata::from_file(&override_path).unwrap_or_default()
    } else {
        metadata::Metadata::default()
    };

    // Apply new overrides.
    for (key, val) in &overrides {
        match *key {
            "env" => {
                let env_group = meta.groups.entry("Environment".to_string()).or_default();
                if let Some((k, v)) = val.split_once('=') {
                    env_group.insert(k.to_string(), v.to_string());
                }
            }
            "session-bus-talk" | "session-bus-own" => {
                let group = meta
                    .groups
                    .entry("Session Bus Policy".to_string())
                    .or_default();
                let (name, policy) = if let Some(stripped) = val.strip_prefix('!') {
                    (stripped, "none".to_string())
                } else {
                    let policy = if *key == "session-bus-own" {
                        "own"
                    } else {
                        "talk"
                    };
                    (val.as_str(), policy.to_string())
                };
                group.insert(name.to_string(), policy);
            }
            "system-bus-talk" | "system-bus-own" => {
                let group = meta
                    .groups
                    .entry("System Bus Policy".to_string())
                    .or_default();
                let (name, policy) = if let Some(stripped) = val.strip_prefix('!') {
                    (stripped, "none".to_string())
                } else {
                    let policy = if *key == "system-bus-own" {
                        "own"
                    } else {
                        "talk"
                    };
                    (val.as_str(), policy.to_string())
                };
                group.insert(name.to_string(), policy);
            }
            _ => {
                let ctx = meta.groups.entry("Context".to_string()).or_default();
                let existing = ctx.entry((*key).to_string()).or_default();
                if !existing.is_empty() {
                    existing.push(';');
                }
                existing.push_str(val);
            }
        }
    }

    // Save.
    let content = meta.serialize();
    fs::write(&override_path, content).unwrap_or_else(|e| {
        eprintln!("flatpak override: write failed: {e}");
        process::exit(1);
    });

    println!("Overrides saved for {app_id}");
}

// ---------------------------------------------------------------------------
// Command: remote-list
// ---------------------------------------------------------------------------

fn cmd_remote_list(installations: &[Installation], _args: &[String]) {
    println!("{:<20} {:<50} Options", "Name", "URL");
    println!("{}", "-".repeat(80));
    for inst in installations {
        let remotes = installation::load_remotes(inst);
        for remote in remotes {
            let opts = if inst.is_user { "user" } else { "system" };
            println!("{:<20} {:<50} {}", remote.name, remote.url, opts);
        }
    }
}

// ---------------------------------------------------------------------------
// Command: remote-add
// ---------------------------------------------------------------------------

fn cmd_remote_add(installations: &[Installation], args: &[String]) {
    let mut name: Option<String> = None;
    let mut url: Option<String> = None;
    let mut title: Option<String> = None;
    let mut from_file: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            s if s.starts_with("--title=") => {
                title = Some(s.strip_prefix("--title=").unwrap().to_string());
            }
            "--from" => {
                // The next positional arg is the .flatpakrepo file path/URL.
                from_file = Some("next-positional".to_string());
            }
            s if s.starts_with("--if-not-exists") => {}
            s if s.starts_with('-') => {}
            _ => {
                if from_file.as_deref() == Some("next-positional") {
                    from_file = Some(args[i].clone());
                } else if name.is_none() {
                    name = Some(args[i].clone());
                } else if url.is_none() {
                    url = Some(args[i].clone());
                }
            }
        }
        i += 1;
    }

    // Handle .flatpakrepo file.
    if let Some(ref file_path) = from_file
        && file_path != "next-positional"
    {
        let content = if file_path.starts_with("http://") || file_path.starts_with("https://") {
            match ostree::fetch_url(file_path) {
                Ok(data) => String::from_utf8_lossy(&data).to_string(),
                Err(e) => {
                    eprintln!("flatpak remote-add: fetch {file_path}: {e}");
                    process::exit(1);
                }
            }
        } else {
            fs::read_to_string(file_path).unwrap_or_else(|e| {
                eprintln!("flatpak remote-add: read {file_path}: {e}");
                process::exit(1);
            })
        };

        if let Ok(meta) = metadata::Metadata::parse(&content) {
            let repo_group = meta
                .groups
                .get("Flatpak Repo")
                .or_else(|| meta.groups.get("Flatpak Remote"));
            if let Some(group) = repo_group {
                if url.is_none() {
                    url = group.get("Url").or_else(|| group.get("url")).cloned();
                }
                if title.is_none() {
                    title = group.get("Title").or_else(|| group.get("title")).cloned();
                }
                if name.is_none() {
                    // Derive name from the filename.
                    let fname = Path::new(file_path)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("remote");
                    name = Some(fname.to_string());
                }
            }
        }
    }

    let name = name.unwrap_or_else(|| {
        eprintln!("flatpak remote-add: no name specified");
        process::exit(1);
    });
    let url = url.unwrap_or_else(|| {
        eprintln!("flatpak remote-add: no URL specified");
        process::exit(1);
    });

    let inst = &installations[0];
    let mut remotes = installation::load_remotes(inst);

    if remotes.iter().any(|r| r.name == name) {
        eprintln!("flatpak remote-add: remote '{name}' already exists");
        process::exit(1);
    }

    remotes.push(Remote {
        name: name.clone(),
        url: url.clone(),
        title,
        default_branch: None,
    });

    installation::save_remotes(inst, &remotes).unwrap_or_else(|e| {
        eprintln!("flatpak remote-add: {e}");
        process::exit(1);
    });

    println!("Added remote '{name}' ({url})");
}

// ---------------------------------------------------------------------------
// Command: remote-delete
// ---------------------------------------------------------------------------

fn cmd_remote_delete(installations: &[Installation], args: &[String]) {
    let name = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .unwrap_or_else(|| {
            eprintln!("flatpak remote-delete: no name specified");
            process::exit(1);
        });

    let inst = &installations[0];
    let mut remotes = installation::load_remotes(inst);
    let len_before = remotes.len();
    remotes.retain(|r| r.name != *name);

    if remotes.len() == len_before {
        eprintln!("flatpak remote-delete: remote '{name}' not found");
        process::exit(1);
    }

    installation::save_remotes(inst, &remotes).unwrap_or_else(|e| {
        eprintln!("flatpak remote-delete: {e}");
        process::exit(1);
    });

    println!("Deleted remote '{name}'");
}

// ---------------------------------------------------------------------------
// Command: remote-info (stub)
// ---------------------------------------------------------------------------

fn cmd_remote_info(installations: &[Installation], args: &[String]) {
    if args.len() < 2 {
        eprintln!("flatpak remote-info: usage: flatpak remote-info REMOTE REF");
        process::exit(1);
    }
    let remote_name = &args[0];
    let ref_name = &args[1];

    let remote = find_remote(installations, remote_name);
    let refs = ostree::fetch_summary(&remote.url).unwrap_or_else(|e| {
        eprintln!("flatpak remote-info: {e}");
        process::exit(1);
    });

    let found = refs
        .iter()
        .find(|r| r.name == *ref_name || r.name.contains(ref_name));
    match found {
        Some(r) => {
            println!("  Ref: {}", r.name);
            println!("  Commit: {}", r.checksum);
            println!("  Size: {} bytes", r.commit_size);
        }
        None => {
            eprintln!("flatpak remote-info: ref '{ref_name}' not found on {remote_name}");
            process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Command: remote-ls
// ---------------------------------------------------------------------------

fn cmd_remote_ls(installations: &[Installation], args: &[String]) {
    let mut show_apps = true;
    let mut show_runtimes = false;
    let mut remote_name: Option<&String> = None;

    for arg in args {
        match arg.as_str() {
            "--app" => {
                show_apps = true;
                show_runtimes = false;
            }
            "--runtime" => {
                show_apps = false;
                show_runtimes = true;
            }
            "-a" | "--all" => {
                show_apps = true;
                show_runtimes = true;
            }
            s if !s.starts_with('-') => remote_name = Some(arg),
            _ => {}
        }
    }

    let remote_name = remote_name.unwrap_or_else(|| {
        eprintln!("flatpak remote-ls: no remote specified");
        process::exit(1);
    });

    let remote = find_remote(installations, remote_name);
    eprintln!("Fetching refs from {}...", remote.url);

    let refs = ostree::fetch_summary(&remote.url).unwrap_or_else(|e| {
        eprintln!("flatpak remote-ls: {e}");
        process::exit(1);
    });

    for r in &refs {
        let is_app = r.name.starts_with("app/");
        let is_runtime = r.name.starts_with("runtime/");
        if (show_apps && is_app) || (show_runtimes && is_runtime) || (!is_app && !is_runtime) {
            println!("{}", r.name);
        }
    }

    eprintln!("{} refs found", refs.len());
}

fn find_remote(installations: &[Installation], name: &str) -> Remote {
    for inst in installations {
        let remotes = installation::load_remotes(inst);
        if let Some(r) = remotes.into_iter().find(|r| r.name == name) {
            return r;
        }
    }
    eprintln!("flatpak: remote '{name}' not found");
    process::exit(1);
}

// ---------------------------------------------------------------------------
// Command: ps
// ---------------------------------------------------------------------------

fn cmd_ps() {
    let instances = instance::list_instances();
    if instances.is_empty() {
        return;
    }

    println!("{:<12} {:<40} Instance", "PID", "Application");
    println!("{}", "-".repeat(60));

    for inst in &instances {
        let pid = inst.pid.map(|p| p.to_string()).unwrap_or_default();
        let app = inst.app_id.as_deref().unwrap_or("unknown");
        println!("{:<12} {:<40} {}", pid, app, inst.id);
    }
}

// ---------------------------------------------------------------------------
// Command: kill
// ---------------------------------------------------------------------------

fn cmd_kill(args: &[String]) {
    let target = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .unwrap_or_else(|| {
            eprintln!("flatpak kill: no application or instance specified");
            process::exit(1);
        });

    // Try to find by app ID first, then by instance ID.
    if let Some(inst) = instance::find_instance_by_app(target) {
        if let Err(e) = instance::kill_instance(&inst.id, libc::SIGTERM) {
            eprintln!("flatpak kill: {e}");
            process::exit(1);
        }
        println!("Sent SIGTERM to {} (PID {})", target, inst.pid.unwrap_or(0));
    } else if let Err(e) = instance::kill_instance(target, libc::SIGTERM) {
        eprintln!("flatpak kill: {e}");
        process::exit(1);
    } else {
        println!("Sent SIGTERM to instance {target}");
    }
}

// ---------------------------------------------------------------------------
// Command: enter
// ---------------------------------------------------------------------------

fn cmd_enter(args: &[String]) {
    let target = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .unwrap_or_else(|| {
            eprintln!("flatpak enter: no application or instance specified");
            process::exit(1);
        });

    let inst = instance::find_instance_by_app(target).unwrap_or_else(|| {
        // Try instance ID directly.
        instance::list_instances()
            .into_iter()
            .find(|i| i.id == *target)
            .unwrap_or_else(|| {
                eprintln!("flatpak enter: no running instance for '{target}'");
                process::exit(1);
            })
    });

    let pid = inst.pid.unwrap_or_else(|| {
        eprintln!("flatpak enter: instance has no PID");
        process::exit(1);
    });

    // Remaining args after the target are the command to run.
    let cmd_args: Vec<&String> = args
        .iter()
        .skip_while(|a| a.starts_with('-') || *a == target)
        .collect();
    let shell = if cmd_args.is_empty() {
        vec!["sh".to_string()]
    } else {
        cmd_args.iter().map(|s| s.to_string()).collect()
    };

    // Use nsenter to join the sandbox namespaces.
    let status = std::process::Command::new("nsenter")
        .arg("--target")
        .arg(pid.to_string())
        .arg("--mount")
        .arg("--pid")
        .arg("--")
        .args(&shell)
        .status()
        .unwrap_or_else(|e| {
            eprintln!("flatpak enter: nsenter failed: {e}");
            process::exit(1);
        });

    process::exit(status.code().unwrap_or(1));
}

// ---------------------------------------------------------------------------
// Command: search (stub)
// ---------------------------------------------------------------------------

fn cmd_search(args: &[String]) {
    let query = args
        .iter()
        .filter(|a| !a.starts_with('-'))
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    if query.is_empty() {
        eprintln!("flatpak search: no search term specified");
        process::exit(1);
    }

    let query_lower = query.to_lowercase();
    let mut found = false;

    for inst in Installation::all() {
        let remotes = installation::load_remotes(&inst);
        for remote in &remotes {
            match ostree::fetch_summary(&remote.url) {
                Ok(refs) => {
                    for r in &refs {
                        if r.name.to_lowercase().contains(&query_lower)
                            && r.name.starts_with("app/")
                            && let Some(parsed) = Ref::parse(&r.name)
                        {
                            println!(
                                "{:<50} {:<12} {:<10} {}",
                                parsed.id, parsed.branch, parsed.arch, remote.name
                            );
                            found = true;
                        }
                    }
                }
                Err(_) => continue,
            }
        }
    }

    if !found {
        eprintln!("No matches found for '{query}'");
        process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// Command: history
// ---------------------------------------------------------------------------

fn cmd_history(installations: &[Installation]) {
    let inst = &installations[0];
    let log_path = inst.path.join("history.log");
    if let Ok(content) = fs::read_to_string(&log_path) {
        if content.trim().is_empty() {
            println!("No history recorded.");
        } else {
            print!("{content}");
        }
    } else {
        println!("No history recorded.");
    }
}

/// Append an entry to the history log.
fn log_history(installations: &[Installation], action: &str, ref_str: &str) {
    let inst = &installations[0];
    let log_path = inst.path.join("history.log");
    let _ = fs::create_dir_all(&inst.path);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let line = format!("{now}\t{action}\t{ref_str}\n");
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
}

// ---------------------------------------------------------------------------
// Command: config
// ---------------------------------------------------------------------------

fn cmd_config(installations: &[Installation], args: &[String]) {
    let inst = &installations[0];
    let config_path = inst.path.join("config");

    if args.is_empty() {
        for inst in installations {
            let label = if inst.is_user { "user" } else { "system" };
            println!("[{label}]");
            println!("  path: {}", inst.path.display());
            let remotes = installation::load_remotes(inst);
            println!("  remotes: {}", remotes.len());
            let refs = inst.list_refs();
            println!(
                "  installed: {} apps, {} runtimes",
                refs.iter().filter(|r| r.ref_.kind == RefKind::App).count(),
                refs.iter()
                    .filter(|r| r.ref_.kind == RefKind::Runtime)
                    .count(),
            );
        }
        return;
    }

    // Parse --set, --get, --unset subcommands.
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--set" => {
                if i + 2 >= args.len() {
                    eprintln!("flatpak config --set: usage: flatpak config --set KEY VALUE");
                    process::exit(1);
                }
                let key = &args[i + 1];
                let value = &args[i + 2];
                let config = if config_path.exists() {
                    fs::read_to_string(&config_path).unwrap_or_default()
                } else {
                    String::new()
                };
                let updated = set_config_value(&config, "config", key, value);
                let _ = fs::create_dir_all(&inst.path);
                let _ = fs::write(&config_path, &updated);
                return;
            }
            "--get" => {
                if i + 1 >= args.len() {
                    eprintln!("flatpak config --get: usage: flatpak config --get KEY");
                    process::exit(1);
                }
                let key = &args[i + 1];
                if config_path.exists() {
                    let config = fs::read_to_string(&config_path).unwrap_or_default();
                    if let Some(val) = get_config_value(&config, "config", key) {
                        println!("{val}");
                    } else {
                        eprintln!("flatpak config: key '{key}' is not set");
                        process::exit(1);
                    }
                } else {
                    eprintln!("flatpak config: key '{key}' is not set");
                    process::exit(1);
                }
                return;
            }
            "--unset" => {
                if i + 1 >= args.len() {
                    eprintln!("flatpak config --unset: usage: flatpak config --unset KEY");
                    process::exit(1);
                }
                let key = &args[i + 1];
                if config_path.exists() {
                    let config = fs::read_to_string(&config_path).unwrap_or_default();
                    let updated = unset_config_value(&config, "config", key);
                    let _ = fs::write(&config_path, &updated);
                }
                return;
            }
            _ => {}
        }
        i += 1;
    }
}

/// Get a value from an INI-style config string under [group].
fn get_config_value(config: &str, group: &str, key: &str) -> Option<String> {
    let group_header = format!("[{group}]");
    let mut in_group = false;
    for line in config.lines() {
        let trimmed = line.trim();
        if trimmed == group_header {
            in_group = true;
            continue;
        }
        if trimmed.starts_with('[') {
            if in_group {
                break;
            }
            continue;
        }
        if in_group {
            if let Some(val) = trimmed.strip_prefix(&format!("{key}=")) {
                return Some(val.to_string());
            }
            if let Some(rest) = trimmed.strip_prefix(&format!("{key} =")) {
                return Some(rest.trim_start().to_string());
            }
        }
    }
    None
}

/// Remove a key from an INI-style config string under [group].
fn unset_config_value(config: &str, group: &str, key: &str) -> String {
    let group_header = format!("[{group}]");
    let mut in_group = false;
    let mut lines: Vec<&str> = Vec::new();
    for line in config.lines() {
        let trimmed = line.trim();
        if trimmed == group_header {
            in_group = true;
            lines.push(line);
            continue;
        }
        if trimmed.starts_with('[') {
            in_group = false;
        }
        if in_group
            && (trimmed.starts_with(&format!("{key}=")) || trimmed.starts_with(&format!("{key} =")))
        {
            continue; // skip this line
        }
        lines.push(line);
    }
    let mut result = lines.join("\n");
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

// ---------------------------------------------------------------------------
// Command: repair
// ---------------------------------------------------------------------------

fn cmd_repair(installations: &[Installation]) {
    for inst in installations {
        let label = if inst.is_user { "user" } else { "system" };
        // Walk the deploy tree directly so we can detect refs whose metadata
        // is missing (which would cause `list_refs` to skip them).
        let mut all_refs: Vec<Ref> = Vec::new();
        for (kind_dir, kind) in [("app", RefKind::App), ("runtime", RefKind::Runtime)] {
            let kind_path = inst.path.join(kind_dir);
            if let Ok(ids) = fs::read_dir(&kind_path) {
                for id_entry in ids.flatten() {
                    let id = id_entry.file_name().to_string_lossy().to_string();
                    if let Ok(arches) = fs::read_dir(id_entry.path()) {
                        for arch_entry in arches.flatten() {
                            let arch = arch_entry.file_name().to_string_lossy().to_string();
                            if let Ok(branches) = fs::read_dir(arch_entry.path()) {
                                for branch_entry in branches.flatten() {
                                    let branch =
                                        branch_entry.file_name().to_string_lossy().to_string();
                                    if branch_entry.path().join("active").exists() {
                                        all_refs.push(Ref {
                                            kind,
                                            id: id.clone(),
                                            arch: arch.clone(),
                                            branch,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let mut broken = 0;
        let mut repaired = 0;
        for ref_ in &all_refs {
            let files = inst.files_path(ref_);
            let deploy = inst.deploy_path(ref_);
            let metadata_path = deploy.join("metadata");

            if !files.exists() {
                eprintln!("  broken: {} (missing files/)", ref_.format_ref());
                broken += 1;
            }
            if !metadata_path.exists() {
                eprintln!("  broken: {} (missing metadata)", ref_.format_ref());
                // Try to regenerate a minimal metadata file.
                let kind = if ref_.kind == RefKind::App {
                    "Application"
                } else {
                    "Runtime"
                };
                let content = format!(
                    "[{kind}]\nname={}\nruntime=org.freedesktop.Platform/x86_64/23.08\n",
                    ref_.id
                );
                if fs::write(&metadata_path, content).is_ok() {
                    eprintln!("  repaired: {} (regenerated metadata)", ref_.format_ref());
                    repaired += 1;
                } else {
                    broken += 1;
                }
            }
            // Check for broken symlinks in files/.
            if files.exists() {
                for entry in fs::read_dir(&files).into_iter().flatten().flatten() {
                    let p = entry.path();
                    if p.is_symlink() && !p.exists() {
                        eprintln!(
                            "  broken symlink: {} -> {:?}",
                            p.display(),
                            fs::read_link(&p).unwrap_or_default()
                        );
                        broken += 1;
                    }
                }
            }
        }
        let refs = all_refs;
        if broken == 0 && repaired == 0 {
            println!("[{label}] No problems found ({} refs checked)", refs.len());
        } else if broken == 0 {
            println!(
                "[{label}] Repaired {repaired} issues ({} refs checked)",
                refs.len()
            );
        } else {
            println!(
                "[{label}] Found {broken} broken, repaired {repaired} ({} refs checked)",
                refs.len()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Command: documents
// ---------------------------------------------------------------------------

fn cmd_documents(args: &[String]) {
    let app_id = args.iter().find(|a| !a.starts_with('-'));
    let docs = portals::list_documents(app_id.map(|s| s.as_str()));
    if docs.is_empty() {
        println!("No exported documents.");
    } else {
        for doc in &docs {
            println!("{}: {}", doc.id, doc.path.display());
        }
    }
}

fn cmd_document_export(args: &[String]) {
    let path = args.iter().find(|a| !a.starts_with('-'));
    match path {
        Some(p) => match portals::export_document(p, &[]) {
            Ok(id) => println!("Exported as: {id}"),
            Err(e) => {
                eprintln!("flatpak document-export: {e}");
                process::exit(1);
            }
        },
        None => {
            eprintln!("flatpak document-export: no path specified");
            process::exit(1);
        }
    }
}

fn cmd_document_unexport(args: &[String]) {
    let doc_id = args.iter().find(|a| !a.starts_with('-'));
    match doc_id {
        Some(id) => match portals::unexport_document(id) {
            Ok(()) => println!("Unexported: {id}"),
            Err(e) => {
                eprintln!("flatpak document-unexport: {e}");
                process::exit(1);
            }
        },
        None => {
            eprintln!("flatpak document-unexport: no document ID specified");
            process::exit(1);
        }
    }
}

fn cmd_document_info(args: &[String]) {
    let doc_id = args.iter().find(|a| !a.starts_with('-'));
    match doc_id {
        Some(id) => match portals::document_info(id) {
            Ok(info) => {
                println!("  ID: {}", info.id);
                println!("  Path: {}", info.path.display());
            }
            Err(e) => {
                eprintln!("flatpak document-info: {e}");
                process::exit(1);
            }
        },
        None => {
            eprintln!("flatpak document-info: no document ID specified");
            process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Command: permissions
// ---------------------------------------------------------------------------

fn cmd_permissions(args: &[String]) {
    let table = args.iter().find(|a| !a.starts_with('-'));
    let perms = portals::list_permissions(table.map(|s| s.as_str()));
    if perms.is_empty() {
        println!("No permissions recorded.");
    } else {
        for p in &perms {
            println!("{}/{}: {} = {:?}", p.table, p.id, p.app_id, p.permissions);
        }
    }
}

fn cmd_permission_show(args: &[String]) {
    let app_id = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .unwrap_or_else(|| {
            eprintln!("flatpak permission-show: no app specified");
            process::exit(1);
        });
    let perms = portals::show_permissions(app_id);
    if perms.is_empty() {
        println!("No permissions for {app_id}.");
    } else {
        for p in &perms {
            println!("{}/{}: {:?}", p.table, p.id, p.permissions);
        }
    }
}

fn cmd_permission_set(args: &[String]) {
    if args.len() < 4 {
        eprintln!("flatpak permission-set: usage: flatpak permission-set TABLE ID APP_ID PERM...");
        process::exit(1);
    }
    let perms: Vec<String> = args[3..].to_vec();
    match portals::set_permission(&args[0], &args[1], &args[2], &perms) {
        Ok(()) => println!("Permission set."),
        Err(e) => {
            eprintln!("flatpak permission-set: {e}");
            process::exit(1);
        }
    }
}

fn cmd_permission_remove(args: &[String]) {
    if args.len() < 2 {
        eprintln!("flatpak permission-remove: usage: flatpak permission-remove TABLE ID");
        process::exit(1);
    }
    match portals::remove_permission(&args[0], &args[1]) {
        Ok(()) => println!("Permission removed."),
        Err(e) => {
            eprintln!("flatpak permission-remove: {e}");
            process::exit(1);
        }
    }
}

fn cmd_permission_reset(args: &[String]) {
    let app_id = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .unwrap_or_else(|| {
            eprintln!("flatpak permission-reset: no app specified");
            process::exit(1);
        });
    match portals::reset_permissions(app_id) {
        Ok(()) => println!("Permissions reset for {app_id}."),
        Err(e) => {
            eprintln!("flatpak permission-reset: {e}");
            process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Command: make-current, mask, pin
// ---------------------------------------------------------------------------

fn cmd_make_current(installations: &[Installation], args: &[String]) {
    if args.len() < 2 {
        eprintln!("flatpak make-current: usage: flatpak make-current APP BRANCH");
        process::exit(1);
    }
    let app_id = &args[0];
    let branch = &args[1];

    // Verify the ref exists.
    let deployed = find_deployed(installations, app_id);
    // Create/update an "active" symlink or marker.
    let active_marker = deployed
        .installation
        .deploy_path(&deployed.ref_)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("current");
    let _ = fs::write(&active_marker, branch);
    println!("Set {app_id} current branch to {branch}");
}

fn cmd_mask(installations: &[Installation], args: &[String]) {
    let pattern = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .unwrap_or_else(|| {
            eprintln!("flatpak mask: no pattern specified");
            process::exit(1);
        });

    let inst = &installations[0];
    let mask_dir = inst.path.join("masks");
    let _ = fs::create_dir_all(&mask_dir);
    let _ = fs::write(mask_dir.join(pattern.replace('/', "_")), pattern.as_bytes());
    println!("Masked: {pattern}");
}

fn cmd_pin(installations: &[Installation], args: &[String]) {
    let pattern = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .unwrap_or_else(|| {
            eprintln!("flatpak pin: no pattern specified");
            process::exit(1);
        });

    let inst = &installations[0];
    let pin_dir = inst.path.join("pins");
    let _ = fs::create_dir_all(&pin_dir);
    let _ = fs::write(pin_dir.join(pattern.replace('/', "_")), pattern.as_bytes());
    println!("Pinned: {pattern}");
}

// ---------------------------------------------------------------------------
// Command: build-*
// ---------------------------------------------------------------------------

fn cmd_build_init(args: &[String]) {
    let mut dir: Option<String> = None;
    let mut sdk: Option<String> = None;
    let mut runtime: Option<String> = None;
    let mut runtime_version = "stable".to_string();
    let mut app_id: Option<String> = None;
    let mut extension_tag: Option<String> = None;

    let mut positionals = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--extension-tag" => {
                i += 1;
                if i < args.len() {
                    extension_tag = Some(args[i].clone());
                }
            }
            s if s.starts_with('-') => {}
            _ => positionals.push(args[i].clone()),
        }
        i += 1;
    }

    // positionals: DIR APP_ID SDK RUNTIME [BRANCH]
    if positionals.len() >= 4 {
        dir = Some(positionals[0].clone());
        app_id = Some(positionals[1].clone());
        sdk = Some(positionals[2].clone());
        runtime = Some(positionals[3].clone());
        if positionals.len() >= 5 {
            runtime_version = positionals[4].clone();
        }
    }

    let dir = dir.unwrap_or_else(|| {
        eprintln!("flatpak build-init: usage: flatpak build-init DIR APP_ID SDK RUNTIME [BRANCH]");
        process::exit(1);
    });

    build::build_init(
        Path::new(&dir),
        sdk.as_deref().unwrap_or("org.freedesktop.Sdk"),
        runtime.as_deref().unwrap_or("org.freedesktop.Platform"),
        &runtime_version,
        app_id.as_deref().unwrap_or("org.example.App"),
        extension_tag.as_deref(),
    )
    .unwrap_or_else(|e| {
        eprintln!("flatpak build-init: {e}");
        process::exit(1);
    });

    eprintln!("Initialized build directory: {dir}");
}

fn cmd_build(installations: &[Installation], args: &[String]) {
    let mut dir: Option<String> = None;
    let mut command = Vec::new();
    let mut runtime_env = false;
    let mut past_dir = false;

    for arg in args {
        match arg.as_str() {
            "--runtime" => runtime_env = true,
            s if s.starts_with('-') && !past_dir => {}
            _ => {
                if dir.is_none() {
                    dir = Some(arg.clone());
                    past_dir = true;
                } else {
                    command.push(arg.clone());
                }
            }
        }
    }

    let dir = dir.unwrap_or_else(|| {
        eprintln!("flatpak build: usage: flatpak build DIR COMMAND [ARGS...]");
        process::exit(1);
    });

    if command.is_empty() {
        eprintln!("flatpak build: no command specified");
        process::exit(1);
    }

    let exit_code = build::build_run(Path::new(&dir), &command, runtime_env, installations)
        .unwrap_or_else(|e| {
            eprintln!("flatpak build: {e}");
            process::exit(1);
        });

    process::exit(exit_code);
}

fn cmd_build_finish(args: &[String]) {
    let mut dir: Option<String> = None;
    let mut command: Option<String> = None;
    let mut sdk: Option<String> = None;
    let mut require_version: Option<String> = None;
    let mut permissions: Vec<(String, String)> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--command" => {
                i += 1;
                if i < args.len() {
                    command = Some(args[i].clone());
                }
            }
            s if s.starts_with("--sdk=") => {
                sdk = Some(s.strip_prefix("--sdk=").unwrap().to_string());
            }
            "--sdk" => {
                i += 1;
                if i < args.len() {
                    sdk = Some(args[i].clone());
                }
            }
            s if s.starts_with("--require-version=") => {
                require_version = Some(s.strip_prefix("--require-version=").unwrap().to_string());
            }
            "--require-version" => {
                i += 1;
                if i < args.len() {
                    require_version = Some(args[i].clone());
                }
            }
            "--share" => {
                i += 1;
                if i < args.len() {
                    permissions.push(("shared".into(), args[i].clone()));
                }
            }
            "--socket" => {
                i += 1;
                if i < args.len() {
                    permissions.push(("sockets".into(), args[i].clone()));
                }
            }
            "--filesystem" => {
                i += 1;
                if i < args.len() {
                    permissions.push(("filesystems".into(), args[i].clone()));
                }
            }
            "--device" => {
                i += 1;
                if i < args.len() {
                    permissions.push(("devices".into(), args[i].clone()));
                }
            }
            "--persist" => {
                i += 1;
                if i < args.len() {
                    permissions.push(("persistent".into(), args[i].clone()));
                }
            }
            s if s.starts_with("--persist=") => {
                if let Some(val) = s.strip_prefix("--persist=") {
                    permissions.push(("persistent".into(), val.to_string()));
                }
            }
            s if !s.starts_with('-') => {
                if dir.is_none() {
                    dir = Some(s.to_string());
                }
            }
            _ => {}
        }
        i += 1;
    }

    let dir = dir.unwrap_or_else(|| {
        eprintln!("flatpak build-finish: usage: flatpak build-finish DIR [--command CMD] [--share ...] [--socket ...]");
        process::exit(1);
    });

    build::build_finish(
        Path::new(&dir),
        command.as_deref(),
        sdk.as_deref(),
        require_version.as_deref(),
        &permissions,
    )
    .unwrap_or_else(|e| {
        eprintln!("flatpak build-finish: {e}");
        process::exit(1);
    });

    eprintln!("Build finished: {dir}");
}

fn cmd_build_export(args: &[String]) {
    let mut repo: Option<String> = None;
    let mut dir: Option<String> = None;
    let mut branch: Option<String> = None;
    let mut subject: Option<String> = None;
    let mut gpg_sign: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-b" | "--branch" => {
                i += 1;
                if i < args.len() {
                    branch = Some(args[i].clone());
                }
            }
            "-s" | "--subject" => {
                i += 1;
                if i < args.len() {
                    subject = Some(args[i].clone());
                }
            }
            s if s.starts_with("--gpg-sign=") => {
                gpg_sign = s.strip_prefix("--gpg-sign=").map(String::from);
            }
            s if !s.starts_with('-') => {
                if repo.is_none() {
                    repo = Some(s.to_string());
                } else if dir.is_none() {
                    dir = Some(s.to_string());
                }
            }
            _ => {}
        }
        i += 1;
    }

    let repo = repo.unwrap_or_else(|| {
        eprintln!("flatpak build-export: usage: flatpak build-export REPO DIR [-b BRANCH] [--gpg-sign=KEYID]");
        process::exit(1);
    });
    let dir = dir.unwrap_or_else(|| {
        eprintln!("flatpak build-export: no build directory specified");
        process::exit(1);
    });

    let ref_str = build::build_export(
        Path::new(&repo),
        Path::new(&dir),
        branch.as_deref(),
        subject.as_deref(),
    )
    .unwrap_or_else(|e| {
        eprintln!("flatpak build-export: {e}");
        process::exit(1);
    });

    // GPG sign if requested.
    if let Some(key_id) = gpg_sign
        && let Err(e) = build::build_sign(Path::new(&repo), &ref_str, &key_id)
    {
        eprintln!("flatpak build-export: GPG signing failed: {e}");
    }
}

fn cmd_build_bundle(args: &[String]) {
    let positionals: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    if positionals.len() < 3 {
        eprintln!("flatpak build-bundle: usage: flatpak build-bundle REPO FILE REF");
        process::exit(1);
    }

    build::build_bundle(
        Path::new(positionals[0]),
        Path::new(positionals[1]),
        positionals[2],
    )
    .unwrap_or_else(|e| {
        eprintln!("flatpak build-bundle: {e}");
        process::exit(1);
    });
}

fn cmd_build_import_bundle(installations: &[Installation], args: &[String]) {
    let file = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .unwrap_or_else(|| {
            eprintln!("flatpak build-import-bundle: usage: flatpak build-import-bundle FILE");
            process::exit(1);
        });

    build::build_import_bundle(installations, Path::new(file)).unwrap_or_else(|e| {
        eprintln!("flatpak build-import-bundle: {e}");
        process::exit(1);
    });
}

fn cmd_build_sign(args: &[String]) {
    let positionals: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    if positionals.len() < 2 {
        eprintln!("flatpak build-sign: usage: flatpak build-sign REPO REF --gpg-sign=KEYID");
        process::exit(1);
    }
    let key_id = args
        .iter()
        .find_map(|a| a.strip_prefix("--gpg-sign="))
        .unwrap_or("default");

    build::build_sign(Path::new(positionals[0]), positionals[1], key_id).unwrap_or_else(|e| {
        eprintln!("flatpak build-sign: {e}");
        process::exit(1);
    });
}

fn cmd_build_update_repo(args: &[String]) {
    let mut repo: Option<&String> = None;
    let mut title: Option<String> = None;
    let mut redirect_url: Option<String> = None;
    let mut default_branch: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            s if s.starts_with("--title=") => {
                title = Some(s.strip_prefix("--title=").unwrap().to_string());
            }
            "--title" => {
                i += 1;
                if i < args.len() {
                    title = Some(args[i].clone());
                }
            }
            s if s.starts_with("--redirect-url=") => {
                redirect_url = Some(s.strip_prefix("--redirect-url=").unwrap().to_string());
            }
            "--redirect-url" => {
                i += 1;
                if i < args.len() {
                    redirect_url = Some(args[i].clone());
                }
            }
            s if s.starts_with("--default-branch=") => {
                default_branch = Some(s.strip_prefix("--default-branch=").unwrap().to_string());
            }
            "--default-branch" => {
                i += 1;
                if i < args.len() {
                    default_branch = Some(args[i].clone());
                }
            }
            s if !s.starts_with('-') => {
                if repo.is_none() {
                    repo = Some(&args[i]);
                }
            }
            _ => {}
        }
        i += 1;
    }

    let repo = repo.unwrap_or_else(|| {
        eprintln!("flatpak build-update-repo: usage: flatpak build-update-repo REPO [--title=TITLE] [--redirect-url=URL] [--default-branch=BRANCH]");
        process::exit(1);
    });

    let repo_path = Path::new(repo);

    // Persist config options in repo/config.
    if title.is_some() || redirect_url.is_some() || default_branch.is_some() {
        let config_path = repo_path.join("config");
        let mut config = if config_path.exists() {
            fs::read_to_string(&config_path).unwrap_or_default()
        } else {
            String::new()
        };

        if let Some(t) = &title {
            config = set_config_value(&config, "flatpak", "title", t);
        }
        if let Some(u) = &redirect_url {
            config = set_config_value(&config, "flatpak", "redirect-url", u);
        }
        if let Some(b) = &default_branch {
            config = set_config_value(&config, "flatpak", "default-branch", b);
        }

        let _ = fs::create_dir_all(repo_path);
        let _ = fs::write(&config_path, &config);
    }

    build::build_update_repo(repo_path).unwrap_or_else(|e| {
        eprintln!("flatpak build-update-repo: {e}");
        process::exit(1);
    });
}

/// Set a key=value in an INI-style config string under [group].
/// If the group or key doesn't exist, append it.
#[allow(clippy::needless_range_loop)]
fn set_config_value(config: &str, group: &str, key: &str, value: &str) -> String {
    let group_header = format!("[{group}]");
    let key_line = format!("{key}={value}");
    let mut lines: Vec<String> = config.lines().map(String::from).collect();

    // Find the group.
    let group_idx = lines.iter().position(|l| l.trim() == group_header);
    if let Some(gi) = group_idx {
        // Find key in this group (before next group or end).
        let mut found = false;
        for j in (gi + 1)..lines.len() {
            if lines[j].trim().starts_with('[') {
                break;
            }
            if lines[j].trim().starts_with(&format!("{key}="))
                || lines[j].trim().starts_with(&format!("{key} ="))
            {
                lines[j] = key_line.clone();
                found = true;
                break;
            }
        }
        if !found {
            lines.insert(gi + 1, key_line);
        }
    } else {
        // Append new group.
        if !lines.is_empty() && !lines.last().unwrap().is_empty() {
            lines.push(String::new());
        }
        lines.push(group_header);
        lines.push(key_line);
    }

    let mut result = lines.join("\n");
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

fn cmd_build_commit_from(args: &[String]) {
    let positionals: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    if positionals.len() < 3 {
        eprintln!(
            "flatpak build-commit-from: usage: flatpak build-commit-from REPO SRC_REF DEST_REF"
        );
        process::exit(1);
    }

    build::build_commit_from(Path::new(positionals[0]), positionals[1], positionals[2])
        .unwrap_or_else(|e| {
            eprintln!("flatpak build-commit-from: {e}");
            process::exit(1);
        });
}

fn cmd_repo(args: &[String]) {
    let repo = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .unwrap_or_else(|| {
            eprintln!("flatpak repo: usage: flatpak repo REPO");
            process::exit(1);
        });

    build::repo_info(Path::new(repo)).unwrap_or_else(|e| {
        eprintln!("flatpak repo: {e}");
        process::exit(1);
    });
}

// ---------------------------------------------------------------------------
// Command: create-usb
// ---------------------------------------------------------------------------

fn cmd_create_usb(installations: &[Installation], args: &[String]) {
    let mut mount_point: Option<String> = None;
    let mut refs_to_export: Vec<String> = Vec::new();

    for arg in args {
        match arg.as_str() {
            s if s.starts_with('-') => {}
            _ => {
                if mount_point.is_none() {
                    mount_point = Some(arg.clone());
                } else {
                    refs_to_export.push(arg.clone());
                }
            }
        }
    }

    let mount_point = mount_point.unwrap_or_else(|| {
        eprintln!("flatpak create-usb: usage: flatpak create-usb MOUNT_POINT REF [REF...]");
        process::exit(1);
    });

    if refs_to_export.is_empty() {
        eprintln!("flatpak create-usb: no refs specified");
        process::exit(1);
    }

    let usb_repo = Path::new(&mount_point).join(".flatpak-usb");
    let _ = fs::create_dir_all(&usb_repo);

    for ref_str in &refs_to_export {
        let deployed = find_deployed(installations, ref_str);
        let ref_ = &deployed.ref_;
        let deploy_path = deployed.installation.deploy_path(ref_);

        // Export to the USB repo.
        let dest_dir = usb_repo
            .join(ref_.kind_dir())
            .join(&ref_.id)
            .join(&ref_.arch)
            .join(&ref_.branch)
            .join("active");
        let _ = fs::create_dir_all(&dest_dir);

        // Copy metadata.
        let src_meta = deploy_path.join("metadata");
        if src_meta.exists() {
            let _ = fs::copy(&src_meta, dest_dir.join("metadata"));
        }

        // Copy files.
        let src_files = deployed.installation.files_path(ref_);
        let dest_files = dest_dir.join("files");
        if src_files.exists() {
            copy_dir_recursive(&src_files, &dest_files);
        }

        // Copy export.
        let src_export = deploy_path.join("export");
        let dest_export = dest_dir.join("export");
        if src_export.exists() {
            copy_dir_recursive(&src_export, &dest_export);
        }

        eprintln!("Exported {} to USB", ref_.format_ref());
    }

    println!("Created USB sideload repo at {}", usb_repo.display());
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn find_deployed(installations: &[Installation], app_id: &str) -> installation::DeployedRef {
    for inst in installations {
        if let Some(d) = inst.find_ref_by_string(app_id) {
            return d;
        }
    }
    eprintln!("flatpak: '{app_id}' is not installed");
    process::exit(1);
}

// ---------------------------------------------------------------------------
// Usage
// ---------------------------------------------------------------------------

/// Compare two simple dotted-numeric version strings (e.g. "1.2.3").
/// Returns true if `a` is strictly less than `b`. Missing trailing
/// components are treated as 0.
fn version_less(a: &str, b: &str) -> bool {
    let pa: Vec<u32> = a.split('.').map(|p| p.parse().unwrap_or(0)).collect();
    let pb: Vec<u32> = b.split('.').map(|p| p.parse().unwrap_or(0)).collect();
    let n = pa.len().max(pb.len());
    for i in 0..n {
        let x = *pa.get(i).unwrap_or(&0);
        let y = *pb.get(i).unwrap_or(&0);
        if x != y {
            return x < y;
        }
    }
    false
}

const ALL_COMMANDS: &[&str] = &[
    "run",
    "list",
    "info",
    "install",
    "uninstall",
    "remove",
    "update",
    "upgrade",
    "override",
    "remotes",
    "remote-list",
    "remote-add",
    "remote-delete",
    "remote-info",
    "remote-ls",
    "ps",
    "kill",
    "enter",
    "search",
    "history",
    "config",
    "repair",
    "documents",
    "document-list",
    "document-export",
    "document-unexport",
    "document-info",
    "permissions",
    "permission-list",
    "permission-show",
    "permission-set",
    "permission-remove",
    "permission-reset",
    "make-current",
    "mask",
    "pin",
    "build-init",
    "build",
    "build-finish",
    "build-export",
    "build-bundle",
    "build-import-bundle",
    "build-sign",
    "build-update-repo",
    "build-commit-from",
    "repo",
    "create-usb",
    "complete",
    "help",
];

fn cmd_complete(args: &[String]) {
    let prefix = args.first().map(|s| s.as_str()).unwrap_or("");
    for cmd in ALL_COMMANDS {
        if cmd.starts_with(prefix) {
            println!("{cmd}");
        }
    }
}

fn print_usage() {
    println!(
        "\
Usage: flatpak [OPTION...] COMMAND

Manage installed applications and runtimes:
  install            Install an application or runtime
  update             Update installed app/runtime
  uninstall          Uninstall an app/runtime
  list               List installed apps/runtimes
  info               Show info for installed app/runtime
  config             Configure flatpak
  repair             Repair installation
  override           Override permissions for an app

Find applications and runtimes:
  search             Search remote apps

Manage running applications:
  run                Run an application
  ps                 List running applications

Manage remote repositories:
  remotes            List configured remotes
  remote-add         Add a new remote repository
  remote-delete      Delete a remote

Shell completion:
  complete           Print candidate completions for a partial command

Global options:
  -u, --user         Work on user installation
  --system           Work on system installation
  -v, --verbose      Verbose output
  -h, --help         Show this help
  --version          Show version"
    );
}
