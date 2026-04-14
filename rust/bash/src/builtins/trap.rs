use super::*;

pub(super) fn builtin_trap(shell: &mut Shell, args: &[String]) -> i32 {
    fn signal_number(s: &str) -> i32 {
        let upper = s.to_uppercase();
        let name = upper.strip_prefix("SIG").unwrap_or(&upper);
        match name {
            "EXIT" | "0" => 0,
            "HUP" | "1" => 1,
            "INT" | "2" => 2,
            "QUIT" | "3" => 3,
            "ILL" | "4" => 4,
            "TRAP" | "5" => 5,
            "ABRT" | "6" => 6,
            "BUS" | "7" => 7,
            "FPE" | "8" => 8,
            "KILL" | "9" => 9,
            "USR1" | "10" => 10,
            "SEGV" | "11" => 11,
            "USR2" | "12" => 12,
            "PIPE" | "13" => 13,
            "ALRM" | "14" => 14,
            "TERM" | "15" => 15,
            "CHLD" | "17" => 17,
            "CONT" | "18" => 18,
            "STOP" | "19" => 19,
            "TSTP" | "20" => 20,
            "DEBUG" => 100,
            "ERR" => 101,
            "RETURN" => 102,
            _ => 999,
        }
    }

    // Normalize signal name for display
    fn normalize_signal_name(s: &str) -> String {
        match s {
            "0" | "EXIT" | "exit" => "EXIT".to_string(),
            "ERR" | "err" => "ERR".to_string(),
            "DEBUG" | "debug" => "DEBUG".to_string(),
            "RETURN" | "return" => "RETURN".to_string(),
            _ => {
                let upper = s.to_uppercase();
                let name = upper.strip_prefix("SIG").unwrap_or(&upper);
                format!("SIG{}", name)
            }
        }
    }

    if args.is_empty() {
        // Print current traps in signal number order
        let mut sorted: Vec<_> = shell.traps.iter().collect();
        sorted.sort_by_key(|(sig, _)| signal_number(sig));
        for (signal, handler) in sorted {
            println!("trap -- '{}' {}", handler, normalize_signal_name(signal));
        }
        return 0;
    }

    // Check for invalid options
    for arg in args {
        if arg.starts_with('-')
            && arg.len() > 1
            && !matches!(arg.as_str(), "-p" | "-P" | "-l" | "-L" | "--")
        {
            eprintln!("{}: trap: {}: invalid option", shell.error_prefix(), arg);
            eprintln!("trap: usage: trap [-Plp] [[action] signal_spec ...]");
            return 2;
        }
        if arg == "--" {
            break;
        }
    }

    if args.len() == 1 {
        // trap '' or trap - : list traps or reset
        if args[0] == "-l" || args[0] == "-L" {
            // List signal names (same format as kill -l)
            let signals = list_all_signals();
            for (i, (num, name)) in signals.iter().enumerate() {
                print!("{:2}) {}", num, name);
                if (i + 1) % 5 == 0 || i == signals.len() - 1 {
                    println!();
                } else {
                    print!("\t");
                }
            }
            return 0;
        }
        if args[0] == "-p" {
            let mut sorted: Vec<_> = shell.traps.iter().collect();
            sorted.sort_by_key(|(sig, _)| signal_number(sig));
            for (signal, handler) in sorted {
                println!("trap -- '{}' {}", handler, normalize_signal_name(signal));
            }
            return 0;
        }
    }

    // Handle -p with signal arguments: trap -p SIG1 SIG2 ...
    if args.first().map(|s| s.as_str()) == Some("-p") && args.len() >= 2 {
        // Check for conflicting -P
        if args.iter().any(|a| a == "-P") {
            eprintln!(
                "{}: trap: cannot specify both -p and -P",
                shell.error_prefix()
            );
            return 2;
        }
        let mut status = 0;
        for sig_arg in &args[1..] {
            let norm = normalize_signal_name(sig_arg);
            // Validate signal name
            if signal_number(&norm) == 999 {
                eprintln!(
                    "{}: trap: {}: invalid signal specification",
                    shell.error_prefix(),
                    sig_arg
                );
                status = 1;
                continue;
            }
            let lookup = norm.strip_prefix("SIG").unwrap_or(&norm);
            let key = if lookup == "EXIT" {
                shell.traps.get("EXIT").or_else(|| shell.traps.get("0"))
            } else {
                shell.traps.get(lookup).or_else(|| shell.traps.get(&norm))
            };
            if let Some(handler) = key {
                println!("trap -- '{}' {}", handler, norm);
            }
        }
        return status;
    }

    // trap [-p|-P] 'handler' signal [signal...]
    let handler_idx = 0;
    let sig_start = 1;

    // Check for conflicting -p and -P
    if args.contains(&"-p".to_string()) && args.contains(&"-P".to_string()) {
        eprintln!(
            "{}: trap: cannot specify both -p and -P",
            shell.error_prefix()
        );
        return 2;
    }

    // Handle -P flag — print just the handler command for specified signals
    if args.first().map(|s| s.as_str()) == Some("-P") {
        if args.len() < 2 {
            eprintln!(
                "{}: trap: -P requires at least one signal name",
                shell.error_prefix()
            );
            return 1;
        }
        for sig_arg in &args[1..] {
            let norm = normalize_signal_name(sig_arg);
            let lookup = norm.strip_prefix("SIG").unwrap_or(&norm);
            let key = if lookup == "EXIT" {
                shell.traps.get("EXIT").or_else(|| shell.traps.get("0"))
            } else {
                shell.traps.get(lookup).or_else(|| shell.traps.get(&norm))
            };
            if let Some(handler) = key {
                println!("{}", handler);
            }
        }
        return 0;
    }

    // Handle -p flag — print traps for specified signals
    if args.first().map(|s| s.as_str()) == Some("-p") {
        if args.len() < 2 {
            let mut sorted: Vec<_> = shell.traps.iter().collect();
            sorted.sort_by_key(|(sig, _)| signal_number(sig));
            for (signal, handler) in sorted {
                println!("trap -- '{}' {}", handler, normalize_signal_name(signal));
            }
            return 0;
        }
        // trap -p SIG1 SIG2 ... — print traps for specific signals
        for sig_arg in &args[1..] {
            let norm = normalize_signal_name(sig_arg);
            // Traps are stored without SIG prefix, so strip it for lookup
            let lookup = norm.strip_prefix("SIG").unwrap_or(&norm);
            let key = if lookup == "EXIT" {
                shell.traps.get("EXIT").or_else(|| shell.traps.get("0"))
            } else {
                shell.traps.get(lookup).or_else(|| shell.traps.get(&norm))
            };
            if let Some(handler) = key {
                println!("trap -- '{}' {}", handler, norm);
            }
        }
        return 0;
    }

    if args.len() < sig_start + 1 {
        // Handler with no signals specified
        if handler_idx == 0 && args.len() == 1 {
            // Single arg that looks like a signal name → reset it
            let norm = normalize_signal_name(&args[0]);
            if signal_number(&norm) != 999 {
                // It's a signal name — reset it
                let signal = norm.strip_prefix("SIG").unwrap_or(&norm).to_string();
                if !shell.original_ignored_signals.contains(&signal) {
                    shell.traps.remove(&signal);
                    // Also try the original arg form (e.g., "0" stored as "0" not "EXIT")
                    let raw_signal = args[0]
                        .to_uppercase()
                        .strip_prefix("SIG")
                        .unwrap_or(&args[0].to_uppercase())
                        .to_string();
                    shell.traps.remove(&raw_signal);
                }
                return 0;
            }
            // Not a signal name — error (e.g., trap "" with no signal)
            eprintln!("trap: usage: trap [-Plp] [[action] signal_spec ...]");
            return 2;
        }
    }

    let handler = &args[handler_idx];

    let mut status = 0;
    for sig in &args[sig_start..] {
        let signal = sig.to_uppercase();
        let signal = signal.strip_prefix("SIG").unwrap_or(&signal).to_string();

        // Validate signal name/number
        let valid = matches!(
            signal.as_str(),
            "EXIT"
                | "0"
                | "HUP"
                | "INT"
                | "QUIT"
                | "ILL"
                | "TRAP"
                | "ABRT"
                | "BUS"
                | "FPE"
                | "KILL"
                | "USR1"
                | "SEGV"
                | "USR2"
                | "PIPE"
                | "ALRM"
                | "TERM"
                | "STKFLT"
                | "CHLD"
                | "CONT"
                | "STOP"
                | "TSTP"
                | "TTIN"
                | "TTOU"
                | "URG"
                | "XCPU"
                | "XFSZ"
                | "VTALRM"
                | "PROF"
                | "WINCH"
                | "IO"
                | "PWR"
                | "SYS"
                | "DEBUG"
                | "ERR"
                | "RETURN"
        ) || signal.parse::<u32>().is_ok_and(|n| n <= 64);

        if !valid {
            eprintln!(
                "{}: trap: {}: invalid signal specification",
                shell.error_prefix(),
                sig
            );
            status = 1;
            continue;
        }

        // Signals that were ignored at startup cannot be trapped or reset
        if shell.original_ignored_signals.contains(&signal) {
            // Silently ignore the request (bash behavior)
            continue;
        }
        if handler == "-" {
            // Reset trap to default
            shell.traps.remove(&signal);
            // Reset Unix signal disposition to default
            #[cfg(unix)]
            if let Some(signum) = signal_name_to_number(&signal) {
                unsafe {
                    libc::signal(signum, libc::SIG_DFL);
                }
            }
        } else if handler.is_empty() {
            // Empty handler = ignore signal
            shell.traps.insert(signal.clone(), handler.clone());
            #[cfg(unix)]
            if let Some(signum) = signal_name_to_number(&signal) {
                unsafe {
                    libc::signal(signum, libc::SIG_IGN);
                }
            }
        } else {
            // Set trap handler and install signal catcher
            shell.traps.insert(signal.clone(), handler.clone());
            #[cfg(unix)]
            if let Some(signum) = signal_name_to_number(&signal) {
                crate::interpreter::install_signal_handler(signum);
            }
        }
    }
    status
}

