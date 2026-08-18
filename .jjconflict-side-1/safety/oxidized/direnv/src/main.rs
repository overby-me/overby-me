mod config;
mod dotenv;
mod env;
mod env_diff;
mod escape;
mod file_times;
mod gzenv;
mod log;
mod rc;
mod shell;

use crate::config::Config;
use crate::env::{DIRENV_DIFF, DIRENV_FILE, DIRENV_REQUIRED, DIRENV_WATCHES, Env};
use crate::env_diff::EnvDiff;
use crate::rc::{RC, find_env_up};
use crate::shell::{ShellExport, detect_shell, supported_shells};
use std::path::{Path, PathBuf};

const VERSION: &str = "2.36.0";
const STDLIB: &str = include_str!("stdlib.sh");

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        cmd_help();
        std::process::exit(1);
    }

    let cmd = args[1].as_str();
    let cmd_args = &args[1..];

    let result = match cmd {
        "allow" | "permit" | "grant" | "approve" => cmd_allow(cmd_args),
        "deny" | "revoke" | "block" => cmd_deny(cmd_args),
        "edit" => cmd_edit(cmd_args),
        "exec" => cmd_exec(cmd_args),
        "export" => cmd_export(cmd_args),
        "hook" => cmd_hook(cmd_args),
        "help" | "--help" | "-h" => {
            cmd_help();
            Ok(())
        }
        "prune" => cmd_prune(),
        "reload" => cmd_reload(),
        "status" => cmd_status(),
        "stdlib" => {
            print!("{STDLIB}");
            Ok(())
        }
        "version" | "--version" => cmd_version(cmd_args),
        "dump" => cmd_dump(cmd_args),
        "apply_dump" => cmd_apply_dump(cmd_args),
        "show_dump" => cmd_show_dump(cmd_args),
        "dotenv" => cmd_dotenv(cmd_args),
        "watch" => cmd_watch(cmd_args),
        "watch-dir" => cmd_watch_dir(cmd_args),
        "watch-list" => cmd_watch_list(),
        "watch-print" => cmd_watch_print(cmd_args),
        "fetchurl" => cmd_fetchurl(cmd_args),
        "current" => cmd_current(),
        "log" => cmd_log(cmd_args),
        "check-required" => cmd_check_required(cmd_args),
        _ => {
            log::log_error_default(&format!("unknown command \"{cmd}\""));
            cmd_help();
            Err("unknown command".to_string())
        }
    };

    if let Err(e) = result {
        log::log_error_default(&e);
        std::process::exit(1);
    }
}

fn cmd_help() {
    let shells = supported_shells().join(", ");
    eprintln!(
        "\
direnv v{VERSION} (Rust)

Usage: direnv COMMAND [...]

Commands:
  allow [PATH_TO_RC]     Grants direnv permission to load the given .envrc or .env file.
  deny [PATH_TO_RC]      Revokes the authorization of a given .envrc or .env file.
  edit [PATH_TO_RC]      Opens PATH_TO_RC or the current .envrc into an $EDITOR and allow
                         the file to be loaded afterwards.
  exec DIR COMMAND       Executes a command after loading the first .envrc or .env found in DIR.
  export SHELL           Loads an .envrc or .env and prints the diff in terms of exports.
                         Supported shells: [{shells}]
  fetchurl URL [HASH]    Fetches a URL into direnv's CAS.
  help                   Shows this help.
  hook SHELL             Used to setup the shell hook.
  prune                  Removes old allowed files.
  reload                 Triggers an env reload.
  status                 Prints some debug status information.
  stdlib                 Displays the stdlib available in the .envrc execution context.
  version                Prints the version or checks that direnv is older than VERSION_AT_LEAST."
    );
}

