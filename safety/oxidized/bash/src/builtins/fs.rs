use super::*;

pub(super) fn builtin_cd(shell: &mut Shell, args: &[String]) -> i32 {
    // Skip -P and -L flags
    let args: Vec<&String> = args
        .iter()
        .filter(|a| !matches!(a.as_str(), "-P" | "-L" | "-e"))
        .collect();
    if args.len() > 1 {
        eprintln!("{}: cd: too many arguments", shell.error_prefix());
        return 1;
    }
    let target = if args.is_empty() {
        match shell
            .vars
            .get("HOME")
            .cloned()
            .or_else(|| std::env::var("HOME").ok())
        {
            Some(h) if !h.is_empty() => h,
            _ => {
                eprintln!("{}: cd: HOME not set", shell.error_prefix());
                return 1;
            }
        }
    } else if args[0].as_str() == "-" {
        shell
            .vars
            .get("OLDPWD")
            .cloned()
            .or_else(|| std::env::var("OLDPWD").ok())
            .unwrap_or_else(|| ".".to_string())
    } else {
        (*args[0]).clone()
    };

    let old = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    let print_dir_on_dash = !args.is_empty() && args[0] == "-";

    // Helper closure: finalize cd after a successful set_current_dir.
    // `print_new_dir` causes the new directory to be printed to stdout
    // (used for CDPATH hits on non-current-dir entries, and for `cd -`).
    let finalize_cd = |shell: &mut Shell, old: String, print_new_dir: bool| -> i32 {
        let saved_status = shell.last_status;
        shell.set_var("OLDPWD", old);
        let new = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        shell.set_var("PWD", new.clone());
        unsafe { std::env::set_var("PWD", &new) };
        unsafe {
            std::env::set_var(
                "OLDPWD",
                shell.vars.get("OLDPWD").cloned().unwrap_or_default(),
            )
        };
        if print_new_dir {
            println!("{}", new);
        }
        if shell.last_status != saved_status {
            shell.last_status
        } else {
            0
        }
    };

    // CDPATH lookup: only for relative paths that don't start with ./ or ../
    let is_cdpath_candidate =
        !target.starts_with('/') && !target.starts_with("./") && !target.starts_with("../");

    if is_cdpath_candidate {
        let cdpath = shell
            .vars
            .get("CDPATH")
            .cloned()
            .or_else(|| std::env::var("CDPATH").ok());

        if let Some(cdpath_val) = cdpath {
            for entry in cdpath_val.split(':') {
                let try_path = if entry.is_empty() || entry == "." {
                    target.clone()
                } else {
                    format!("{}/{}", entry, target)
                };
                if std::env::set_current_dir(&try_path).is_ok() {
                    // Print directory when found via a non-current-dir CDPATH entry
                    let print = (!entry.is_empty() && entry != ".") || print_dir_on_dash;
                    return finalize_cd(shell, old, print);
                }
            }
            // CDPATH search exhausted – fall through to try target directly
        }
    }

    match std::env::set_current_dir(&target) {
        Ok(()) => finalize_cd(shell, old, print_dir_on_dash),
        Err(e) => {
            let msg = match e.kind() {
                std::io::ErrorKind::NotFound => "No such file or directory",
                std::io::ErrorKind::PermissionDenied => "Permission denied",
                std::io::ErrorKind::NotADirectory if cfg!(unix) => "Not a directory",
                _ => "No such file or directory",
            };
            eprintln!("{}: cd: {}: {}", shell.error_prefix(), target, msg);
            1
        }
    }
}

pub(super) fn builtin_pwd(shell: &mut Shell, _args: &[String]) -> i32 {
    match std::env::current_dir() {
        Ok(dir) => {
            println!("{}", dir.display());
            0
        }
        Err(e) => {
            eprintln!("{}: pwd: {}", shell.error_prefix(), e);
            1
        }
    }
}

// ── directory stack helpers ──────────────────────────────────────────────