pub(super) fn signal_name_to_number(name: &str) -> Option<i32> {
    match name {
        "HUP" => Some(libc::SIGHUP),
        "INT" => Some(libc::SIGINT),
        "QUIT" => Some(libc::SIGQUIT),
        "ILL" => Some(libc::SIGILL),
        "TRAP" => Some(libc::SIGTRAP),
        "ABRT" => Some(libc::SIGABRT),
        "BUS" => Some(libc::SIGBUS),
        "FPE" => Some(libc::SIGFPE),
        "KILL" => Some(libc::SIGKILL),
        "USR1" => Some(libc::SIGUSR1),
        "SEGV" => Some(libc::SIGSEGV),
        "USR2" => Some(libc::SIGUSR2),
        "PIPE" => Some(libc::SIGPIPE),
        "ALRM" => Some(libc::SIGALRM),
        "TERM" => Some(libc::SIGTERM),
        "CHLD" => Some(libc::SIGCHLD),
        "CONT" => Some(libc::SIGCONT),
        "STOP" => Some(libc::SIGSTOP),
        "TSTP" => Some(libc::SIGTSTP),
        _ => None,
    }
}

pub(super) fn builtin_wait(shell: &mut Shell, args: &[String]) -> i32 {
    #[cfg(unix)]
    {
        use nix::sys::wait::{WaitStatus, waitpid};
        use nix::unistd::Pid;

        // Parse flags: -n (wait for next), -f (force wait for terminated),
        // -p var (store PID in variable)
        let mut flag_n = false;
        let mut flag_f = false;
        let mut p_var: Option<String> = None;
        let mut id_args: Vec<String> = Vec::new();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "-n" => flag_n = true,
                "-f" => flag_f = true,
                "-p" => {
                    if i + 1 < args.len() {
                        i += 1;
                        p_var = Some(args[i].clone());
                    } else {
                        eprintln!("bash: wait: -p: option requires an argument");
                        return 2;
                    }
                }
                "-fn" | "-nf" => {
                    flag_f = true;
                    flag_n = true;
                }
                arg if arg.starts_with('-')
                    && arg.len() > 1
                    && !arg[1..].starts_with(|c: char| c.is_ascii_digit()) =>
                {
                    // Parse combined flags like -np var
                    let flags = &arg[1..];
                    let mut j = 0;
                    let chars: Vec<char> = flags.chars().collect();
                    while j < chars.len() {
                        match chars[j] {
                            'n' => flag_n = true,
                            'f' => flag_f = true,
                            'p' => {
                                // Rest of this arg or next arg is the variable name
                                let rest: String = chars[j + 1..].iter().collect();
                                if !rest.is_empty() {
                                    p_var = Some(rest);
                                } else if i + 1 < args.len() {
                                    i += 1;
                                    p_var = Some(args[i].clone());
                                } else {
                                    eprintln!("bash: wait: -p: option requires an argument");
                                    return 2;
                                }
                                j = chars.len(); // consumed
                                continue;
                            }
                            _ => {
                                // Unknown flag — treat entire arg as an id
                                id_args.push(args[i].clone());
                                j = chars.len();
                                continue;
                            }
                        }
                        j += 1;
                    }
                    i += 1;
                    continue;
                }
                _ => {
                    id_args.push(args[i].clone());
                }
            }
            i += 1;
        }

        // Helper: resolve a job spec like %N to a PID
        let resolve_job_spec = |spec: &str, jobs: &[crate::interpreter::Job]| -> Option<i32> {
            if let Some(num_str) = spec.strip_prefix('%') {
                if let Ok(num) = num_str.parse::<usize>() {
                    // %N — find job by number
                    jobs.iter().find(|j| j.number == num).map(|j| j.pid)
                } else if num_str == "%" || num_str.is_empty() {
                    // %% or % — current job (last)
                    jobs.last().map(|j| j.pid)
                } else if num_str == "+" {
                    jobs.last().map(|j| j.pid)
                } else if num_str == "-" {
                    // Previous job
                    if jobs.len() >= 2 {
                        Some(jobs[jobs.len() - 2].pid)
                    } else {
                        jobs.last().map(|j| j.pid)
                    }
                } else {
                    // %string — match by command prefix
                    jobs.iter()
                        .rev()
                        .find(|j| j.command.starts_with(num_str))
                        .map(|j| j.pid)
                }
            } else {
                spec.parse::<i32>().ok()
            }
        };

        // Helper: store PID in the -p variable (supports array subscripts like A[$key])
        let store_pid_var = |shell: &mut Shell, var: &str, pid: i32| {
            // Check if it's an array subscript like A[key]
            // When assoc_expand_once is ON, use rfind(']') so that ']' can be a key
            let aeo = shell.is_array_expand_once();
            let bracket_open = var.find('[');
            let bracket_close = if aeo {
                var.rfind(']')
            } else {
                // Find the matching ']' for the first '['
                bracket_open.and_then(|open| var[open + 1..].find(']').map(|p| open + 1 + p))
            };
            if let (Some(open), Some(close)) = (bracket_open, bracket_close)
                && close == var.len() - 1
            {
                let arr_name = &var[..open];
                let subscript = &var[open + 1..close];
                if shell.assoc_arrays.contains_key(arr_name) {
                    // Expand the subscript for variable references
                    let expanded_key = if shell.is_array_expand_once() {
                        subscript.to_string()
                    } else {
                        shell.expand_assoc_subscript(subscript)
                    };
                    if let Some(assoc) = shell.assoc_arrays.get_mut(arr_name) {
                        assoc.insert(expanded_key, pid.to_string());
                    }
                    return;
                } else if shell.arrays.contains_key(arr_name) {
                    if aeo {
                        shell.arith_skip_comsub_expand = true;
                    }
                    let idx = shell.eval_arith_expr(subscript) as usize;
                    shell.arith_skip_comsub_expand = false;
                    if crate::expand::take_arith_error() {
                        // Subscript evaluation failed (e.g. $(cmd) with
                        // array_expand_once) — skip the assignment.
                        return;
                    }
                    let arr = shell.arrays.get_mut(arr_name).unwrap();
                    while arr.len() <= idx {
                        arr.push(None);
                    }
                    arr[idx] = Some(pid.to_string());
                    return;
                }
            }
            shell.set_var(var, pid.to_string());
        };

        // Helper: fire CHLD trap if set
        let fire_chld_trap = |shell: &mut Shell| {
            if let Some(handler) = shell.traps.get("CHLD").cloned()
                && !handler.is_empty()
            {
                crate::interpreter::take_pending_signal(libc::SIGCHLD);
                shell.in_trap_handler += 1;
                shell.run_string(&handler);
                shell.in_trap_handler -= 1;
            }
        };

        let _ = flag_f; // flag_f acknowledged but not changing blocking behavior

        if flag_n {
            // wait -n: wait for the next child to complete
            // If id_args are given, wait for next among those specific jobs/PIDs
            if id_args.is_empty() {
                // Wait for any child
                match waitpid(Pid::from_raw(-1), None) {
                    Ok(WaitStatus::Exited(pid, code)) => {
                        shell.last_status = code;
                        if let Some(ref var) = p_var {
                            store_pid_var(shell, var, pid.as_raw());
                        }
                        // Update job status
                        for job in shell.jobs.iter_mut() {
                            if job.pid == pid.as_raw() {
                                job.status = crate::interpreter::JobStatus::Done(code);
                            }
                        }
                        fire_chld_trap(shell);
                        return code;
                    }
                    Ok(WaitStatus::Signaled(pid, sig, _)) => {
                        let code = 128 + sig as i32;
                        shell.last_status = code;
                        if let Some(ref var) = p_var {
                            store_pid_var(shell, var, pid.as_raw());
                        }
                        for job in shell.jobs.iter_mut() {
                            if job.pid == pid.as_raw() {
                                job.status = crate::interpreter::JobStatus::Done(code);
                            }
                        }
                        fire_chld_trap(shell);
                        return code;
                    }
                    _ => return 127,
                }
            } else {
                // Wait for next among specific PIDs/job specs
                // Resolve all to PIDs first
                let mut target_pids: Vec<i32> = Vec::new();
                for arg in &id_args {
                    if let Some(pid) = resolve_job_spec(arg, &shell.jobs) {
                        target_pids.push(pid);
                    }
                }

                if target_pids.is_empty() {
                    return 127;
                }

                // Poll in a loop until one of our targets finishes
                loop {
                    for &pid in &target_pids {
                        match waitpid(
                            Pid::from_raw(pid),
                            Some(nix::sys::wait::WaitPidFlag::WNOHANG),
                        ) {
                            Ok(WaitStatus::Exited(rpid, code)) => {
                                shell.last_status = code;
                                if let Some(ref var) = p_var {
                                    store_pid_var(shell, var, rpid.as_raw());
                                }
                                for job in shell.jobs.iter_mut() {
                                    if job.pid == rpid.as_raw() {
                                        job.status = crate::interpreter::JobStatus::Done(code);
                                    }
                                }
                                fire_chld_trap(shell);
                                return code;
                            }
                            Ok(WaitStatus::Signaled(rpid, sig, _)) => {
                                let code = 128 + sig as i32;
                                shell.last_status = code;
                                if let Some(ref var) = p_var {
                                    store_pid_var(shell, var, rpid.as_raw());
                                }
                                for job in shell.jobs.iter_mut() {
                                    if job.pid == rpid.as_raw() {
                                        job.status = crate::interpreter::JobStatus::Done(code);
                                    }
                                }
                                fire_chld_trap(shell);
                                return code;
                            }
                            Ok(_) => {} // still running or stopped
                            Err(nix::errno::Errno::ECHILD) => {
                                // Process already reaped — check job table for saved status
                                for job in shell.jobs.iter() {
                                    if job.pid == pid
                                        && let crate::interpreter::JobStatus::Done(code) =
                                            job.status
                                    {
                                        shell.last_status = code;
                                        if let Some(ref var) = p_var {
                                            store_pid_var(shell, var, pid);
                                        }
                                        return code;
                                    }
                                }
                            }
                            Err(_) => {}
                        }
                    }
                    // Brief sleep to avoid busy-waiting
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        }

        if id_args.is_empty() {
            // Wait for all background children
            let has_chld_trap = shell.traps.contains_key("CHLD");
            loop {
                match waitpid(Pid::from_raw(-1), None) {
                    Ok(WaitStatus::Exited(_, code)) => {
                        shell.last_status = code;
                        if has_chld_trap {
                            crate::interpreter::take_pending_signal(libc::SIGCHLD);
                            if let Some(handler) = shell.traps.get("CHLD").cloned()
                                && !handler.is_empty()
                            {
                                shell.in_trap_handler += 1;
                                shell.run_string(&handler);
                                shell.in_trap_handler -= 1;
                            }
                        }
                    }
                    Ok(WaitStatus::Signaled(_, sig, _)) => {
                        shell.last_status = 128 + sig as i32;
                        if has_chld_trap {
                            crate::interpreter::take_pending_signal(libc::SIGCHLD);
                            if let Some(handler) = shell.traps.get("CHLD").cloned()
                                && !handler.is_empty()
                            {
                                shell.in_trap_handler += 1;
                                shell.run_string(&handler);
                                shell.in_trap_handler -= 1;
                            }
                        }
                    }
                    Ok(_) => continue,
                    Err(_) => break,
                }
            }
            // wait with no arguments returns 0 per POSIX
            return 0;
        } else {
            // Wait for specific PIDs/job specs
            for arg in &id_args {
                if let Some(pid) = resolve_job_spec(arg, &shell.jobs) {
                    match waitpid(Pid::from_raw(pid), None) {
                        Ok(WaitStatus::Exited(rpid, code)) => {
                            shell.last_status = code;
                            if let Some(ref var) = p_var {
                                store_pid_var(shell, var, rpid.as_raw());
                            }
                            for job in shell.jobs.iter_mut() {
                                if job.pid == rpid.as_raw() {
                                    job.status = crate::interpreter::JobStatus::Done(code);
                                }
                            }
                        }
                        Ok(WaitStatus::Signaled(rpid, sig, _)) => {
                            let sig_code = 128 + sig as i32;
                            shell.last_status = sig_code;
                            if let Some(ref var) = p_var {
                                store_pid_var(shell, var, rpid.as_raw());
                            }
                            for job in shell.jobs.iter_mut() {
                                if job.pid == rpid.as_raw() {
                                    job.status = crate::interpreter::JobStatus::Done(sig_code);
                                }
                            }
                        }
                        Err(nix::errno::Errno::ECHILD) => {
                            // Already reaped — check job table
                            for job in shell.jobs.iter() {
                                if job.pid == pid
                                    && let crate::interpreter::JobStatus::Done(code) = job.status
                                {
                                    shell.last_status = code;
                                    break;
                                }
                            }
                        }
                        _ => {}
                    }
                } else {
                    eprintln!("bash: wait: {}: no such job", arg);
                    shell.last_status = 127;
                }
            }
        }
    }
    // Clean up any coprocs whose PIDs were reaped by wait
    cleanup_reaped_coprocs(shell);
    shell.last_status
}