fn cmd_allow(args: &[String]) -> Result<(), String> {
    let current_env = env::get_env();
    let config = Config::load(&current_env)?;

    let path = if args.len() > 1 {
        let p = &args[1];
        let p = PathBuf::from(p);
        let p = std::fs::canonicalize(&p).unwrap_or(p);
        if p.is_dir() { p.join(".envrc") } else { p }
    } else {
        // Find current RC
        let wd = config.work_dir.to_string_lossy().to_string();
        let rc_path = find_env_up(&wd, config.load_dotenv)
            .ok_or_else(|| "No .envrc or .env file found".to_string())?;
        PathBuf::from(rc_path)
    };

    // Handle data migration from old allow dir (XDG_CONFIG -> XDG_DATA)
    let allow_dir = config.allow_dir();
    if !allow_dir.exists() {
        let old_allow_dir = config.conf_dir.join("allow");
        if old_allow_dir.exists() {
            eprintln!("Migrating the allow data to the new location");
            if let Some(parent) = allow_dir.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            if std::fs::rename(&old_allow_dir, &allow_dir).is_ok() {
                // Create symlink for back-compat
                #[cfg(unix)]
                {
                    std::os::unix::fs::symlink(&allow_dir, &old_allow_dir).ok();
                }
            }
        }
    }

    let path_str = path.to_string_lossy().to_string();
    let mut rc = RC::from_path(&path_str, &config)?;
    rc.allow()?;

    // Handle required files if DIRENV_REQUIRED is set
    if let Some(required_paths) = current_env.get(DIRENV_REQUIRED)
        && !required_paths.is_empty()
    {
        allow_required_files(&path_str, required_paths, &config)?;
    }

    log::log_status(
        &config.log_format,
        config.log_color,
        &format!("allowed {}", path.display()),
    );
    Ok(())
}

fn allow_required_files(
    rc_path: &str,
    required_paths: &str,
    config: &Config,
) -> Result<(), String> {
    use sha2::{Digest, Sha256};

    let rc_dir = Path::new(rc_path)
        .parent()
        .ok_or("no parent dir")?
        .to_string_lossy()
        .to_string();

    // Hash the envrc path
    let abs_rc = std::fs::canonicalize(rc_path).unwrap_or_else(|_| PathBuf::from(rc_path));
    let mut hasher = Sha256::new();
    hasher.update(format!("{}\n", abs_rc.to_string_lossy()).as_bytes());
    let envrc_path_hash = format!("{:x}", hasher.finalize());

    let allowed_required_dir = config
        .data_dir
        .join("allowed-required")
        .join(&envrc_path_hash);
    std::fs::create_dir_all(&allowed_required_dir).map_err(|e| e.to_string())?;

    for rel_path in required_paths.split(':') {
        let abs_path = PathBuf::from(&rc_dir).join(rel_path);

        // Hash the file content + path
        let content = std::fs::read(&abs_path)
            .map_err(|_| format!("required file does not exist: {rel_path}"))?;
        let abs_str = std::fs::canonicalize(&abs_path)
            .unwrap_or(abs_path.clone())
            .to_string_lossy()
            .to_string();
        let mut hasher = Sha256::new();
        hasher.update(format!("{abs_str}\n").as_bytes());
        hasher.update(&content);
        let file_hash = format!("{:x}", hasher.finalize());

        let allowed_file = allowed_required_dir.join(&file_hash);
        std::fs::write(&allowed_file, format!("{rel_path}\n"))
            .map_err(|e| format!("failed to write allowed-required: {e}"))?;

        eprintln!("direnv: allowing {rel_path}");
    }

    Ok(())
}

fn cmd_deny(args: &[String]) -> Result<(), String> {
    let current_env = env::get_env();
    let config = Config::load(&current_env)?;

    let path = if args.len() > 1 {
        let p = &args[1];
        if Path::new(p).is_dir() {
            PathBuf::from(p).join(".envrc")
        } else {
            PathBuf::from(p)
        }
    } else {
        let wd = config.work_dir.to_string_lossy().to_string();
        let rc_path = find_env_up(&wd, config.load_dotenv)
            .ok_or_else(|| "No .envrc or .env file found".to_string())?;
        PathBuf::from(rc_path)
    };

    let path_str = path.to_string_lossy().to_string();
    let rc = RC::from_path(&path_str, &config)?;
    rc.deny()?;
    log::log_status(
        &config.log_format,
        config.log_color,
        &format!("denied {}", path.display()),
    );
    Ok(())
}