/// Get the current working directory as a string.
fn get_pwd() -> String {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// Contract a path: replace $HOME prefix with ~.
fn tilde_contract(path: &str, home: Option<&str>) -> String {
    if let Some(h) = home
        && !h.is_empty()
    {
        if path == h {
            return "~".to_string();
        }
        if let Some(rest) = path.strip_prefix(h)
            && rest.starts_with('/')
        {
            return format!("~{}", rest);
        }
    }
    path.to_string()
}

/// Build the full logical stack: element 0 = PWD, then dir_stack entries.
/// This is the view that `dirs` displays and that +N/-N index into.
fn full_stack(shell: &Shell) -> Vec<String> {
    let mut v = vec![get_pwd()];
    v.extend(shell.dir_stack.iter().cloned());
    v
}

/// Resolve a +N or -N index against a stack of the given length.
/// Returns `Ok(index)` (0-based into the full stack) or `Err(())` on error.
fn resolve_stack_index(arg: &str, stack_len: usize) -> Result<usize, ()> {
    if let Some(n_str) = arg.strip_prefix('+') {
        let n: usize = n_str.parse().map_err(|_| ())?;
        if n < stack_len { Ok(n) } else { Err(()) }
    } else if let Some(n_str) = arg.strip_prefix('-') {
        let n: usize = n_str.parse().map_err(|_| ())?;
        if n < stack_len {
            Ok(stack_len - 1 - n)
        } else {
            Err(())
        }
    } else {
        Err(())
    }
}

/// Print the directory stack according to flags.
fn print_dirs(shell: &Shell, long: bool, per_line: bool, verbose: bool) {
    let stack = full_stack(shell);
    let home = shell
        .vars
        .get("HOME")
        .cloned()
        .or_else(|| std::env::var("HOME").ok());

    if verbose {
        for (i, d) in stack.iter().enumerate() {
            let display = if long {
                d.clone()
            } else {
                tilde_contract(d, home.as_deref())
            };
            println!("{:>2}  {}", i, display);
        }
    } else if per_line {
        for d in &stack {
            let display = if long {
                d.clone()
            } else {
                tilde_contract(d, home.as_deref())
            };
            println!("{}", display);
        }
    } else {
        let parts: Vec<String> = stack
            .iter()
            .map(|d| {
                if long {
                    d.clone()
                } else {
                    tilde_contract(d, home.as_deref())
                }
            })
            .collect();
        println!("{}", parts.join(" "));
    }
}

/// Sync the DIRSTACK array variable from the shell's dir_stack + PWD.
pub(crate) fn sync_dirstack(shell: &mut Shell) {
    let mut arr: Vec<Option<String>> = Vec::with_capacity(shell.dir_stack.len() + 1);
    // DIRSTACK[0] = PWD (current directory)
    arr.push(Some(get_pwd()));
    for d in &shell.dir_stack {
        arr.push(Some(d.clone()));
    }
    shell.arrays.insert("DIRSTACK".to_string(), arr);
}

/// Change directory (low-level): update the process CWD plus PWD/OLDPWD vars.
/// Does NOT parse cd flags or print on `-`.
fn cd_to(shell: &mut Shell, dir: &str) -> Result<(), String> {
    match std::env::set_current_dir(dir) {
        Ok(()) => {
            let old = shell.vars.get("PWD").cloned().unwrap_or_default();
            let new = get_pwd();
            shell.set_var("OLDPWD", old);
            shell.set_var("PWD", new.clone());
            unsafe {
                std::env::set_var("PWD", &new);
            }
            unsafe {
                std::env::set_var(
                    "OLDPWD",
                    shell.vars.get("OLDPWD").cloned().unwrap_or_default(),
                );
            }
            Ok(())
        }
        Err(e) => {
            let msg = match e.kind() {
                std::io::ErrorKind::NotFound => "No such file or directory".to_string(),
                std::io::ErrorKind::PermissionDenied => "Permission denied".to_string(),
                _ => e.to_string(),
            };
            Err(msg)
        }
    }
}

// ── dirs ─────────────────────────────────────────────────────────────────

pub(super) fn builtin_dirs(shell: &mut Shell, args: &[String]) -> i32 {
    let mut clear = false;
    let mut long = false;
    let mut per_line = false;
    let mut verbose = false;
    let mut positional: Option<&str> = None;

    let mut i = 0;
    let mut options_done = false;
    while i < args.len() {
        let a = args[i].as_str();
        if !options_done && a == "--" {
            options_done = true;
            i += 1;
            continue;
        }
        if a.starts_with('+') || a.starts_with('-') {
            // Check if it's a numeric index (+N / -N)
            if a.len() > 1 && a[1..].chars().all(|c| c.is_ascii_digit()) {
                positional = Some(a);
                i += 1;
                continue;
            }
            if !options_done && a.starts_with('-') {
                // It's a flag bundle
                if let Some(flags) = a.strip_prefix('-') {
                    if flags.is_empty() {
                        // bare `-` — not valid for dirs
                        positional = Some(a);
                        i += 1;
                        continue;
                    }
                    for ch in flags.chars() {
                        match ch {
                            'c' => clear = true,
                            'l' => long = true,
                            'p' => per_line = true,
                            'v' => verbose = true,
                            _ => {
                                // Check if it could be a number (e.g., -m)
                                if ch.is_ascii_digit() {
                                    // Already handled above
                                } else {
                                    // Check if it looks like an invalid number arg
                                    if flags.parse::<i64>().is_err() {
                                        eprintln!(
                                            "{}: dirs: -{}: invalid number",
                                            shell.error_prefix(),
                                            flags
                                        );
                                        eprintln!("dirs: usage: dirs [-clpv] [+N] [-N]");
                                        return 1;
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                // After --, non-numeric -X / +X is invalid
                eprintln!("{}: dirs: {}: invalid option", shell.error_prefix(), a);
                eprintln!("dirs: usage: dirs [-clpv] [+N] [-N]");
                return 2;
            }
        } else {
            // Bare word — invalid option
            eprintln!("{}: dirs: {}: invalid option", shell.error_prefix(), a);
            eprintln!("dirs: usage: dirs [-clpv] [+N] [-N]");
            return 2;
        }
        i += 1;
    }

    if clear {
        shell.dir_stack.clear();
        sync_dirstack(shell);
        return 0;
    }

    if let Some(idx_arg) = positional {
        let stack = full_stack(shell);
        match resolve_stack_index(idx_arg, stack.len()) {
            Ok(idx) => {
                let home = shell
                    .vars
                    .get("HOME")
                    .cloned()
                    .or_else(|| std::env::var("HOME").ok());
                let display = if long {
                    stack[idx].clone()
                } else {
                    tilde_contract(&stack[idx], home.as_deref())
                };
                if verbose {
                    println!("{:2}  {}", idx, display);
                } else {
                    println!("{}", display);
                }
                0
            }
            Err(()) => {
                // Extract the numeric part for the error message
                let n_str = if idx_arg.starts_with('+') || idx_arg.starts_with('-') {
                    &idx_arg[1..]
                } else {
                    idx_arg
                };
                eprintln!(
                    "{}: dirs: {}: directory stack index out of range",
                    shell.error_prefix(),
                    n_str
                );
                1
            }
        }
    } else {
        print_dirs(shell, long, per_line, verbose);
        0
    }
}

// ── pushd ────────────────────────────────────────────────────────────────

pub(super) fn builtin_pushd(shell: &mut Shell, args: &[String]) -> i32 {
    let mut no_cd = false;
    let mut positional_args: Vec<&str> = Vec::new();

    let mut options_done = false;
    for a in args {
        let s = a.as_str();
        if !options_done && s == "--" {
            options_done = true;
            continue;
        }
        if !options_done && s == "-n" {
            no_cd = true;
        } else {
            positional_args.push(s);
        }
    }

    if positional_args.is_empty() {
        // pushd with no args: swap PWD with dir_stack[0]
        if shell.dir_stack.is_empty() {
            eprintln!("{}: pushd: no other directory", shell.error_prefix());
            return 1;
        }
        let old_pwd = get_pwd();
        let target = shell.dir_stack[0].clone();
        if no_cd {
            // -n: swap without changing directory
            shell.dir_stack[0] = old_pwd;
        } else {
            match cd_to(shell, &target) {
                Ok(()) => {
                    shell.dir_stack[0] = old_pwd;
                }
                Err(msg) => {
                    eprintln!("{}: pushd: {}: {}", shell.error_prefix(), target, msg);
                    return 1;
                }
            }
        }
        sync_dirstack(shell);
        print_dirs(shell, false, false, false);
        return 0;
    }

    let arg = positional_args[0];

    // Check for +N / -N rotation
    if (arg.starts_with('+') || arg.starts_with('-'))
        && arg.len() > 1
        && arg[1..].chars().all(|c| c.is_ascii_digit())
    {
        let stack = full_stack(shell);
        match resolve_stack_index(arg, stack.len()) {
            Ok(idx) => {
                if no_cd {
                    // -n with +N/-N: rotate the stack without changing directory.
                    // Bash does NOT print dirs output for `pushd -n +N/-N`.
                    // For +0, nothing changes at all (noop).
                    if idx == 0 {
                        sync_dirstack(shell);
                        return 0;
                    }
                    // For +N/-N with -n: rotate only the dir_stack entries
                    // (not PWD). dir_stack entries are indices 1.. of full stack.
                    let mut ds = stack[1..].to_vec();
                    // Rotate dir_stack so that (idx-1) comes to front
                    if idx - 1 < ds.len() {
                        ds.rotate_left(idx - 1);
                    }
                    shell.dir_stack = ds;
                    sync_dirstack(shell);
                    return 0;
                }
                // Rotate so that entry idx becomes the new top (PWD)
                let mut new_stack = stack;
                new_stack.rotate_left(idx);
                let new_pwd = new_stack[0].clone();
                match cd_to(shell, &new_pwd) {
                    Ok(()) => {
                        shell.dir_stack = new_stack[1..].to_vec();
                        sync_dirstack(shell);
                        print_dirs(shell, false, false, false);
                        0
                    }
                    Err(msg) => {
                        eprintln!("{}: pushd: {}: {}", shell.error_prefix(), new_pwd, msg);
                        1
                    }
                }
            }
            Err(()) => {
                eprintln!(
                    "{}: pushd: {}: directory stack index out of range",
                    shell.error_prefix(),
                    arg
                );
                1
            }
        }
    } else if arg.starts_with('-') && arg.len() > 1 && !arg[1..].chars().all(|c| c.is_ascii_digit())
    {
        // Invalid flag like -m
        eprintln!("{}: pushd: {}: invalid number", shell.error_prefix(), arg);
        eprintln!("pushd: usage: pushd [-n] [+N | -N | dir]");
        1
    } else {
        // pushd dir: push current PWD onto stack and cd to dir
        let old_pwd = get_pwd();
        if no_cd {
            // -n: add dir to stack at position 1 (after PWD) without cd'ing
            shell.dir_stack.insert(0, arg.to_string());
            sync_dirstack(shell);
            print_dirs(shell, false, false, false);
            0
        } else {
            match cd_to(shell, arg) {
                Ok(()) => {
                    shell.dir_stack.insert(0, old_pwd);
                    sync_dirstack(shell);
                    print_dirs(shell, false, false, false);
                    0
                }
                Err(msg) => {
                    eprintln!("{}: pushd: {}: {}", shell.error_prefix(), arg, msg);
                    1
                }
            }
        }
    }
}

// ── popd ─────────────────────────────────────────────────────────────────

pub(super) fn builtin_popd(shell: &mut Shell, args: &[String]) -> i32 {
    let mut no_cd = false;
    let mut positional_args: Vec<&str> = Vec::new();

    let mut options_done = false;
    for a in args {
        let s = a.as_str();
        if !options_done && s == "--" {
            options_done = true;
            // After --, popd ignores remaining args (bash behavior:
            // `popd -- +8` just does a default pop, ignoring +8)
            continue;
        }
        if options_done {
            // Everything after -- is ignored for popd
            continue;
        }
        if s == "-n" {
            no_cd = true;
        } else {
            positional_args.push(s);
        }
    }

    // Validate positional args: must be +N or -N
    for a in &positional_args {
        if (a.starts_with('+') || a.starts_with('-'))
            && a.len() > 1
            && a[1..].chars().all(|c| c.is_ascii_digit())
        {
            // valid +N / -N
        } else if a.starts_with('-') && a.len() > 1 {
            eprintln!("{}: popd: {}: invalid number", shell.error_prefix(), a);
            eprintln!("popd: usage: popd [-n] [+N | -N]");
            return 1;
        } else {
            eprintln!("{}: popd: {}: invalid argument", shell.error_prefix(), a);
            eprintln!("popd: usage: popd [-n] [+N | -N]");
            return 1;
        }
    }

    let stack = full_stack(shell);

    if stack.len() <= 1 {
        eprintln!("{}: popd: directory stack empty", shell.error_prefix());
        return 1;
    }

    if positional_args.is_empty() {
        // popd: remove top entry (dir_stack[0]) and cd there
        if shell.dir_stack.is_empty() {
            eprintln!("{}: popd: directory stack empty", shell.error_prefix());
            return 1;
        }
        let target = shell.dir_stack.remove(0);
        if no_cd {
            sync_dirstack(shell);
            print_dirs(shell, false, false, false);
            return 0;
        }
        match cd_to(shell, &target) {
            Ok(()) => {
                sync_dirstack(shell);
                print_dirs(shell, false, false, false);
                0
            }
            Err(msg) => {
                eprintln!("{}: popd: {}: {}", shell.error_prefix(), target, msg);
                1
            }
        }
    } else {
        let arg = positional_args[0];
        match resolve_stack_index(arg, stack.len()) {
            Ok(idx) => {
                if idx == 0 {
                    // Remove PWD entry — cd to dir_stack[0]
                    if shell.dir_stack.is_empty() {
                        eprintln!("{}: popd: directory stack empty", shell.error_prefix());
                        return 1;
                    }
                    let target = shell.dir_stack.remove(0);
                    if !no_cd && let Err(msg) = cd_to(shell, &target) {
                        eprintln!("{}: popd: {}: {}", shell.error_prefix(), target, msg);
                        return 1;
                    }
                } else {
                    // Remove dir_stack entry at index (idx-1) since idx 0 is PWD
                    let stack_idx = idx - 1;
                    if stack_idx < shell.dir_stack.len() {
                        shell.dir_stack.remove(stack_idx);
                    }
                }
                sync_dirstack(shell);
                print_dirs(shell, false, false, false);
                0
            }
            Err(()) => {
                eprintln!(
                    "{}: popd: {}: directory stack index out of range",
                    shell.error_prefix(),
                    arg
                );
                1
            }
        }
    }
}