/// Clean up coproc arrays/PIDs after wait has reaped their processes.
/// Unlike reap_coprocs (which uses waitpid), this checks if the PID
/// process no longer exists (already reaped by wait) and cleans up.
fn cleanup_reaped_coprocs(shell: &mut crate::interpreter::Shell) {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;

    let coproc_names: Vec<(String, i32)> = shell
        .vars
        .iter()
        .filter(|(k, _)| k.ends_with("_PID"))
        .filter_map(|(k, v)| {
            let name = k.strip_suffix("_PID")?;
            let pid: i32 = v.parse().ok()?;
            // Check if this is actually a coproc (has matching array)
            if shell.arrays.contains_key(name) {
                Some((name.to_string(), pid))
            } else {
                None
            }
        })
        .collect();

    for (name, pid) in coproc_names {
        // Check if process is still alive
        let alive = kill(Pid::from_raw(pid), None).is_ok();
        if !alive {
            // Process was reaped — clean up
            let pid_key = format!("{}_PID", name);
            if shell.readonly_vars.contains(name.as_str()) {
                eprintln!(
                    "{}: {}: cannot unset: readonly variable",
                    shell.error_prefix(),
                    name
                );
            } else {
                shell.arrays.remove(&name);
            }
            if !shell.readonly_vars.contains(pid_key.as_str()) {
                shell.vars.remove(&pid_key);
            }
        }
    }

    // Also check coproc_info for cases where readonly prevented storing
    // the PID variable (so coproc_names above is empty)
    if let Some((name, pid)) = shell.coproc_info.take() {
        let alive = kill(Pid::from_raw(pid), None).is_ok();
        if !alive {
            if shell.readonly_vars.contains(name.as_str()) {
                eprintln!(
                    "{}: {}: cannot unset: readonly variable",
                    shell.error_prefix(),
                    name
                );
            }
            // Also clean up the _PID variable (force remove even if readonly,
            // since coproc cleanup in bash removes _PID unconditionally)
            let pid_key = format!("{}_PID", name);
            shell.vars.remove(&pid_key);
            shell.readonly_vars.remove(&pid_key);
            shell.declared_unset.remove(&pid_key);
        }
    }
}

