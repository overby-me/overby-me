use crate::runtime_info::*;
use crate::units::Unit;
use std::convert::TryInto;

fn find_bin(cmd: &str) -> String {
    // Search common binary directories for the command
    for dir in &[
        "/usr/bin",
        "/bin",
        "/usr/local/bin",
        "/run/current-system/sw/bin",
    ] {
        let path = format!("{}/{}", dir, cmd);
        if std::path::Path::new(&path).exists() {
            return path;
        }
    }
    // On NixOS and similar, binaries may only be in PATH via the nix store.
    // Use /usr/bin/env to resolve at runtime by returning it as a wrapper.
    if std::path::Path::new("/usr/bin/env").exists() {
        return format!("/usr/bin/env {}", cmd);
    }
    panic!("Could not find `{}` in any standard location", cmd);
}

/// Run the state-transition tests with a global timeout so they never hang
/// indefinitely in CI.  The signal-handler thread runs an infinite loop, and
/// if something goes wrong with SIGCHLD delivery inside the cargo-test process
/// the polling wait could spin forever.
#[test]
fn test_service_state_transitions() {
    let handle = std::thread::spawn(test_service_state_transitions_inner);

    let timeout = std::time::Duration::from_secs(30);
    let start = std::time::Instant::now();

    loop {
        if handle.is_finished() {
            handle.join().unwrap();
            return;
        }
        if start.elapsed() >= timeout {
            panic!(
                "test_service_state_transitions timed out after {timeout:?} — \
                 this likely indicates a deadlock or missed SIGCHLD in the test harness"
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

fn test_service_state_transitions_inner() {
    let run_info = std::sync::Arc::new(std::sync::RwLock::new(RuntimeInfo {
        config: crate::config::Config {
            notification_sockets_dir: "./notifications".into(),
            target_unit: "".into(),
            unit_dirs: vec![],
            self_path: std::path::PathBuf::from("./target/debug/rust-systemd"),
        },
        fd_store: std::sync::RwLock::new(crate::fd_store::FDStore::default()),
        pid_table: std::sync::Arc::new(std::sync::Mutex::new(PidTable::default())),
        unit_table: UnitTable::default(),
        stdout_eventfd: crate::platform::make_event_fd().unwrap(),
        stderr_eventfd: crate::platform::make_event_fd().unwrap(),
        notification_eventfd: crate::platform::make_event_fd().unwrap(),
        socket_activation_eventfd: crate::platform::make_event_fd().unwrap(),
        pending_activations: std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::HashSet::new(),
        )),
        manager_environment: std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::HashMap::new(),
        )),
    }));

    let signals = signal_hook::iterator::Signals::new([signal_hook::consts::SIGCHLD]).unwrap();

    let run_info_clone = run_info.clone();
    let pid_table = run_info.read().unwrap().pid_table.clone();
    let _handle = std::thread::spawn(move || {
        // listen on signals from the child processes
        crate::signal_handler::handle_signals(signals, run_info_clone, pid_table);
    });

    // TODO this can probably done better with a setup function. Need to look into the test framework more.
    // This needs to be used by all tests that need the signal handling, because else the signal handlers interfere.
    successful(run_info.clone());
    failing_startexec(run_info.clone());
}

fn successful(run_info: ArcMutRuntimeInfo) {
    let descr = "This is a description";
    let bin_sleep = find_bin("sleep");
    let bin_true = find_bin("true");
    let service_execstart = format!("{} 10", bin_sleep);
    let service_execpre = &bin_true;
    let service_execpost = &bin_true;
    let service_stop = &bin_true;
    let service_stoppost = &bin_true;

    let test_service_str = format!(
        r#"
    [Unit]
    Description = {}
    [Service]
    ExecStart = {}
    ExecStartPre = {}
    ExecStartPost = {}
    ExecStop = {}
    ExecStopPost = {}

    "#,
        descr, service_execstart, service_execpre, service_execpost, service_stop, service_stoppost,
    );

    let parsed_file = crate::units::parse_file(&test_service_str).unwrap();
    let service = crate::units::parse_service(
        parsed_file,
        &std::path::PathBuf::from("/path/to/unitfile.service"),
    )
    .unwrap();
    let unit: Unit = service.try_into().unwrap();

    let unit_id = unit.id.clone();

    run_info
        .write()
        .unwrap()
        .unit_table
        .insert(unit.id.clone(), unit);

    let run_info_locked = run_info.read().unwrap();
    let unit = run_info_locked.unit_table.get(&unit_id).unwrap();

    unit.activate(
        &run_info.read().unwrap(),
        crate::units::ActivationSource::Regular,
    )
    .unwrap();
    let status = unit.common.status.read().unwrap();

    assert_eq!(
        *status,
        crate::units::UnitStatus::Started(crate::units::StatusStarted::Running)
    );
}

fn failing_startexec(run_info: ArcMutRuntimeInfo) {
    let descr = "This is a description";
    let service_type = "oneshot";
    let bin_false = find_bin("false");
    let bin_true = find_bin("true");
    let service_execstart = &bin_false;
    let service_execpre = &bin_true;
    let service_execpost = &bin_true;
    let service_stop = &bin_true;
    let service_stoppost = &bin_true;

    let test_service_str = format!(
        r#"
    [Unit]
    Description = {}
    [Service]
    Type= {}
    ExecStart = {}
    ExecStartPre = {}
    ExecStartPost = {}
    ExecStop = {}
    ExecStopPost = {}

    "#,
        descr,
        service_type,
        service_execstart,
        service_execpre,
        service_execpost,
        service_stop,
        service_stoppost,
    );

    let parsed_file = crate::units::parse_file(&test_service_str).unwrap();
    let service = crate::units::parse_service(
        parsed_file,
        &std::path::PathBuf::from("/path/to/unitfile.service"),
    )
    .unwrap();
    let unit: Unit = service.try_into().unwrap();

    let unit_id = unit.id.clone();

    run_info
        .write()
        .unwrap()
        .unit_table
        .insert(unit.id.clone(), unit);

    let run_info_locked = run_info.read().unwrap();
    let unit = run_info_locked.unit_table.get(&unit_id).unwrap();

    assert!(
        unit.activate(
            &run_info.read().unwrap(),
            crate::units::ActivationSource::Regular
        )
        .is_err()
    );
    let status = unit.common.status.read().unwrap();

    match &*status {
        crate::units::UnitStatus::Stopped(
            crate::units::StatusStopped::StoppedUnexpected,
            errors,
        ) => {
            if errors.len() != 1 {
                panic!("Wrong amount of errors. Should be 1. Is: {}", errors.len());
            }
            match &errors[0] {
                crate::units::UnitOperationErrorReason::ServiceStartError(
                    crate::services::ServiceErrorReason::StartFailed(
                        crate::services::RunCmdError::BadExitCode(_, _),
                    ),
                ) => {
                    // HAPPY
                }
                other => {
                    panic!(
                        "Wrong error. Should have been ServiceStartError(StartFailed(BadExitCode(_,_))). Is: {:?}",
                        other
                    );
                }
            }
        }
        other => panic!(
            "Wrong status. Should have been StoppedUnexpected. Is: {:?}",
            other
        ),
    };
}
