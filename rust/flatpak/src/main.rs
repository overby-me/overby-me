// rust-flatpak: A Flatpak-compatible application sandboxing and distribution tool.
//
// Implements the core Flatpak CLI for running, installing, listing, and
// managing sandboxed applications. Uses bwrap (rust-bubblewrap) for
// sandboxing.

mod dbus_proxy;
mod installation;
mod instance;
mod metadata;
mod ostree;
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

    let status = setup.command.status().unwrap_or_else(|e| {
        eprintln!("flatpak run: failed to execute bwrap: {e}");
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

    for arg in args {
        match arg.as_str() {
            s if s.starts_with("--title=") => {
                title = Some(s.strip_prefix("--title=").unwrap().to_string());
            }
            s if s.starts_with('-') => {}
            _ => {
                if name.is_none() {
                    name = Some(arg.clone());
                } else if url.is_none() {
                    url = Some(arg.clone());
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
    let query = args.join(" ");
    if query.is_empty() {
        eprintln!("flatpak search: no search term specified");
        process::exit(1);
    }
    eprintln!("flatpak search: remote search not yet implemented");
    eprintln!("Use 'flatpak remote-ls <remote>' to list available refs");
    process::exit(1);
}

// ---------------------------------------------------------------------------
// Command: history (stub)
// ---------------------------------------------------------------------------

fn cmd_history(_installations: &[Installation]) {
    println!("No history recorded.");
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
