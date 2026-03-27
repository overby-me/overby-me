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

    match std::env::set_current_dir(&target) {
        Ok(()) => {
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
            if !args.is_empty() && args[0] == "-" {
                println!("{}", new);
            }
            // If set_var failed (readonly), return 1
            if shell.last_status != saved_status {
                shell.last_status
            } else {
                0
            }
        }
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

pub(super) fn builtin_dirs(_shell: &mut Shell, _args: &[String]) -> i32 {
    match std::env::current_dir() {
        Ok(dir) => {
            println!("{}", dir.display());
            0
        }
        Err(e) => {
            eprintln!("bash: dirs: {}", e);
            1
        }
    }
}

pub(super) fn builtin_pushd(shell: &mut Shell, args: &[String]) -> i32 {
    let dir = args.first().cloned().unwrap_or_else(|| {
        shell
            .vars
            .get("HOME")
            .cloned()
            .unwrap_or_else(|| "/".to_string())
    });

    let current = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    shell.dir_stack.push(current);

    builtin_cd(shell, &[dir])
}

pub(super) fn builtin_popd(shell: &mut Shell, _args: &[String]) -> i32 {
    if let Some(dir) = shell.dir_stack.pop() {
        builtin_cd(shell, &[dir])
    } else {
        eprintln!("bash: popd: directory stack empty");
        1
    }
}