pub(super) fn builtin_kill(shell: &mut Shell, args: &[String]) -> i32 {
    #[cfg(unix)]
    {
        use nix::sys::signal::{self, Signal};
        use nix::unistd::Pid;

        if args.is_empty() {
            eprintln!(
                "kill: usage: kill [-s sigspec | -n signum | -sigspec] pid | jobspec ... or kill -l [sigspec]"
            );
            return 2;
        }

        // Handle kill -l [signum]
        if args.first().map(|s| s.as_str()) == Some("-l")
            || args.first().map(|s| s.as_str()) == Some("-L")
        {
            if args.len() > 1 {
                let sig_names: &[(&str, i32)] = &[
                    ("HUP", 1),
                    ("INT", 2),
                    ("QUIT", 3),
                    ("ILL", 4),
                    ("TRAP", 5),
                    ("ABRT", 6),
                    ("BUS", 7),
                    ("FPE", 8),
                    ("KILL", 9),
                    ("USR1", 10),
                    ("SEGV", 11),
                    ("USR2", 12),
                    ("PIPE", 13),
                    ("ALRM", 14),
                    ("TERM", 15),
                    ("STKFLT", 16),
                    ("CHLD", 17),
                    ("CONT", 18),
                    ("STOP", 19),
                    ("TSTP", 20),
                    ("TTIN", 21),
                    ("TTOU", 22),
                    ("URG", 23),
                    ("XCPU", 24),
                    ("XFSZ", 25),
                    ("VTALRM", 26),
                    ("PROF", 27),
                    ("WINCH", 28),
                    ("IO", 29),
                    ("PWR", 30),
                    ("SYS", 31),
                ];
                for arg in &args[1..] {
                    if let Ok(num) = arg.parse::<i32>() {
                        // kill -l <signum> — print signal name
                        let num = if num > 128 { num - 128 } else { num };
                        if let Some((name, _)) = sig_names.iter().find(|(_, n)| *n == num) {
                            println!("{}", name);
                        } else {
                            eprintln!(
                                "{}: kill: {}: invalid signal specification",
                                shell.error_prefix(),
                                arg
                            );
                            return 1;
                        }
                    } else {
                        // kill -l <name> — print signal number
                        let upper = arg.to_uppercase();
                        let upper = upper.strip_prefix("SIG").unwrap_or(&upper);
                        if let Some((_, num)) = sig_names.iter().find(|(n, _)| *n == upper) {
                            println!("{}", num);
                        } else {
                            eprintln!(
                                "{}: kill: {}: invalid signal specification",
                                shell.error_prefix(),
                                arg
                            );
                            return 1;
                        }
                    }
                }
            } else {
                // kill -l — list all signals
                // Use same signal list as trap -l
                let signals = list_all_signals();
                for (i, (num, name)) in signals.iter().enumerate() {
                    print!("{:2}) {}", num, name);
                    if (i + 1) % 5 == 0 || i == signals.len() - 1 {
                        println!();
                    } else {
                        print!("\t");
                    }
                }
            }
            return 0;
        }

        let mut signal = Signal::SIGTERM;
        let mut pids = Vec::new();

        let parse_signal = |name: &str| -> Option<Signal> {
            let upper = name.to_uppercase();
            let upper = upper.strip_prefix("SIG").unwrap_or(&upper);
            match upper {
                "HUP" => Some(Signal::SIGHUP),
                "INT" => Some(Signal::SIGINT),
                "QUIT" => Some(Signal::SIGQUIT),
                "ILL" => Some(Signal::SIGILL),
                "TRAP" => Some(Signal::SIGTRAP),
                "ABRT" => Some(Signal::SIGABRT),
                "BUS" => Some(Signal::SIGBUS),
                "FPE" => Some(Signal::SIGFPE),
                "KILL" => Some(Signal::SIGKILL),
                "USR1" => Some(Signal::SIGUSR1),
                "SEGV" => Some(Signal::SIGSEGV),
                "USR2" => Some(Signal::SIGUSR2),
                "PIPE" => Some(Signal::SIGPIPE),
                "ALRM" => Some(Signal::SIGALRM),
                "TERM" => Some(Signal::SIGTERM),
                "CHLD" => Some(Signal::SIGCHLD),
                "CONT" => Some(Signal::SIGCONT),
                "STOP" => Some(Signal::SIGSTOP),
                "TSTP" => Some(Signal::SIGTSTP),
                "TTIN" => Some(Signal::SIGTTIN),
                "TTOU" => Some(Signal::SIGTTOU),
                _ => None,
            }
        };

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if arg == "-s" || arg == "-n" {
                i += 1;
                if i >= args.len() {
                    eprintln!(
                        "{}: kill: {}: option requires an argument",
                        shell.error_prefix(),
                        arg
                    );
                    return 2;
                }
                let sig_arg = &args[i];
                if let Ok(n) = sig_arg.parse::<i32>() {
                    signal = Signal::try_from(n).unwrap_or(Signal::SIGTERM);
                } else if let Some(sig) = parse_signal(sig_arg) {
                    signal = sig;
                } else {
                    eprintln!(
                        "{}: kill: {}: invalid signal specification",
                        shell.error_prefix(),
                        sig_arg
                    );
                    return 1;
                }
            } else if arg.starts_with('-') && arg.len() > 1 {
                let sig_name = &arg[1..];
                if let Ok(n) = sig_name.parse::<i32>() {
                    signal = Signal::try_from(n).unwrap_or(Signal::SIGTERM);
                } else if let Some(sig) = parse_signal(sig_name) {
                    signal = sig;
                } else {
                    eprintln!(
                        "{}: kill: {}: invalid signal specification",
                        shell.error_prefix(),
                        sig_name
                    );
                    return 1;
                }
            } else if arg.is_empty() {
                eprintln!(
                    "{}: kill: `': not a pid or valid job spec",
                    shell.error_prefix()
                );
                return 1;
            } else if let Ok(pid) = arg.parse::<i32>() {
                pids.push(pid);
            } else {
                eprintln!(
                    "{}: kill: `{}': not a pid or valid job spec",
                    shell.error_prefix(),
                    arg
                );
                return 1;
            }
            i += 1;
        }

        if pids.is_empty() {
            eprintln!(
                "kill: usage: kill [-s sigspec | -n signum | -sigspec] pid | jobspec ... or kill -l [sigspec]"
            );
            return 2;
        }
        let mut status = 0;
        for pid in pids {
            if signal::kill(Pid::from_raw(pid), signal).is_err() {
                eprintln!(
                    "{}: kill: ({}) - No such process",
                    shell.error_prefix(),
                    pid
                );
                status = 1;
            }
        }
        status
    }
    #[cfg(not(unix))]
    {
        let _ = args;
        eprintln!(
            "{}: kill: not supported on this platform",
            shell.error_prefix()
        );
        1
    }
}