fn cmd_edit(args: &[String]) -> Result<(), String> {
    let current_env = env::get_env();
    let config = Config::load(&current_env)?;

    let path = if args.len() > 1 {
        let p = &args[1];
        if Path::new(p).is_dir() {
            PathBuf::from(p).join(".envrc")
        } else {
            PathBuf::from(p)
        }
    } else {
        let wd = config.work_dir.to_string_lossy().to_string();
        match find_env_up(&wd, config.load_dotenv) {
            Some(p) => PathBuf::from(p),
            None => PathBuf::from(wd).join(".envrc"),
        }
    };

    let editor = std::env::var("EDITOR")
        .unwrap_or_else(|_| std::env::var("VISUAL").unwrap_or_else(|_| "vi".to_string()));

    let status = std::process::Command::new(&editor)
        .arg(&path)
        .status()
        .map_err(|e| format!("failed to run editor: {e}"))?;

    if !status.success() {
        return Err(format!("editor exited with status {status}"));
    }

    // Allow the file after editing
    let path_str = path.to_string_lossy().to_string();
    if Path::new(&path_str).exists() {
        let mut rc = RC::from_path(&path_str, &config)?;
        rc.allow()?;
    }

    Ok(())
}

fn cmd_exec(args: &[String]) -> Result<(), String> {
    if args.len() < 3 {
        return Err("Usage: direnv exec DIR COMMAND [ARGS...]".to_string());
    }

    let dir = &args[1];
    let command = &args[2];
    let cmd_args = &args[3..];

    let current_env = env::get_env();
    let config = Config::load(&current_env)?;

    let rc_path = find_env_up(dir, config.load_dotenv);

    let new_env = if let Some(ref rc_path) = rc_path {
        let previous_env = config.revert(&current_env)?;
        let rc = RC::from_path(rc_path, &config)?;
        rc.load(&previous_env, &config)?
    } else {
        config.revert(&current_env)?
    };

    // On Unix, use exec to replace the process
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = std::process::Command::new(command)
            .args(cmd_args)
            .env_clear()
            .envs(new_env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .exec();
        Err(format!("exec failed: {err}"))
    }

    #[cfg(not(unix))]
    {
        let status = std::process::Command::new(command)
            .args(cmd_args)
            .env_clear()
            .envs(new_env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .status()
            .map_err(|e| format!("exec failed: {e}"))?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn cmd_export(args: &[String]) -> Result<(), String> {
    let target = args.get(1).map(|s| s.as_str()).unwrap_or("");

    let shell = detect_shell(target).ok_or_else(|| format!("unknown target shell '{target}'"))?;

    let current_env = env::get_env();
    let config = Config::load(&current_env)?;

    let loaded_rc = config.env.get(DIRENV_FILE).and_then(|path| {
        let times_str = config.env.get(DIRENV_WATCHES)?;
        RC::from_env(path, times_str, &config)
    });

    let to_load = find_env_up(&config.work_dir.to_string_lossy(), config.load_dotenv);

    // Determine if we need to do anything
    if loaded_rc.is_none() && to_load.is_none() {
        return Ok(());
    }

    let needs_update = match (&loaded_rc, &to_load) {
        (_, None) => true,                                        // unload
        (None, Some(_)) => true,                                  // load
        (Some(rc), Some(path)) if rc.path != *path => true,       // new RC
        (Some(rc), Some(_)) if rc.times.check().is_err() => true, // file changed
        _ => {
            // check DIRENV_REQUIRED
            current_env
                .get(DIRENV_REQUIRED)
                .is_some_and(|v| !v.is_empty())
        }
    };

    if !needs_update {
        return Ok(());
    }

    let previous_env = config.revert(&current_env)?;

    let new_env = match to_load {
        None => {
            log::log_status(&config.log_format, config.log_color, "unloading");
            let mut env = previous_env.clone();
            env::clean_context(&mut env);
            env
        }
        Some(ref path) => {
            let rc = RC::from_path(path, &config)?;
            match rc.load(&previous_env, &config) {
                Ok(env) => env,
                Err(e) => {
                    log::log_error(&config.log_format, config.log_color, &e);
                    // Build a minimal diff to avoid retrying on every prompt
                    let mut env = previous_env.clone();
                    let dir = Path::new(path)
                        .parent()
                        .map(|p| format!("-{}", p.to_string_lossy()))
                        .unwrap_or_default();
                    env.insert(DIRENV_FILE.to_string(), path.clone());
                    env.insert(env::DIRENV_DIR.to_string(), dir);
                    let diff = EnvDiff::build(&previous_env, &env);
                    env.insert(DIRENV_DIFF.to_string(), diff.serialize());
                    env
                }
            }
        }
    };

    // Output diff status
    let diff = EnvDiff::build(&previous_env, &new_env);
    if diff.any() && !config.hide_env_diff {
        let status = diff_status(&diff);
        if !status.is_empty() {
            log::log_status(
                &config.log_format,
                config.log_color,
                &format!("export {status}"),
            );
        }
    }

    // Output the shell diff
    let current_diff = EnvDiff::build(&current_env, &new_env);
    let export = current_diff.to_shell_export();
    let diff_string = shell.export(&export)?;
    print!("{diff_string}");

    Ok(())
}

fn diff_status(diff: &EnvDiff) -> String {
    let mut out = Vec::new();

    for key in diff.prev.keys() {
        if !diff.next.contains_key(key) && !key.starts_with("DIRENV_") {
            out.push(format!("-{key}"));
        }
    }

    for key in diff.next.keys() {
        if key.starts_with("DIRENV_") {
            continue;
        }
        if diff.prev.contains_key(key) {
            out.push(format!("~{key}"));
        } else {
            out.push(format!("+{key}"));
        }
    }

    out.sort();
    out.join(" ")
}

fn cmd_hook(args: &[String]) -> Result<(), String> {
    let target = args.get(1).map(|s| s.as_str()).unwrap_or("");
    let shell = detect_shell(target).ok_or_else(|| format!("unknown target shell '{target}'"))?;

    let self_path = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .to_string_lossy()
        .to_string();

    let hook = shell.hook(&self_path)?;
    print!("{hook}");
    Ok(())
}

fn cmd_version(args: &[String]) -> Result<(), String> {
    if args.len() > 1 {
        // Version check mode: direnv version <version_at_least>
        let required = &args[1];
        let current_parts: Vec<u32> = VERSION.split('.').filter_map(|s| s.parse().ok()).collect();
        let required_parts: Vec<u32> = required.split('.').filter_map(|s| s.parse().ok()).collect();

        for i in 0..3 {
            let c = current_parts.get(i).copied().unwrap_or(0);
            let r = required_parts.get(i).copied().unwrap_or(0);
            if c > r {
                return Ok(());
            }
            if c < r {
                return Err(format!(
                    "current version {VERSION} is older than {required}"
                ));
            }
        }
        Ok(())
    } else {
        println!("{VERSION}");
        Ok(())
    }
}

fn cmd_status() -> Result<(), String> {
    let current_env = env::get_env();
    let config = Config::load(&current_env)?;

    println!("direnv exec path    {}", config.self_path.display());
    println!("DIRENV_CONFIG       {}", config.conf_dir.display());
    println!("bash_path           {}", config.bash_path.display());
    println!("disable_stdin       {}", config.disable_stdin);
    println!("warn_timeout        {:?}", config.warn_timeout);

    if let Some(ref tp) = config.toml_path {
        println!("config_path         {}", tp.display());
    }

    println!("load_dotenv         {}", config.load_dotenv);

    println!(
        "\nLoaded RC path      {}",
        config.env.get(DIRENV_FILE).unwrap_or(&String::new())
    );
    println!(
        "Loaded watch count  {}",
        config
            .env
            .get(DIRENV_WATCHES)
            .and_then(|w| file_times::FileTimes::unmarshal(w).ok())
            .map(|ft| ft.list.len())
            .unwrap_or(0)
    );
    println!(
        "Loaded RC allowed   {}",
        if let Some(path) = config.env.get(DIRENV_FILE) {
            if let Ok(rc) = RC::from_path(path, &config) {
                format!("{:?}", rc.allowed(&config))
            } else {
                "unknown".to_string()
            }
        } else {
            "".to_string()
        }
    );

    if let Some(watches_str) = config.env.get(DIRENV_WATCHES)
        && let Ok(times) = file_times::FileTimes::unmarshal(watches_str)
    {
        println!("\nLoaded watch files:");
        for ft in &times.list {
            println!("  {}", ft.formatted(&config.work_dir));
        }
    }

    if let Some(diff_str) = config.env.get(DIRENV_DIFF)
        && !diff_str.is_empty()
        && let Ok(diff) = EnvDiff::load(diff_str)
    {
        println!("\nLoaded diff:");
        for key in diff.prev.keys() {
            if !diff.next.contains_key(key) {
                println!("  -{key}");
            }
        }
        for key in diff.next.keys() {
            if diff.prev.contains_key(key) {
                println!("  ~{key}");
            } else {
                println!("  +{key}");
            }
        }
    }

    let wd = config.work_dir.to_string_lossy().to_string();
    if let Some(rc_path) = find_env_up(&wd, config.load_dotenv) {
        println!("\nFound RC path       {rc_path}");
        if let Ok(rc) = RC::from_path(&rc_path, &config) {
            println!("Found RC allowed    {:?}", rc.allowed(&config));
        }
    } else {
        println!("\nNo .envrc or .env loaded");
    }

    Ok(())
}

fn cmd_prune() -> Result<(), String> {
    let current_env = env::get_env();
    let config = Config::load(&current_env)?;

    let allow_dir = config.allow_dir();
    if !allow_dir.exists() {
        return Ok(());
    }

    let entries = std::fs::read_dir(&allow_dir).map_err(|e| e.to_string())?;
    let mut count = 0;

    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if let Ok(contents) = std::fs::read_to_string(&path) {
            let rc_path = contents.trim();
            if !Path::new(rc_path).exists() {
                let _ = std::fs::remove_file(&path);
                count += 1;
            }
        }
    }

    if count > 0 {
        log::log_status_default(&format!("pruned {count} entries"));
    }

    Ok(())
}

fn cmd_reload() -> Result<(), String> {
    let current_env = env::get_env();
    let config = Config::load(&current_env)?;

    if let Some(path) = config.env.get(DIRENV_FILE)
        && Path::new(path).exists()
    {
        let now = filetime::FileTime::now();
        filetime::set_file_mtime(path, now).map_err(|e| e.to_string())?;
    }

    Ok(())
}

fn cmd_dump(args: &[String]) -> Result<(), String> {
    let current_env = env::get_env();

    let format = args.get(1).map(|s| s.as_str()).unwrap_or("gzenv");

    match format {
        "json" => {
            let dump_path = current_env.get(env::DIRENV_DUMP_FILE_PATH);
            let json = serde_json::to_string(&current_env).map_err(|e| e.to_string())?;
            if let Some(path) = dump_path
                && !path.is_empty()
            {
                std::fs::write(path, &json).map_err(|e| e.to_string())?;
                return Ok(());
            }
            println!("{json}");
        }
        _ => {
            let dump_path = current_env.get(env::DIRENV_DUMP_FILE_PATH);
            let encoded = env::serialize_env(&current_env);
            if let Some(path) = dump_path
                && !path.is_empty()
            {
                std::fs::write(path, &encoded).map_err(|e| e.to_string())?;
                return Ok(());
            }
            println!("{encoded}");
        }
    }
    Ok(())
}

fn cmd_apply_dump(args: &[String]) -> Result<(), String> {
    if args.len() < 2 {
        return Err("Usage: direnv apply_dump FILE".to_string());
    }

    let path = &args[1];
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;

    // Try JSON first, then gzenv
    let new_env: Env = serde_json::from_str(&content)
        .or_else(|_| env::load_env(content.trim()))
        .map_err(|e| format!("failed to parse dump: {e}"))?;

    let current_env = env::get_env();
    let diff = EnvDiff::build(&current_env, &new_env);
    let shell_export = diff.to_shell_export();

    // Output as bash
    let shell = detect_shell("bash").unwrap();
    let out = shell.export(&shell_export)?;
    print!("{out}");

    Ok(())
}

fn cmd_show_dump(args: &[String]) -> Result<(), String> {
    if args.len() < 2 {
        return Err("Usage: direnv show_dump GZENV".to_string());
    }

    let dump_env: Env = env::load_env(&args[1])?;
    for (key, value) in &dump_env {
        println!("{key}={value}");
    }
    Ok(())
}

fn cmd_dotenv(args: &[String]) -> Result<(), String> {
    let shell_name = args.get(1).map(|s| s.as_str()).unwrap_or("bash");
    let shell = detect_shell(shell_name).ok_or_else(|| format!("unknown shell '{shell_name}'"))?;

    let target = if args.len() > 2 {
        args[2].clone()
    } else {
        ".env".to_string()
    };

    let content = std::fs::read_to_string(&target).map_err(|e| format!("reading {target}: {e}"))?;

    // Set PWD to the directory of the .env file for variable expansion
    if let Ok(abs_path) = std::fs::canonicalize(&target)
        && let Some(dir) = abs_path.parent()
    {
        // SAFETY: we're single-threaded at this point
        unsafe { std::env::set_var("PWD", dir) };
    }

    let new_env = dotenv::parse(&content)?;

    let mut export = ShellExport::new();
    for (key, value) in &new_env {
        export.add(key, value);
    }

    let out = shell.export(&export)?;
    print!("{out}");
    Ok(())
}

fn cmd_watch(args: &[String]) -> Result<(), String> {
    // Usage: direnv watch <shell> <path> [<path> ...]
    if args.len() < 3 {
        return Err("Usage: direnv watch SHELL PATH [PATH ...]".to_string());
    }

    let shell_name = &args[1];
    let shell = detect_shell(shell_name).ok_or_else(|| format!("unknown shell '{shell_name}'"))?;

    let current_env = env::get_env();

    let mut times = if let Some(watches) = current_env.get(DIRENV_WATCHES) {
        if !watches.is_empty() {
            file_times::FileTimes::unmarshal(watches)
                .unwrap_or_else(|_| file_times::FileTimes::new())
        } else {
            file_times::FileTimes::new()
        }
    } else {
        file_times::FileTimes::new()
    };

    for path in &args[2..] {
        times.update(path).ok();
    }

    let mut export = ShellExport::new();
    export.add(DIRENV_WATCHES, &times.marshal());
    let out = shell.export(&export)?;
    print!("{out}");
    Ok(())
}

fn cmd_watch_dir(args: &[String]) -> Result<(), String> {
    // Usage: direnv watch-dir <shell> <dir>
    if args.len() < 3 {
        return Err("Usage: direnv watch-dir SHELL DIR".to_string());
    }

    let shell_name = &args[1];
    let shell = detect_shell(shell_name).ok_or_else(|| format!("unknown shell '{shell_name}'"))?;

    let current_env = env::get_env();
    let dir = &args[2];

    let mut times = if let Some(watches) = current_env.get(DIRENV_WATCHES) {
        if !watches.is_empty() {
            file_times::FileTimes::unmarshal(watches)
                .unwrap_or_else(|_| file_times::FileTimes::new())
        } else {
            file_times::FileTimes::new()
        }
    } else {
        file_times::FileTimes::new()
    };

    // Watch the directory itself
    times.update(dir).ok();

    // Walk directory and watch all files
    fn walk_dir(dir: &Path, times: &mut file_times::FileTimes) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let path_str = path.to_string_lossy().to_string();
                times.update(&path_str).ok();
                if path.is_dir() {
                    walk_dir(&path, times);
                }
            }
        }
    }

    walk_dir(Path::new(dir), &mut times);

    let mut export = ShellExport::new();
    export.add(DIRENV_WATCHES, &times.marshal());
    let out = shell.export(&export)?;
    print!("{out}");
    Ok(())
}

