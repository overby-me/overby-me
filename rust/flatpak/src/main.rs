// rust-flatpak: A Flatpak-compatible application sandboxing and distribution tool.
//
// Implements the core Flatpak CLI for running, installing, listing, and
// managing sandboxed applications. Uses bwrap (rust-bubblewrap) for
// sandboxing.

mod build;
mod dbus_proxy;
mod extensions;
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
                println!("Flatpak 0.1.0 (rust-flatpak)");
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
    let columns = args.contains(&"--columns=all".to_string());

    println!(
        "{:<40} {:<12} {:<12} Installation",
        "Name", "Branch", "Arch"
    );
    println!("{}", "-".repeat(80));

    for inst in installations {
        for deployed in inst.list_refs() {
            let is_app = deployed.ref_.kind == RefKind::App;
            if (show_all) || (show_app && is_app) || (show_runtime && !is_app) {
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
    let app_id = args.last().unwrap();

    let deployed = find_deployed(installations, app_id);

    if show_metadata {
        println!("{}", deployed.metadata.serialize());
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

    for arg in args {
        match arg.as_str() {
            "--reinstall" => _reinstall = true,
            s if s.starts_with('-') => {}
            _ => {
                if source.is_none() {
                    source = Some(arg.clone());
                } else if ref_str.is_none() {
                    ref_str = Some(arg.clone());
                }
            }
        }
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
        install_from_dir(installations, source_path, ref_str.as_deref());
    } else if let Some(ref ref_name) = ref_str {
        // Install from a remote.
        install_from_remote(installations, &source, ref_name);
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

fn install_from_remote(installations: &[Installation], remote_name: &str, ref_name: &str) {
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
    let _commit = ostree::pull_ref(&remote.url, &ref_str, &deploy_path, true).unwrap_or_else(|e| {
        eprintln!("flatpak install: pull failed: {e}");
        let _ = fs::remove_dir_all(&deploy_path);
        process::exit(1);
    });

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

fn install_from_dir(installations: &[Installation], source: &Path, ref_override: Option<&str>) {
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

    // Copy files directory.
    let src_files = source.join("files");
    let dest_files = deploy_path.join("files");
    if src_files.exists() {
        copy_dir_recursive(&src_files, &dest_files);
    }

    // Copy export directory.
    let src_export = source.join("export");
    let dest_export = deploy_path.join("export");
    if src_export.exists() {
        copy_dir_recursive(&src_export, &dest_export);
    }

    println!("Installation complete: {} ({}/{})", app_name, arch, branch);

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

    println!("Uninstalled: {}", ref_.format_ref());
}

// ---------------------------------------------------------------------------
// Command: update (stub)
// ---------------------------------------------------------------------------

fn cmd_update(installations: &[Installation], _args: &[String]) {
    let mut updated = 0;
    for inst in installations {
        let remotes = installation::load_remotes(inst);
        let deployed_refs = inst.list_refs();

        for deployed in &deployed_refs {
            // Find which remote might have this ref.
            for remote in &remotes {
                let ref_str = deployed.ref_.format_ref();
                match ostree::fetch_summary(&remote.url) {
                    Ok(refs) => {
                        if refs.iter().any(|r| r.name == ref_str) {
                            eprintln!("Checking {ref_str} on {}...", remote.name);
                            // For a real update, we'd compare commit checksums.
                            // For now, just report what we'd update.
                            updated += 1;
                        }
                    }
                    Err(_) => continue,
                }
                break; // Only check first matching remote.
            }
        }
    }
    if updated == 0 {
        println!("Nothing to update.");
    } else {
        println!("Checked {updated} refs. Full update pull not yet implemented.");
    }
}

// ---------------------------------------------------------------------------
// Command: override
// ---------------------------------------------------------------------------

fn cmd_override(installations: &[Installation], args: &[String]) {
    let mut app_id: Option<String> = None;
    let mut overrides = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--filesystem" => {
                i += 1;
                if i < args.len() {
                    overrides.push(("filesystems", args[i].clone()));
                }
            }
            "--share" => {
                i += 1;
                if i < args.len() {
                    overrides.push(("shared", args[i].clone()));
                }
            }
            "--unshare" => {
                i += 1;
                if i < args.len() {
                    overrides.push(("shared", format!("!{}", args[i])));
                }
            }
            "--socket" => {
                i += 1;
                if i < args.len() {
                    overrides.push(("sockets", args[i].clone()));
                }
            }
            "--nosocket" => {
                i += 1;
                if i < args.len() {
                    overrides.push(("sockets", format!("!{}", args[i])));
                }
            }
            "--device" => {
                i += 1;
                if i < args.len() {
                    overrides.push(("devices", args[i].clone()));
                }
            }
            "--nodevice" => {
                i += 1;
                if i < args.len() {
                    overrides.push(("devices", format!("!{}", args[i])));
                }
            }
            "--env" => {
                i += 1;
                if i < args.len() {
                    overrides.push(("env", args[i].clone()));
                }
            }
            "--reset" => {
                if let Some(ref id) = app_id {
                    let inst = &installations[0];
                    let path = inst.override_path(id);
                    let _ = fs::remove_file(&path);
                    println!("Reset overrides for {id}");
                    return;
                }
            }
            s if !s.starts_with('-') => {
                app_id = Some(s.to_string());
            }
            _ => {}
        }
        i += 1;
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
        if *key == "env" {
            let env_group = meta.groups.entry("Environment".to_string()).or_default();
            if let Some((k, v)) = val.split_once('=') {
                env_group.insert(k.to_string(), v.to_string());
            }
        } else {
            let ctx = meta.groups.entry("Context".to_string()).or_default();
            let existing = ctx.entry(key.to_string()).or_default();
            if !existing.is_empty() {
                existing.push(';');
            }
            existing.push_str(val);
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
        && file_path != "next-positional" {
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
                            && let Some(parsed) = Ref::parse(&r.name) {
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
    }
}

// ---------------------------------------------------------------------------
// Command: repair
// ---------------------------------------------------------------------------

fn cmd_repair(installations: &[Installation]) {
    for inst in installations {
        let label = if inst.is_user { "user" } else { "system" };
        let refs = inst.list_refs();
        let mut broken = 0;
        for r in &refs {
            let files = inst.files_path(&r.ref_);
            if !files.exists() {
                eprintln!("  broken: {} (missing files)", r.ref_.format_ref());
                broken += 1;
            }
        }
        if broken == 0 {
            println!("[{label}] No problems found ({} refs checked)", refs.len());
        } else {
            println!("[{label}] Found {broken} broken refs");
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

    println!("Initialized build directory: {dir}");
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

    build::build_finish(Path::new(&dir), command.as_deref(), &permissions).unwrap_or_else(|e| {
        eprintln!("flatpak build-finish: {e}");
        process::exit(1);
    });

    println!("Build finished: {dir}");
}

fn cmd_build_export(args: &[String]) {
    let mut repo: Option<String> = None;
    let mut dir: Option<String> = None;
    let mut branch: Option<String> = None;
    let mut subject: Option<String> = None;

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
        eprintln!("flatpak build-export: usage: flatpak build-export REPO DIR [-b BRANCH]");
        process::exit(1);
    });
    let dir = dir.unwrap_or_else(|| {
        eprintln!("flatpak build-export: no build directory specified");
        process::exit(1);
    });

    build::build_export(
        Path::new(&repo),
        Path::new(&dir),
        branch.as_deref(),
        subject.as_deref(),
    )
    .unwrap_or_else(|e| {
        eprintln!("flatpak build-export: {e}");
        process::exit(1);
    });
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
    let repo = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .unwrap_or_else(|| {
            eprintln!("flatpak build-update-repo: usage: flatpak build-update-repo REPO");
            process::exit(1);
        });

    build::build_update_repo(Path::new(repo)).unwrap_or_else(|e| {
        eprintln!("flatpak build-update-repo: {e}");
        process::exit(1);
    });
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

Global options:
  -u, --user         Work on user installation
  --system           Work on system installation
  -v, --verbose      Verbose output
  -h, --help         Show this help
  --version          Show version"
    );
}