pub(super) fn builtin_enable(shell: &mut Shell, args: &[String]) -> i32 {
    let builtin_map = builtins();

    // POSIX special builtins
    let special_builtins: std::collections::HashSet<&str> = [
        ".", ":", "break", "continue", "eval", "exec", "exit", "export", "readonly", "return",
        "set", "shift", "source", "times", "trap", "unset",
    ]
    .iter()
    .copied()
    .collect();

    let mut flag_n = false; // disable
    let mut _flag_p = false; // print
    let mut flag_s = false; // special builtins only
    let mut flag_a = false; // all (include disabled)
    let mut flag_d = false; // delete dynamically loaded
    let mut names: Vec<String> = Vec::new();

    for arg in args {
        if arg.starts_with('-') && arg.len() > 1 {
            for ch in arg[1..].chars() {
                match ch {
                    'n' => flag_n = true,
                    'p' => _flag_p = true,
                    's' => flag_s = true,
                    'a' => flag_a = true,
                    'd' => flag_d = true,
                    _ => {
                        eprintln!("{}: enable: -{}: invalid option", shell.error_prefix(), ch);
                        return 2;
                    }
                }
            }
        } else {
            names.push(arg.clone());
        }
    }

    // enable -d NAME: attempt to unload dynamically loaded builtin (we don't support dynamic loading)
    if flag_d {
        for name in &names {
            if builtin_map.contains_key(name.as_str()) {
                eprintln!(
                    "{}: enable: {}: not dynamically loaded",
                    shell.error_prefix(),
                    name
                );
            } else {
                eprintln!(
                    "{}: enable: {}: not a shell builtin",
                    shell.error_prefix(),
                    name
                );
            }
        }
        return if names.is_empty() { 0 } else { 1 };
    }

    // If no names given, list builtins
    if names.is_empty() {
        let mut all_names: Vec<&&str> = builtin_map.keys().collect();
        all_names.sort();

        if flag_n && !flag_a {
            // enable -n: list disabled builtins
            let mut disabled: Vec<&String> = shell.disabled_builtins.iter().collect();
            disabled.sort();
            for name in disabled {
                if flag_s && !special_builtins.contains(name.as_str()) {
                    continue;
                }
                println!("enable -n {}", name);
            }
        } else {
            // List enabled (and optionally disabled) builtins
            for name in &all_names {
                let is_disabled = shell.disabled_builtins.contains(**name);
                if flag_s && !special_builtins.contains(**name) {
                    continue;
                }
                if flag_a {
                    // Show all
                    if is_disabled {
                        println!("enable -n {}", name);
                    } else {
                        println!("enable {}", name);
                    }
                } else if !is_disabled {
                    // Only show enabled
                    println!("enable {}", name);
                }
            }
        }
        return 0;
    }

    // Process named builtins
    let mut status = 0;
    for name in &names {
        if !builtin_map.contains_key(name.as_str()) {
            eprintln!(
                "{}: enable: {}: not a shell builtin",
                shell.error_prefix(),
                name
            );
            status = 1;
            continue;
        }
        if flag_n {
            // Disable the builtin
            shell.disabled_builtins.insert(name.clone());
        } else {
            // Re-enable the builtin
            shell.disabled_builtins.remove(name.as_str());
        }
    }
    status
}