fn cmd_watch_list() -> Result<(), String> {
    let current_env = env::get_env();

    if let Some(watches) = current_env.get(DIRENV_WATCHES)
        && !watches.is_empty()
    {
        let times = file_times::FileTimes::unmarshal(watches)?;
        for ft in &times.list {
            println!("{}", ft.path);
        }
    }
    Ok(())
}

fn cmd_watch_print(_args: &[String]) -> Result<(), String> {
    let current_env = env::get_env();

    if let Some(watches) = current_env.get(DIRENV_WATCHES)
        && !watches.is_empty()
    {
        let times = file_times::FileTimes::unmarshal(watches)?;
        let wd = std::env::current_dir().unwrap_or_default();
        for ft in &times.list {
            println!("{}", ft.formatted(&wd));
        }
    }
    Ok(())
}

fn cmd_fetchurl(args: &[String]) -> Result<(), String> {
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
    use sha2::{Digest, Sha256};

    if args.len() < 2 {
        return Err("Usage: direnv fetchurl URL [INTEGRITY_HASH]".to_string());
    }

    let url = &args[1];
    let integrity_hash = args.get(2).map(|s| s.as_str());

    let current_env = env::get_env();
    let config = Config::load(&current_env)?;

    let cas_dir = config.cache_dir.join("cas");
    std::fs::create_dir_all(&cas_dir).map_err(|e| e.to_string())?;

    // Parse SRI hash if provided
    let parsed_sri = integrity_hash.map(|h| {
        // Support base64 where '/' was replaced with '_'
        let h = h.replace('_', "/");
        parse_sri(&h)
    });

    // Check if we already have the file in CAS
    if let Some(Ok((_, ref hex_hash))) = parsed_sri {
        let cas_path = cas_dir.join(hex_hash);
        if cas_path.exists() {
            println!("{}", cas_path.display());
            return Ok(());
        }
    }

    // Download the URL using built-in HTTP client
    let response = ureq::get(url)
        .call()
        .map_err(|e| format!("HTTP request failed: {e}"))?;

    if response.status() != 200 {
        return Err(format!(
            "expected status code 200 but got {}",
            response.status()
        ));
    }

    let content = response
        .into_body()
        .read_to_vec()
        .map_err(|e| format!("reading response body: {e}"))?;

    // Calculate SHA256
    let digest = Sha256::digest(&content);
    let hex_hash = format!("{:x}", digest);
    let b64_hash = BASE64.encode(digest.as_slice());
    let sri_hash = format!("sha256-{b64_hash}");

    // Validate hash if provided
    if let Some(Ok((_, ref expected_hex))) = parsed_sri
        && hex_hash != *expected_hex
    {
        return Err(format!(
            "hash mismatch. Expected '{expected_hex}' but got '{hex_hash}'"
        ));
    }

    // Store in CAS
    let cas_path = cas_dir.join(&hex_hash);
    if !cas_path.exists() {
        std::fs::write(&cas_path, &content).map_err(|e| e.to_string())?;
        // Make read-only and executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o500);
            std::fs::set_permissions(&cas_path, perms).ok();
        }
    }

    if integrity_hash.is_none() {
        let is_tty = atty_stdout();
        if is_tty {
            eprintln!(
                "Found hash: {sri_hash}\n\n\
                 Invoke fetchurl again with the hash as an argument to get the disk location:\n\n  \
                 direnv fetchurl \"{url}\" \"{sri_hash}\"\n  \
                 #=> {}",
                cas_path.display()
            );
        } else {
            println!("{sri_hash}");
        }
    } else {
        println!("{}", cas_path.display());
    }
    Ok(())
}