pub(super) fn builtin_suspend(_shell: &mut Shell, _args: &[String]) -> i32 {
    eprintln!("bash: suspend: cannot suspend");
    1
}

pub(super) fn builtin_times(_shell: &mut Shell, _args: &[String]) -> i32 {
    #[cfg(unix)]
    {
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        // Get resource usage for this process
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
        println!(
            "{}m{}.{:03}s {}m{}.{:03}s",
            usage.ru_utime.tv_sec / 60,
            usage.ru_utime.tv_sec % 60,
            usage.ru_utime.tv_usec / 1000,
            usage.ru_stime.tv_sec / 60,
            usage.ru_stime.tv_sec % 60,
            usage.ru_stime.tv_usec / 1000,
        );
        // Get resource usage for children
        let mut child_usage: libc::rusage = unsafe { std::mem::zeroed() };
        unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, &mut child_usage) };
        println!(
            "{}m{}.{:03}s {}m{}.{:03}s",
            child_usage.ru_utime.tv_sec / 60,
            child_usage.ru_utime.tv_sec % 60,
            child_usage.ru_utime.tv_usec / 1000,
            child_usage.ru_stime.tv_sec / 60,
            child_usage.ru_stime.tv_sec % 60,
            child_usage.ru_stime.tv_usec / 1000,
        );
        0
    }
    #[cfg(not(unix))]
    {
        println!("0m0.000s 0m0.000s");
        println!("0m0.000s 0m0.000s");
        0
    }
}

pub(super) fn builtin_ulimit(shell: &mut Shell, args: &[String]) -> i32 {
    #[cfg(unix)]
    {
        // Resource descriptor: (flag char, libc resource, description, block divisor)
        // divisor: 0 = no scaling, 512 = 512-byte blocks, 1024 = 1024-byte blocks
        struct ResInfo {
            flag: char,
            resource: libc::__rlimit_resource_t,
            desc: &'static str,
            divisor: u64,
        }

        let resources: &[ResInfo] = &[
            ResInfo {
                flag: 'c',
                resource: libc::RLIMIT_CORE,
                desc: "core file size",
                divisor: 512,
            },
            ResInfo {
                flag: 'd',
                resource: libc::RLIMIT_DATA,
                desc: "data seg size",
                divisor: 1024,
            },
            ResInfo {
                flag: 'e',
                resource: libc::RLIMIT_NICE,
                desc: "scheduling priority",
                divisor: 0,
            },
            ResInfo {
                flag: 'f',
                resource: libc::RLIMIT_FSIZE,
                desc: "file size",
                divisor: 512,
            },
            ResInfo {
                flag: 'i',
                resource: libc::RLIMIT_SIGPENDING,
                desc: "pending signals",
                divisor: 0,
            },
            ResInfo {
                flag: 'k',
                resource: libc::RLIMIT_MSGQUEUE,
                desc: "max kqueues",
                divisor: 0,
            },
            ResInfo {
                flag: 'l',
                resource: libc::RLIMIT_MEMLOCK,
                desc: "max locked memory",
                divisor: 1024,
            },
            ResInfo {
                flag: 'm',
                resource: libc::RLIMIT_RSS,
                desc: "max memory size",
                divisor: 1024,
            },
            ResInfo {
                flag: 'n',
                resource: libc::RLIMIT_NOFILE,
                desc: "open files",
                divisor: 0,
            },
            ResInfo {
                flag: 'p',
                resource: libc::RLIMIT_NPROC,
                desc: "pipe size",
                divisor: 512,
            },
            ResInfo {
                flag: 'q',
                resource: libc::RLIMIT_MSGQUEUE,
                desc: "POSIX message queues",
                divisor: 0,
            },
            ResInfo {
                flag: 'r',
                resource: libc::RLIMIT_RTPRIO,
                desc: "real-time priority",
                divisor: 0,
            },
            ResInfo {
                flag: 's',
                resource: libc::RLIMIT_STACK,
                desc: "stack size",
                divisor: 1024,
            },
            ResInfo {
                flag: 't',
                resource: libc::RLIMIT_CPU,
                desc: "cpu time",
                divisor: 0,
            },
            ResInfo {
                flag: 'u',
                resource: libc::RLIMIT_NPROC,
                desc: "max user processes",
                divisor: 0,
            },
            ResInfo {
                flag: 'v',
                resource: libc::RLIMIT_AS,
                desc: "virtual memory",
                divisor: 1024,
            },
            ResInfo {
                flag: 'x',
                resource: libc::RLIMIT_LOCKS,
                desc: "file locks",
                divisor: 0,
            },
            ResInfo {
                flag: 'P',
                resource: libc::RLIMIT_NPROC,
                desc: "number of pseudoterminals",
                divisor: 0,
            },
            ResInfo {
                flag: 'R',
                resource: libc::RLIMIT_RTTIME,
                desc: "real-time non-blocking time",
                divisor: 0,
            },
            ResInfo {
                flag: 'T',
                resource: libc::RLIMIT_NPROC,
                desc: "number of threads",
                divisor: 0,
            },
        ];

        let find_res =
            |flag: char| -> Option<&ResInfo> { resources.iter().find(|r| r.flag == flag) };

        let mut use_soft = true;
        let mut use_hard = false;
        let mut explicit_sh = false; // whether -S or -H was explicitly given
        let mut print_all = false;
        let mut selected_flag: char = 'f'; // default resource
        let mut set_value: Option<u64> = None;
        let mut set_value_str: Option<String> = None;
        let mut past_options = false;
        let mut had_error = false;

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];

            if arg == "--" && !past_options {
                past_options = true;
                i += 1;
                continue;
            }

            if !past_options && arg.starts_with('-') && arg.len() > 1 && !arg.starts_with("--") {
                // Parse combined flags like -Sc, -Hn, -SHf
                let chars: Vec<char> = arg[1..].chars().collect();
                let mut j = 0;
                while j < chars.len() {
                    match chars[j] {
                        'S' => {
                            use_soft = true;
                            use_hard = false;
                            explicit_sh = true;
                        }
                        'H' => {
                            use_hard = true;
                            use_soft = false;
                            explicit_sh = true;
                        }
                        'a' => print_all = true,
                        ch => {
                            if find_res(ch).is_some() {
                                selected_flag = ch;
                            } else {
                                eprintln!(
                                    "{}: ulimit: -{}: invalid option",
                                    shell.error_prefix(),
                                    ch
                                );
                                eprintln!(
                                    "ulimit: usage: ulimit [-SHabcdefiklmnpqrstuvxPRT] [limit]"
                                );
                                return 2;
                            }
                        }
                    }
                    j += 1;
                }
                i += 1;
                continue;
            }

            // Value argument
            let val = arg.as_str();
            match val {
                "unlimited" => set_value = Some(libc::RLIM_INFINITY),
                "soft" => {
                    // Set to current soft limit value
                    if let Some(res) = find_res(selected_flag) {
                        let mut rlim: libc::rlimit = unsafe { std::mem::zeroed() };
                        unsafe { libc::getrlimit(res.resource, &mut rlim) };
                        set_value = Some(rlim.rlim_cur);
                    }
                }
                "hard" => {
                    // Set to current hard limit value
                    if let Some(res) = find_res(selected_flag) {
                        let mut rlim: libc::rlimit = unsafe { std::mem::zeroed() };
                        unsafe { libc::getrlimit(res.resource, &mut rlim) };
                        set_value = Some(rlim.rlim_max);
                    }
                }
                _ => {
                    // Check for leading +
                    if val.starts_with('+') {
                        eprintln!("{}: ulimit: {}: invalid number", shell.error_prefix(), val);
                        had_error = true;
                        i += 1;
                        continue;
                    }
                    if let Ok(n) = val.parse::<u64>() {
                        set_value = Some(n);
                        set_value_str = Some(val.to_string());
                    } else {
                        eprintln!("{}: ulimit: {}: invalid number", shell.error_prefix(), val);
                        had_error = true;
                    }
                }
            }
            i += 1;
        }

        if had_error {
            return 1;
        }

        // Helper to get the limit value to display
        let get_limit = |res: &ResInfo, soft: bool, hard: bool| -> String {
            let mut rlim: libc::rlimit = unsafe { std::mem::zeroed() };
            unsafe { libc::getrlimit(res.resource, &mut rlim) };
            let val = if hard && !soft {
                rlim.rlim_max
            } else {
                rlim.rlim_cur
            };
            if val == libc::RLIM_INFINITY {
                "unlimited".to_string()
            } else if res.divisor > 0 {
                format!("{}", val / res.divisor)
            } else {
                format!("{}", val)
            }
        };

        if print_all {
            // Print all limits (like ulimit -a)
            for res in resources {
                // Skip duplicate flag entries (p and P map differently)
                let val_str = get_limit(res, use_soft, use_hard);
                println!("-{}: {:<30} {}", res.flag, res.desc, val_str);
            }
            return 0;
        }

        let res = match find_res(selected_flag) {
            Some(r) => r,
            None => {
                eprintln!(
                    "{}: ulimit: -{}: invalid option",
                    shell.error_prefix(),
                    selected_flag
                );
                return 2;
            }
        };

        if let Some(val) = set_value {
            // Scale the value if needed
            let scaled = if val == libc::RLIM_INFINITY {
                val
            } else if res.divisor > 0 {
                val * res.divisor
            } else {
                val
            };

            let mut rlim: libc::rlimit = unsafe { std::mem::zeroed() };
            unsafe { libc::getrlimit(res.resource, &mut rlim) };

            if !explicit_sh || (use_soft && use_hard) {
                // Set both soft and hard
                rlim.rlim_cur = scaled;
                rlim.rlim_max = scaled;
            } else if use_hard {
                rlim.rlim_max = scaled;
                if rlim.rlim_cur > scaled {
                    rlim.rlim_cur = scaled;
                }
            } else {
                // soft only
                rlim.rlim_cur = scaled;
            }

            let ret = unsafe { libc::setrlimit(res.resource, &rlim) };
            if ret != 0 {
                let val_desc = set_value_str
                    .as_deref()
                    .unwrap_or(if val == libc::RLIM_INFINITY {
                        "unlimited"
                    } else {
                        "value"
                    });
                let _ = val_desc;
                // Use nix::errno for strerror-style description without
                // Rust's "(os error N)" suffix.
                let err_desc = nix::errno::Errno::last().desc();
                eprintln!(
                    "{}: ulimit: {}: cannot modify limit: {}",
                    shell.error_prefix(),
                    res.desc,
                    // Capitalize first letter to match bash's strerror output
                    {
                        let mut c = err_desc.chars();
                        match c.next() {
                            None => String::new(),
                            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                        }
                    }
                );
                return 1;
            }
        } else {
            println!("{}", get_limit(res, use_soft, use_hard));
        }

        0
    }
    #[cfg(not(unix))]
    {
        let _ = (shell, args);
        0
    }
}