/// Parse an SRI hash like "sha256-BASE64" into (algo, hex_hash)
fn parse_sri(sri: &str) -> Result<(String, String), String> {
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

    if let Some(b64) = sri.strip_prefix("sha256-") {
        let bytes = BASE64
            .decode(b64)
            .map_err(|e| format!("invalid SRI base64: {e}"))?;
        let hex = bytes.iter().fold(String::new(), |mut s, b| {
            use std::fmt::Write;
            write!(s, "{b:02x}").unwrap();
            s
        });
        Ok(("sha256".to_string(), hex))
    } else {
        // Assume it's already a hex hash
        Ok(("sha256".to_string(), sri.to_string()))
    }
}

fn atty_stdout() -> bool {
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn isatty(fd: std::ffi::c_int) -> std::ffi::c_int;
        }
        (unsafe { isatty(1) }) != 0
    }
    #[cfg(not(unix))]
    {
        false
    }
}

fn cmd_current() -> Result<(), String> {
    let current_env = env::get_env();
    if let Some(path) = current_env.get(DIRENV_FILE) {
        println!("{path}");
    }
    Ok(())
}

fn cmd_log(args: &[String]) -> Result<(), String> {
    // Usage: direnv log [-status|-error] MESSAGE
    if args.len() < 2 {
        return Ok(());
    }

    let current_env = env::get_env();
    let config = Config::load(&current_env).ok();
    let format = config
        .as_ref()
        .map(|c| c.log_format.as_str())
        .unwrap_or(config::DEFAULT_LOG_FORMAT);

    let (level, msg_start) = if args[1] == "-status" || args[1] == "-error" {
        (&args[1], 2)
    } else {
        (&"-status".to_string(), 1)
    };

    let msg = args[msg_start..].join(" ");

    let color = config.as_ref().map(|c| c.log_color).unwrap_or(false);
    if level == "-error" {
        log::log_error(format, color, &msg);
    } else {
        log::log_status(format, color, &msg);
    }

    Ok(())
}

fn cmd_check_required(args: &[String]) -> Result<(), String> {
    // Usage: direnv check-required <shell> <envrc_path> <file1> [<file2> ...]
    if args.len() < 4 {
        return Err("Usage: direnv check-required SHELL ENVRC_PATH FILE [FILE ...]".to_string());
    }

    let shell_name = &args[1];
    let _envrc_path = &args[2];
    let files = &args[3..];

    let shell = detect_shell(shell_name).ok_or_else(|| format!("unknown shell '{shell_name}'"))?;

    // For now, just accept - full implementation would track required files
    let mut export = ShellExport::new();
    let required = files.join(":");
    export.add(DIRENV_REQUIRED, &required);
    let out = shell.export(&export)?;
    print!("{out}");
    Ok(())
}
