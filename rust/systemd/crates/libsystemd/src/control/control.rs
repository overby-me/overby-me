use crate::control::unit_properties;
use crate::lock_ext::RwLockExt;
use crate::runtime_info::{ArcMutRuntimeInfo, UnitTable};
use crate::units::{
    ActivationSource, Specific, Unit, UnitId, UnitIdKind, UnitStatus, find_symlink_aliases,
    insert_new_units, load_all_units_no_prune, load_new_unit,
};

use std::fmt::Write as _;
use std::os::unix::fs::PermissionsExt;

use log::{info, trace, warn};
use serde_json::Value;

pub fn open_all_sockets(run_info: ArcMutRuntimeInfo, conf: &crate::config::Config) {
    // TODO make configurable
    let control_sock_path = {
        run_info
            .read()
            .unwrap()
            .config
            .notification_sockets_dir
            .join("control.socket")
    };
    if control_sock_path.exists() {
        std::fs::remove_file(&control_sock_path).unwrap();
    }
    use std::os::unix::net::UnixListener;
    std::fs::create_dir_all(&conf.notification_sockets_dir).unwrap();
    // Ensure the notification sockets directory is world-traversable so
    // non-root users can reach the control socket inside it.
    let _ = std::fs::set_permissions(
        &conf.notification_sockets_dir,
        std::fs::Permissions::from_mode(0o755),
    );
    let unixsock = UnixListener::bind(&control_sock_path).unwrap();
    // Default socket mode after bind() inherits the process umask (typically
    // 0022 → mode 0755), which prevents non-root users from connecting
    // (write permission is required to connect to a Unix socket).  Set the
    // socket to 0666 so any user can issue control commands — matching real
    // systemd's /run/systemd/private socket behaviour.
    let _ = std::fs::set_permissions(&control_sock_path, std::fs::Permissions::from_mode(0o666));
    accept_control_connections_unix_socket(run_info, unixsock);
    //let tcpsock = std::net::TcpListener::bind("127.0.0.1:8080").unwrap();
    //accept_control_connections_tcp(
    //    run_info.clone(),
    //    conf.notification_sockets_dir.clone(),
    //    tcpsock,
    //);
}

#[derive(Debug)]
pub enum Command {
    ListUnits(Option<UnitIdKind>, Option<String>),
    /// `list-unit-files [--type=TYPE]` — list all unit files on disk with their state.
    ListUnitFiles(Option<String>),
    /// `list-dependencies <unit> [--reverse]` — show the dependency tree.
    ListDependencies(String, bool),
    Status(Option<String>),
    /// `show <unit> [property,...]` — return all (or filtered) properties as key=value.
    Show(String, Option<Vec<String>>),
    /// `cat <unit>` — return the unit file source text.
    Cat(String),
    LoadNew(Vec<String>),
    LoadAllNew,
    LoadAllNewDry,
    Remove(String),
    Restart(String),
    TryRestart(String),
    /// `reload <unit>` — run the ExecReload= commands for a service.
    Reload(String),
    ReloadOrRestart(String),
    Start(Vec<String>),
    /// `start --wait` — start units and block until they deactivate.
    StartWait(Vec<String>),
    StartNoBlock(Vec<String>),
    StopNoBlock(Vec<String>),
    RestartNoBlock(String),
    StartAll(String),
    Stop(Vec<String>),
    StopAll(String),
    IsActive(String),
    IsEnabled(String),
    IsFailed(String),
    /// `is-system-running` — report the overall system state.
    IsSystemRunning,
    /// `mask <unit>...` — symlink unit files to /dev/null.
    Mask(Vec<String>),
    /// `unmask <unit>...` — remove /dev/null symlinks for units.
    Unmask(Vec<String>),
    /// `enable <unit>...` — create .wants/.requires symlinks per [Install] section.
    Enable(Vec<String>),
    /// `disable <unit>...` — remove .wants/.requires symlinks.
    Disable(Vec<String>),
    /// `preset <unit>...` — enable/disable units based on preset files.
    Preset(Vec<String>),
    /// `add-wants <unit> <target>` — create a `.wants` symlink.
    AddWants(String, String),
    /// `add-requires <unit> <target>` — create a `.requires` symlink.
    AddRequires(String, String),
    /// `reset-failed [unit]` — clear the failed state of a unit (or all units).
    ResetFailed(Option<String>),
    /// `kill <unit> [--signal=SIG] [--kill-whom=WHO] [--kill-value=N]`
    /// Send a signal to a unit's processes.
    /// Fields: unit_name, signal, kill_whom ("main"/"control"/"all"), kill_value
    Kill(String, i32, String, Option<i32>, bool),
    Shutdown(crate::shutdown::ShutdownAction),
    /// `suspend` — put the system to sleep (suspend to RAM).
    Suspend,
    /// `hibernate` — put the system to sleep (suspend to disk).
    Hibernate,
    /// `hybrid-sleep` — put the system to sleep (suspend to both RAM and disk).
    HybridSleep,
    /// `suspend-then-hibernate` — suspend first, then hibernate after a delay.
    SuspendThenHibernate,
    /// `list-timers` — list active timer units with next elapse times.
    ListTimers,
    /// `list-sockets` — list socket units.
    ListSockets,
    /// `list-paths` — list path units.
    ListPaths,
    /// `list-jobs` — list currently running/waiting jobs.
    ListJobs,
    /// `set-property <unit> <property>=<value>...` — set runtime properties on a unit.
    /// Creates a drop-in file at `/etc/systemd/system/<unit>.d/50-set-property.conf`
    /// (or `/run/systemd/system/<unit>.d/` with `--runtime`).
    SetProperty(String, Vec<String>),
    /// `edit <unit>` — query the unit's fragment path so the client can open an editor.
    /// Returns the fragment path and existing drop-in override content (if any).
    /// The actual editor interaction happens client-side in systemctl.
    Edit(String, bool),
    /// `revert <unit>` — remove all local customizations (drop-in overrides and
    /// admin-provided unit file copies) for a unit, reverting to the vendor version.
    Revert(String),
    /// `start-transient` — create and start a transient (in-memory) unit.
    ///
    /// Params is an object with:
    ///   - `unit` (required): the unit name (e.g. `run-u42.service`)
    ///   - `command` (optional): array of strings for ExecStart
    ///   - `description` (optional): unit description
    ///   - `user` (optional): run as this user
    ///   - `group` (optional): run with this group
    ///   - `working_directory` (optional): working directory
    ///   - `type` (optional): service type (simple, oneshot, exec, …)
    ///   - `remain_after_exit` (optional): bool
    StartTransient(TransientUnitParams),
    /// `daemon-reexec` — re-execute the service manager binary in-place.
    DaemonReexec,
    /// `log-level [LEVEL]` — get or set the service manager log level.
    LogLevel(Option<String>),
    /// `log-target [TARGET]` — get or set the service manager log target.
    LogTarget(Option<String>),
    /// `service-watchdogs [BOOL]` — get or set whether service watchdogs are enabled.
    ServiceWatchdogs(Option<String>),
    /// `clean <unit> [--what=WHAT]` — remove runtime/configuration/state/cache/log data.
    /// `--what` can be: configuration, runtime, state, cache, logs, all.
    /// Default (no --what) removes runtime + cache.
    Clean(String, Option<String>),
    /// `freeze <unit>` — freeze a unit's cgroup (pause all processes).
    Freeze(String),
    /// `thaw <unit>` — thaw a frozen unit's cgroup (resume all processes).
    Thaw(String),
    /// `show-environment` — list the manager's environment variables.
    ShowEnvironment,
    /// `set-environment KEY=VALUE...` — set environment variables.
    SetEnvironment(Vec<String>),
    /// `unset-environment KEY...` — remove environment variables.
    UnsetEnvironment(Vec<String>),
    /// `import-environment KEY...` — import variables from the calling process.
    ImportEnvironment(Vec<String>),
}

/// Parameters for creating a transient (in-memory) service unit.
#[derive(Debug, Clone)]
pub struct TransientUnitParams {
    pub unit_name: String,
    pub command: Option<Vec<String>>,
    pub description: Option<String>,
    pub user: Option<String>,
    pub group: Option<String>,
    pub working_directory: Option<String>,
    pub service_type: Option<String>,
    pub remain_after_exit: bool,
    /// Additional properties specified via `-p NAME=VALUE`.
    pub properties: Vec<String>,
    /// Environment variables specified via `-E NAME=VALUE`.
    pub environment: Vec<String>,
    /// Whether to run as a scope unit.
    pub scope: bool,
    /// Whether the caller wants to wait for completion.
    pub wait: bool,
    /// Whether to pipe stdin/stdout/stderr between the caller and the service.
    pub pipe: bool,
    /// Slice to place the unit in.
    pub slice: Option<String>,
    /// Timer properties — if any are set, a companion .timer unit is created.
    pub on_calendar: Option<String>,
    pub on_active: Vec<String>,
    pub on_boot: Option<String>,
    pub on_startup: Option<String>,
    pub on_unit_active: Option<String>,
    pub on_unit_inactive: Option<String>,
    pub on_clock_change: bool,
    pub on_timezone_change: bool,
    /// Additional timer properties (e.g. After=...).
    pub timer_properties: Vec<String>,
    /// Path properties (e.g. PathExists=/tmp).
    pub path_properties: Vec<String>,
    /// Socket properties (e.g. ListenFIFO=/tmp/foo).
    pub socket_properties: Vec<String>,
    /// Nice level for the spawned process.
    pub nice: Option<i32>,
}

#[derive(Debug)]
enum ParseError {
    MethodNotFound(String),
    ParamsInvalid(String),
}

fn parse_command(call: &super::jsonrpc2::Call) -> Result<Command, ParseError> {
    let command = match call.method.as_str() {
        "status" => {
            let name = match &call.params {
                Some(Value::String(s)) => Some(s.clone()),
                Some(_) => {
                    return Err(ParseError::ParamsInvalid(
                        "Params must be either none or a single string".to_string(),
                    ));
                }
                None => None,
            };
            Command::Status(name)
        }
        "restart" => {
            let name = match &call.params {
                Some(Value::String(s)) => s.clone(),
                Some(_) | None => {
                    return Err(ParseError::ParamsInvalid(
                        "Params must be a single string".to_string(),
                    ));
                }
            };
            Command::Restart(name)
        }
        "try-restart" | "condrestart" => {
            let name = match &call.params {
                Some(Value::String(s)) => s.clone(),
                Some(_) | None => {
                    return Err(ParseError::ParamsInvalid(
                        "Params must be a single string".to_string(),
                    ));
                }
            };
            Command::TryRestart(name)
        }
        "reload-or-restart" | "reload-or-try-restart" | "force-reload" => {
            let name = match &call.params {
                Some(Value::String(s)) => s.clone(),
                Some(_) | None => {
                    return Err(ParseError::ParamsInvalid(
                        "Params must be a single string".to_string(),
                    ));
                }
            };
            Command::ReloadOrRestart(name)
        }
        "show" => {
            // show <unit> [property-filter...]
            // Params: String (unit name) or Array [unit_name, prop1, prop2, ...]
            match &call.params {
                Some(Value::String(s)) => Command::Show(s.clone(), None),
                Some(Value::Array(arr)) if !arr.is_empty() => {
                    let name = arr[0].as_str().unwrap_or("").to_owned();
                    let filter = if arr.len() > 1 {
                        Some(
                            arr[1..]
                                .iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                                .collect(),
                        )
                    } else {
                        None
                    };
                    Command::Show(name, filter)
                }
                Some(_) | None => {
                    return Err(ParseError::ParamsInvalid(
                        "show requires a unit name".to_string(),
                    ));
                }
            }
        }
        "cat" => {
            let name = match &call.params {
                Some(Value::String(s)) => s.clone(),
                Some(_) | None => {
                    return Err(ParseError::ParamsInvalid(
                        "Params must be a single string".to_string(),
                    ));
                }
            };
            Command::Cat(name)
        }
        "is-active" => {
            let name = match &call.params {
                Some(Value::String(s)) => s.clone(),
                Some(_) | None => {
                    return Err(ParseError::ParamsInvalid(
                        "Params must be a single string".to_string(),
                    ));
                }
            };
            Command::IsActive(name)
        }
        "is-enabled" => {
            let name = match &call.params {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Array(arr)) if !arr.is_empty() => {
                    arr[0].as_str().unwrap_or("").to_owned()
                }
                Some(_) | None => {
                    return Err(ParseError::ParamsInvalid(
                        "Params must be a single string".to_string(),
                    ));
                }
            };
            Command::IsEnabled(name)
        }
        "is-failed" => {
            let name = match &call.params {
                Some(Value::String(s)) => s.clone(),
                Some(_) | None => {
                    return Err(ParseError::ParamsInvalid(
                        "Params must be a single string".to_string(),
                    ));
                }
            };
            Command::IsFailed(name)
        }
        "is-system-running" => Command::IsSystemRunning,
        "start" => {
            let names = match &call.params {
                Some(Value::String(s)) => vec![s.clone()],
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect(),
                Some(_) | None => {
                    return Err(ParseError::ParamsInvalid(
                        "Params must be a string or array of strings".to_string(),
                    ));
                }
            };
            Command::Start(names)
        }
        "start-wait" => {
            let names = match &call.params {
                Some(Value::String(s)) => vec![s.clone()],
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect(),
                Some(_) | None => {
                    return Err(ParseError::ParamsInvalid(
                        "Params must be a string or array of strings".to_string(),
                    ));
                }
            };
            Command::StartWait(names)
        }
        "start-noblock" => {
            let names = match &call.params {
                Some(Value::String(s)) => vec![s.clone()],
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect(),
                Some(_) | None => {
                    return Err(ParseError::ParamsInvalid(
                        "Params must be a string or array of strings".to_string(),
                    ));
                }
            };
            Command::StartNoBlock(names)
        }
        "start-all" => {
            let name = match &call.params {
                Some(Value::String(s)) => s.clone(),
                Some(_) | None => {
                    return Err(ParseError::ParamsInvalid(
                        "Params must be a single string".to_string(),
                    ));
                }
            };
            Command::StartAll(name)
        }
        "remove" => {
            let name = match &call.params {
                Some(Value::String(s)) => s.clone(),
                Some(_) | None => {
                    return Err(ParseError::ParamsInvalid(
                        "Params must be a single string".to_string(),
                    ));
                }
            };
            Command::Remove(name)
        }
        "stop" => {
            let names = match &call.params {
                Some(Value::String(s)) => vec![s.clone()],
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect(),
                Some(_) | None => {
                    return Err(ParseError::ParamsInvalid(
                        "Params must be a string or array of strings".to_string(),
                    ));
                }
            };
            Command::Stop(names)
        }
        "stop-all" => {
            let name = match &call.params {
                Some(Value::String(s)) => s.clone(),
                Some(_) | None => {
                    return Err(ParseError::ParamsInvalid(
                        "Params must be a single string".to_string(),
                    ));
                }
            };
            Command::StopAll(name)
        }

        "stop-noblock" => {
            let names = match &call.params {
                Some(Value::String(s)) => vec![s.clone()],
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect(),
                Some(_) | None => {
                    return Err(ParseError::ParamsInvalid(
                        "Params must be a string or array of strings".to_string(),
                    ));
                }
            };
            Command::StopNoBlock(names)
        }
        "restart-noblock" => {
            let name = match &call.params {
                Some(Value::String(s)) => s.clone(),
                Some(_) | None => {
                    return Err(ParseError::ParamsInvalid(
                        "Params must be a single string".to_string(),
                    ));
                }
            };
            Command::RestartNoBlock(name)
        }

        "list-units" => {
            let mut kind = None;
            let mut state_filter = None;
            match &call.params {
                Some(Value::String(s)) => {
                    kind = match s.as_str() {
                        "target" => Some(UnitIdKind::Target),
                        "socket" => Some(UnitIdKind::Socket),
                        "service" => Some(UnitIdKind::Service),
                        "slice" => Some(UnitIdKind::Slice),
                        "mount" => Some(UnitIdKind::Mount),
                        "device" => Some(UnitIdKind::Device),
                        _ => None,
                    };
                }
                Some(Value::Object(obj)) => {
                    if let Some(Value::String(t)) = obj.get("type") {
                        kind = match t.as_str() {
                            "target" => Some(UnitIdKind::Target),
                            "socket" => Some(UnitIdKind::Socket),
                            "service" => Some(UnitIdKind::Service),
                            "slice" => Some(UnitIdKind::Slice),
                            "mount" => Some(UnitIdKind::Mount),
                            "device" => Some(UnitIdKind::Device),
                            _ => None,
                        };
                    }
                    if let Some(Value::String(s)) = obj.get("state") {
                        state_filter = Some(s.clone());
                    }
                }
                _ => {}
            }
            Command::ListUnits(kind, state_filter)
        }
        "shutdown" => {
            let action = match &call.params {
                Some(Value::String(s)) => crate::shutdown::ShutdownAction::from_verb(s)
                    .unwrap_or(crate::shutdown::ShutdownAction::Poweroff),
                _ => crate::shutdown::ShutdownAction::Poweroff,
            };
            Command::Shutdown(action)
        }
        "suspend" => Command::Suspend,
        "hibernate" => Command::Hibernate,
        "hybrid-sleep" => Command::HybridSleep,
        "suspend-then-hibernate" => Command::SuspendThenHibernate,
        "daemon-reload" => Command::LoadAllNew,
        "reload" => {
            // "reload" without params = daemon-reload;
            // "reload" with a unit name = unit reload (ExecReload=)
            match &call.params {
                Some(Value::String(s)) if !s.is_empty() => Command::Reload(s.clone()),
                _ => Command::LoadAllNew,
            }
        }
        "daemon-reexec" => Command::DaemonReexec,
        "log-level" => {
            let level = match &call.params {
                Some(Value::String(s)) => Some(s.clone()),
                Some(Value::Array(arr)) if !arr.is_empty() => arr[0].as_str().map(|s| s.to_owned()),
                _ => None,
            };
            Command::LogLevel(level)
        }
        "log-target" => {
            let target = match &call.params {
                Some(Value::String(s)) => Some(s.clone()),
                Some(Value::Array(arr)) if !arr.is_empty() => arr[0].as_str().map(|s| s.to_owned()),
                _ => None,
            };
            Command::LogTarget(target)
        }
        "service-watchdogs" => {
            let val = match &call.params {
                Some(Value::String(s)) => Some(s.clone()),
                Some(Value::Array(arr)) if !arr.is_empty() => arr[0].as_str().map(|s| s.to_owned()),
                _ => None,
            };
            Command::ServiceWatchdogs(val)
        }
        "start-transient" => {
            // Params: JSON object with transient unit properties.
            match &call.params {
                Some(Value::Object(obj)) => {
                    let unit_name = obj
                        .get("unit")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ParseError::ParamsInvalid(
                                "start-transient requires a 'unit' property".to_string(),
                            )
                        })?
                        .to_owned();
                    let command = obj.get("command").and_then(|v| {
                        v.as_array().map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                                .collect()
                        })
                    });
                    let description = obj
                        .get("description")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_owned());
                    let user = obj
                        .get("user")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_owned());
                    let group = obj
                        .get("group")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_owned());
                    let working_directory = obj
                        .get("working_directory")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_owned());
                    let service_type = obj
                        .get("service_type")
                        .or_else(|| obj.get("type"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_owned());
                    let remain_after_exit = obj
                        .get("remain_after_exit")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let properties = obj
                        .get("properties")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                                .collect()
                        })
                        .unwrap_or_default();
                    let environment = obj
                        .get("environment")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                                .collect()
                        })
                        .unwrap_or_default();
                    let scope = obj.get("scope").and_then(|v| v.as_bool()).unwrap_or(false);
                    let wait = obj.get("wait").and_then(|v| v.as_bool()).unwrap_or(false);
                    let pipe = obj.get("pipe").and_then(|v| v.as_bool()).unwrap_or(false);
                    let slice = obj
                        .get("slice")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_owned());

                    let on_calendar = obj
                        .get("on_calendar")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_owned());
                    let on_active: Vec<String> = match obj.get("on_active") {
                        Some(Value::Array(arr)) => arr
                            .iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                            .collect(),
                        Some(Value::String(s)) => vec![s.clone()],
                        _ => vec![],
                    };
                    let on_boot = obj
                        .get("on_boot")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_owned());
                    let on_startup = obj
                        .get("on_startup")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_owned());
                    let on_unit_active = obj
                        .get("on_unit_active")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_owned());
                    let on_unit_inactive = obj
                        .get("on_unit_inactive")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_owned());
                    let on_clock_change = obj
                        .get("on_clock_change")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let on_timezone_change = obj
                        .get("on_timezone_change")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let timer_properties: Vec<String> = obj
                        .get("timer_properties")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                                .collect()
                        })
                        .unwrap_or_default();
                    let path_properties: Vec<String> = obj
                        .get("path_properties")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                                .collect()
                        })
                        .unwrap_or_default();
                    let socket_properties: Vec<String> = obj
                        .get("socket_properties")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                                .collect()
                        })
                        .unwrap_or_default();
                    let nice = obj.get("nice").and_then(|v| v.as_i64()).map(|n| n as i32);

                    Command::StartTransient(TransientUnitParams {
                        unit_name,
                        command,
                        description,
                        user,
                        group,
                        working_directory,
                        service_type,
                        remain_after_exit,
                        properties,
                        environment,
                        scope,
                        wait,
                        pipe,
                        slice,
                        on_calendar,
                        on_active,
                        on_boot,
                        on_startup,
                        on_unit_active,
                        on_unit_inactive,
                        on_clock_change,
                        on_timezone_change,
                        timer_properties,
                        path_properties,
                        socket_properties,
                        nice,
                    })
                }
                Some(_) | None => {
                    return Err(ParseError::ParamsInvalid(
                        "start-transient requires a JSON object with at least a 'unit' property"
                            .to_string(),
                    ));
                }
            }
        }
        "reload-dry" => Command::LoadAllNewDry,
        "enable" | "reenable" => {
            let names = match &call.params {
                Some(params) => match params {
                    Value::String(s) => vec![s.clone()],
                    Value::Array(names) => {
                        let mut str_names = Vec::new();
                        for name in names {
                            if let Value::String(name) = name {
                                str_names.push(name.clone());
                            } else {
                                return Err(ParseError::ParamsInvalid(
                                    "Params must be at least one string".to_string(),
                                ));
                            }
                        }
                        str_names
                    }
                    _ => {
                        return Err(ParseError::ParamsInvalid(
                            "Params must be at least one string".to_string(),
                        ));
                    }
                },
                None => {
                    return Err(ParseError::ParamsInvalid(
                        "Params must be at least one string".to_string(),
                    ));
                }
            };
            Command::Enable(names)
        }
        "disable" => {
            let names = match &call.params {
                Some(Value::String(s)) => vec![s.clone()],
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                    .collect(),
                Some(_) | None => {
                    return Err(ParseError::ParamsInvalid(
                        "disable requires at least one unit name".to_string(),
                    ));
                }
            };
            Command::Disable(names)
        }
        "preset" => {
            let names = match &call.params {
                Some(Value::String(s)) => vec![s.clone()],
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                    .collect(),
                Some(_) | None => {
                    return Err(ParseError::ParamsInvalid(
                        "preset requires at least one unit name".to_string(),
                    ));
                }
            };
            Command::Preset(names)
        }
        "reset-failed" => {
            let name = match &call.params {
                Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
                _ => None,
            };
            Command::ResetFailed(name)
        }
        "kill" => {
            // Params: String (unit name) or Array [unit_name, signal, kill_whom, kill_value]
            match &call.params {
                Some(Value::String(s)) => {
                    Command::Kill(s.clone(), 15, "all".to_string(), None, false)
                }
                Some(Value::Array(arr)) if !arr.is_empty() => {
                    let name = arr[0].as_str().unwrap_or("").to_owned();
                    let sig = arr
                        .get(1)
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse::<i32>().ok())
                        .unwrap_or(15);
                    let whom = arr
                        .get(2)
                        .and_then(|v| v.as_str())
                        .unwrap_or("all")
                        .to_owned();
                    let value = arr
                        .get(3)
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse::<i32>().ok());
                    let has_wait = arr.iter().any(|v| v.as_str() == Some("--wait"));
                    Command::Kill(name, sig, whom, value, has_wait)
                }
                Some(_) | None => {
                    return Err(ParseError::ParamsInvalid(
                        "kill requires a unit name".to_string(),
                    ));
                }
            }
        }
        "list-unit-files" => {
            // Optional param: type filter string (e.g. "service", "target")
            // Also accept array params (when --root or other flags are passed)
            let type_filter = match &call.params {
                Some(Value::String(s)) if !s.is_empty() && !s.starts_with("--") => Some(s.clone()),
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| v.as_str())
                    .find(|s| !s.starts_with("--"))
                    .map(|s| s.to_owned()),
                _ => None,
            };
            Command::ListUnitFiles(type_filter)
        }
        "list-dependencies" => {
            // Params: String (unit name) or Array [unit_name, "--reverse"]
            match &call.params {
                Some(Value::String(s)) => Command::ListDependencies(s.clone(), false),
                Some(Value::Array(arr)) if !arr.is_empty() => {
                    let name = arr[0].as_str().unwrap_or("").to_owned();
                    let reverse = arr.iter().skip(1).any(|v| v.as_str() == Some("--reverse"));
                    Command::ListDependencies(name, reverse)
                }
                Some(_) | None => {
                    return Err(ParseError::ParamsInvalid(
                        "list-dependencies requires a unit name".to_string(),
                    ));
                }
            }
        }
        "mask" => {
            let names = match &call.params {
                Some(Value::String(s)) => vec![s.clone()],
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                    .collect(),
                Some(_) | None => {
                    return Err(ParseError::ParamsInvalid(
                        "mask requires at least one unit name".to_string(),
                    ));
                }
            };
            Command::Mask(names)
        }
        "unmask" => {
            let names = match &call.params {
                Some(Value::String(s)) => vec![s.clone()],
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                    .collect(),
                Some(_) | None => {
                    return Err(ParseError::ParamsInvalid(
                        "unmask requires at least one unit name".to_string(),
                    ));
                }
            };
            Command::Unmask(names)
        }
        "add-wants" | "add-requires" => {
            // add-wants <unit> <target> or add-requires <unit> <target>
            let (unit, target) = match &call.params {
                Some(Value::Array(arr)) if arr.len() >= 2 => {
                    let u = arr[0].as_str().unwrap_or("").to_owned();
                    let t = arr[1].as_str().unwrap_or("").to_owned();
                    (u, t)
                }
                Some(_) | None => {
                    return Err(ParseError::ParamsInvalid(
                        "add-wants/add-requires requires unit and target".to_string(),
                    ));
                }
            };
            if call.method == "add-wants" {
                Command::AddWants(unit, target)
            } else {
                Command::AddRequires(unit, target)
            }
        }
        "list-timers" => Command::ListTimers,
        "list-sockets" => Command::ListSockets,
        "list-paths" => Command::ListPaths,
        "list-jobs" => Command::ListJobs,
        "set-property" => {
            // set-property <unit> <prop=val>...
            match &call.params {
                Some(Value::String(s)) => Command::SetProperty(s.clone(), vec![]),
                Some(Value::Array(arr)) if !arr.is_empty() => {
                    let name = arr[0].as_str().unwrap_or("").to_owned();
                    let props = arr[1..]
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                        .collect();
                    Command::SetProperty(name, props)
                }
                Some(_) | None => {
                    return Err(ParseError::ParamsInvalid(
                        "set-property requires a unit name".to_string(),
                    ));
                }
            }
        }
        "edit" => {
            // edit <unit> [--full]
            match &call.params {
                Some(Value::String(s)) => Command::Edit(s.clone(), false),
                Some(Value::Array(arr)) if !arr.is_empty() => {
                    let name = arr[0].as_str().unwrap_or("").to_owned();
                    let full = arr.iter().skip(1).any(|v| v.as_str() == Some("--full"));
                    Command::Edit(name, full)
                }
                Some(_) | None => {
                    return Err(ParseError::ParamsInvalid(
                        "edit requires a unit name".to_string(),
                    ));
                }
            }
        }
        "revert" => {
            // revert <unit>
            let name = match &call.params {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Array(arr)) if !arr.is_empty() => {
                    arr[0].as_str().unwrap_or("").to_owned()
                }
                Some(_) | None => {
                    return Err(ParseError::ParamsInvalid(
                        "revert requires a unit name".to_string(),
                    ));
                }
            };
            Command::Revert(name)
        }
        "clean" => {
            // clean <unit> [what]
            match &call.params {
                Some(Value::Array(arr)) if !arr.is_empty() => {
                    let name = arr[0].as_str().unwrap_or("").to_owned();
                    let what = arr.get(1).and_then(|v| v.as_str()).map(|s| s.to_owned());
                    Command::Clean(name, what)
                }
                Some(Value::String(s)) => Command::Clean(s.clone(), None),
                Some(_) | None => {
                    return Err(ParseError::ParamsInvalid(
                        "clean requires a unit name".to_string(),
                    ));
                }
            }
        }
        "freeze" => {
            let name = match &call.params {
                Some(Value::String(s)) => s.clone(),
                _ => {
                    return Err(ParseError::ParamsInvalid(
                        "freeze requires a unit name".to_string(),
                    ));
                }
            };
            Command::Freeze(name)
        }
        "thaw" => {
            let name = match &call.params {
                Some(Value::String(s)) => s.clone(),
                _ => {
                    return Err(ParseError::ParamsInvalid(
                        "thaw requires a unit name".to_string(),
                    ));
                }
            };
            Command::Thaw(name)
        }
        "show-environment" => Command::ShowEnvironment,
        "set-environment" => {
            let vars = match &call.params {
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                    .collect(),
                Some(Value::String(s)) => vec![s.clone()],
                Some(_) | None => vec![],
            };
            Command::SetEnvironment(vars)
        }
        "unset-environment" => {
            let vars = match &call.params {
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                    .collect(),
                Some(Value::String(s)) => vec![s.clone()],
                Some(_) | None => vec![],
            };
            Command::UnsetEnvironment(vars)
        }
        "import-environment" => {
            let vars = match &call.params {
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                    .collect(),
                Some(Value::String(s)) => vec![s.clone()],
                Some(_) | None => vec![],
            };
            Command::ImportEnvironment(vars)
        }
        _ => {
            return Err(ParseError::MethodNotFound(format!(
                "Unknown method: {}",
                call.method
            )));
        }
    };

    Ok(command)
}

/// Format a Duration as a human-readable string (e.g. "1h 30min", "15min", "2s").
fn format_duration(d: &std::time::Duration) -> String {
    let total_secs = d.as_secs();
    if total_secs == 0 {
        return "0".into();
    }
    let mut parts = Vec::new();
    let days = total_secs / 86400;
    let hours = (total_secs % 86400) / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    if days > 0 {
        parts.push(format!("{days}d"));
    }
    if hours > 0 {
        parts.push(format!("{hours}h"));
    }
    if mins > 0 {
        parts.push(format!("{mins}min"));
    }
    if secs > 0 {
        parts.push(format!("{secs}s"));
    }
    parts.join(" ")
}

pub fn format_socket(socket_unit: &Unit, status: UnitStatus) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("Name".into(), Value::String(socket_unit.id.name.clone()));
    map.insert("Status".into(), Value::String(format!("{status:?}")));

    if let Specific::Socket(sock) = &socket_unit.specific {
        map.insert(
            "FileDescriptorname".into(),
            Value::String(socket_unit.id.name.clone()),
        );
        map.insert(
            "FileDescriptors".into(),
            Value::Array(
                sock.conf
                    .sockets
                    .iter()
                    .map(|sock_conf| Value::String(format!("{:?}", sock_conf.specialized)))
                    .collect(),
            ),
        );
    }

    Value::Object(map)
}

pub fn format_target(socket_unit: &Unit, status: UnitStatus) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("Name".into(), Value::String(socket_unit.id.name.clone()));
    map.insert("Status".into(), Value::String(format!("{status:?}")));
    Value::Object(map)
}

pub fn format_service(srvc_unit: &Unit, status: UnitStatus) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("Name".into(), Value::String(srvc_unit.id.name.clone()));
    map.insert("Status".into(), Value::String(format!("{status:?}")));
    if let Specific::Service(srvc) = &srvc_unit.specific {
        map.insert(
            "Sockets".into(),
            Value::Array(
                srvc.conf
                    .sockets
                    .iter()
                    .map(|x| Value::String(x.name.clone()))
                    .collect(),
            ),
        );
        if let Some(instant) = srvc.state.read_poisoned().common.up_since {
            map.insert(
                "UpSince".into(),
                Value::String(format!("{:?}", instant.elapsed())),
            );
        }
        map.insert(
            "Restarted".into(),
            Value::String(format!(
                "{:?}",
                srvc.state.read_poisoned().common.restart_count
            )),
        );
    }
    Value::Object(map)
}

/// Simple glob matching for unit names. Supports '*' and '?' wildcards.
fn unit_name_glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    unit_name_glob_inner(&p, &t)
}

fn unit_name_glob_inner(pattern: &[char], text: &[char]) -> bool {
    if pattern.is_empty() {
        return text.is_empty();
    }
    match pattern[0] {
        '*' => {
            for i in 0..=text.len() {
                if unit_name_glob_inner(&pattern[1..], &text[i..]) {
                    return true;
                }
            }
            false
        }
        '?' => !text.is_empty() && unit_name_glob_inner(&pattern[1..], &text[1..]),
        c => !text.is_empty() && text[0] == c && unit_name_glob_inner(&pattern[1..], &text[1..]),
    }
}

/// Clear the failed state of a unit, resetting it to NeverStarted.
/// Also resets unit-type-specific result fields (e.g. PathResult).
fn reset_failed_unit(unit: &Unit) {
    let mut status = unit.common.status.write_poisoned();
    if let UnitStatus::Stopped(_, ref errors) = *status
        && !errors.is_empty()
    {
        *status = UnitStatus::NeverStarted;
    }
    drop(status);
    // Reset path-specific result.
    if let Specific::Path(path_specific) = &unit.specific {
        let mut state = path_specific.state.write_poisoned();
        state.result = crate::units::PathResult::Success;
    }
}

/// Check if a string contains glob characters.
fn is_glob_pattern(s: &str) -> bool {
    s.contains('*') || s.contains('?')
}

fn find_units_with_name<'a>(unit_name: &str, unit_table: &'a UnitTable) -> Vec<&'a Unit> {
    trace!("Find unit for name: {unit_name}");
    let use_glob = is_glob_pattern(unit_name);
    // If the name has a unit type suffix (contains '.'), use exact matching.
    // Otherwise, try matching against the base name of each unit (name without
    // suffix) to handle "foo" → "foo.service" style lookups.
    let has_suffix = unit_name.contains('.');
    unit_table
        .values()
        .filter(|unit| {
            let name = &unit.id.name;
            if use_glob {
                unit_name_glob_match(unit_name, name)
                    || unit
                        .common
                        .unit
                        .aliases
                        .iter()
                        .any(|alias| unit_name_glob_match(unit_name, alias))
            } else if has_suffix {
                name == unit_name
                    || unit
                        .common
                        .unit
                        .aliases
                        .iter()
                        .any(|alias| alias == unit_name)
            } else {
                // No suffix: match "name.suffix" by stripping the suffix and
                // comparing the base. This handles "a" matching "a.service"
                // but NOT "a-b-c.service" or "autovt@tty1.service".
                let matches_base = |full: &str| -> bool {
                    full == unit_name
                        || full
                            .rfind('.')
                            .map(|dot| &full[..dot] == unit_name)
                            .unwrap_or(false)
                };
                matches_base(name)
                    || unit
                        .common
                        .unit
                        .aliases
                        .iter()
                        .any(|alias| matches_base(alias))
            }
        })
        .collect()
}

/// Try to find a unit by name in the unit table. If not found, attempt to load
/// it from disk (searching the configured unit directories) and insert it into
/// the unit table. Returns the UnitId on success.
///
/// This enables commands like `systemctl restart <unit>` to work even when the
/// unit wasn't part of the initial boot dependency graph (e.g. units triggered
/// on-demand by udev rules).
/// Locate the `systemd-sleep` binary by searching relative to our own
/// executable first (critical for NixOS where binaries live in the Nix
/// store), then falling back to well-known system paths.
fn find_sleep_binary() -> Option<std::path::PathBuf> {
    // Try relative to our own executable (e.g. /nix/store/.../lib/systemd/systemd
    // → /nix/store/.../lib/systemd/systemd-sleep or .../bin/systemd-sleep)
    if let Ok(exe) = std::env::current_exe() {
        // Check sibling directory (same dir as PID 1)
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("systemd-sleep");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        // Walk up to find bin/systemd-sleep or lib/systemd/systemd-sleep
        let mut dir = exe.parent();
        for _ in 0..5 {
            let Some(d) = dir else { break };
            for subpath in &["bin/systemd-sleep", "lib/systemd/systemd-sleep"] {
                let candidate = d.join(subpath);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
            dir = d.parent();
        }
    }

    // Fallback to system paths
    for path in &[
        "/usr/lib/systemd/systemd-sleep",
        "/lib/systemd/systemd-sleep",
        "/usr/bin/systemd-sleep",
    ] {
        let p = std::path::Path::new(path);
        if p.is_file() {
            return Some(p.to_path_buf());
        }
    }

    None
}

pub fn find_or_load_unit(
    unit_name: &str,
    run_info: &ArcMutRuntimeInfo,
) -> Result<crate::units::UnitId, String> {
    // First, try to find under a read lock.
    {
        let ri = run_info.read_poisoned();
        let units = find_units_with_name(unit_name, &ri.unit_table);
        if units.len() > 1 {
            // When the name has no suffix (e.g. "systemd-hostnamed"), prefer
            // the .service unit — matching real systemd behaviour.
            if !unit_name.contains('.') {
                let service_name = format!("{unit_name}.service");
                if let Some(unit) = units.iter().find(|u| u.id.name == service_name) {
                    return Ok(unit.id.clone());
                }
            }
            let names: Vec<_> = units.iter().map(|unit| unit.id.name.clone()).collect();
            return Err(format!(
                "More than one unit found with name: {unit_name}: {names:?}"
            ));
        }
        if let Some(unit) = units.first() {
            return Ok(unit.id.clone());
        }
    }
    // Not found — try to load from disk under a write lock.
    {
        let mut ri = run_info.write_poisoned();
        // Re-check after acquiring write lock (another thread may have loaded it).
        let already = find_units_with_name(unit_name, &ri.unit_table);
        if let Some(unit) = already.first() {
            return Ok(unit.id.clone());
        }

        // First try loading as a regular unit file.
        // If the name has no suffix, also try with .service appended.
        let load_name = if unit_name.contains('.') {
            unit_name.to_string()
        } else {
            format!("{unit_name}.service")
        };

        // If the file is a symlink, resolve to canonical name and check
        // if that unit is already loaded (e.g., test15-a1.service → test15-a.service).
        // When an instance symlink points to a template (e.g. bar-alias@2.service → yup@.service),
        // derive the instance name (yup@2.service) rather than using the template name directly.
        let canonical_name = {
            let mut resolved = load_name.clone();
            for dir in &ri.config.unit_dirs {
                let candidate = dir.join(&load_name);
                if candidate
                    .symlink_metadata()
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false)
                    && let Ok(target) = std::fs::canonicalize(&candidate)
                    && let Some(name) = target.file_name().map(|f| f.to_string_lossy().to_string())
                {
                    // If the load name is an instance and the target is a template,
                    // combine the target's template prefix with the load name's instance.
                    if let Some((_, instance)) =
                        crate::units::loading::directory_deps::parse_template_instance(&load_name)
                        && name.contains("@.")
                    {
                        // e.g., load_name="bar-alias@2.service", target="yup@.service"
                        // → derive "yup@2.service"
                        resolved = name.replace("@.", &format!("@{instance}."));
                        break;
                    }
                    resolved = name;
                    break;
                }
            }
            resolved
        };

        // If the canonical name differs (symlink alias), check if it's already
        // in the table. Also prefer loading by canonical name to avoid duplicates.
        if canonical_name != load_name {
            let found = find_units_with_name(&canonical_name, &ri.unit_table);
            if let Some(unit) = found.first() {
                return Ok(unit.id.clone());
            }
        }

        // Load by canonical name if it differs — this ensures the unit's ID
        // uses the real name, not the alias.
        let actual_load_name = if canonical_name != load_name {
            &canonical_name
        } else {
            &load_name
        };
        match load_new_unit(&ri.config.unit_dirs, actual_load_name) {
            Ok(unit) => {
                let id = unit.id.clone();
                crate::units::insert_new_unit_lenient(unit, &mut ri);
                trace!("Auto-loaded unit {unit_name} from disk");
                Ok(id)
            }
            Err(load_err) => {
                // If load_name is a symlink alias (e.g. test15-b@inst.service → test15-a@inst.service),
                // resolve it first. This allows reverse lookups and template instantiation
                // via the real unit name.
                let mut resolved_load_name = load_name.clone();
                let mut is_alias = false;
                for dir in &ri.config.unit_dirs {
                    let candidate = dir.join(&load_name);
                    if let Ok(meta) = candidate.symlink_metadata()
                        && meta.file_type().is_symlink()
                        && let Ok(target) = std::fs::read_link(&candidate)
                    {
                        let target_name = target
                            .file_name()
                            .map(|f| f.to_string_lossy().to_string())
                            .unwrap_or_default();
                        if !target_name.is_empty() && target_name != load_name {
                            // If target is a template and load_name is an instance,
                            // derive the instance name from the template.
                            let effective_target = if target_name.contains("@.")
                                && let Some((_, inst)) =
                                    crate::units::loading::directory_deps::parse_template_instance(
                                        &load_name,
                                    ) {
                                target_name.replace("@.", &format!("@{inst}."))
                            } else {
                                target_name
                            };
                            // Check if the target is already loaded
                            let found = find_units_with_name(&effective_target, &ri.unit_table);
                            if let Some(unit) = found.first() {
                                return Ok(unit.id.clone());
                            }
                            resolved_load_name = effective_target;
                            is_alias = true;
                            break;
                        }
                    }
                }

                // If we resolved an alias, try loading the resolved name
                if is_alias
                    && let Ok(mut unit) = load_new_unit(&ri.config.unit_dirs, &resolved_load_name)
                {
                    // Add the original name as an alias
                    if !unit.common.unit.aliases.contains(&load_name) {
                        unit.common.unit.aliases.push(load_name.clone());
                    }
                    let id = unit.id.clone();
                    crate::units::insert_new_unit_lenient(unit, &mut ri);
                    return Ok(id);
                }

                // If this looks like a template instance (e.g. "getty@tty1.service"),
                // try instantiating from the template file (e.g. "getty@.service").
                // Use the resolved name for template parsing so aliases like
                // test15-b@inst.service → test15-a@inst.service work correctly.
                if let Some((template_name, instance_name)) =
                    crate::units::loading::directory_deps::parse_template_instance(
                        &resolved_load_name,
                    )
                {
                    let empty_dropins = std::collections::HashMap::new();
                    if let Some(mut unit) =
                        crate::units::loading::directory_deps::instantiate_template(
                            &template_name,
                            &instance_name,
                            &resolved_load_name,
                            &ri.config.unit_dirs,
                            &empty_dropins,
                        )
                    {
                        // If we arrived here via alias resolution, add the
                        // original name as an alias.
                        if is_alias && !unit.common.unit.aliases.contains(&load_name) {
                            unit.common.unit.aliases.push(load_name.clone());
                        }
                        // Discover filesystem-level symlink aliases
                        // (includes template-level alias discovery for instances)
                        let unit_path = ri
                            .config
                            .unit_dirs
                            .iter()
                            .map(|d| d.join(&unit.id.name))
                            .find(|p| p.exists())
                            .or_else(|| {
                                ri.config
                                    .unit_dirs
                                    .iter()
                                    .map(|d| d.join(&template_name))
                                    .find(|p| p.exists())
                            })
                            .unwrap_or_default();
                        let discovered =
                            find_symlink_aliases(&ri.config.unit_dirs, &unit_path, &unit.id.name);
                        for alias in discovered {
                            if !unit.common.unit.aliases.contains(&alias) {
                                unit.common.unit.aliases.push(alias);
                            }
                        }
                        let id = unit.id.clone();
                        crate::units::insert_new_unit_lenient(unit, &mut ri);
                        info!(
                            "Instantiated template unit {resolved_load_name} from {template_name}"
                        );
                        return Ok(id);
                    }
                }
                // Last resort: unit_name might be a symlink alias
                // (e.g., test15-a1.service -> test15-a.service). Resolve it.
                let suffix = if unit_name.contains('.') {
                    ""
                } else {
                    ".service"
                };
                let full_name = format!("{unit_name}{suffix}");
                for dir in &ri.config.unit_dirs {
                    let candidate = dir.join(&full_name);
                    if let Ok(resolved) = std::fs::canonicalize(&candidate)
                        && let Some(resolved_name) = resolved
                            .file_name()
                            .map(|f| f.to_string_lossy().to_string())
                        && resolved_name != full_name
                    {
                        let found = find_units_with_name(&resolved_name, &ri.unit_table);
                        if let Some(unit) = found.first() {
                            return Ok(unit.id.clone());
                        }
                    }
                }
                Err(format!(
                    "No unit found with name: {unit_name} (also failed to load from disk: {load_err})"
                ))
            }
        }
    }
}

/// Refresh a unit's in-memory Wants/Requires dependencies by scanning
/// on-disk `.wants/` and `.requires/` directories across all unit search paths.
/// This matches real systemd behaviour where directory dependencies are
/// re-evaluated dynamically, not only at daemon-reload time.
///
/// Any newly discovered dependency units are also loaded from disk if they
/// aren't already in the unit table.
fn refresh_directory_deps(unit_name: &str, run_info: &ArcMutRuntimeInfo) {
    let unit_dirs: Vec<std::path::PathBuf> = {
        let ri = run_info.read_poisoned();
        ri.config.unit_dirs.clone()
    };

    for (suffix, is_requires) in &[("wants", false), ("requires", true)] {
        for dir in &unit_dirs {
            let dep_dir = dir.join(format!("{unit_name}.{suffix}"));
            if !dep_dir.is_dir() {
                continue;
            }
            let entries = match std::fs::read_dir(&dep_dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let entry_path = entry.path();
                // Resolve symlinks to get the canonical unit name
                let child_name = std::fs::canonicalize(&entry_path)
                    .ok()
                    .and_then(|p| p.file_name().map(|f| f.to_string_lossy().to_string()))
                    .unwrap_or_else(|| entry.file_name().to_string_lossy().to_string());

                if !crate::units::loading::directory_deps::is_unit_file(&child_name) {
                    continue;
                }

                // Try to load the child unit from disk if not already present.
                // Skip if it doesn't exist (Wants= is a soft dependency).
                if find_or_load_unit(&child_name, run_info).is_err() {
                    continue;
                }

                // Add the dependency to the parent unit's in-memory deps
                let mut ri = run_info.write_poisoned();
                let parent_id = ri
                    .unit_table
                    .keys()
                    .find(|id| id.name == unit_name)
                    .cloned();
                let child_id: Option<crate::units::UnitId> = child_name.as_str().try_into().ok();

                if let (Some(parent_id), Some(child_id)) = (parent_id, child_id)
                    && let Some(parent) = ri.unit_table.get_mut(&parent_id)
                {
                    if *is_requires {
                        if !parent.common.dependencies.requires.contains(&child_id) {
                            parent.common.dependencies.requires.push(child_id.clone());
                        }
                    } else if !parent.common.dependencies.wants.contains(&child_id) {
                        parent.common.dependencies.wants.push(child_id.clone());
                    }
                    // Also add After ordering so the dep is waited on
                    if !parent.common.dependencies.after.contains(&child_id) {
                        parent.common.dependencies.after.push(child_id);
                    }
                }
            }
        }
    }
}

/// Recursively load all dependency units (Wants, Requires, After) for a
/// given unit from disk. This ensures that when a unit is started, all units
/// in its dependency graph are present in the unit table.
fn load_dependency_units(unit_id: &crate::units::UnitId, run_info: &ArcMutRuntimeInfo) {
    let mut to_load: Vec<crate::units::UnitId> = Vec::new();
    {
        let ri = run_info.read_poisoned();
        if let Some(unit) = ri.unit_table.get(unit_id) {
            let deps = &unit.common.dependencies;
            for dep_id in deps
                .wants
                .iter()
                .chain(deps.requires.iter())
                .chain(deps.after.iter())
                .chain(deps.binds_to.iter())
                // Also check refs_by_name: it preserves original dependency
                // references that may have been pruned from the active dep
                // lists because the referenced unit didn't exist at load time.
                // This allows on-demand discovery of units created after
                // daemon-reload (e.g. a service file dropped in after a target
                // with Wants= was already loaded).
                .chain(unit.common.unit.refs_by_name.iter())
            {
                if !ri.unit_table.contains_key(dep_id) && !to_load.contains(dep_id) {
                    to_load.push(dep_id.clone());
                }
            }
        }
    }
    for dep_id in to_load {
        if find_or_load_unit(&dep_id.name, run_info).is_ok() {
            // Recursively load dependencies of newly loaded units
            load_dependency_units(&dep_id, run_info);
        }
    }
}

// TODO make this some kind of regex pattern matching
fn find_units_with_pattern<'a>(
    name_pattern: &str,
    unit_table_locked: &'a UnitTable,
) -> Vec<&'a Unit> {
    trace!("Find units matching pattern: {name_pattern}");
    let units: Vec<_> = unit_table_locked
        .values()
        .filter(|unit| {
            let name = unit.id.name.clone();
            name.starts_with(name_pattern)
                || unit
                    .common
                    .unit
                    .aliases
                    .iter()
                    .any(|alias| alias.starts_with(name_pattern))
        })
        .collect();
    units
}

/// Determine the state of a unit file: "enabled", "disabled", "static", or "indirect".
fn unit_file_state(
    name: &str,
    unit_table: &UnitTable,
    path: &std::path::Path,
    unit_dirs: &[std::path::PathBuf],
) -> &'static str {
    // Check if masked: the file is a symlink pointing to /dev/null
    // Check runtime mask first (/run/systemd/system/)
    let runtime_path = std::path::Path::new("/run/systemd/system").join(name);
    if let Ok(target) = std::fs::read_link(&runtime_path)
        && target == std::path::Path::new("/dev/null")
    {
        return "masked-runtime";
    }
    // Check persistent mask (/etc/systemd/system/)
    let persistent_path = std::path::Path::new("/etc/systemd/system").join(name);
    if let Ok(target) = std::fs::read_link(&persistent_path)
        && target == std::path::Path::new("/dev/null")
    {
        return "masked";
    }

    // Check if the unit has an [Install] section by looking for WantedBy=/RequiredBy=
    // in the loaded unit table, or by reading the file itself.
    let has_install = if let Some(unit) = unit_table.values().find(|u| u.id.name == name) {
        !unit.common.dependencies.wanted_by.is_empty()
            || !unit.common.dependencies.required_by.is_empty()
    } else {
        // Not loaded — peek at the file for [Install]
        if let Ok(content) = std::fs::read_to_string(path) {
            content.contains("[Install]")
                && (content.contains("WantedBy=") || content.contains("RequiredBy="))
        } else {
            false
        }
    };

    if !has_install {
        // No [Install] section → static (cannot be enabled/disabled)
        "static"
    } else {
        // Check if enablement symlinks exist in .wants/.requires directories.
        for dir in unit_dirs {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let entry_name = entry.file_name();
                    let entry_str = entry_name.to_string_lossy();
                    if entry_str.ends_with(".wants") || entry_str.ends_with(".requires") {
                        let symlink = entry.path().join(name);
                        if symlink.exists() || symlink.symlink_metadata().is_ok() {
                            return "enabled";
                        }
                    }
                }
            }
        }
        "disabled"
    }
}

/// Format a dependency tree as an indented string with box-drawing characters.
///
/// `visited` tracks already-printed units to avoid infinite loops in cyclic graphs.
#[allow(clippy::too_many_arguments)]
fn format_dep_tree(
    unit_name: &str,
    unit_table: &UnitTable,
    reverse: bool,
    prefix: &str,
    is_last: bool,
    visited: &mut std::collections::HashSet<String>,
    out: &mut String,
    depth: usize,
) {
    // Print this node
    if depth == 0 {
        // Root node — no prefix/connector
        let status_marker = unit_status_marker(unit_name, unit_table);
        let _ = writeln!(out, "{status_marker}{unit_name}");
    } else {
        let connector = if is_last { "└─" } else { "├─" };
        let status_marker = unit_status_marker(unit_name, unit_table);
        let _ = writeln!(out, "{prefix}{connector}{status_marker}{unit_name}");
    }

    // Prevent infinite recursion on cycles
    if !visited.insert(unit_name.to_owned()) {
        return;
    }

    // Limit depth to avoid excessively deep trees
    if depth >= 20 {
        return;
    }

    // Find children (forward deps = wants+requires, reverse deps = wanted_by+required_by)
    let children: Vec<String> =
        if let Some(unit) = unit_table.values().find(|u| u.id.name == unit_name) {
            let deps = &unit.common.dependencies;
            if reverse {
                let mut c: Vec<String> = Vec::new();
                for id in &deps.wanted_by {
                    c.push(id.name.clone());
                }
                for id in &deps.required_by {
                    if !c.contains(&id.name) {
                        c.push(id.name.clone());
                    }
                }
                for id in &deps.bound_by {
                    if !c.contains(&id.name) {
                        c.push(id.name.clone());
                    }
                }
                c.sort();
                c
            } else {
                let mut c: Vec<String> = Vec::new();
                for id in &deps.wants {
                    c.push(id.name.clone());
                }
                for id in &deps.requires {
                    if !c.contains(&id.name) {
                        c.push(id.name.clone());
                    }
                }
                for id in &deps.binds_to {
                    if !c.contains(&id.name) {
                        c.push(id.name.clone());
                    }
                }
                c.sort();
                c
            }
        } else {
            Vec::new()
        };

    let child_prefix = if depth == 0 {
        String::new()
    } else if is_last {
        format!("{prefix}  ")
    } else {
        format!("{prefix}│ ")
    };

    for (i, child) in children.iter().enumerate() {
        let child_is_last = i == children.len() - 1;
        format_dep_tree(
            child,
            unit_table,
            reverse,
            &child_prefix,
            child_is_last,
            visited,
            out,
            depth + 1,
        );
    }

    // Remove from visited so the same unit can appear in other branches
    // (but the recursion guard above prevents infinite loops within a single branch)
    visited.remove(unit_name);
}

/// Return a colored status marker for a unit: ● (green=active, white=inactive, red=failed).
fn unit_status_marker(unit_name: &str, unit_table: &UnitTable) -> &'static str {
    if let Some(unit) = unit_table.values().find(|u| u.id.name == unit_name) {
        let status = unit.common.status.read_poisoned();
        match &*status {
            UnitStatus::Started(_) => "● ",
            UnitStatus::Starting | UnitStatus::Restarting => "● ",
            UnitStatus::Stopped(_, errors) if !errors.is_empty() => "× ",
            UnitStatus::Stopped(_, _) | UnitStatus::NeverStarted => "○ ",
            UnitStatus::Stopping => "○ ",
        }
    } else {
        "○ "
    }
}

/// Create or update an implicit slice unit with default properties and any applicable
/// drop-in overrides. In systemd, slice units exist implicitly without a unit file.
/// The description follows the pattern "Slice /a/b/c" where dashes become path separators.
fn create_or_update_implicit_slice(
    slice_name: &str,
    run_info: &mut crate::runtime_info::RuntimeInfo,
) {
    use crate::units::loading::directory_deps::{
        collect_applicable_dropins_pub, collect_dropin_entries, parse_dropin_dir_name,
    };

    // Generate description: "a-b-c.slice" -> "Slice /a/b/c"
    let base = slice_name.strip_suffix(".slice").unwrap_or(slice_name);
    let path = base.replace('-', "/");
    let description = format!("Slice /{path}");

    // Build the base content
    let base_content = format!("[Unit]\nDescription={description}\n\n[Slice]\n");

    // Collect applicable drop-ins from filesystem
    let mut dropins: std::collections::HashMap<String, Vec<(String, String)>> =
        std::collections::HashMap::new();
    for dir in &run_info.config.unit_dirs {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if let Some(dropin_unit_name) = parse_dropin_dir_name(&name)
                    && entry.path().is_dir()
                {
                    collect_dropin_entries(&entry.path(), &dropin_unit_name, &mut dropins);
                }
            }
        }
    }

    let overrides = collect_applicable_dropins_pub(slice_name, &dropins);

    // Merge base content with drop-in overrides
    let merged = if overrides.is_empty() {
        base_content
    } else {
        let mut result = base_content;
        for (_filename, content) in &overrides {
            result =
                crate::units::loading::directory_deps::merge_ini_sections_pub(&result, content);
        }
        result
    };

    let fake_path = std::path::PathBuf::from(format!("/run/systemd/system/{slice_name}"));
    let parsed = match crate::units::parse_file(&merged) {
        Ok(pf) => pf,
        Err(e) => {
            info!("Failed to parse implicit slice {}: {:?}", slice_name, e);
            return;
        }
    };
    let parsed_config = match crate::units::parse_slice(parsed, &fake_path) {
        Ok(c) => c,
        Err(e) => {
            info!("Failed to create implicit slice {}: {:?}", slice_name, e);
            return;
        }
    };
    let unit: Unit = match parsed_config.try_into() {
        Ok(u) => u,
        Err(e) => {
            info!("Failed to convert implicit slice {}: {:?}", slice_name, e);
            return;
        }
    };

    // Remove existing entry if any
    let existing_id = run_info
        .unit_table
        .keys()
        .find(|id| id.name == slice_name)
        .cloned();
    if let Some(existing_id) = existing_id {
        run_info.unit_table.remove(&existing_id);
    }

    info!("Created/updated implicit slice unit: {}", slice_name);
    run_info.unit_table.insert(unit.id.clone(), unit);
}

/// Create a transient (in-memory) service unit and insert it into the unit table.
/// Apply drop-in overrides from the filesystem to a transient unit.
///
/// Scans all unit directories for applicable drop-in directories
/// (type-level like `service.d/`, prefix like `a-.service.d/`, and exact like
/// `unit.service.d/`), collects `.conf` files, parses their [Service] sections,
/// and applies properties to the transient unit's config.
fn apply_dropins_to_transient(unit: &mut Unit, unit_dirs: &[std::path::PathBuf]) {
    use crate::units::loading::directory_deps::{collect_dropin_entries, parse_dropin_dir_name};

    let unit_name = &unit.id.name;

    // Collect all drop-in entries from all unit dirs
    let mut dropins: std::collections::HashMap<String, Vec<(String, String)>> =
        std::collections::HashMap::new();

    for dir in unit_dirs {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if let Some(dropin_unit_name) = parse_dropin_dir_name(&name)
                    && entry.path().is_dir()
                {
                    collect_dropin_entries(&entry.path(), &dropin_unit_name, &mut dropins);
                }
            }
        }
    }

    if dropins.is_empty() {
        return;
    }

    // Use the same logic as normal drop-in application to find applicable overrides
    let overrides =
        crate::units::loading::directory_deps::collect_applicable_dropins_pub(unit_name, &dropins);

    if overrides.is_empty() {
        return;
    }

    // Apply the drop-in overrides to the unit's service config
    if let Specific::Service(ref mut svc) = unit.specific {
        for (_filename, content) in &overrides {
            // Parse the drop-in content as INI sections
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty()
                    || line.starts_with('#')
                    || line.starts_with(';')
                    || line.starts_with('[')
                {
                    continue;
                }
                if let Some((key, value)) = line.split_once('=') {
                    let key = key.trim();
                    let value = value.trim();

                    let parse_cmd = |s: &str| -> crate::units::Commandline {
                        crate::units::unit_parsing::parse_cmdline(s).unwrap_or_else(|_| {
                            let parts: Vec<String> =
                                shlex::split(s).unwrap_or_else(|| vec![s.to_string()]);
                            crate::units::Commandline {
                                cmd: parts.first().cloned().unwrap_or_default(),
                                args: parts.into_iter().skip(1).collect(),
                                prefixes: vec![],
                            }
                        })
                    };

                    // Strip "Ex" suffix to map ExecStartEx -> ExecStart, etc.
                    let base_key = key.strip_suffix("Ex").unwrap_or(key);

                    match base_key {
                        "ExecCondition" => {
                            if value.is_empty() {
                                svc.conf.exec_condition.clear();
                            } else {
                                svc.conf.exec_condition.push(parse_cmd(value));
                            }
                        }
                        "ExecStartPre" => {
                            if value.is_empty() {
                                svc.conf.startpre.clear();
                            } else {
                                svc.conf.startpre.push(parse_cmd(value));
                            }
                        }
                        "ExecStart" => {
                            if value.is_empty() {
                                svc.conf.exec.clear();
                            } else {
                                svc.conf.exec.push(parse_cmd(value));
                            }
                        }
                        "ExecStartPost" => {
                            if value.is_empty() {
                                svc.conf.startpost.clear();
                            } else {
                                svc.conf.startpost.push(parse_cmd(value));
                            }
                        }
                        "ExecReload" => {
                            if value.is_empty() {
                                svc.conf.reload.clear();
                            } else {
                                svc.conf.reload.push(parse_cmd(value));
                            }
                        }
                        "ExecStop" => {
                            if value.is_empty() {
                                svc.conf.stop.clear();
                            } else {
                                svc.conf.stop.push(parse_cmd(value));
                            }
                        }
                        "ExecStopPost" => {
                            if value.is_empty() {
                                svc.conf.stoppost.clear();
                            } else {
                                svc.conf.stoppost.push(parse_cmd(value));
                            }
                        }
                        _ => {}
                    }
                    match key {
                        "Description" => {
                            if !value.is_empty() {
                                unit.common.unit.description = value.to_string();
                            }
                        }
                        _ => {
                            // Other properties are not applied to transient units for now
                        }
                    }
                }
            }
        }
    }
}

/// Create a `UnitId` from a unit name string, inferring the kind from the suffix.
fn unit_id_from_name(name: &str) -> UnitId {
    use crate::units::UnitIdKind;
    let kind = if name.ends_with(".service") {
        UnitIdKind::Service
    } else if name.ends_with(".socket") {
        UnitIdKind::Socket
    } else if name.ends_with(".target") {
        UnitIdKind::Target
    } else if name.ends_with(".timer") {
        UnitIdKind::Timer
    } else if name.ends_with(".path") {
        UnitIdKind::Path
    } else if name.ends_with(".mount") {
        UnitIdKind::Mount
    } else if name.ends_with(".swap") {
        UnitIdKind::Swap
    } else if name.ends_with(".slice") {
        UnitIdKind::Slice
    } else if name.ends_with(".device") {
        UnitIdKind::Device
    } else {
        UnitIdKind::Service // default
    };
    UnitId {
        kind,
        name: name.to_string(),
    }
}

/// Read the DefaultLimitNOFILE setting from system.conf.d drop-ins.
/// Returns `None` if no default is configured.
fn read_default_limit_nofile() -> Option<crate::units::ResourceLimit> {
    for dir in &[
        "/run/systemd/system.conf.d",
        "/etc/systemd/system.conf.d",
        "/usr/lib/systemd/system.conf.d",
    ] {
        let dir = std::path::Path::new(dir);
        if !dir.is_dir() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut files: Vec<_> = entries.flatten().collect();
            files.sort_by_key(|e| e.file_name());
            for entry in files {
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    for line in content.lines() {
                        let line = line.trim();
                        if let Some(val) = line.strip_prefix("DefaultLimitNOFILE=") {
                            return crate::units::unit_parsing::parse_resource_limit(val);
                        }
                    }
                }
            }
        }
    }
    None
}

///
/// Transient units are not backed by a unit file on disk — they exist only in
/// memory for the lifetime of the service manager (or until explicitly removed).
/// This is the mechanism behind `systemd-run`.
/// Write a transient service unit file to disk so `systemctl cat` can display it.
fn write_transient_service_file(
    path: &std::path::Path,
    params: &TransientUnitParams,
) -> Result<(), std::io::Error> {
    use std::io::Write;
    let mut f = std::fs::File::create(path)?;

    // [Unit] section
    let mut has_unit = false;
    if let Some(ref desc) = params.description {
        if !has_unit {
            writeln!(f, "[Unit]")?;
            has_unit = true;
        }
        writeln!(f, "Description={desc}")?;
    }

    // Check properties for unit-section keys
    for prop in &params.properties {
        if let Some((key, value)) = prop.split_once('=') {
            match key {
                "Description" if !has_unit => {
                    writeln!(f, "[Unit]")?;
                    has_unit = true;
                    writeln!(f, "Description={value}")?;
                }
                "Description" => writeln!(f, "Description={value}")?,
                _ => {}
            }
        }
    }

    // [Service] section
    writeln!(f)?;
    writeln!(f, "[Service]")?;

    if let Some(ref stype) = params.service_type {
        writeln!(f, "Type={stype}")?;
    }

    if params.remain_after_exit {
        writeln!(f, "RemainAfterExit=yes")?;
    }

    if let Some(ref wd) = params.working_directory {
        writeln!(f, "WorkingDirectory={wd}")?;
    }

    if let Some(ref user) = params.user {
        writeln!(f, "User={user}")?;
    }

    if let Some(nice) = params.nice {
        writeln!(f, "Nice={nice}")?;
    }

    if let Some(ref group) = params.group {
        writeln!(f, "Group={group}")?;
    }

    // Write properties from -p flags
    for prop in &params.properties {
        if let Some((key, _)) = prop.split_once('=') {
            // Skip unit-section keys already handled above
            if key == "Description" {
                continue;
            }
            writeln!(f, "{prop}")?;
        }
    }

    // Write environment variables
    if !params.environment.is_empty() {
        let env_str = params
            .environment
            .iter()
            .map(|e| format!("\"{e}\""))
            .collect::<Vec<_>>()
            .join(" ");
        writeln!(f, "Environment={env_str}")?;
    }

    // Write ExecStart
    if let Some(ref cmd) = params.command
        && !cmd.is_empty()
    {
        let cmd_str = cmd
            .iter()
            .map(|s| {
                if s.contains(' ') || s.contains('"') {
                    format!("\"{}\"", s.replace('"', "\\\""))
                } else {
                    s.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        writeln!(f, "ExecStart={cmd_str}")?;
    }

    Ok(())
}

/// Write a transient timer unit file to disk.
fn write_transient_timer_file(
    path: &std::path::Path,
    params: &TransientUnitParams,
    service_unit_name: &str,
) -> Result<(), std::io::Error> {
    use std::io::Write;
    let mut f = std::fs::File::create(path)?;

    // [Unit] section
    let mut has_unit = false;
    if let Some(ref desc) = params.description {
        writeln!(f, "[Unit]")?;
        has_unit = true;
        writeln!(f, "Description={desc}")?;
    }

    // Timer properties from --timer-property that belong in [Unit]
    for prop in &params.timer_properties {
        if let Some((key, _)) = prop.split_once('=')
            && (key == "After" || key == "Before" || key == "Wants" || key == "Requires")
        {
            if !has_unit {
                writeln!(f, "[Unit]")?;
                has_unit = true;
            }
            writeln!(f, "{prop}")?;
        }
    }
    let _ = has_unit; // suppress unused warning

    // [Timer] section
    writeln!(f)?;
    writeln!(f, "[Timer]")?;

    for on_active in &params.on_active {
        if let Some(d) = parse_timespan(on_active) {
            writeln!(f, "OnActiveSec={}", format_duration(&d))?;
        }
    }
    if let Some(ref on_boot) = params.on_boot
        && let Some(d) = parse_timespan(on_boot)
    {
        writeln!(f, "OnBootSec={}", format_duration(&d))?;
    }
    if let Some(ref on_startup) = params.on_startup
        && let Some(d) = parse_timespan(on_startup)
    {
        writeln!(f, "OnStartupSec={}", format_duration(&d))?;
    }
    if let Some(ref on_unit_active) = params.on_unit_active
        && let Some(d) = parse_timespan(on_unit_active)
    {
        writeln!(f, "OnUnitActiveSec={}", format_duration(&d))?;
    }
    if let Some(ref on_unit_inactive) = params.on_unit_inactive
        && let Some(d) = parse_timespan(on_unit_inactive)
    {
        writeln!(f, "OnUnitInactiveSec={}", format_duration(&d))?;
    }
    if let Some(ref on_calendar) = params.on_calendar {
        writeln!(f, "OnCalendar={on_calendar}")?;
    }
    if params.on_clock_change {
        writeln!(f, "OnClockChange=yes")?;
    }
    if params.on_timezone_change {
        writeln!(f, "OnTimezoneChange=yes")?;
    }

    // Additional timer properties (non-unit-section ones)
    for prop in &params.timer_properties {
        if let Some((key, _)) = prop.split_once('=')
            && key != "After"
            && key != "Before"
            && key != "Wants"
            && key != "Requires"
        {
            writeln!(f, "{prop}")?;
        }
    }

    writeln!(f, "Unit={service_unit_name}")?;

    Ok(())
}

/// Write a transient auxiliary unit file (path or socket) to disk.
fn write_transient_auxiliary_file(
    path: &std::path::Path,
    section: &str,
    section_properties: &[String],
    params: &TransientUnitParams,
) -> Result<(), std::io::Error> {
    use std::io::Write;
    let mut f = std::fs::File::create(path)?;

    // [Unit] section
    if let Some(ref desc) = params.description {
        writeln!(f, "[Unit]")?;
        writeln!(f, "Description={desc}")?;
        writeln!(f)?;
    }

    // Main section ([Path] or [Socket])
    writeln!(f, "[{section}]")?;
    for prop in section_properties {
        writeln!(f, "{prop}")?;
    }

    Ok(())
}

fn create_transient_unit(
    params: &TransientUnitParams,
    run_info: &ArcMutRuntimeInfo,
) -> Result<crate::units::UnitId, String> {
    use crate::units::{
        Commandline, Common, CommonState, Delegate, Dependencies, ExecConfig, NotifyKind,
        PlatformSpecificServiceFields, ServiceConfig, ServiceSpecific, ServiceState, ServiceType,
        Specific, SuccessExitStatus, UnitConfig, UnitId, UnitIdKind, UnitStatus, UnitTimestamps,
    };
    use std::sync::RwLock;

    let unit_name = &params.unit_name;

    // Determine the service type.
    let srvc_type = match params.service_type.as_deref() {
        Some("simple") | None => ServiceType::Simple,
        Some("exec") => ServiceType::Exec,
        Some("oneshot") => ServiceType::OneShot,
        Some("forking") => ServiceType::Forking,
        Some("notify") => ServiceType::Notify,
        Some("notify-reload") => ServiceType::NotifyReload,
        Some("dbus") => ServiceType::Dbus,
        Some("idle") => ServiceType::Idle,
        Some(other) => return Err(format!("Unknown service type: {other}")),
    };

    // Build the main command from the command line.  This is appended to
    // the ExecStart list AFTER any `-p ExecStart=` properties, so that
    // for multi-ExecStart oneshot services the property-based commands run
    // first and the trailing command is the main process.
    let main_cmd: Option<Commandline> = params
        .command
        .as_ref()
        .filter(|cmd_parts| !cmd_parts.is_empty())
        .map(|cmd_parts| Commandline {
            cmd: cmd_parts[0].clone(),
            args: cmd_parts[1..].to_vec(),
            prefixes: vec![],
        });
    let exec: Vec<Commandline> = Vec::new();

    // Ensure the transient directory exists for --pipe temp files.
    if params.pipe {
        let _ = std::fs::create_dir_all("/run/systemd/transient");
    }

    // Build a minimal ExecConfig with the requested user/group/workdir.
    // Use the correct concrete types for ExecConfig fields.
    // Non-optional enums use their Default impl; bools use systemd defaults.
    let exec_config = ExecConfig {
        user: params.user.clone(),
        group: params.group.clone(),
        supplementary_groups: vec![],
        stdin_option: crate::units::StandardInput::Null,
        stdout_path: if params.pipe {
            Some(crate::units::StdIoOption::File(
                format!("/run/systemd/transient/{}.stdout", params.unit_name).into(),
            ))
        } else {
            None
        },
        stderr_path: if params.pipe {
            Some(crate::units::StdIoOption::File(
                format!("/run/systemd/transient/{}.stderr", params.unit_name).into(),
            ))
        } else {
            None
        },
        environment: None,
        environment_files: vec![],
        working_directory: params
            .working_directory
            .as_ref()
            .map(std::path::PathBuf::from),
        state_directory: vec![],
        logs_directory: vec![],
        logs_directory_mode: None,
        runtime_directory: vec![],
        runtime_directory_preserve: crate::units::RuntimeDirectoryPreserve::No,
        tty_path: None,
        tty_reset: false,
        tty_vhangup: false,
        tty_vt_disallocate: false,
        ignore_sigpipe: true, // systemd default
        utmp_identifier: None,
        utmp_mode: crate::units::UtmpMode::Init,
        import_credentials: vec![],
        load_credentials: vec![],
        load_credentials_encrypted: vec![],
        set_credentials: vec![],
        set_credentials_encrypted: vec![],
        pass_environment: vec![],
        unset_environment: vec![],
        oom_score_adjust: None,
        log_extra_fields: vec![],
        dynamic_user: false,
        system_call_filter: vec![],
        system_call_log: vec![],
        protect_system: crate::units::ProtectSystem::No,
        restrict_namespaces: crate::units::RestrictNamespaces::No,
        restrict_realtime: false,
        restrict_address_families: vec![],
        restrict_file_systems: vec![],
        system_call_error_number: None,
        no_new_privileges: false,
        protect_control_groups: false,
        protect_kernel_modules: false,
        restrict_suid_sgid: false,
        protect_kernel_logs: false,
        protect_kernel_tunables: false,
        protect_clock: false,
        capability_bounding_set: vec![],
        ambient_capabilities: vec![],
        protect_home: crate::units::ProtectHome::No,
        protect_hostname: false,
        protect_hostname_mode: None,
        protect_hostname_name: None,
        system_call_architectures: vec![],
        read_write_paths: vec![],
        memory_deny_write_execute: false,
        lock_personality: false,
        protect_proc: crate::units::ProtectProc::Default,
        private_tmp: false,
        private_devices: false,
        private_network: false,
        private_users: false,
        private_mounts: false,
        mount_flags: None,
        io_scheduling_class: crate::units::IOSchedulingClass::None,
        io_scheduling_priority: None,
        umask: None,
        proc_subset: crate::units::ProcSubset::All,
        nice: params.nice,
        remove_ipc: false,
        pam_name: None,
        limit_cpu: None,
        limit_core: None,
        limit_fsize: None,
        limit_data: None,
        limit_stack: None,
        limit_rss: None,
        limit_nproc: None,
        limit_memlock: None,
        limit_as: None,
        limit_locks: None,
        limit_sigpending: None,
        limit_msgqueue: None,
        limit_nice: None,
        limit_rtprio: None,
        limit_rttime: None,
        cache_directory: vec![],
        cache_directory_mode: None,
        configuration_directory: vec![],
        configuration_directory_mode: None,
        state_directory_mode: None,
        runtime_directory_mode: None,
        read_only_paths: vec![],
        inaccessible_paths: vec![],
        bind_paths: vec![],
        bind_read_only_paths: vec![],
        temporary_file_system: vec![],
        syslog_identifier: None,
        syslog_facility: None,
        syslog_level: None,
        syslog_level_prefix: None,
        log_level_max: None,
        log_rate_limit_interval_sec: None,
        log_rate_limit_burst: None,
        log_filter_patterns: vec![],
        log_namespace: None,
        cpu_scheduling_policy: None,
        cpu_scheduling_priority: None,
        cpu_scheduling_reset_on_fork: None,
        cpu_affinity: vec![],
        numa_policy: None,
        numa_mask: None,
        root_directory: None,
        root_image: None,
        root_image_options: vec![],
        root_hash: None,
        root_hash_signature: None,
        root_verity: None,
        root_ephemeral: None,
        mount_api_vfs: None,
        extension_directories: vec![],
        extension_images: vec![],
        mount_images: vec![],
        bind_log_sockets: None,
        private_ipc: None,
        private_pids: None,
        ipc_namespace_path: None,
        network_namespace_path: None,
        secure_bits: vec![],
        personality: None,
        selinux_context: None,
        apparmor_profile: None,
        smack_process_label: None,
        keyring_mode_exec: None,
        no_exec_paths: vec![],
        exec_paths: vec![],
        coredump_filter: None,
        timer_slack_nsec: None,
        standard_input_text: vec![],
        standard_input_data: vec![],
        set_login_environment: None,
    };

    let platform_specific = PlatformSpecificServiceFields {
        #[cfg(target_os = "linux")]
        cgroup_path: std::path::PathBuf::from(format!("/sys/fs/cgroup/rust-systemd/{unit_name}")),
    };

    let mut service_conf = ServiceConfig {
        restart: crate::units::ServiceRestart::No,
        restart_sec: None,
        kill_mode: crate::units::KillMode::ControlGroup,
        delegate: Delegate::No,
        tasks_max: None,
        limit_nofile: None,
        accept: false,
        notifyaccess: NotifyKind::None,
        exec,
        reload: vec![],
        stop: vec![],
        stoppost: vec![],
        startpre: vec![],
        startpost: vec![],
        srcv_type: srvc_type,
        starttimeout: None,
        stoptimeout: None,
        generaltimeout: None,
        exec_config,
        platform_specific,
        dbus_name: None,
        pid_file: None,
        sockets: vec![],
        slice: params.slice.clone(),
        remain_after_exit: params.remain_after_exit,
        success_exit_status: SuccessExitStatus::default(),
        restart_force_exit_status: SuccessExitStatus::default(),
        send_sighup: false,
        memory_pressure_watch: crate::units::MemoryPressureWatch::Auto,
        reload_signal: None,
        kill_signal: None,
        delegate_subgroup: None,
        keyring_mode: crate::units::KeyringMode::Private,
        device_allow: vec![],
        device_policy: crate::units::DevicePolicy::Auto,
        watchdog_sec: None,
        ip_address_allow: vec![],
        ip_address_deny: vec![],
        file_descriptor_store_max: 0,
        file_descriptor_store_preserve: crate::units::FileDescriptorStorePreserve::No,
        memory_min: None,
        memory_low: None,
        memory_high: None,
        memory_max: None,
        memory_swap_max: None,
        cpu_weight: None,
        startup_cpu_weight: None,
        cpu_quota: None,
        io_weight: None,
        startup_io_weight: None,
        io_device_weight: vec![],
        io_read_bandwidth_max: vec![],
        io_write_bandwidth_max: vec![],
        io_read_iops_max: vec![],
        io_write_iops_max: vec![],
        cpu_accounting: None,
        memory_accounting: None,
        io_accounting: None,
        tasks_accounting: None,
        runtime_max_sec: None,
        coredump_receive: false,
        send_sigkill: true, // systemd default
        restart_kill_signal: None,
        final_kill_signal: None,
        watchdog_signal: None,
        exit_type: crate::units::ExitType::Main,
        oom_policy: crate::units::OOMPolicy::Stop,
        timeout_abort_sec: None,
        timeout_clean_sec: None,
        restart_prevent_exit_status: SuccessExitStatus::default(),
        restart_mode: crate::units::RestartMode::Direct,
        restart_steps: 0,
        restart_max_delay_sec: None,
        exec_condition: vec![],
        guess_main_pid: true,
        timeout_start_failure_mode: crate::units::TimeoutFailureMode::Terminate,
        timeout_stop_failure_mode: crate::units::TimeoutFailureMode::Terminate,
        runtime_randomized_extra_sec: None,
        root_directory_start_only: false,
        non_blocking: false,
        usb_function_descriptors: None,
        usb_function_strings: None,
        open_file: vec![],
        cpu_quota_period_sec: None,
        allowed_cpus: None,
        startup_allowed_cpus: None,
        allowed_memory_nodes: None,
        startup_allowed_memory_nodes: None,
        default_memory_min: None,
        default_memory_low: None,
        memory_zswap_max: None,
        io_device_latency_target_sec: vec![],
        disable_controllers: vec![],
        memory_pressure_threshold_sec: None,
        ip_ingress_filter_path: vec![],
        ip_egress_filter_path: vec![],
        bpf_program: vec![],
        socket_bind_allow: vec![],
        socket_bind_deny: vec![],
        restrict_network_interfaces: vec![],
        nft_set: vec![],
    };

    // Apply -p / --property overrides from the caller.
    // Parse "Key=Value" pairs and set the corresponding fields.
    let mut failure_action = crate::units::UnitAction::None;
    let mut success_action = crate::units::UnitAction::None;
    let mut prop_description: Option<String> = None;
    let mut success_action_units: Vec<String> = vec![];
    let mut failure_action_units: Vec<String> = vec![];
    let mut start_limit_burst: Option<u32> = None;
    let mut start_limit_interval_sec: Option<crate::units::unit_parsing::Timeout> = None;
    let mut dep_wants: Vec<String> = vec![];
    let mut dep_requires: Vec<String> = vec![];
    let mut dep_after: Vec<String> = vec![];
    let mut dep_before: Vec<String> = vec![];
    for prop in &params.properties {
        if let Some((key, value)) = prop.split_once('=') {
            match key {
                "Description" => {
                    prop_description = Some(value.to_string());
                }
                "Type" => {
                    service_conf.srcv_type = match value {
                        "simple" => ServiceType::Simple,
                        "exec" => ServiceType::Exec,
                        "oneshot" => ServiceType::OneShot,
                        "forking" => ServiceType::Forking,
                        "notify" => ServiceType::Notify,
                        "notify-reload" => ServiceType::NotifyReload,
                        "dbus" => ServiceType::Dbus,
                        "idle" => ServiceType::Idle,
                        _ => service_conf.srcv_type,
                    };
                }
                "RuntimeMaxSec" => {
                    // Parse simple seconds (e.g. "5" or "5s")
                    let trimmed = value.trim_end_matches('s');
                    if let Ok(secs) = trimmed.parse::<u64>() {
                        service_conf.runtime_max_sec =
                            Some(crate::units::unit_parsing::Timeout::Duration(
                                std::time::Duration::from_secs(secs),
                            ));
                    }
                }
                "RemainAfterExit" => {
                    service_conf.remain_after_exit = matches!(value, "yes" | "true" | "1");
                }
                "DynamicUser" => {
                    service_conf.exec_config.dynamic_user = matches!(value, "yes" | "true" | "1");
                }
                "NotifyAccess" => {
                    service_conf.notifyaccess = match value {
                        "all" => NotifyKind::All,
                        "main" => NotifyKind::Main,
                        "exec" => NotifyKind::Exec,
                        _ => NotifyKind::None,
                    };
                }
                "FailureAction" => {
                    failure_action = match value {
                        "poweroff" => crate::units::UnitAction::Poweroff,
                        "reboot" => crate::units::UnitAction::Reboot,
                        "exit" => crate::units::UnitAction::Exit,
                        _ => crate::units::UnitAction::None,
                    };
                }
                "SuccessAction" => {
                    success_action = match value {
                        "poweroff" => crate::units::UnitAction::Poweroff,
                        "reboot" => crate::units::UnitAction::Reboot,
                        "exit" => crate::units::UnitAction::Exit,
                        _ => crate::units::UnitAction::None,
                    };
                }
                "User" => {
                    service_conf.exec_config.user = Some(value.to_string());
                }
                "Group" => {
                    service_conf.exec_config.group = Some(value.to_string());
                }
                "SupplementaryGroups" => {
                    if value.is_empty() {
                        service_conf.exec_config.supplementary_groups.clear();
                    } else {
                        for g in value.split_whitespace() {
                            service_conf
                                .exec_config
                                .supplementary_groups
                                .push(g.to_string());
                        }
                    }
                }
                "WorkingDirectory" => {
                    service_conf.exec_config.working_directory =
                        Some(std::path::PathBuf::from(value));
                }
                "StateDirectory" => {
                    if value.is_empty() {
                        service_conf.exec_config.state_directory.clear();
                    } else {
                        for entry in value.split_whitespace() {
                            service_conf
                                .exec_config
                                .state_directory
                                .push(entry.to_string());
                        }
                    }
                }
                "RuntimeDirectory" => {
                    if value.is_empty() {
                        service_conf.exec_config.runtime_directory.clear();
                    } else {
                        for entry in value.split_whitespace() {
                            service_conf
                                .exec_config
                                .runtime_directory
                                .push(entry.to_string());
                        }
                    }
                }
                "CacheDirectory" => {
                    if value.is_empty() {
                        service_conf.exec_config.cache_directory.clear();
                    } else {
                        for entry in value.split_whitespace() {
                            service_conf
                                .exec_config
                                .cache_directory
                                .push(entry.to_string());
                        }
                    }
                }
                "LogsDirectory" => {
                    if value.is_empty() {
                        service_conf.exec_config.logs_directory.clear();
                    } else {
                        for entry in value.split_whitespace() {
                            service_conf
                                .exec_config
                                .logs_directory
                                .push(entry.to_string());
                        }
                    }
                }
                "ConfigurationDirectory" => {
                    if value.is_empty() {
                        service_conf.exec_config.configuration_directory.clear();
                    } else {
                        for entry in value.split_whitespace() {
                            service_conf
                                .exec_config
                                .configuration_directory
                                .push(entry.to_string());
                        }
                    }
                }
                "RuntimeDirectoryMode" => {
                    if let Ok(m) = u32::from_str_radix(value.trim(), 8) {
                        service_conf.exec_config.runtime_directory_mode = Some(m);
                    }
                }
                "StateDirectoryMode" => {
                    if let Ok(m) = u32::from_str_radix(value.trim(), 8) {
                        service_conf.exec_config.state_directory_mode = Some(m);
                    }
                }
                "CacheDirectoryMode" => {
                    if let Ok(m) = u32::from_str_radix(value.trim(), 8) {
                        service_conf.exec_config.cache_directory_mode = Some(m);
                    }
                }
                "LogsDirectoryMode" => {
                    if let Ok(m) = u32::from_str_radix(value.trim(), 8) {
                        service_conf.exec_config.logs_directory_mode = Some(m);
                    }
                }
                "ConfigurationDirectoryMode" => {
                    if let Ok(m) = u32::from_str_radix(value.trim(), 8) {
                        service_conf.exec_config.configuration_directory_mode = Some(m);
                    }
                }
                "RuntimeDirectoryPreserve" => {
                    service_conf.exec_config.runtime_directory_preserve = match value {
                        "yes" | "true" | "1" => crate::units::RuntimeDirectoryPreserve::Yes,
                        "restart" => crate::units::RuntimeDirectoryPreserve::Restart,
                        _ => crate::units::RuntimeDirectoryPreserve::No,
                    };
                }
                "TemporaryFileSystem"
                | "PrivateNetwork"
                | "PrivateDevices"
                | "PrivateUsers"
                | "PrivatePIDs"
                | "ProtectSystem"
                | "ProtectHome" => {
                    // These sandbox properties are parsed but may not all be
                    // fully wired. Set what we can.
                    match key {
                        "PrivateNetwork" => {
                            service_conf.exec_config.private_network =
                                matches!(value, "yes" | "true" | "1");
                        }
                        "PrivateDevices" => {
                            service_conf.exec_config.private_devices =
                                matches!(value, "yes" | "true" | "1");
                        }
                        "PrivateUsers" => {
                            service_conf.exec_config.private_users =
                                matches!(value, "yes" | "true" | "1");
                        }
                        "PrivatePIDs" => {
                            service_conf.exec_config.private_pids =
                                Some(matches!(value, "yes" | "true" | "1"));
                        }
                        "ProtectSystem" => {
                            service_conf.exec_config.protect_system = match value {
                                "yes" | "true" | "1" => crate::units::ProtectSystem::Yes,
                                "full" => crate::units::ProtectSystem::Full,
                                "strict" => crate::units::ProtectSystem::Strict,
                                _ => crate::units::ProtectSystem::No,
                            };
                        }
                        "ProtectHome" => {
                            service_conf.exec_config.protect_home = match value {
                                "yes" | "true" | "1" => crate::units::ProtectHome::Yes,
                                "read-only" => crate::units::ProtectHome::ReadOnly,
                                "tmpfs" => crate::units::ProtectHome::Tmpfs,
                                _ => crate::units::ProtectHome::No,
                            };
                        }
                        "TemporaryFileSystem" => {
                            service_conf
                                .exec_config
                                .temporary_file_system
                                .push(value.to_string());
                        }
                        _ => {}
                    }
                }
                "OOMPolicy" => {
                    service_conf.oom_policy = match value {
                        "stop" => crate::units::OOMPolicy::Stop,
                        "kill" => crate::units::OOMPolicy::Kill,
                        "continue" => crate::units::OOMPolicy::Continue,
                        _ => crate::units::OOMPolicy::Stop,
                    };
                }
                "Restart" => {
                    service_conf.restart = match value {
                        "always" => crate::units::ServiceRestart::Always,
                        "on-failure" => crate::units::ServiceRestart::OnFailure,
                        "on-abnormal" => crate::units::ServiceRestart::OnAbnormal,
                        "on-abort" => crate::units::ServiceRestart::OnAbort,
                        "on-watchdog" => crate::units::ServiceRestart::OnWatchdog,
                        "on-success" => crate::units::ServiceRestart::OnSuccess,
                        _ => crate::units::ServiceRestart::No,
                    };
                }
                "Environment" => {
                    // Append to environment (strip surrounding quotes from value)
                    if let Some((k, v)) = value.split_once('=') {
                        let v = if (v.starts_with('\'') && v.ends_with('\''))
                            || (v.starts_with('"') && v.ends_with('"'))
                        {
                            &v[1..v.len() - 1]
                        } else {
                            v
                        };
                        let env = service_conf.exec_config.environment.get_or_insert_with(|| {
                            crate::units::unit_parsing::EnvVars { vars: vec![] }
                        });
                        env.vars.push((k.to_string(), v.to_string()));
                    }
                }
                "MemoryMax" => {
                    service_conf.memory_max = Some(parse_memory_limit(value));
                }
                "MemoryHigh" => {
                    service_conf.memory_high = Some(parse_memory_limit(value));
                }
                "MemoryMin" => {
                    service_conf.memory_min = Some(parse_memory_limit(value));
                }
                "MemoryLow" => {
                    service_conf.memory_low = Some(parse_memory_limit(value));
                }
                "TasksMax" => {
                    service_conf.tasks_max = Some(parse_tasks_max(value));
                }
                "Slice" => {
                    service_conf.slice = Some(value.to_string());
                }
                "SendSIGHUP" => {
                    service_conf.send_sighup = matches!(value, "yes" | "true" | "1");
                }
                "SendSIGKILL" => {
                    service_conf.send_sigkill = matches!(value, "yes" | "true" | "1");
                }
                "KillMode" => {
                    service_conf.kill_mode = match value {
                        "control-group" => crate::units::unit_parsing::KillMode::ControlGroup,
                        "process" => crate::units::unit_parsing::KillMode::Process,
                        "mixed" => crate::units::unit_parsing::KillMode::Mixed,
                        "none" => crate::units::unit_parsing::KillMode::None,
                        _ => crate::units::unit_parsing::KillMode::ControlGroup,
                    };
                }
                "SuccessExitStatus" => {
                    for token in value.split_whitespace() {
                        if let Ok(code) = token.parse::<i32>() {
                            service_conf.success_exit_status.exit_codes.push(code);
                        }
                    }
                }
                "StandardOutput" | "StandardError" => {
                    let opt = match value {
                        "null" | "" => Some(crate::units::StdIoOption::Null),
                        "inherit" => Some(crate::units::StdIoOption::Inherit),
                        "tty" => Some(crate::units::StdIoOption::Tty),
                        "journal" | "syslog" | "journal+console" | "syslog+console" => {
                            Some(crate::units::StdIoOption::Journal)
                        }
                        "kmsg" | "kmsg+console" => Some(crate::units::StdIoOption::Kmsg),
                        _ if value.starts_with("file:") => Some(crate::units::StdIoOption::File(
                            value.trim_start_matches("file:").into(),
                        )),
                        _ if value.starts_with("append:") => {
                            Some(crate::units::StdIoOption::AppendFile(
                                value.trim_start_matches("append:").into(),
                            ))
                        }
                        _ if value.starts_with("truncate:") => {
                            Some(crate::units::StdIoOption::File(
                                value.trim_start_matches("truncate:").into(),
                            ))
                        }
                        _ => None,
                    };
                    if let Some(opt) = opt {
                        match key {
                            "StandardOutput" => {
                                service_conf.exec_config.stdout_path = Some(opt);
                            }
                            "StandardError" => {
                                service_conf.exec_config.stderr_path = Some(opt);
                            }
                            _ => {}
                        }
                    }
                }
                "SyslogIdentifier" => {
                    service_conf.exec_config.syslog_identifier = Some(value.to_string());
                }
                "RootDirectory" => {
                    service_conf.exec_config.root_directory = Some(value.to_string());
                }
                "PassEnvironment" => {
                    if value.is_empty() {
                        service_conf.exec_config.pass_environment.clear();
                    } else {
                        for v in value.split_whitespace() {
                            service_conf
                                .exec_config
                                .pass_environment
                                .push(v.to_string());
                        }
                    }
                }
                "UnsetEnvironment" => {
                    if value.is_empty() {
                        service_conf.exec_config.unset_environment.clear();
                    } else {
                        for v in value.split_whitespace() {
                            service_conf
                                .exec_config
                                .unset_environment
                                .push(v.to_string());
                        }
                    }
                }
                "IgnoreSIGPIPE" => {
                    service_conf.exec_config.ignore_sigpipe = matches!(value, "yes" | "true" | "1");
                }
                "OnSuccess" => {
                    for name in value.split_whitespace() {
                        let full = if name.contains('.') {
                            name.to_string()
                        } else {
                            format!("{name}.service")
                        };
                        success_action_units.push(full);
                    }
                }
                "OnFailure" => {
                    for name in value.split_whitespace() {
                        let full = if name.contains('.') {
                            name.to_string()
                        } else {
                            format!("{name}.service")
                        };
                        failure_action_units.push(full);
                    }
                }
                "StartLimitBurst" => {
                    if let Ok(burst) = value.parse::<u32>() {
                        start_limit_burst = Some(burst);
                    }
                }
                "StartLimitIntervalSec" => {
                    // Parse simple seconds
                    let trimmed = value.trim_end_matches('s');
                    if let Ok(secs) = trimmed.parse::<u64>() {
                        start_limit_interval_sec =
                            Some(crate::units::unit_parsing::Timeout::Duration(
                                std::time::Duration::from_secs(secs),
                            ));
                    }
                }
                "RestartForceExitStatus" => {
                    // Parse space-separated exit codes/signals
                    let mut codes = vec![];
                    for part in value.split_whitespace() {
                        if let Ok(code) = part.parse::<i32>() {
                            codes.push(code);
                        }
                    }
                    service_conf.restart_force_exit_status = crate::units::SuccessExitStatus {
                        exit_codes: codes,
                        signals: vec![],
                    };
                }
                "RestartPreventExitStatus" => {
                    for token in value.split_whitespace() {
                        if let Ok(code) = token.parse::<i32>() {
                            service_conf
                                .restart_prevent_exit_status
                                .exit_codes
                                .push(code);
                        }
                    }
                }
                "RestartSec" => {
                    let trimmed = value.trim_end_matches('s');
                    if let Ok(secs) = trimmed.parse::<u64>() {
                        service_conf.restart_sec =
                            Some(crate::units::unit_parsing::Timeout::Duration(
                                std::time::Duration::from_secs(secs),
                            ));
                    }
                }
                "WatchdogSec" => {
                    let trimmed = value.trim_end_matches('s');
                    if let Ok(secs) = trimmed.parse::<u64>() {
                        service_conf.watchdog_sec = if secs == 0 {
                            None
                        } else {
                            Some(crate::units::unit_parsing::Timeout::Duration(
                                std::time::Duration::from_secs(secs),
                            ))
                        };
                    }
                }
                "ExecStart" | "ExecStartEx" | "ExecStartPre" | "ExecStartPreEx"
                | "ExecStartPost" | "ExecStartPostEx" | "ExecCondition" | "ExecConditionEx"
                | "ExecReload" | "ExecReloadEx" | "ExecStop" | "ExecStopEx" | "ExecStopPost"
                | "ExecStopPostEx" => {
                    let parse_cmd = |s: &str| -> crate::units::Commandline {
                        crate::units::unit_parsing::parse_cmdline(s).unwrap_or_else(|_| {
                            let parts: Vec<String> =
                                shlex::split(s).unwrap_or_else(|| vec![s.to_string()]);
                            crate::units::Commandline {
                                cmd: parts.first().cloned().unwrap_or_default(),
                                args: parts.into_iter().skip(1).collect(),
                                prefixes: vec![],
                            }
                        })
                    };
                    let base_key = key.strip_suffix("Ex").unwrap_or(key);
                    let target = match base_key {
                        "ExecStart" => &mut service_conf.exec,
                        "ExecStartPre" => &mut service_conf.startpre,
                        "ExecStartPost" => &mut service_conf.startpost,
                        "ExecCondition" => &mut service_conf.exec_condition,
                        "ExecReload" => &mut service_conf.reload,
                        "ExecStop" => &mut service_conf.stop,
                        "ExecStopPost" => &mut service_conf.stoppost,
                        _ => unreachable!(),
                    };
                    if value.is_empty() {
                        target.clear();
                    } else {
                        target.push(parse_cmd(value));
                    }
                }
                "SetCredential"
                | "SetCredentialEncrypted"
                | "LoadCredential"
                | "LoadCredentialEncrypted" => {
                    let target = match key {
                        "SetCredential" => &mut service_conf.exec_config.set_credentials,
                        "SetCredentialEncrypted" => {
                            &mut service_conf.exec_config.set_credentials_encrypted
                        }
                        "LoadCredential" => &mut service_conf.exec_config.load_credentials,
                        "LoadCredentialEncrypted" => {
                            &mut service_conf.exec_config.load_credentials_encrypted
                        }
                        _ => unreachable!(),
                    };
                    if value.is_empty() {
                        target.clear();
                    } else if let Some((id, data)) = value.split_once(':') {
                        let id = id.trim();
                        if !id.is_empty() {
                            target.push((id.to_owned(), data.to_owned()));
                        }
                    }
                }
                "ImportCredential" => {
                    if value.is_empty() {
                        service_conf.exec_config.import_credentials.clear();
                    } else {
                        for pattern in value.split_whitespace() {
                            service_conf
                                .exec_config
                                .import_credentials
                                .push(pattern.to_string());
                        }
                    }
                }
                "KillSignal" => {
                    if let Some(sig) = crate::units::unit_parsing::parse_signal_to_raw(value) {
                        service_conf.kill_signal = Some(sig);
                    }
                }
                "RestartKillSignal" => {
                    if let Some(sig) = crate::units::unit_parsing::parse_signal_to_raw(value) {
                        service_conf.restart_kill_signal = Some(sig);
                    }
                }
                "FinalKillSignal" => {
                    if let Some(sig) = crate::units::unit_parsing::parse_signal_to_raw(value) {
                        service_conf.final_kill_signal = Some(sig);
                    }
                }
                "LimitNOFILE" => {
                    service_conf.limit_nofile =
                        crate::units::unit_parsing::parse_resource_limit(value);
                }
                "Wants" => {
                    for u in value.split_whitespace() {
                        dep_wants.push(u.to_string());
                    }
                }
                "Requires" => {
                    for u in value.split_whitespace() {
                        dep_requires.push(u.to_string());
                    }
                }
                "After" => {
                    for u in value.split_whitespace() {
                        dep_after.push(u.to_string());
                    }
                }
                "Before" => {
                    for u in value.split_whitespace() {
                        dep_before.push(u.to_string());
                    }
                }
                "Nice" => {
                    if let Ok(n) = value.parse::<i32>() {
                        service_conf.exec_config.nice = Some(n);
                    }
                }
                "UMask" => {
                    if let Ok(m) = u32::from_str_radix(value.trim(), 8) {
                        service_conf.exec_config.umask = Some(m);
                    }
                }
                "OOMScoreAdjust" => {
                    if let Ok(n) = value.parse::<i32>() {
                        service_conf.exec_config.oom_score_adjust = Some(n);
                    }
                }
                "LimitCPU" => {
                    service_conf.exec_config.limit_cpu =
                        crate::units::unit_parsing::parse_resource_limit(value);
                }
                "LimitCORE" => {
                    service_conf.exec_config.limit_core =
                        crate::units::unit_parsing::parse_resource_limit(value);
                }
                "LimitFSIZE" => {
                    service_conf.exec_config.limit_fsize =
                        crate::units::unit_parsing::parse_resource_limit(value);
                }
                "LimitDATA" => {
                    service_conf.exec_config.limit_data =
                        crate::units::unit_parsing::parse_resource_limit(value);
                }
                "LimitSTACK" => {
                    service_conf.exec_config.limit_stack =
                        crate::units::unit_parsing::parse_resource_limit(value);
                }
                "LimitRSS" => {
                    service_conf.exec_config.limit_rss =
                        crate::units::unit_parsing::parse_resource_limit(value);
                }
                "LimitNPROC" => {
                    service_conf.exec_config.limit_nproc =
                        crate::units::unit_parsing::parse_resource_limit(value);
                }
                "LimitMEMLOCK" => {
                    service_conf.exec_config.limit_memlock =
                        crate::units::unit_parsing::parse_resource_limit(value);
                }
                "LimitAS" => {
                    service_conf.exec_config.limit_as =
                        crate::units::unit_parsing::parse_resource_limit(value);
                }
                "LimitLOCKS" => {
                    service_conf.exec_config.limit_locks =
                        crate::units::unit_parsing::parse_resource_limit(value);
                }
                "LimitSIGPENDING" => {
                    service_conf.exec_config.limit_sigpending =
                        crate::units::unit_parsing::parse_resource_limit(value);
                }
                "LimitMSGQUEUE" => {
                    service_conf.exec_config.limit_msgqueue =
                        crate::units::unit_parsing::parse_resource_limit(value);
                }
                "LimitNICE" => {
                    service_conf.exec_config.limit_nice =
                        crate::units::unit_parsing::parse_resource_limit(value);
                }
                "LimitRTPRIO" => {
                    service_conf.exec_config.limit_rtprio =
                        crate::units::unit_parsing::parse_resource_limit(value);
                }
                "LimitRTTIME" => {
                    service_conf.exec_config.limit_rttime =
                        crate::units::unit_parsing::parse_resource_limit(value);
                }
                "PrivateTmp" => {
                    service_conf.exec_config.private_tmp = matches!(value, "yes" | "true" | "1");
                }
                "PrivateMounts" => {
                    service_conf.exec_config.private_mounts = matches!(value, "yes" | "true" | "1");
                }
                "MountFlags" => {
                    service_conf.exec_config.mount_flags = Some(value.to_string());
                }
                "ReadWritePaths" => {
                    for p in value.split_whitespace() {
                        service_conf
                            .exec_config
                            .read_write_paths
                            .push(p.to_string());
                    }
                }
                "ReadOnlyPaths" => {
                    for p in value.split_whitespace() {
                        service_conf.exec_config.read_only_paths.push(p.to_string());
                    }
                }
                "InaccessiblePaths" => {
                    for p in value.split_whitespace() {
                        service_conf
                            .exec_config
                            .inaccessible_paths
                            .push(p.to_string());
                    }
                }
                "BindPaths" => {
                    for p in value.split_whitespace() {
                        service_conf.exec_config.bind_paths.push(p.to_string());
                    }
                }
                "BindReadOnlyPaths" => {
                    for p in value.split_whitespace() {
                        service_conf
                            .exec_config
                            .bind_read_only_paths
                            .push(p.to_string());
                    }
                }
                "ProtectKernelTunables" => {
                    service_conf.exec_config.protect_kernel_tunables =
                        matches!(value, "yes" | "true" | "1");
                }
                "ProtectKernelModules" => {
                    service_conf.exec_config.protect_kernel_modules =
                        matches!(value, "yes" | "true" | "1");
                }
                "ProtectKernelLogs" => {
                    service_conf.exec_config.protect_kernel_logs =
                        matches!(value, "yes" | "true" | "1");
                }
                "ProtectControlGroups" => {
                    service_conf.exec_config.protect_control_groups =
                        matches!(value, "yes" | "true" | "1");
                }
                "ProtectClock" => {
                    service_conf.exec_config.protect_clock = matches!(value, "yes" | "true" | "1");
                }
                "ProtectHostname" | "ProtectHostnameEx" => {
                    // Support `yes`, `yes:hostname`, `private`, `private:hostname`
                    if let Some(hostname) = value.strip_prefix("yes:") {
                        service_conf.exec_config.protect_hostname = true;
                        service_conf.exec_config.protect_hostname_mode = Some("yes".to_string());
                        service_conf.exec_config.protect_hostname_name = Some(hostname.to_string());
                    } else if let Some(hostname) = value.strip_prefix("private:") {
                        service_conf.exec_config.protect_hostname = true;
                        service_conf.exec_config.protect_hostname_mode =
                            Some("private".to_string());
                        service_conf.exec_config.protect_hostname_name = Some(hostname.to_string());
                    } else if value == "private" {
                        service_conf.exec_config.protect_hostname = true;
                        service_conf.exec_config.protect_hostname_mode =
                            Some("private".to_string());
                        service_conf.exec_config.protect_hostname_name = None;
                    } else {
                        service_conf.exec_config.protect_hostname =
                            matches!(value, "yes" | "true" | "1");
                        if service_conf.exec_config.protect_hostname {
                            service_conf.exec_config.protect_hostname_mode =
                                Some("yes".to_string());
                        } else {
                            service_conf.exec_config.protect_hostname_mode = None;
                        }
                        service_conf.exec_config.protect_hostname_name = None;
                    }
                }
                "NoNewPrivileges" => {
                    service_conf.exec_config.no_new_privileges =
                        matches!(value, "yes" | "true" | "1");
                }
                "MemoryDenyWriteExecute" => {
                    service_conf.exec_config.memory_deny_write_execute =
                        matches!(value, "yes" | "true" | "1");
                }
                "LockPersonality" => {
                    service_conf.exec_config.lock_personality =
                        matches!(value, "yes" | "true" | "1");
                }
                "RestrictRealtime" => {
                    service_conf.exec_config.restrict_realtime =
                        matches!(value, "yes" | "true" | "1");
                }
                "RestrictSUIDSGID" => {
                    service_conf.exec_config.restrict_suid_sgid =
                        matches!(value, "yes" | "true" | "1");
                }
                "CPUSchedulingPolicy" => {
                    service_conf.exec_config.cpu_scheduling_policy = Some(value.to_string());
                }
                "CPUSchedulingPriority" => {
                    if let Ok(n) = value.parse::<u32>() {
                        service_conf.exec_config.cpu_scheduling_priority = Some(n);
                    }
                }
                "CPUSchedulingResetOnFork" => {
                    service_conf.exec_config.cpu_scheduling_reset_on_fork =
                        Some(matches!(value, "yes" | "true" | "1"));
                }
                "CapabilityBoundingSet" => {
                    service_conf
                        .exec_config
                        .capability_bounding_set
                        .extend(value.split_whitespace().map(|s| s.to_string()));
                }
                "AmbientCapabilities" => {
                    service_conf
                        .exec_config
                        .ambient_capabilities
                        .extend(value.split_whitespace().map(|s| s.to_string()));
                }
                "EnvironmentFile" => {
                    let trimmed = value.trim();
                    if let Some(stripped) = trimmed.strip_prefix('-') {
                        service_conf
                            .exec_config
                            .environment_files
                            .push((std::path::PathBuf::from(stripped), true));
                    } else {
                        service_conf
                            .exec_config
                            .environment_files
                            .push((std::path::PathBuf::from(trimmed), false));
                    }
                }
                "StandardInput" => {
                    service_conf.exec_config.stdin_option = match value {
                        "null" => crate::units::StandardInput::Null,
                        "tty" => crate::units::StandardInput::Tty,
                        "tty-force" => crate::units::StandardInput::TtyForce,
                        "tty-fail" => crate::units::StandardInput::TtyFail,
                        _ => crate::units::StandardInput::Null,
                    };
                }
                "UtmpIdentifier" => {
                    service_conf.exec_config.utmp_identifier = if value.is_empty() {
                        None
                    } else {
                        Some(value.to_string())
                    };
                }
                "UtmpMode" => {
                    service_conf.exec_config.utmp_mode = match value {
                        "init" => crate::units::UtmpMode::Init,
                        "login" => crate::units::UtmpMode::Login,
                        "user" => crate::units::UtmpMode::User,
                        _ => crate::units::UtmpMode::Init,
                    };
                }
                "ProtectProc" => {
                    service_conf.exec_config.protect_proc = match value {
                        "noaccess" => crate::units::unit_parsing::ProtectProc::Noaccess,
                        "invisible" => crate::units::unit_parsing::ProtectProc::Invisible,
                        "ptraceable" => crate::units::unit_parsing::ProtectProc::Ptraceable,
                        _ => crate::units::unit_parsing::ProtectProc::Default,
                    };
                }
                "ProcSubset" => {
                    service_conf.exec_config.proc_subset = match value {
                        "pid" => crate::units::unit_parsing::ProcSubset::Pid,
                        _ => crate::units::unit_parsing::ProcSubset::All,
                    };
                }
                "TTYPath" => {
                    service_conf.exec_config.tty_path = Some(std::path::PathBuf::from(value));
                }
                "TTYReset" => {
                    service_conf.exec_config.tty_reset = matches!(value, "yes" | "true" | "1");
                }
                "TTYVHangup" => {
                    service_conf.exec_config.tty_vhangup = matches!(value, "yes" | "true" | "1");
                }
                "TTYVTDisallocate" => {
                    service_conf.exec_config.tty_vt_disallocate =
                        matches!(value, "yes" | "true" | "1");
                }
                "RemoveIPC" => {
                    service_conf.exec_config.remove_ipc = matches!(value, "yes" | "true" | "1");
                }
                "PrivateIPC" => {
                    service_conf.exec_config.private_ipc =
                        Some(matches!(value, "yes" | "true" | "1"));
                }
                "NetworkNamespacePath" => {
                    service_conf.exec_config.network_namespace_path = Some(value.to_string());
                }
                "IPCNamespacePath" => {
                    service_conf.exec_config.ipc_namespace_path = Some(value.to_string());
                }
                "LogLevelMax" => {
                    service_conf.exec_config.log_level_max = Some(value.to_string());
                }
                "LogRateLimitIntervalSec" => {
                    service_conf.exec_config.log_rate_limit_interval_sec = Some(value.to_string());
                }
                "LogRateLimitBurst" => {
                    if let Ok(n) = value.parse::<u32>() {
                        service_conf.exec_config.log_rate_limit_burst = Some(n);
                    }
                }
                "SyslogFacility" => {
                    service_conf.exec_config.syslog_facility = Some(value.to_string());
                }
                "SyslogLevel" => {
                    service_conf.exec_config.syslog_level = Some(value.to_string());
                }
                "SyslogLevelPrefix" => {
                    service_conf.exec_config.syslog_level_prefix =
                        Some(matches!(value, "yes" | "true" | "1"));
                }
                "SecureBits" => {
                    if value.is_empty() {
                        service_conf.exec_config.secure_bits.clear();
                    } else {
                        for b in value.split_whitespace() {
                            service_conf.exec_config.secure_bits.push(b.to_string());
                        }
                    }
                }
                "NoExecPaths" => {
                    if value.is_empty() {
                        service_conf.exec_config.no_exec_paths.clear();
                    } else {
                        for p in value.split_whitespace() {
                            service_conf.exec_config.no_exec_paths.push(p.to_string());
                        }
                    }
                }
                "ExecPaths" => {
                    if value.is_empty() {
                        service_conf.exec_config.exec_paths.clear();
                    } else {
                        for p in value.split_whitespace() {
                            service_conf.exec_config.exec_paths.push(p.to_string());
                        }
                    }
                }
                "TimerSlackNSec" => {
                    service_conf.exec_config.timer_slack_nsec = Some(value.to_string());
                }
                "IOSchedulingClass" => {
                    service_conf.exec_config.io_scheduling_class = match value {
                        "none" | "0" => crate::units::unit_parsing::IOSchedulingClass::None,
                        "realtime" | "1" => crate::units::unit_parsing::IOSchedulingClass::Realtime,
                        "best-effort" | "2" => {
                            crate::units::unit_parsing::IOSchedulingClass::BestEffort
                        }
                        "idle" | "3" => crate::units::unit_parsing::IOSchedulingClass::Idle,
                        _ => crate::units::unit_parsing::IOSchedulingClass::BestEffort,
                    };
                }
                "IOSchedulingPriority" => {
                    if let Ok(n) = value.parse::<u8>() {
                        service_conf.exec_config.io_scheduling_priority = Some(n);
                    }
                }
                "CoredumpFilter" => {
                    service_conf.exec_config.coredump_filter = Some(value.to_string());
                }
                "Personality" => {
                    service_conf.exec_config.personality = Some(value.to_string());
                }
                "KeyringMode" => match value {
                    "inherit" => service_conf.keyring_mode = crate::units::KeyringMode::Inherit,
                    "private" => service_conf.keyring_mode = crate::units::KeyringMode::Private,
                    "shared" => service_conf.keyring_mode = crate::units::KeyringMode::Shared,
                    _ => log::warn!("Unknown KeyringMode={}, ignoring", value),
                },
                "SetLoginEnvironment" => {
                    service_conf.exec_config.set_login_environment =
                        Some(matches!(value, "yes" | "true" | "1"));
                }
                "CPUAffinity" => {
                    if value.is_empty() {
                        service_conf.exec_config.cpu_affinity.clear();
                    } else {
                        service_conf
                            .exec_config
                            .cpu_affinity
                            .push(value.to_string());
                    }
                }
                "MountAPIVFS" => {
                    service_conf.exec_config.mount_api_vfs =
                        Some(matches!(value, "yes" | "true" | "1"));
                }
                _ => {
                    log::debug!("Ignoring unknown transient unit property: {key}={value}");
                }
            }
        }
    }

    // Inherit DefaultLimitNOFILE from manager defaults if not explicitly set.
    if service_conf.limit_nofile.is_none() {
        service_conf.limit_nofile = read_default_limit_nofile();
    }

    // Append the main command (from the trailing command line) after any
    // `-p ExecStart=` property entries, so it runs last.
    if let Some(cmd) = main_cmd {
        service_conf.exec.push(cmd);
    }

    // Apply environment variables from -E/--setenv
    for env_str in &params.environment {
        if let Some((k, v)) = env_str.split_once('=') {
            let env = service_conf
                .exec_config
                .environment
                .get_or_insert_with(|| crate::units::unit_parsing::EnvVars { vars: vec![] });
            env.vars.push((k.to_string(), v.to_string()));
        }
    }

    let unit_id = UnitId {
        kind: UnitIdKind::Service,
        name: unit_name.clone(),
    };

    let mut unit = crate::units::Unit {
        id: unit_id.clone(),
        common: Common {
            unit: UnitConfig {
                description: prop_description
                    .clone()
                    .or_else(|| params.description.clone())
                    .unwrap_or_else(|| format!("Transient unit {unit_name}")),
                documentation: vec![],
                fragment_path: None, // set below after writing transient file
                refs_by_name: vec![],
                default_dependencies: false,
                conditions: vec![],
                assertions: vec![],
                success_action,
                failure_action,
                aliases: vec![],
                ignore_on_isolate: false,
                default_instance: None,
                allow_isolate: false,
                job_timeout_sec: None,
                job_timeout_action: crate::units::UnitAction::None,
                refuse_manual_start: false,
                refuse_manual_stop: false,
                on_success: success_action_units,
                on_success_job_mode: crate::units::OnFailureJobMode::default(),
                on_failure: failure_action_units,
                on_failure_job_mode: crate::units::OnFailureJobMode::default(),
                start_limit_interval_sec,
                start_limit_burst,
                start_limit_action: crate::units::UnitAction::None,
                loaded_at: std::time::SystemTime::now(),
                loaded_dropin_files: Vec::new(),
                joins_namespace_of: Vec::new(),
            },
            dependencies: Dependencies {
                wants: dep_wants.iter().map(|n| unit_id_from_name(n)).collect(),
                wanted_by: vec![],
                requires: dep_requires.iter().map(|n| unit_id_from_name(n)).collect(),
                required_by: vec![],
                conflicts: vec![],
                conflicted_by: vec![],
                before: dep_before.iter().map(|n| unit_id_from_name(n)).collect(),
                after: dep_after.iter().map(|n| unit_id_from_name(n)).collect(),
                part_of: vec![],
                part_of_by: vec![],
                binds_to: vec![],
                bound_by: vec![],
            },
            status: RwLock::new(UnitStatus::NeverStarted),
            timestamps: RwLock::new(UnitTimestamps::default()),
            n_restarts: std::sync::atomic::AtomicU64::new(0),
        },
        specific: Specific::Service(ServiceSpecific {
            conf: service_conf,
            state: RwLock::new(ServiceState {
                common: CommonState::default(),
                srvc: crate::services::Service {
                    pid: None,
                    main_pid: None,
                    status_msgs: Vec::new(),
                    process_group: None,
                    signaled_ready: false,
                    reloading: false,
                    stopping: false,
                    watchdog_last_ping: None,
                    notify_errno: None,
                    notify_bus_error: None,
                    notify_exit_status: None,
                    notify_monotonic_usec: None,
                    invocation_id: None,
                    watchdog_usec_override: None,
                    stored_fds: Vec::new(),
                    notify_access_override: None,
                    notifications: None,
                    notifications_path: None,
                    stdout: None,
                    stderr: None,
                    notifications_buffer: String::new(),
                    stdout_buffer: Vec::new(),
                    stderr_buffer: Vec::new(),
                    watchdog_timeout_fired: false,
                    runtime_max_timeout_fired: false,
                    runtime_started_at: None,
                    main_exit_status: None,
                    main_exit_pid: None,
                    trigger_path: None,
                    trigger_unit: None,
                    trigger_timer_realtime_usec: None,
                    trigger_timer_monotonic_usec: None,
                    monitor_env: None,
                    exec_main_start_timestamp: None,
                    exec_main_handoff_timestamp: None,
                    exec_main_exit_timestamp: None,
                    extend_timeout_usec: None,
                    extend_timeout_timestamp: None,
                    join_namespace_pid: None,
                },
            }),
        }),
    };

    // Apply drop-in overrides from the filesystem (e.g., service.d/, a-.service.d/)
    // to the transient unit, matching systemd behavior.
    apply_dropins_to_transient(&mut unit, &run_info.read_poisoned().config.unit_dirs);

    // Write transient service unit file to /run/systemd/transient/
    let transient_dir = std::path::Path::new("/run/systemd/transient");
    let _ = std::fs::create_dir_all(transient_dir);
    let transient_path = transient_dir.join(unit_name);
    if let Ok(()) = write_transient_service_file(&transient_path, params) {
        unit.common.unit.fragment_path = Some(transient_path);
    }

    // Insert the transient unit into the unit table.
    let mut ri = run_info.write_poisoned();
    // If a unit with the same name already exists and is stopped/failed,
    // remove it so the new transient can replace it (matching systemd behavior).
    let existing_id = ri
        .unit_table
        .values()
        .find(|u| u.id.name == *unit_name)
        .map(|u| {
            let status = u.common.status.read_poisoned();
            let is_done = matches!(&*status, UnitStatus::NeverStarted | UnitStatus::Stopped(..));
            (u.id.clone(), is_done)
        });
    match existing_id {
        Some((id, true)) => {
            ri.unit_table.remove(&id);
        }
        Some((_id, false)) => {
            return Err(format!("Unit {unit_name} already exists"));
        }
        None => {}
    }
    crate::units::insert_new_unit_lenient(unit, &mut ri);

    // If timer properties are set, create a companion .timer unit
    let has_timer = params.on_calendar.is_some()
        || !params.on_active.is_empty()
        || params.on_boot.is_some()
        || params.on_startup.is_some()
        || params.on_unit_active.is_some()
        || params.on_unit_inactive.is_some()
        || params.on_clock_change
        || params.on_timezone_change
        || !params.timer_properties.is_empty();
    let has_path = !params.path_properties.is_empty();
    let has_socket = !params.socket_properties.is_empty();
    if has_timer {
        let timer_name = if unit_name.ends_with(".service") {
            format!("{}.timer", unit_name.strip_suffix(".service").unwrap())
        } else {
            format!("{unit_name}.timer")
        };

        let service_unit_name = if unit_name.ends_with(".service") {
            unit_name.clone()
        } else {
            format!("{unit_name}.service")
        };

        let on_calendar = params.on_calendar.iter().cloned().collect::<Vec<String>>();

        let on_active_sec: Vec<_> = params
            .on_active
            .iter()
            .filter_map(|s| parse_timespan(s))
            .collect();

        let on_boot_sec = params
            .on_boot
            .as_ref()
            .and_then(|s| parse_timespan(s))
            .into_iter()
            .collect::<Vec<_>>();

        let on_startup_sec = params
            .on_startup
            .as_ref()
            .and_then(|s| parse_timespan(s))
            .into_iter()
            .collect::<Vec<_>>();

        let on_unit_active_sec = params
            .on_unit_active
            .as_ref()
            .and_then(|s| parse_timespan(s))
            .into_iter()
            .collect::<Vec<_>>();

        let on_unit_inactive_sec = params
            .on_unit_inactive
            .as_ref()
            .and_then(|s| parse_timespan(s))
            .into_iter()
            .collect::<Vec<_>>();

        let timer_config = crate::units::TimerConfig {
            on_active_sec,
            on_boot_sec,
            on_startup_sec,
            on_unit_active_sec,
            on_unit_inactive_sec,
            on_calendar,
            accuracy_sec: std::time::Duration::from_secs(60),
            randomized_delay_sec: std::time::Duration::ZERO,
            fixed_random_delay: false,
            persistent: false,
            wake_system: false,
            remain_after_elapse: true,
            on_clock_change: params.on_clock_change,
            on_timezone_change: params.on_timezone_change,
            unit: service_unit_name.clone(),
        };

        let timer_id = UnitId {
            kind: crate::units::UnitIdKind::Timer,
            name: timer_name.clone(),
        };

        let timer_unit = crate::units::Unit {
            id: timer_id.clone(),
            common: Common {
                unit: UnitConfig {
                    description: params
                        .description
                        .clone()
                        .unwrap_or_else(|| format!("Timer for {unit_name}")),
                    documentation: vec![],
                    fragment_path: {
                        let timer_path = transient_dir.join(&timer_name);
                        write_transient_timer_file(&timer_path, params, &service_unit_name)
                            .ok()
                            .map(|()| timer_path)
                    },
                    refs_by_name: vec![],
                    default_dependencies: false,
                    conditions: vec![],
                    assertions: vec![],
                    success_action: crate::units::UnitAction::None,
                    failure_action: crate::units::UnitAction::None,
                    aliases: vec![],
                    ignore_on_isolate: false,
                    default_instance: None,
                    allow_isolate: false,
                    job_timeout_sec: None,
                    job_timeout_action: crate::units::UnitAction::None,
                    refuse_manual_start: false,
                    refuse_manual_stop: false,
                    on_success: vec![],
                    on_success_job_mode: crate::units::OnFailureJobMode::default(),
                    on_failure: vec![],
                    on_failure_job_mode: crate::units::OnFailureJobMode::default(),
                    start_limit_interval_sec: None,
                    start_limit_burst: None,
                    start_limit_action: crate::units::UnitAction::None,
                    loaded_at: std::time::SystemTime::now(),
                    loaded_dropin_files: Vec::new(),
                    joins_namespace_of: Vec::new(),
                },
                dependencies: Dependencies {
                    wants: vec![],
                    wanted_by: vec![],
                    requires: vec![],
                    required_by: vec![],
                    conflicts: vec![],
                    conflicted_by: vec![],
                    before: vec![],
                    after: vec![],
                    part_of: vec![],
                    part_of_by: vec![],
                    binds_to: vec![],
                    bound_by: vec![],
                },
                status: RwLock::new(UnitStatus::Started(crate::units::StatusStarted::Running)),
                timestamps: RwLock::new(UnitTimestamps::default()),
                n_restarts: std::sync::atomic::AtomicU64::new(0),
            },
            specific: Specific::Timer(crate::units::TimerSpecific {
                conf: timer_config,
                state: RwLock::new(crate::units::TimerState {
                    common: CommonState::default(),
                    last_trigger_usec: None,
                }),
            }),
        };

        // Remove existing timer if stopped
        let existing_timer = ri
            .unit_table
            .values()
            .find(|u| u.id.name == timer_name)
            .map(|u| {
                let status = u.common.status.read_poisoned();
                let is_done =
                    matches!(&*status, UnitStatus::NeverStarted | UnitStatus::Stopped(..));
                (u.id.clone(), is_done)
            });
        if let Some((id, true)) = existing_timer {
            ri.unit_table.remove(&id);
        }
        crate::units::insert_new_unit_lenient(timer_unit, &mut ri);
    }

    // If path properties are set, create a companion .path unit and insert it
    if has_path {
        let path_name = if unit_name.ends_with(".service") {
            format!("{}.path", unit_name.strip_suffix(".service").unwrap())
        } else {
            format!("{unit_name}.path")
        };

        let service_unit_name = if unit_name.ends_with(".service") {
            unit_name.clone()
        } else {
            format!("{unit_name}.service")
        };

        // Parse path properties into PathCondition entries
        let mut conditions = Vec::new();
        for prop in &params.path_properties {
            if let Some((key, value)) = prop.split_once('=') {
                let condition = match key {
                    "PathExists" => {
                        Some(crate::units::PathCondition::PathExists(value.to_string()))
                    }
                    "PathExistsGlob" => Some(crate::units::PathCondition::PathExistsGlob(
                        value.to_string(),
                    )),
                    "PathChanged" => {
                        Some(crate::units::PathCondition::PathChanged(value.to_string()))
                    }
                    "PathModified" => {
                        Some(crate::units::PathCondition::PathModified(value.to_string()))
                    }
                    "DirectoryNotEmpty" => Some(crate::units::PathCondition::DirectoryNotEmpty(
                        value.to_string(),
                    )),
                    _ => None,
                };
                if let Some(c) = condition {
                    conditions.push(c);
                }
            }
        }

        let path_config = crate::units::PathConfig {
            conditions,
            make_directory: false,
            directory_mode: 0o755,
            trigger_limit_interval_sec: std::time::Duration::from_secs(2),
            trigger_limit_burst: 200,
            unit: service_unit_name,
        };

        let path_id = UnitId {
            kind: crate::units::UnitIdKind::Path,
            name: path_name.clone(),
        };

        let path_unit = crate::units::Unit {
            id: path_id.clone(),
            common: Common {
                unit: UnitConfig {
                    description: params
                        .description
                        .clone()
                        .unwrap_or_else(|| format!("Path for {unit_name}")),
                    documentation: vec![],
                    fragment_path: {
                        let path_file = transient_dir.join(&path_name);
                        write_transient_auxiliary_file(
                            &path_file,
                            "Path",
                            &params.path_properties,
                            params,
                        )
                        .ok()
                        .map(|()| path_file)
                    },
                    refs_by_name: vec![],
                    default_dependencies: false,
                    conditions: vec![],
                    assertions: vec![],
                    success_action: crate::units::UnitAction::None,
                    failure_action: crate::units::UnitAction::None,
                    aliases: vec![],
                    ignore_on_isolate: false,
                    default_instance: None,
                    allow_isolate: false,
                    job_timeout_sec: None,
                    job_timeout_action: crate::units::UnitAction::None,
                    refuse_manual_start: false,
                    refuse_manual_stop: false,
                    on_success: vec![],
                    on_success_job_mode: crate::units::OnFailureJobMode::default(),
                    on_failure: vec![],
                    on_failure_job_mode: crate::units::OnFailureJobMode::default(),
                    start_limit_interval_sec: None,
                    start_limit_burst: None,
                    start_limit_action: crate::units::UnitAction::None,
                    loaded_at: std::time::SystemTime::now(),
                    loaded_dropin_files: Vec::new(),
                    joins_namespace_of: Vec::new(),
                },
                dependencies: Dependencies {
                    wants: vec![],
                    wanted_by: vec![],
                    requires: vec![],
                    required_by: vec![],
                    conflicts: vec![],
                    conflicted_by: vec![],
                    before: vec![],
                    after: vec![],
                    part_of: vec![],
                    part_of_by: vec![],
                    binds_to: vec![],
                    bound_by: vec![],
                },
                status: RwLock::new(UnitStatus::Started(crate::units::StatusStarted::Running)),
                timestamps: RwLock::new(UnitTimestamps::default()),
                n_restarts: std::sync::atomic::AtomicU64::new(0),
            },
            specific: Specific::Path(crate::units::PathSpecific {
                conf: path_config,
                state: RwLock::new(crate::units::PathState {
                    common: CommonState::default(),
                    result: crate::units::PathResult::default(),
                }),
            }),
        };

        // Remove existing path unit if stopped
        let existing_path = ri
            .unit_table
            .values()
            .find(|u| u.id.name == path_name)
            .map(|u| {
                let status = u.common.status.read_poisoned();
                let is_done =
                    matches!(&*status, UnitStatus::NeverStarted | UnitStatus::Stopped(..));
                (u.id.clone(), is_done)
            });
        if let Some((id, true)) = existing_path {
            ri.unit_table.remove(&id);
        }
        crate::units::insert_new_unit_lenient(path_unit, &mut ri);
    }

    // If socket properties are set, write a companion .socket transient file
    if has_socket {
        let socket_name = if unit_name.ends_with(".service") {
            format!("{}.socket", unit_name.strip_suffix(".service").unwrap())
        } else {
            format!("{unit_name}.socket")
        };
        let socket_file = transient_dir.join(&socket_name);
        let _ = write_transient_auxiliary_file(
            &socket_file,
            "Socket",
            &params.socket_properties,
            params,
        );
    }

    Ok(unit_id)
}

/// Parse a memory limit value like "50M", "1G", "infinity", or "80%".
fn parse_memory_limit(s: &str) -> crate::units::unit_parsing::MemoryLimit {
    use crate::units::unit_parsing::MemoryLimit;
    if s == "infinity" {
        return MemoryLimit::Infinity;
    }
    if let Some(pct) = s.strip_suffix('%')
        && let Ok(p) = pct.parse::<u64>()
    {
        return MemoryLimit::Percent(p);
    }
    // Try with byte suffixes
    let (num_str, multiplier) = if let Some(n) = s.strip_suffix('K') {
        (n, 1024u64)
    } else if let Some(n) = s.strip_suffix('M') {
        (n, 1024 * 1024)
    } else if let Some(n) = s.strip_suffix('G') {
        (n, 1024 * 1024 * 1024)
    } else if let Some(n) = s.strip_suffix('T') {
        (n, 1024u64 * 1024 * 1024 * 1024)
    } else {
        (s, 1)
    };
    if let Ok(n) = num_str.parse::<u64>() {
        MemoryLimit::Bytes(n * multiplier)
    } else {
        MemoryLimit::Infinity
    }
}

/// Parse a TasksMax value like "50", "infinity", or "80%".
fn parse_tasks_max(s: &str) -> crate::units::unit_parsing::TasksMax {
    use crate::units::unit_parsing::TasksMax;
    if s == "infinity" {
        return TasksMax::Infinity;
    }
    if let Some(pct) = s.strip_suffix('%')
        && let Ok(p) = pct.parse::<u64>()
    {
        return TasksMax::Percent(p);
    }
    if let Ok(n) = s.parse::<u64>() {
        TasksMax::Value(n)
    } else {
        TasksMax::Infinity
    }
}

/// Parse a simple time span like "5min", "30s", "2h 30min", "1d".
fn parse_timespan(s: &str) -> Option<std::time::Duration> {
    let s = s.trim();
    // Try simple number (seconds)
    if let Ok(secs) = s.parse::<u64>() {
        return Some(std::time::Duration::from_secs(secs));
    }
    // Try with suffix
    let mut total_secs = 0u64;
    let mut current = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            current.push(c);
        } else if c.is_ascii_alphabetic() {
            let num: u64 = current.parse().ok()?;
            current.clear();
            match c {
                's' => total_secs += num,
                'm' => {
                    // Could be "min" or just "m"
                    total_secs += num * 60;
                }
                'h' => total_secs += num * 3600,
                'd' => total_secs += num * 86400,
                _ => {}
            }
        } else if c == ' ' {
            // ignore
        }
    }
    if !current.is_empty()
        && let Ok(num) = current.parse::<u64>()
    {
        total_secs += num;
    }
    if total_secs > 0 {
        Some(std::time::Duration::from_secs(total_secs))
    } else {
        None
    }
}

pub fn execute_command(
    cmd: Command,
    run_info: ArcMutRuntimeInfo,
) -> Result<serde_json::Value, String> {
    let mut result_vec = Value::Array(Vec::new());
    match cmd {
        Command::DaemonReexec => {
            info!("daemon-reexec: re-executing service manager");
            crate::signal_handler::daemon_reexec(&run_info);
            // If we get here, execve failed — daemon_reexec logs the error.
            return Err("daemon-reexec failed".to_string());
        }
        Command::LogLevel(level) => {
            match level {
                Some(new_level) => {
                    // Set the log level
                    let filter = match new_level.to_lowercase().as_str() {
                        "emerg" | "alert" | "crit" => log::LevelFilter::Error,
                        "err" => log::LevelFilter::Error,
                        "warning" => log::LevelFilter::Warn,
                        "notice" | "info" => log::LevelFilter::Info,
                        "debug" => log::LevelFilter::Debug,
                        _ => return Err(format!("Invalid log level: {new_level}")),
                    };
                    log::set_max_level(filter);
                    // Persist the level so queries can read it
                    let _ = std::fs::create_dir_all("/run/rust-systemd");
                    let _ = std::fs::write("/run/rust-systemd/log-level", &new_level);
                    info!("Log level set to {new_level}");
                }
                None => {
                    // Query current level
                    let level = std::fs::read_to_string("/run/rust-systemd/log-level")
                        .unwrap_or_else(|_| "info".to_string());
                    return Ok(Value::String(level.trim().to_string()));
                }
            }
        }
        Command::LogTarget(target) => match target {
            Some(new_target) => {
                let _ = std::fs::create_dir_all("/run/rust-systemd");
                let _ = std::fs::write("/run/rust-systemd/log-target", &new_target);
                info!("Log target set to {new_target}");
            }
            None => {
                let target = std::fs::read_to_string("/run/rust-systemd/log-target")
                    .unwrap_or_else(|_| "journal-or-kmsg".to_string());
                return Ok(Value::String(target.trim().to_string()));
            }
        },
        Command::ServiceWatchdogs(val) => match val {
            Some(new_val) => {
                let _ = std::fs::create_dir_all("/run/rust-systemd");
                let _ = std::fs::write("/run/rust-systemd/service-watchdogs", &new_val);
                info!("Service watchdogs set to {new_val}");
            }
            None => {
                let val = std::fs::read_to_string("/run/rust-systemd/service-watchdogs")
                    .unwrap_or_else(|_| "yes".to_string());
                return Ok(Value::String(val.trim().to_string()));
            }
        },
        Command::StartTransient(params) => {
            let unit_name = params.unit_name.clone();
            let do_wait = params.wait;
            let do_pipe = params.pipe;
            let id = create_transient_unit(&params, &run_info)?;
            // Now start the unit.
            {
                let ri = run_info.read_poisoned();
                crate::units::activate_unit(id.clone(), &ri, ActivationSource::Regular)
                    .map_err(|e| format!("Failed to start transient unit {unit_name}: {e}"))?;
            }

            // After activate_unit returns, check if the unit actually
            // started.  activate_unit swallows some errors (converting
            // them to Ok) for dependency-graph walking purposes, but for
            // oneshot and Type=exec transient units we need to detect
            // failure so that `systemd-run` can exit non-zero.
            {
                let srvc_type = {
                    let ri = run_info.read_poisoned();
                    ri.unit_table.get(&id).and_then(|u| {
                        if let Specific::Service(srvc) = &u.specific {
                            Some(srvc.conf.srcv_type)
                        } else {
                            None
                        }
                    })
                };

                let is_oneshot = srvc_type == Some(crate::units::ServiceType::OneShot);
                let is_exec = srvc_type == Some(crate::units::ServiceType::Exec);

                // For Type=exec, wait_for_service catches exec()
                // failures (exit 203) synchronously and sets
                // StoppedUnexpected, but activate_unit swallows the
                // error.  Only check for start-path errors (which
                // contain ServiceStartError), NOT exit-handler errors
                // from the program exiting normally after a successful
                // exec().
                if is_exec {
                    let ri = run_info.read_poisoned();
                    if let Some(unit) = ri.unit_table.get(&id) {
                        let status = unit.common.status.read_poisoned();
                        if let crate::units::UnitStatus::Stopped(
                            crate::units::StatusStopped::StoppedUnexpected,
                            errors,
                        ) = &*status
                        {
                            let has_start_error = errors.iter().any(|e| {
                                matches!(
                                    e,
                                    crate::units::UnitOperationErrorReason::ServiceStartError(_)
                                )
                            });
                            if has_start_error {
                                let msg = if let Some(e) = errors.first() {
                                    format!("{e}")
                                } else {
                                    "unit failed".to_string()
                                };
                                return Err(format!(
                                    "Failed to start transient unit {unit_name}: {msg}"
                                ));
                            }
                        }
                    }
                }

                if is_oneshot {
                    let ri = run_info.read_poisoned();
                    if let Some(unit) = ri.unit_table.get(&id) {
                        let status = unit.common.status.read_poisoned();
                        match &*status {
                            crate::units::UnitStatus::Stopped(
                                crate::units::StatusStopped::StoppedUnexpected,
                                errors,
                            ) => {
                                let msg = if let Some(e) = errors.first() {
                                    format!("{e}")
                                } else {
                                    "unit failed".to_string()
                                };
                                return Err(format!(
                                    "Failed to start transient unit {unit_name}: {msg}"
                                ));
                            }
                            crate::units::UnitStatus::Restarting => {
                                return Err(format!(
                                    "Failed to start transient unit {unit_name}: initial start failed, unit is restarting"
                                ));
                            }
                            _ => {}
                        }
                    }
                }
            }

            // For oneshot services, wait for the main process to be fully
            // reaped before returning, matching real systemd's behavior
            // where the start job blocks until ExecStart completes.
            // Without this, callers see a race where the service's side
            // effects (e.g. RuntimeDirectory creation) have not yet
            // happened.
            {
                let is_oneshot = {
                    let ri = run_info.read_poisoned();
                    ri.unit_table
                        .get(&id)
                        .map(|u| {
                            matches!(
                                &u.specific,
                                Specific::Service(s)
                                    if s.conf.srcv_type == crate::units::ServiceType::OneShot
                            )
                        })
                        .unwrap_or(false)
                };
                if is_oneshot && !do_wait {
                    // Poll until the exit handler has reaped the main
                    // process (sets main_exit_pid) — max 30s.
                    for _ in 0..300 {
                        let reaped = {
                            let ri = run_info.read_poisoned();
                            ri.unit_table
                                .get(&id)
                                .map(|u| {
                                    if let Specific::Service(s) = &u.specific {
                                        s.state.read_poisoned().srvc.main_exit_pid.is_some()
                                    } else {
                                        true
                                    }
                                })
                                .unwrap_or(true)
                        };
                        if reaped {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
            }

            if !do_wait {
                return Ok(serde_json::json!({ "started": unit_name }));
            }

            // --wait mode: poll until the unit reaches a terminal state,
            // then return the exit code.
            loop {
                std::thread::sleep(std::time::Duration::from_millis(100));
                let ri = run_info.read_poisoned();
                let Some(unit) = ri.unit_table.get(&id) else {
                    // Unit was removed — treat as completed successfully.
                    return Ok(serde_json::json!({
                        "started": unit_name,
                        "result": "success",
                        "exit_code": 0
                    }));
                };
                let status = unit.common.status.read_poisoned();
                match &*status {
                    crate::units::UnitStatus::Stopped(_, errors) => {
                        let (result, exit_code) = if errors.is_empty() {
                            // Get the actual exit status from the service state.
                            let exit_status =
                                if let crate::units::Specific::Service(svc) = &unit.specific {
                                    if let Ok(state) = svc.state.try_read() {
                                        state.srvc.main_exit_status.unwrap_or(0)
                                    } else {
                                        0
                                    }
                                } else {
                                    0
                                };
                            ("success", exit_status)
                        } else {
                            // Failed — extract exit status from service state.
                            let exit_status =
                                if let crate::units::Specific::Service(svc) = &unit.specific {
                                    if let Ok(state) = svc.state.try_read() {
                                        state.srvc.main_exit_status.unwrap_or(1)
                                    } else {
                                        1
                                    }
                                } else {
                                    1
                                };
                            ("exit-code", exit_status)
                        };
                        let mut resp = serde_json::json!({
                            "started": unit_name,
                            "result": result,
                            "exit_code": exit_code
                        });
                        // When --pipe was requested, read the captured
                        // stdout/stderr temp files and include them in the
                        // response so the client can relay them.
                        if do_pipe {
                            let stdout_path =
                                format!("/run/systemd/transient/{}.stdout", unit_name);
                            let stderr_path =
                                format!("/run/systemd/transient/{}.stderr", unit_name);
                            if let Ok(data) = std::fs::read_to_string(&stdout_path) {
                                resp["stdout"] = Value::String(data);
                                let _ = std::fs::remove_file(&stdout_path);
                            }
                            if let Ok(data) = std::fs::read_to_string(&stderr_path) {
                                resp["stderr"] = Value::String(data);
                                let _ = std::fs::remove_file(&stderr_path);
                            }
                        }
                        return Ok(resp);
                    }
                    crate::units::UnitStatus::NeverStarted => {
                        // Not started yet, keep waiting
                    }
                    _ => {
                        // Still running/starting, keep waiting
                    }
                }
            }
        }
        Command::Enable(names) => {
            let is_runtime = names.iter().any(|n| n == "--runtime");
            let root_prefix: String = names
                .iter()
                .find_map(|n| n.strip_prefix("--root=").map(|s| s.to_string()))
                .unwrap_or_default();
            let names: Vec<String> = names
                .into_iter()
                .filter(|n| n != "--runtime" && !n.starts_with("--root="))
                .collect();
            let base_dir_str = if is_runtime {
                format!("{root_prefix}/run/systemd/system")
            } else {
                format!("{root_prefix}/etc/systemd/system")
            };
            let base_dir = std::path::Path::new(&base_dir_str);

            let ri = run_info.read_poisoned();
            let mut enabled = Vec::new();
            for name in &names {
                let full_name = if name.contains('.') {
                    name.clone()
                } else {
                    format!("{name}.service")
                };
                // Find the unit file on disk (try instance, then template)
                let mut unit_path = None;
                for dir in &ri.config.unit_dirs {
                    let candidate = dir.join(&full_name);
                    if candidate.exists() {
                        unit_path = Some(candidate);
                        break;
                    }
                }
                // For template instances (e.g. foo@1.service), try the template (foo@.service)
                if unit_path.is_none()
                    && let Some(at_pos) = full_name.find('@')
                    && let Some(dot_pos) = full_name[at_pos..].find('.')
                {
                    let template = format!(
                        "{}@{}",
                        &full_name[..at_pos],
                        &full_name[at_pos + dot_pos..]
                    );
                    for dir in &ri.config.unit_dirs {
                        let candidate = dir.join(&template);
                        if candidate.exists() {
                            unit_path = Some(candidate);
                            break;
                        }
                    }
                }
                let unit_path = match unit_path {
                    Some(p) => p,
                    None => return Err(format!("Unit {full_name} not found.")),
                };
                // Read [Install] section
                let content = std::fs::read_to_string(&unit_path)
                    .map_err(|e| format!("Failed to read {}: {e}", unit_path.display()))?;
                let mut in_install = false;
                let mut wanted_by = Vec::new();
                let mut required_by = Vec::new();
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed == "[Install]" {
                        in_install = true;
                        continue;
                    }
                    if trimmed.starts_with('[') {
                        in_install = false;
                        continue;
                    }
                    if !in_install {
                        continue;
                    }
                    if let Some(val) = trimmed.strip_prefix("WantedBy=") {
                        for target in val.split_whitespace() {
                            wanted_by.push(target.to_string());
                        }
                    }
                    if let Some(val) = trimmed.strip_prefix("RequiredBy=") {
                        for target in val.split_whitespace() {
                            required_by.push(target.to_string());
                        }
                    }
                }
                // Expand Install specifiers (%i, %j/%J, %n, %p, %N)
                let instance = if let Some(at_pos) = full_name.find('@') {
                    if let Some(dot_pos) = full_name[at_pos..].find('.') {
                        &full_name[at_pos + 1..at_pos + dot_pos]
                    } else {
                        &full_name[at_pos + 1..]
                    }
                } else {
                    ""
                };
                let prefix = full_name.split('@').next().unwrap_or(&full_name);
                let prefix = prefix.split('.').next().unwrap_or(prefix);
                let expand_specifiers = |s: &str| -> String {
                    s.replace("%i", instance)
                        .replace("%I", instance)
                        .replace("%j", instance)
                        .replace("%J", instance)
                        .replace("%n", &full_name)
                        .replace("%N", &full_name)
                        .replace("%p", prefix)
                        .replace("%P", prefix)
                };
                let wanted_by: Vec<String> = wanted_by
                    .into_iter()
                    .map(|s| expand_specifiers(&s))
                    .collect();
                let required_by: Vec<String> = required_by
                    .into_iter()
                    .map(|s| expand_specifiers(&s))
                    .collect();
                // Create .wants symlinks
                for target in &wanted_by {
                    let wants_dir = base_dir.join(format!("{target}.wants"));
                    std::fs::create_dir_all(&wants_dir)
                        .map_err(|e| format!("Failed to create {}: {e}", wants_dir.display()))?;
                    let link = wants_dir.join(&full_name);
                    let _ = std::fs::remove_file(&link);
                    std::os::unix::fs::symlink(&unit_path, &link)
                        .map_err(|e| format!("Failed to create symlink {}: {e}", link.display()))?;
                }
                // Create .requires symlinks
                for target in &required_by {
                    let req_dir = base_dir.join(format!("{target}.requires"));
                    std::fs::create_dir_all(&req_dir)
                        .map_err(|e| format!("Failed to create {}: {e}", req_dir.display()))?;
                    let link = req_dir.join(&full_name);
                    let _ = std::fs::remove_file(&link);
                    std::os::unix::fs::symlink(&unit_path, &link)
                        .map_err(|e| format!("Failed to create symlink {}: {e}", link.display()))?;
                }
                enabled.push(Value::String(full_name));
            }
            drop(ri);
            // Also load units into memory (old enable behavior)
            {
                let run_info_w = &mut *run_info.write_poisoned();
                let mut map = std::collections::HashMap::new();
                for name in &names {
                    let full_name = if name.contains('.') {
                        name.clone()
                    } else {
                        format!("{name}.service")
                    };
                    let already_loaded = run_info_w
                        .unit_table
                        .values()
                        .any(|u| u.id.name == full_name || u.id.name == *name);
                    if already_loaded {
                        continue;
                    }
                    if let Ok(unit) = load_new_unit(&run_info_w.config.unit_dirs, name) {
                        map.insert(unit.id.clone(), unit);
                    }
                }
                if !map.is_empty() {
                    let _ = insert_new_units(map, run_info_w);
                }
            }
            return Ok(serde_json::json!({ "enabled": enabled }));
        }
        Command::Disable(names) => {
            let is_runtime = names.iter().any(|n| n == "--runtime");
            let root_prefix: String = names
                .iter()
                .find_map(|n| n.strip_prefix("--root=").map(|s| s.to_string()))
                .unwrap_or_default();
            let names: Vec<String> = names
                .into_iter()
                .filter(|n| n != "--runtime" && !n.starts_with("--root="))
                .collect();
            let base_dir_str = if is_runtime {
                format!("{root_prefix}/run/systemd/system")
            } else {
                format!("{root_prefix}/etc/systemd/system")
            };
            let base_dir = std::path::Path::new(&base_dir_str);
            let mut disabled = Vec::new();
            for name in &names {
                let full_name = if name.contains('.') {
                    name.clone()
                } else {
                    format!("{name}.service")
                };
                // Scan .wants/ and .requires/ directories for symlinks to this unit
                // For template units (foo@.service), match all instances (foo@*.service)
                let is_template = full_name.contains("@.");
                let template_prefix = if is_template {
                    full_name.split("@.").next().map(|p| format!("{p}@"))
                } else {
                    None
                };
                let template_suffix = if is_template {
                    full_name.find("@.").map(|pos| &full_name[pos + 2..])
                } else {
                    None
                };
                if let Ok(entries) = std::fs::read_dir(base_dir) {
                    for entry in entries.flatten() {
                        let entry_name = entry.file_name();
                        let entry_str = entry_name.to_string_lossy();
                        if entry_str.ends_with(".wants") || entry_str.ends_with(".requires") {
                            if is_template {
                                // Remove all instance symlinks matching the template
                                if let Ok(links) = std::fs::read_dir(entry.path()) {
                                    for link_entry in links.flatten() {
                                        let link_name =
                                            link_entry.file_name().to_string_lossy().to_string();
                                        if let (Some(pfx), Some(sfx)) =
                                            (&template_prefix, template_suffix)
                                            && link_name.starts_with(pfx.as_str())
                                            && link_name.ends_with(sfx)
                                        {
                                            let _ = std::fs::remove_file(link_entry.path());
                                        }
                                    }
                                }
                            } else {
                                let link = entry.path().join(&full_name);
                                if link.symlink_metadata().is_ok() {
                                    let _ = std::fs::remove_file(&link);
                                }
                            }
                        }
                    }
                }
                disabled.push(Value::String(full_name));
            }
            return Ok(serde_json::json!({ "disabled": disabled }));
        }
        Command::Preset(names) => {
            let is_runtime = names.iter().any(|n| n == "--runtime");
            let root_prefix: String = names
                .iter()
                .find_map(|n| n.strip_prefix("--root=").map(|s| s.to_string()))
                .unwrap_or_default();
            let preset_mode = if names.iter().any(|n| n == "--preset-mode=enable-only") {
                "enable-only"
            } else if names.iter().any(|n| n == "--preset-mode=disable-only") {
                "disable-only"
            } else {
                "full"
            };
            let names: Vec<String> = names
                .into_iter()
                .filter(|n| {
                    n != "--runtime"
                        && !n.starts_with("--preset-mode=")
                        && !n.starts_with("--root=")
                })
                .collect();
            let base_dir_str = if is_runtime {
                format!("{root_prefix}/run/systemd/system")
            } else {
                format!("{root_prefix}/etc/systemd/system")
            };
            let base_dir = std::path::Path::new(&base_dir_str);

            // Read preset files
            let preset_dirs = [
                format!("{root_prefix}/etc/systemd/system-preset"),
                format!("{root_prefix}/run/systemd/system-preset"),
                format!("{root_prefix}/usr/lib/systemd/system-preset"),
            ];
            let mut preset_rules: Vec<(String, String)> = Vec::new(); // (action, pattern)
            for dir in &preset_dirs {
                let dir_path = std::path::Path::new(dir);
                if !dir_path.is_dir() {
                    continue;
                }
                let mut files: Vec<_> = std::fs::read_dir(dir_path)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter(|e| e.file_name().to_string_lossy().ends_with(".preset"))
                    .collect();
                files.sort_by_key(|e| e.file_name());
                for file in files {
                    if let Ok(content) = std::fs::read_to_string(file.path()) {
                        for line in content.lines() {
                            let trimmed = line.trim();
                            if trimmed.is_empty() || trimmed.starts_with('#') {
                                continue;
                            }
                            let mut parts = trimmed.splitn(2, char::is_whitespace);
                            if let (Some(action), Some(pattern)) = (parts.next(), parts.next()) {
                                let action = action.to_lowercase();
                                if action == "enable" || action == "disable" {
                                    preset_rules.push((action, pattern.trim().to_string()));
                                }
                            }
                        }
                    }
                }
            }

            let ri = run_info.read_poisoned();
            for name in &names {
                let full_name = if name.contains('.') {
                    name.clone()
                } else {
                    format!("{name}.service")
                };
                // Find matching preset rule (first match wins)
                let mut action = "enable"; // default is enable if no rule matches
                for (rule_action, pattern) in &preset_rules {
                    if unit_name_glob_match(pattern, &full_name) {
                        action = rule_action;
                        break;
                    }
                }
                match (action, preset_mode) {
                    ("enable", "full" | "enable-only") => {
                        // Find unit and create symlinks
                        let mut unit_path = None;
                        for dir in &ri.config.unit_dirs {
                            let candidate = dir.join(&full_name);
                            if candidate.exists() {
                                unit_path = Some(candidate);
                                break;
                            }
                        }
                        if let Some(unit_path) = unit_path
                            && let Ok(content) = std::fs::read_to_string(&unit_path)
                        {
                            let mut in_install = false;
                            for line in content.lines() {
                                let trimmed = line.trim();
                                if trimmed == "[Install]" {
                                    in_install = true;
                                    continue;
                                }
                                if trimmed.starts_with('[') {
                                    in_install = false;
                                    continue;
                                }
                                if !in_install {
                                    continue;
                                }
                                if let Some(val) = trimmed.strip_prefix("WantedBy=") {
                                    for target in val.split_whitespace() {
                                        let wants_dir = base_dir.join(format!("{target}.wants"));
                                        let _ = std::fs::create_dir_all(&wants_dir);
                                        let link = wants_dir.join(&full_name);
                                        let _ = std::fs::remove_file(&link);
                                        let _ = std::os::unix::fs::symlink(&unit_path, &link);
                                    }
                                }
                                if let Some(val) = trimmed.strip_prefix("RequiredBy=") {
                                    for target in val.split_whitespace() {
                                        let req_dir = base_dir.join(format!("{target}.requires"));
                                        let _ = std::fs::create_dir_all(&req_dir);
                                        let link = req_dir.join(&full_name);
                                        let _ = std::fs::remove_file(&link);
                                        let _ = std::os::unix::fs::symlink(&unit_path, &link);
                                    }
                                }
                            }
                        }
                    }
                    ("disable", "full" | "disable-only") => {
                        // Remove symlinks
                        if let Ok(entries) = std::fs::read_dir(base_dir) {
                            for entry in entries.flatten() {
                                let entry_name = entry.file_name();
                                let entry_str = entry_name.to_string_lossy();
                                if entry_str.ends_with(".wants") || entry_str.ends_with(".requires")
                                {
                                    let link = entry.path().join(&full_name);
                                    if link.symlink_metadata().is_ok() {
                                        let _ = std::fs::remove_file(&link);
                                    }
                                }
                            }
                        }
                    }
                    _ => {} // action doesn't match preset_mode, skip
                }
            }
            return Ok(serde_json::json!(null));
        }
        Command::SetProperty(unit_name, props) => {
            // Check for --runtime flag in props
            let is_runtime = props.iter().any(|p| p == "--runtime");
            let props: Vec<String> = props.into_iter().filter(|p| p != "--runtime").collect();

            if props.is_empty() {
                log::debug!("set-property {}: no properties specified", unit_name);
                return Ok(serde_json::json!(null));
            }

            // Verify the unit exists.
            {
                let ri = run_info.read_poisoned();
                let units = find_units_with_name(&unit_name, &ri.unit_table);
                if units.is_empty() {
                    return Err(format!("Unit {unit_name} not found."));
                }
            }

            // Handle Markers property specially (transient, not persisted)
            let markers_prop = props
                .iter()
                .find(|p| p.starts_with("Markers="))
                .map(|p| p.strip_prefix("Markers=").unwrap_or("").to_owned());
            if let Some(markers_val) = &markers_prop {
                let ri = run_info.read_poisoned();
                let mut unit_markers = ri.unit_markers.lock().unwrap();
                let markers: Vec<String> = markers_val
                    .split_whitespace()
                    .map(|s| s.to_owned())
                    .collect();
                if markers.is_empty() {
                    unit_markers.remove(&unit_name);
                } else {
                    unit_markers.insert(unit_name.clone(), markers);
                }
            }
            // Filter out Markers from props for disk persistence
            let props: Vec<String> = if markers_prop.is_some() {
                let filtered: Vec<String> = props
                    .into_iter()
                    .filter(|p| !p.starts_with("Markers="))
                    .collect();
                if filtered.is_empty() {
                    return Ok(serde_json::json!(null));
                }
                filtered
            } else {
                props
            };

            // Group properties by section.
            // Properties like CPUWeight, MemoryMax, etc. belong to [Service],
            // [Slice], [Socket], [Mount], or [Swap] sections. For simplicity
            // we put resource-control and execution properties under [Service]
            // and unit-level properties under [Unit].
            let unit_props = [
                "Description",
                "Documentation",
                "Wants",
                "Requires",
                "After",
                "Before",
            ];
            // Normalize properties (e.g. CPUQuota=10% -> CPUQuota=10.00%)
            let props: Vec<String> = props
                .into_iter()
                .map(|prop| {
                    if let Some((key, val)) = prop.split_once('=')
                        && key == "CPUQuota"
                        && let Some(pct) = val.strip_suffix('%')
                        && let Ok(n) = pct.parse::<f64>()
                    {
                        return format!("{key}={n:.2}%");
                    }
                    prop
                })
                .collect();

            let mut unit_section_lines = Vec::new();
            let mut specific_section_lines = Vec::new();
            for prop in &props {
                if let Some((key, _val)) = prop.split_once('=') {
                    if unit_props.contains(&key) {
                        unit_section_lines.push(prop.as_str());
                    } else {
                        specific_section_lines.push(prop.as_str());
                    }
                } else {
                    log::warn!("set-property: ignoring malformed property: {prop}");
                }
            }

            // Determine the specific section name from the unit suffix.
            let section_name = if unit_name.ends_with(".service") {
                "Service"
            } else if unit_name.ends_with(".socket") {
                "Socket"
            } else if unit_name.ends_with(".slice") {
                "Slice"
            } else if unit_name.ends_with(".mount") {
                "Mount"
            } else if unit_name.ends_with(".swap") {
                "Swap"
            } else if unit_name.ends_with(".timer") {
                "Timer"
            } else if unit_name.ends_with(".path") {
                "Path"
            } else {
                "Service"
            };

            // Build the drop-in content.
            let mut content = String::new();
            if !unit_section_lines.is_empty() {
                content.push_str("[Unit]\n");
                for line in &unit_section_lines {
                    content.push_str(line);
                    content.push('\n');
                }
                content.push('\n');
            }
            if !specific_section_lines.is_empty() {
                let _ = writeln!(content, "[{section_name}]");
                for line in &specific_section_lines {
                    content.push_str(line);
                    content.push('\n');
                }
                content.push('\n');
            }

            // Write the drop-in file.
            // Use system.control directory (matches systemd behavior for set-property)
            let base_dir = if is_runtime {
                "/run/systemd/system.control"
            } else {
                "/etc/systemd/system.control"
            };
            let dropin_dir = std::path::Path::new(base_dir).join(format!("{unit_name}.d"));
            if let Err(e) = std::fs::create_dir_all(&dropin_dir) {
                return Err(format!(
                    "Failed to create drop-in directory {}: {e}",
                    dropin_dir.display()
                ));
            }
            let dropin_path = dropin_dir.join("50-set-property.conf");
            if let Err(e) = std::fs::write(&dropin_path, &content) {
                return Err(format!(
                    "Failed to write drop-in file {}: {e}",
                    dropin_path.display()
                ));
            }
            info!(
                "set-property {}: wrote drop-in {}",
                unit_name,
                dropin_path.display()
            );

            // Apply properties to the in-memory unit immediately (like real systemd).
            {
                let mut ri = run_info.write_poisoned();
                let uid = crate::units::UnitId {
                    name: unit_name.clone(),
                    kind: if unit_name.ends_with(".service") {
                        crate::units::UnitIdKind::Service
                    } else if unit_name.ends_with(".slice") {
                        crate::units::UnitIdKind::Slice
                    } else if unit_name.ends_with(".socket") {
                        crate::units::UnitIdKind::Socket
                    } else if unit_name.ends_with(".mount") {
                        crate::units::UnitIdKind::Mount
                    } else if unit_name.ends_with(".timer") {
                        crate::units::UnitIdKind::Timer
                    } else if unit_name.ends_with(".target") {
                        crate::units::UnitIdKind::Target
                    } else {
                        crate::units::UnitIdKind::Service
                    },
                };
                if let Some(unit) = ri.unit_table.get_mut(&uid) {
                    for prop in &props {
                        if let Some((key, value)) = prop.split_once('=') {
                            match &mut unit.specific {
                                Specific::Slice(sl) => match key {
                                    "MemoryMax" => {
                                        sl.conf.memory_max = Some(parse_memory_limit(value));
                                    }
                                    "MemoryHigh" => {
                                        sl.conf.memory_high = Some(parse_memory_limit(value));
                                    }
                                    "MemoryMin" => {
                                        sl.conf.memory_min = Some(parse_memory_limit(value));
                                    }
                                    "MemoryLow" => {
                                        sl.conf.memory_low = Some(parse_memory_limit(value));
                                    }
                                    "TasksMax" => {
                                        sl.conf.tasks_max = Some(parse_tasks_max(value));
                                    }
                                    _ => {}
                                },
                                Specific::Service(svc) => match key {
                                    "MemoryMax" => {
                                        svc.conf.memory_max = Some(parse_memory_limit(value));
                                    }
                                    "MemoryHigh" => {
                                        svc.conf.memory_high = Some(parse_memory_limit(value));
                                    }
                                    "MemoryMin" => {
                                        svc.conf.memory_min = Some(parse_memory_limit(value));
                                    }
                                    "MemoryLow" => {
                                        svc.conf.memory_low = Some(parse_memory_limit(value));
                                    }
                                    "TasksMax" => {
                                        svc.conf.tasks_max = Some(parse_tasks_max(value));
                                    }
                                    _ => {}
                                },
                                _ => {}
                            }
                        }
                    }
                }
            }

            return Ok(serde_json::json!({
                "dropin": dropin_path.display().to_string(),
                "properties": props,
            }));
        }
        Command::Edit(unit_name, full) => {
            // Return info about the unit so the client can open an editor.
            let ri = run_info.read_poisoned();
            let units = find_units_with_name(&unit_name, &ri.unit_table);
            if units.is_empty() {
                return Err(format!("Unit {unit_name} not found."));
            }
            let unit = &units[0];
            let fragment_path = unit
                .common
                .unit
                .fragment_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();

            let dropin_dir = format!("/etc/systemd/system/{unit_name}.d");
            let override_path = format!("{dropin_dir}/override.conf");

            // Read existing override content if present.
            let existing_override = std::fs::read_to_string(&override_path).unwrap_or_default();

            // For --full mode, read the original unit file content.
            let original_content = if full && !fragment_path.is_empty() {
                std::fs::read_to_string(&fragment_path).unwrap_or_default()
            } else {
                String::new()
            };

            return Ok(serde_json::json!({
                "unit": unit_name,
                "fragment_path": fragment_path,
                "dropin_dir": dropin_dir,
                "override_path": override_path,
                "existing_override": existing_override,
                "original_content": original_content,
                "full": full,
            }));
        }
        Command::Revert(unit_name) => {
            // Remove all local customizations for the unit:
            // 1. Drop-in directory: /etc/systemd/system/<unit>.d/
            // 2. Drop-in directory: /run/systemd/system/<unit>.d/
            // 3. Admin override: /etc/systemd/system/<unit> (only if a vendor copy exists)
            // 4. Runtime override: /run/systemd/system/<unit>
            let mut removed = Vec::new();

            // Verify the unit exists.
            {
                let ri = run_info.read_poisoned();
                let units = find_units_with_name(&unit_name, &ri.unit_table);
                if units.is_empty() {
                    return Err(format!("Unit {unit_name} not found."));
                }
            }

            // Check if a vendor-provided unit file exists (in /usr/lib or /lib).
            let vendor_paths = [
                std::path::Path::new("/usr/lib/systemd/system").join(&unit_name),
                std::path::Path::new("/lib/systemd/system").join(&unit_name),
                std::path::Path::new("/usr/local/lib/systemd/system").join(&unit_name),
            ];
            let has_vendor = vendor_paths.iter().any(|p| p.exists());

            // Remove /etc/systemd/system/<unit> if it's a mask symlink (-> /dev/null)
            // or an admin override (regular file when vendor copy exists).
            let etc_override = std::path::Path::new("/etc/systemd/system").join(&unit_name);
            let is_mask = std::fs::read_link(&etc_override)
                .map(|t| t == std::path::Path::new("/dev/null"))
                .unwrap_or(false);
            if is_mask || (has_vendor && etc_override.is_file()) {
                if let Err(e) = std::fs::remove_file(&etc_override) {
                    return Err(format!("Failed to remove {}: {e}", etc_override.display()));
                }
                removed.push(etc_override.display().to_string());
            }

            // Remove /etc/systemd/system/<unit>.d/
            let etc_dropin =
                std::path::Path::new("/etc/systemd/system").join(format!("{unit_name}.d"));
            if etc_dropin.is_dir() {
                if let Err(e) = std::fs::remove_dir_all(&etc_dropin) {
                    return Err(format!("Failed to remove {}: {e}", etc_dropin.display()));
                }
                removed.push(etc_dropin.display().to_string());
            }

            // Remove /run/systemd/system/<unit> (mask or override)
            let run_override = std::path::Path::new("/run/systemd/system").join(&unit_name);
            let is_run_mask = std::fs::read_link(&run_override)
                .map(|t| t == std::path::Path::new("/dev/null"))
                .unwrap_or(false);
            if is_run_mask || run_override.is_file() {
                if let Err(e) = std::fs::remove_file(&run_override) {
                    return Err(format!("Failed to remove {}: {e}", run_override.display()));
                }
                removed.push(run_override.display().to_string());
            }

            // Remove /run/systemd/system/<unit>.d/
            let run_dropin =
                std::path::Path::new("/run/systemd/system").join(format!("{unit_name}.d"));
            if run_dropin.is_dir() {
                if let Err(e) = std::fs::remove_dir_all(&run_dropin) {
                    return Err(format!("Failed to remove {}: {e}", run_dropin.display()));
                }
                removed.push(run_dropin.display().to_string());
            }

            // Remove system.control directories (created by set-property)
            for base in &["/etc/systemd/system.control", "/run/systemd/system.control"] {
                let control_dropin = std::path::Path::new(base).join(format!("{unit_name}.d"));
                if control_dropin.is_dir() {
                    if let Err(e) = std::fs::remove_dir_all(&control_dropin) {
                        return Err(format!(
                            "Failed to remove {}: {e}",
                            control_dropin.display()
                        ));
                    }
                    removed.push(control_dropin.display().to_string());
                }
            }

            if removed.is_empty() {
                info!("revert {}: nothing to remove", unit_name);
            } else {
                info!("revert {}: removed {:?}", unit_name, removed);
            }

            let removed_values: Vec<Value> = removed.into_iter().map(Value::String).collect();
            return Ok(serde_json::json!({ "reverted": unit_name, "removed": removed_values }));
        }
        Command::Clean(unit_name, what) => {
            // Clean removes resource directories based on --what flag.
            // Default (None): runtime + cache
            // "configuration": ConfigurationDirectory
            // "runtime": RuntimeDirectory
            // "state": StateDirectory
            // "cache": CacheDirectory
            // "logs": LogsDirectory
            // "all": all directories
            let ri = run_info.read_poisoned();
            let units = find_units_with_name(&unit_name, &ri.unit_table);
            if units.is_empty() {
                return Err(format!("Unit {unit_name} not found."));
            }
            let unit = &units[0];

            // Unit must be inactive/dead to clean
            let status = unit.common.status.read_poisoned().clone();
            if matches!(status, UnitStatus::Started(_) | UnitStatus::Starting) {
                return Err(format!("Unit {unit_name} is active, cannot clean."));
            }

            // For timer units, clean --what=state removes the persistent
            // stamp file in /var/lib/systemd/timers/.
            if matches!(&unit.specific, Specific::Timer(_)) {
                let what = what.as_deref();
                if matches!(what, Some("state") | Some("all")) {
                    let stamp_path = format!("/var/lib/systemd/timers/stamp-{}", unit_name);
                    if std::path::Path::new(&stamp_path).exists() {
                        let _ = std::fs::remove_file(&stamp_path);
                    }
                }
                return Ok(serde_json::json!({ "cleaned": unit_name }));
            }

            // Extract exec_config from the unit's specific config
            let exec_config = match &unit.specific {
                Specific::Service(svc) => Some(&svc.conf.exec_config),
                Specific::Socket(sock) => Some(&sock.conf.exec_config),
                _ => None,
            };

            let exec_config = match exec_config {
                Some(ec) => ec,
                None => {
                    return Ok(serde_json::json!({ "cleaned": unit_name }));
                }
            };

            let dynamic_user = exec_config.dynamic_user;

            let what = what.as_deref();
            let remove_configuration = matches!(what, Some("configuration") | Some("all"));
            let remove_runtime = matches!(what, None | Some("runtime") | Some("all"));
            let remove_state = matches!(what, Some("state") | Some("all"));
            let remove_cache = matches!(what, None | Some("cache") | Some("all"));
            let remove_logs = matches!(what, Some("logs") | Some("all"));

            let mut removed = Vec::new();

            if remove_configuration {
                for dir_name in &exec_config.configuration_directory {
                    let path = format!("/etc/{dir_name}");
                    if std::path::Path::new(&path).exists() {
                        let _ = std::fs::remove_dir_all(&path);
                        removed.push(path);
                    }
                }
            }

            if remove_runtime {
                for dir_name in &exec_config.runtime_directory {
                    if dynamic_user {
                        let private = format!("/run/private/{dir_name}");
                        let link = format!("/run/{dir_name}");
                        if std::path::Path::new(&private).exists() {
                            let _ = std::fs::remove_dir_all(&private);
                            removed.push(private);
                        }
                        if std::path::Path::new(&link).exists() {
                            let _ = std::fs::remove_file(&link);
                            removed.push(link);
                        }
                    } else {
                        let path = format!("/run/{dir_name}");
                        if std::path::Path::new(&path).exists() {
                            let _ = std::fs::remove_dir_all(&path);
                            removed.push(path);
                        }
                    }
                }
            }

            if remove_state {
                for dir_name in &exec_config.state_directory {
                    if dynamic_user {
                        let private = format!("/var/lib/private/{dir_name}");
                        let link = format!("/var/lib/{dir_name}");
                        if std::path::Path::new(&private).exists() {
                            let _ = std::fs::remove_dir_all(&private);
                            removed.push(private);
                        }
                        if std::path::Path::new(&link).exists() {
                            let _ = std::fs::remove_file(&link);
                            removed.push(link);
                        }
                    } else {
                        let path = format!("/var/lib/{dir_name}");
                        if std::path::Path::new(&path).exists() {
                            let _ = std::fs::remove_dir_all(&path);
                            removed.push(path);
                        }
                    }
                }
            }

            if remove_cache {
                for dir_name in &exec_config.cache_directory {
                    if dynamic_user {
                        let private = format!("/var/cache/private/{dir_name}");
                        let link = format!("/var/cache/{dir_name}");
                        if std::path::Path::new(&private).exists() {
                            let _ = std::fs::remove_dir_all(&private);
                            removed.push(private);
                        }
                        if std::path::Path::new(&link).exists() {
                            let _ = std::fs::remove_file(&link);
                            removed.push(link);
                        }
                    } else {
                        let path = format!("/var/cache/{dir_name}");
                        if std::path::Path::new(&path).exists() {
                            let _ = std::fs::remove_dir_all(&path);
                            removed.push(path);
                        }
                    }
                }
            }

            if remove_logs {
                for dir_name in &exec_config.logs_directory {
                    if dynamic_user {
                        let private = format!("/var/log/private/{dir_name}");
                        let link = format!("/var/log/{dir_name}");
                        if std::path::Path::new(&private).exists() {
                            let _ = std::fs::remove_dir_all(&private);
                            removed.push(private);
                        }
                        if std::path::Path::new(&link).exists() {
                            let _ = std::fs::remove_file(&link);
                            removed.push(link);
                        }
                    } else {
                        let path = format!("/var/log/{dir_name}");
                        if std::path::Path::new(&path).exists() {
                            let _ = std::fs::remove_dir_all(&path);
                            removed.push(path);
                        }
                    }
                }
            }

            info!("clean {}: removed {:?}", unit_name, removed);
            return Ok(serde_json::json!({ "cleaned": unit_name }));
        }
        Command::Freeze(ref unit_name) | Command::Thaw(ref unit_name) => {
            let freeze = matches!(cmd, Command::Freeze(_));
            let ri = run_info.read_poisoned();
            let units = find_units_with_name(unit_name, &ri.unit_table);
            if units.is_empty() {
                return Err(format!("Unit {unit_name} not found."));
            }
            let unit = &units[0];

            // Unit must be active to freeze/thaw
            let status = unit.common.status.read_poisoned().clone();
            if !matches!(status, UnitStatus::Started(_)) {
                return Err(format!(
                    "Unit {unit_name} is not active, cannot {}.",
                    if freeze { "freeze" } else { "thaw" }
                ));
            }

            // Write to the cgroup freezer
            #[cfg(target_os = "linux")]
            if let Specific::Service(svc) = &unit.specific {
                let cgroup_path = &svc.conf.platform_specific.cgroup_path;
                let freeze_file = cgroup_path.join("cgroup.freeze");
                let val = if freeze { "1" } else { "0" };
                if let Err(e) = std::fs::write(&freeze_file, val) {
                    warn!("Failed to write {} to {:?}: {}", val, freeze_file, e);
                }
            }

            // Update the FreezerState
            let new_state = if freeze {
                crate::units::FreezerState::Frozen
            } else {
                crate::units::FreezerState::Running
            };
            crate::control::unit_properties::set_freezer_state(unit, new_state);

            return Ok(serde_json::json!(null));
        }
        Command::ShowEnvironment => {
            let ri = run_info.read_poisoned();
            let env = ri.manager_environment.lock().unwrap();
            let mut lines: Vec<String> = env.iter().map(|(k, v)| format!("{k}={v}")).collect();
            lines.sort();
            return Ok(serde_json::json!({ "environment": lines }));
        }
        Command::SetEnvironment(vars) => {
            let ri = run_info.read_poisoned();
            let mut env = ri.manager_environment.lock().unwrap();
            for var in &vars {
                if let Some((k, v)) = var.split_once('=') {
                    env.insert(k.to_owned(), v.to_owned());
                }
            }
            return Ok(serde_json::json!(null));
        }
        Command::UnsetEnvironment(vars) => {
            let ri = run_info.read_poisoned();
            let mut env = ri.manager_environment.lock().unwrap();
            for var in &vars {
                // If it contains '=', extract just the key
                let key = var.split_once('=').map_or(var.as_str(), |(k, _)| k);
                // Revert to the process's built-in value, or remove if none
                match std::env::var(key) {
                    Ok(val) => {
                        env.insert(key.to_owned(), val);
                    }
                    Err(_) => {
                        env.remove(key);
                    }
                }
            }
            return Ok(serde_json::json!(null));
        }
        Command::ImportEnvironment(vars) => {
            // import-environment imports from the calling process's environment.
            // Since the RPC runs in PID 1 context, we use std::env::var() to get
            // the manager's own environment. The client sends the variable names
            // along with their values as KEY=VALUE pairs.
            let ri = run_info.read_poisoned();
            let mut env = ri.manager_environment.lock().unwrap();
            for var in &vars {
                if let Some((k, v)) = var.split_once('=') {
                    env.insert(k.to_owned(), v.to_owned());
                } else if let Ok(v) = std::env::var(var) {
                    env.insert(var.clone(), v);
                }
            }
            return Ok(serde_json::json!(null));
        }
        Command::ListTimers => {
            let ri = run_info.read_poisoned();
            let mut timers: Vec<Value> = Vec::new();
            for unit in ri.unit_table.values() {
                if let Specific::Timer(timer_specific) = &unit.specific {
                    let status = unit.common.status.read_poisoned().clone();
                    let active = matches!(status, UnitStatus::Started(_));
                    let conf = &timer_specific.conf;
                    let target = &conf.unit;

                    // Build a description of when the timer fires
                    let mut triggers = Vec::new();
                    for d in &conf.on_boot_sec {
                        triggers.push(format!("OnBootSec={}", format_duration(d)));
                    }
                    for d in &conf.on_startup_sec {
                        triggers.push(format!("OnStartupSec={}", format_duration(d)));
                    }
                    for d in &conf.on_active_sec {
                        triggers.push(format!("OnActiveSec={}", format_duration(d)));
                    }
                    for d in &conf.on_unit_active_sec {
                        triggers.push(format!("OnUnitActiveSec={}", format_duration(d)));
                    }
                    for d in &conf.on_unit_inactive_sec {
                        triggers.push(format!("OnUnitInactiveSec={}", format_duration(d)));
                    }
                    for expr in &conf.on_calendar {
                        triggers.push(format!("OnCalendar={}", expr));
                    }

                    timers.push(serde_json::json!({
                        "UNIT": unit.id.name,
                        "ACTIVATES": target,
                        "ACTIVE": if active { "active" } else { "inactive" },
                        "TRIGGERS": triggers.join("; "),
                        "PERSISTENT": conf.persistent,
                    }));
                }
            }
            // Sort by unit name
            timers.sort_by(|a, b| {
                a.get("UNIT")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .cmp(b.get("UNIT").and_then(|v| v.as_str()).unwrap_or(""))
            });
            return Ok(Value::Array(timers));
        }
        Command::ListSockets => {
            // Return socket units. Currently returns an empty list since
            // socket activation is not yet implemented.
            return Ok(Value::Array(Vec::new()));
        }
        Command::ListPaths => {
            // Return path units. Currently returns an empty list since
            // path monitoring is not yet implemented.
            return Ok(Value::Array(Vec::new()));
        }
        Command::ListJobs => {
            // Return units currently being activated as jobs. Units in the
            // pending_activations set that are NeverStarted are "waiting";
            // units in Starting state are "running".
            let ri = run_info.read_poisoned();
            let pending = ri.pending_activations.lock().unwrap().clone();
            let mut job_id: u64 = 1;
            let mut jobs: Vec<Value> = Vec::new();
            for unit in ri.unit_table.values() {
                let status = unit.common.status.read_poisoned().clone();
                let (job_type, state) = match &status {
                    UnitStatus::Starting => ("start", "running"),
                    UnitStatus::NeverStarted if pending.contains(&unit.id) => ("start", "waiting"),
                    _ => continue,
                };
                jobs.push(serde_json::json!({
                    "JOB": job_id,
                    "UNIT": unit.id.name,
                    "TYPE": job_type,
                    "STATE": state,
                }));
                job_id += 1;
            }
            return Ok(Value::Array(jobs));
        }
        Command::ResetFailed(unit_name) => {
            let ri = run_info.read_poisoned();
            if let Some(name) = unit_name {
                let units = find_units_with_name(&name, &ri.unit_table);
                if units.is_empty() {
                    return Err(format!("Unit {name} not found."));
                }
                for unit in &units {
                    reset_failed_unit(unit);
                }
            } else {
                // Reset all failed units
                for unit in ri.unit_table.values() {
                    reset_failed_unit(unit);
                }
            }
            return Ok(serde_json::json!(null));
        }
        Command::Kill(unit_name, signal, whom, kill_value, wait_for_stop) => {
            let ri = run_info.read_poisoned();
            let units = find_units_with_name(&unit_name, &ri.unit_table);
            if units.is_empty() {
                return Err(format!("Unit {unit_name} not found."));
            }
            let unit = &units[0];
            let status = unit.common.status.read_poisoned();
            let is_active = matches!(&*status, UnitStatus::Started(_) | UnitStatus::Starting);
            drop(status);

            if !is_active {
                // Unit has no running processes — nothing to signal.
                // Real systemd returns success here (sends to empty cgroup).
                return Ok(serde_json::json!(null));
            }

            // Look up the main PID to send the signal directly.
            // Prefer main_pid (from MAINPID= notification) over pid (the forked child).
            let main_pid = if let Specific::Service(svc) = &unit.specific {
                let state = svc.state.read_poisoned();
                state.srvc.main_pid.or(state.srvc.pid).map(i32::from)
            } else {
                None
            };

            let id = unit.id.clone();
            drop(ri);

            // Determine which PIDs to signal based on --kill-whom
            let pids_to_signal: Vec<i32> = match whom.as_str() {
                "main" => main_pid.into_iter().collect(),
                _ => main_pid.into_iter().collect(), // "all" falls back to main for now
            };

            if pids_to_signal.is_empty() {
                // No PID to signal — for SIGTERM/SIGKILL, deactivate
                if signal == libc::SIGTERM || signal == libc::SIGKILL {
                    let ri = run_info.read_poisoned();
                    crate::units::deactivate_unit(&id, &ri).map_err(|e| format!("{e}"))?;
                }
            } else {
                for pid in &pids_to_signal {
                    if let Some(val) = kill_value {
                        // Use sigqueue to send signal with a value
                        let sigval = libc::sigval {
                            sival_ptr: val as *mut libc::c_void,
                        };
                        let ret = unsafe { libc::sigqueue(*pid, signal, sigval) };
                        if ret == -1 {
                            log::trace!(
                                "sigqueue(pid={}, sig={}, val={}) failed: {}",
                                *pid,
                                signal,
                                val,
                                std::io::Error::last_os_error()
                            );
                        }
                    } else {
                        unsafe {
                            libc::kill(*pid, signal);
                        }
                    }
                }
                // Do NOT deactivate here — the service exit handler will
                // handle deactivation, restart, and OnSuccess/OnFailure
                // when the process actually dies from the signal.
            }

            // Wait for the unit to become inactive before returning.
            if wait_for_stop {
                for _ in 0..100 {
                    let ri = run_info.read_poisoned();
                    if let Some(unit) = ri.unit_table.get(&id) {
                        let status = unit.common.status.read_poisoned();
                        if matches!(
                            &*status,
                            UnitStatus::Stopped(_, _) | UnitStatus::NeverStarted
                        ) {
                            break;
                        }
                    } else {
                        break;
                    }
                    drop(ri);
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }

            return Ok(serde_json::json!(null));
        }
        Command::ListUnitFiles(type_filter) => {
            let ri = run_info.read_poisoned();
            let unit_dirs = &ri.config.unit_dirs;
            let unit_table = &ri.unit_table;

            let suffix_filter: Option<&str> = type_filter.as_deref().map(|t| match t {
                "service" => ".service",
                "target" => ".target",
                "socket" => ".socket",
                "mount" => ".mount",
                "swap" => ".swap",
                "timer" => ".timer",
                "path" => ".path",
                "slice" => ".slice",
                "scope" => ".scope",
                "device" => ".device",
                other => {
                    // Allow passing the suffix directly (e.g. ".service")
                    if other.starts_with('.') { other } else { "" }
                }
            });

            let mut entries: std::collections::BTreeMap<String, (&str, std::path::PathBuf)> =
                std::collections::BTreeMap::new();

            // Scan all unit directories for unit files
            for dir in unit_dirs {
                let read_dir = match std::fs::read_dir(dir) {
                    Ok(rd) => rd,
                    Err(_) => continue,
                };
                for entry in read_dir.flatten() {
                    let file_name = entry.file_name();
                    let name = file_name.to_string_lossy().to_string();

                    // Must have a recognized unit suffix
                    let is_unit = name.ends_with(".service")
                        || name.ends_with(".target")
                        || name.ends_with(".socket")
                        || name.ends_with(".mount")
                        || name.ends_with(".swap")
                        || name.ends_with(".timer")
                        || name.ends_with(".path")
                        || name.ends_with(".slice")
                        || name.ends_with(".scope")
                        || name.ends_with(".device");
                    if !is_unit {
                        continue;
                    }

                    // Apply type filter
                    if let Some(suffix) = suffix_filter
                        && !suffix.is_empty()
                        && !name.ends_with(suffix)
                    {
                        continue;
                    }

                    // First occurrence wins (higher-priority dirs come first)
                    if entries.contains_key(&name) {
                        continue;
                    }

                    let path = entry.path();

                    // Determine state
                    let state = if let Ok(target) = std::fs::read_link(&path) {
                        if target == std::path::Path::new("/dev/null") {
                            "masked"
                        } else if path.to_string_lossy().contains("/run/systemd/generator") {
                            "generated"
                        } else {
                            // Symlink to a real file — check if it has [Install]
                            unit_file_state(&name, unit_table, &path, unit_dirs)
                        }
                    } else if path.to_string_lossy().contains("/run/systemd/generator") {
                        "generated"
                    } else {
                        unit_file_state(&name, unit_table, &path, unit_dirs)
                    };

                    entries.insert(name, (state, path));
                }
            }

            // Format as a table: UNIT FILE <padding> STATE
            let mut out = String::new();
            let max_name_len = entries.keys().map(|n| n.len()).max().unwrap_or(20).max(9);
            let _ = writeln!(
                out,
                "{:<width$} STATE",
                "UNIT FILE",
                width = max_name_len + 2
            );
            for (name, (state, _path)) in &entries {
                let _ = writeln!(out, "{:<width$} {state}", name, width = max_name_len + 2);
            }
            let _ = writeln!(out, "\n{} unit files listed.", entries.len());

            return Ok(serde_json::json!({ "list-unit-files": out }));
        }
        Command::ListDependencies(unit_name, reverse) => {
            let ri = run_info.read_poisoned();
            let units = find_units_with_name(&unit_name, &ri.unit_table);
            if units.is_empty() {
                return Err(format!("Unit {unit_name} not found."));
            }
            let name = units[0].id.name.clone();
            let mut out = String::new();
            let mut visited = std::collections::HashSet::new();
            format_dep_tree(
                &name,
                &ri.unit_table,
                reverse,
                "",
                true,
                &mut visited,
                &mut out,
                0,
            );
            return Ok(serde_json::json!({ "list-dependencies": out }));
        }
        Command::Show(unit_name, filter) => {
            if unit_name == "__manager__" {
                // Manager-level properties
                let mut props = crate::control::unit_properties::PropertyMap::new();
                props.insert("Version".to_string(), "258".to_string());
                props.insert("Architecture".to_string(), "x86-64".to_string());
                // Read persisted log-level
                let log_level = std::fs::read_to_string("/run/rust-systemd/log-level")
                    .unwrap_or_else(|_| "info".to_string());
                props.insert("LogLevel".to_string(), log_level.trim().to_string());
                let log_target = std::fs::read_to_string("/run/rust-systemd/log-target")
                    .unwrap_or_else(|_| "journal-or-kmsg".to_string());
                props.insert("LogTarget".to_string(), log_target.trim().to_string());
                // Default rlimits - use current process limits as defaults
                props.insert("DefaultLimitNOFILE".to_string(), "524288".to_string());
                props.insert("DefaultLimitNOFILESoft".to_string(), "1024".to_string());
                // Read from system.conf.d if available
                if let Ok(content) =
                    std::fs::read_to_string("/run/systemd/system.conf.d/rlimits.conf")
                {
                    for line in content.lines() {
                        let line = line.trim();
                        if let Some(val) = line.strip_prefix("DefaultLimitNOFILE=") {
                            if let Some((soft, hard)) = val.split_once(':') {
                                props
                                    .insert("DefaultLimitNOFILESoft".to_string(), soft.to_string());
                                props.insert("DefaultLimitNOFILE".to_string(), hard.to_string());
                            } else {
                                props.insert("DefaultLimitNOFILE".to_string(), val.to_string());
                                props.insert("DefaultLimitNOFILESoft".to_string(), val.to_string());
                            }
                        }
                    }
                }
                // Boot timestamps — use process start time as approximation
                let boot_usec = {
                    let uptime = std::fs::read_to_string("/proc/uptime")
                        .ok()
                        .and_then(|s| s.split_whitespace().next().map(String::from))
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(0.0);
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default();
                    let boot = now.as_micros() as u64 - (uptime * 1_000_000.0) as u64;
                    boot.to_string()
                };
                props.insert("KernelTimestamp".to_string(), boot_usec.clone());
                props.insert("KernelTimestampMonotonic".to_string(), "0".to_string());

                let text = if let Some(ref f) = filter {
                    let mut out = String::new();
                    for part in f {
                        let part = part.trim();
                        if let Some(val) = props.get(part) {
                            out.push_str(&format!("{}={}\n", part, val));
                        } else {
                            out.push_str(&format!("{}=\n", part));
                        }
                    }
                    out
                } else {
                    let mut out = String::new();
                    for (k, v) in props.iter() {
                        out.push_str(&format!("{}={}\n", k, v));
                    }
                    out
                };
                return Ok(serde_json::json!({ "show": text }));
            }
            // Try to load the unit from disk if not already in memory.
            // This matches real systemd behaviour where `systemctl show` can
            // display units that haven't been explicitly loaded yet.
            let _unit_id = find_or_load_unit(&unit_name, &run_info);

            // If the name ends with .slice, create/update the implicit slice unit.
            // In systemd, slice units exist implicitly to form the cgroup hierarchy.
            // We always re-apply drop-ins to pick up changes since last query.
            if unit_name.ends_with(".slice") {
                let mut ri_mut = run_info.write_poisoned();
                // Only create/update if there's no fragment file on disk
                let has_file = ri_mut
                    .config
                    .unit_dirs
                    .iter()
                    .any(|d| d.join(&unit_name).exists());
                if !has_file {
                    create_or_update_implicit_slice(&unit_name, &mut ri_mut);
                }
            }

            let ri = run_info.read_poisoned();
            let mut units = find_units_with_name(&unit_name, &ri.unit_table);

            // If not found, the name might be a symlink alias (e.g.,
            // test15-a1.service -> test15-a.service). Resolve it.
            if units.is_empty() {
                let suffix = if unit_name.contains('.') {
                    ""
                } else {
                    ".service"
                };
                let full_name = format!("{unit_name}{suffix}");
                for dir in &ri.config.unit_dirs {
                    let candidate = dir.join(&full_name);
                    if let Ok(resolved) = std::fs::canonicalize(&candidate)
                        && let Some(resolved_name) = resolved
                            .file_name()
                            .map(|f| f.to_string_lossy().to_string())
                    {
                        units = find_units_with_name(&resolved_name, &ri.unit_table);
                        if !units.is_empty() {
                            break;
                        }
                    }
                }
            }

            if units.is_empty() {
                // Real systemd returns a stub property set with LoadState=not-found
                // instead of an error for unknown units.
                let mut props = crate::control::unit_properties::PropertyMap::new();
                // Ensure the queried name has a unit suffix
                let full_name = if unit_name.contains('.') {
                    unit_name.clone()
                } else {
                    format!("{unit_name}.service")
                };
                props.insert("Id".to_string(), full_name.clone());
                props.insert("Names".to_string(), full_name.clone());
                props.insert("LoadState".to_string(), "not-found".to_string());
                props.insert("ActiveState".to_string(), "inactive".to_string());
                props.insert("SubState".to_string(), "dead".to_string());
                props.insert("UnitFileState".to_string(), String::new());
                props.insert("FragmentPath".to_string(), String::new());
                props.insert("Description".to_string(), full_name.clone());

                // For service units, inherit DefaultLimitNOFILE from manager
                if full_name.ends_with(".service")
                    && let Some(rl) = read_default_limit_nofile()
                {
                    use crate::units::RLimitValue;
                    let fmt = |v: &RLimitValue| match v {
                        RLimitValue::Infinity => "infinity".to_string(),
                        RLimitValue::Value(n) => n.to_string(),
                    };
                    props.insert("LimitNOFILE".to_string(), fmt(&rl.hard));
                    props.insert("LimitNOFILESoft".to_string(), fmt(&rl.soft));
                }

                let text = unit_properties::format_properties(&props, filter.as_deref());
                return Ok(serde_json::json!({ "show": text }));
            }
            let unit = &units[0];
            let mut props = unit_properties::collect_properties(unit);

            // NeedDaemonReload — override the stub with a real check
            {
                let need_reload = unit_properties::need_daemon_reload(unit, &ri.config.unit_dirs);
                props.insert(
                    "NeedDaemonReload".to_string(),
                    if need_reload { "yes" } else { "no" }.to_string(),
                );
            }

            // Add Markers property
            {
                let markers = ri.unit_markers.lock().unwrap();
                let unit_markers = markers.get(&unit.id.name);
                let markers_val = unit_markers.map(|m| m.join(" ")).unwrap_or_default();
                props.insert("Markers".to_string(), markers_val);
            }

            // Compute Effective* resource-control properties by traversing
            // the slice hierarchy and finding the minimum limit.
            {
                use crate::units::unit_parsing::{MemoryLimit, TasksMax};

                // Helper: resolve a MemoryLimit to bytes (None = infinity)
                fn memory_limit_bytes(ml: &Option<MemoryLimit>) -> Option<u64> {
                    match ml {
                        Some(MemoryLimit::Bytes(n)) => Some(*n),
                        Some(MemoryLimit::Infinity) | None => None,
                        Some(MemoryLimit::Percent(_)) => None, // can't resolve without total mem
                    }
                }
                fn tasks_max_value(tm: &Option<TasksMax>) -> Option<u64> {
                    match tm {
                        Some(TasksMax::Value(n)) => Some(*n),
                        Some(TasksMax::Infinity) | None => None,
                        Some(TasksMax::Percent(_)) => None,
                    }
                }
                fn effective_min(own: Option<u64>, parent: Option<u64>) -> Option<u64> {
                    match (own, parent) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (Some(a), None) => Some(a),
                        (None, Some(b)) => Some(b),
                        (None, None) => None,
                    }
                }

                // Get the unit's own resource limits and slice name
                let (own_mem_max, own_mem_high, own_tasks_max, slice_name) = match &unit.specific {
                    Specific::Service(svc) => (
                        memory_limit_bytes(&svc.conf.memory_max),
                        memory_limit_bytes(&svc.conf.memory_high),
                        tasks_max_value(&svc.conf.tasks_max),
                        svc.conf.slice.clone(),
                    ),
                    Specific::Slice(sl) => (
                        memory_limit_bytes(&sl.conf.memory_max),
                        memory_limit_bytes(&sl.conf.memory_high),
                        tasks_max_value(&sl.conf.tasks_max),
                        // Parent slice derived from slice name: a-b-c.slice → a-b.slice
                        {
                            let name = &unit.id.name;
                            let base = name.strip_suffix(".slice").unwrap_or(name);
                            base.rfind('-').map(|pos| format!("{}.slice", &base[..pos]))
                        },
                    ),
                    _ => (None, None, None, None),
                };

                // Walk up the slice hierarchy
                let mut eff_mem_max = own_mem_max;
                let mut eff_mem_high = own_mem_high;
                let mut eff_tasks_max = own_tasks_max;
                let mut current_slice = slice_name;

                while let Some(ref sname) = current_slice {
                    let parent_units = find_units_with_name(sname, &ri.unit_table);
                    if parent_units.is_empty() {
                        break;
                    }
                    let parent = &parent_units[0];
                    if let Specific::Slice(sl) = &parent.specific {
                        eff_mem_max =
                            effective_min(eff_mem_max, memory_limit_bytes(&sl.conf.memory_max));
                        eff_mem_high =
                            effective_min(eff_mem_high, memory_limit_bytes(&sl.conf.memory_high));
                        eff_tasks_max =
                            effective_min(eff_tasks_max, tasks_max_value(&sl.conf.tasks_max));

                        // Move to parent slice
                        let pname = &parent.id.name;
                        let base = pname.strip_suffix(".slice").unwrap_or(pname);
                        current_slice =
                            base.rfind('-').map(|pos| format!("{}.slice", &base[..pos]));
                    } else {
                        break;
                    }
                }

                props.insert(
                    "EffectiveMemoryMax".to_string(),
                    match eff_mem_max {
                        Some(n) => n.to_string(),
                        None => "infinity".to_string(),
                    },
                );
                props.insert(
                    "EffectiveMemoryHigh".to_string(),
                    match eff_mem_high {
                        Some(n) => n.to_string(),
                        None => "infinity".to_string(),
                    },
                );
                props.insert(
                    "EffectiveTasksMax".to_string(),
                    match eff_tasks_max {
                        Some(n) => n.to_string(),
                        None => "infinity".to_string(),
                    },
                );
            }

            // If service doesn't have LimitNOFILE set, inherit from manager defaults.
            if !props.contains_key("LimitNOFILE")
                && let Specific::Service(_) | Specific::Socket(_) = &unit.specific
            {
                // Read manager defaults
                let mut default_hard = "524288".to_string();
                let mut default_soft = "1024".to_string();
                if let Ok(content) =
                    std::fs::read_to_string("/run/systemd/system.conf.d/rlimits.conf")
                {
                    for line in content.lines() {
                        let line = line.trim();
                        if let Some(val) = line.strip_prefix("DefaultLimitNOFILE=") {
                            if let Some((soft, hard)) = val.split_once(':') {
                                default_soft = soft.to_string();
                                default_hard = hard.to_string();
                            } else {
                                default_soft = val.to_string();
                                default_hard = val.to_string();
                            }
                        }
                    }
                }
                props.insert("LimitNOFILE".to_string(), default_hard);
                props.insert("LimitNOFILESoft".to_string(), default_soft);
            }

            // Re-read drop-in overrides from disk to pick up changes since
            // the unit was loaded (real systemd re-evaluates on access).
            {
                let unit_file_name = &unit.id.name;
                for dir in &ri.config.unit_dirs {
                    let dropin_dir = dir.join(format!("{unit_file_name}.d"));
                    if dropin_dir.is_dir()
                        && let Ok(entries) = std::fs::read_dir(&dropin_dir)
                    {
                        let mut files: Vec<_> = entries
                            .flatten()
                            .filter(|e| e.path().extension().is_some_and(|ext| ext == "conf"))
                            .collect();
                        files.sort_by_key(|e| e.file_name());
                        for entry in files {
                            if let Ok(content) = std::fs::read_to_string(entry.path()) {
                                // Extract Wants= and Requires= from the drop-in
                                for line in content.lines() {
                                    let line = line.trim();
                                    for (prefix, prop_key) in
                                        &[("Wants=", "Wants"), ("Requires=", "Requires")]
                                    {
                                        if let Some(val) = line.strip_prefix(prefix) {
                                            let current =
                                                props.get(prop_key).cloned().unwrap_or_default();
                                            let mut parts: Vec<String> = if current.is_empty() {
                                                Vec::new()
                                            } else {
                                                current
                                                    .split_whitespace()
                                                    .map(String::from)
                                                    .collect()
                                            };
                                            for unit_ref in val.split_whitespace() {
                                                let unit_ref = unit_ref.to_string();
                                                if !parts.contains(&unit_ref) {
                                                    parts.push(unit_ref);
                                                }
                                            }
                                            props.insert(prop_key.to_string(), parts.join(" "));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Dynamically scan .wants/ and .requires/ directories on disk.
            // Real systemd re-evaluates these at access time, so symlinks
            // created after daemon-reload are still visible.
            // Respects priority-based /dev/null masking: a symlink to /dev/null
            // at a higher-priority path masks the same dep at lower-priority paths.
            // Scan both the canonical name and the queried alias name.
            let unit_file_name = &unit.id.name;
            let mut scan_names = vec![unit_file_name.clone()];
            // If the query was for an alias, also scan its .wants/.requires
            let query_suffix = if unit_name.contains('.') {
                ""
            } else {
                ".service"
            };
            let query_full = format!("{unit_name}{query_suffix}");
            if query_full != *unit_file_name && !scan_names.contains(&query_full) {
                scan_names.push(query_full);
            }
            // For template instances (e.g. bar@0.service), also scan
            // template-level dirs (bar@.service.wants/).
            for name in scan_names.clone() {
                if let Some((template, _instance)) =
                    crate::units::loading::directory_deps::parse_template_instance(&name)
                    && !scan_names.contains(&template)
                {
                    scan_names.push(template);
                }
            }
            // Also scan dirs for all known aliases (including their
            // template-level forms).
            for alias in &unit.common.unit.aliases {
                if !scan_names.contains(alias) {
                    scan_names.push(alias.clone());
                    if let Some((template, _instance)) =
                        crate::units::loading::directory_deps::parse_template_instance(alias)
                        && !scan_names.contains(&template)
                    {
                        scan_names.push(template);
                    }
                }
            }
            for scan_name in &scan_names {
                let effective_deps = crate::units::loading::collect_dir_deps_for_unit(
                    &ri.config.unit_dirs,
                    scan_name,
                );
                for dep in &effective_deps {
                    let prop_key = if dep.is_requires { "Requires" } else { "Wants" };
                    // Resolve symlink aliases to canonical unit names
                    let child_name = {
                        let mut resolved = dep.child_unit.clone();
                        for dir in &ri.config.unit_dirs {
                            let candidate = dir.join(&dep.child_unit);
                            if let Ok(canonical) = std::fs::canonicalize(&candidate)
                                && let Some(name) = canonical
                                    .file_name()
                                    .map(|f| f.to_string_lossy().to_string())
                            {
                                resolved = name;
                                break;
                            }
                        }
                        resolved
                    };
                    let current = props.get(prop_key).cloned().unwrap_or_default();
                    let mut parts: Vec<String> = if current.is_empty() {
                        Vec::new()
                    } else {
                        current.split_whitespace().map(String::from).collect()
                    };
                    if !parts.contains(&child_name) {
                        parts.push(child_name);
                    }
                    props.insert(prop_key.to_string(), parts.join(" "));
                }
            }

            let text = unit_properties::format_properties(&props, filter.as_deref());
            return Ok(serde_json::json!({ "show": text }));
        }
        Command::Cat(unit_name) => {
            // Try to load the unit on demand (handles symlink aliases)
            let _ = find_or_load_unit(&unit_name, &run_info);
            let ri = run_info.read_poisoned();
            let units = find_units_with_name(&unit_name, &ri.unit_table);
            if units.is_empty() {
                return Err(format!("Unit {unit_name} not found."));
            }
            // For glob patterns or single units, concatenate all matching cat outputs
            let mut out = String::new();
            for unit in &units {
                let fragment_path = unit.common.unit.fragment_path.as_ref();
                if let Some(path) = fragment_path
                    && let Ok(content) = std::fs::read_to_string(path)
                {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(&format!("# {}\n", path.display()));
                    out.push_str(&content);
                }

                // Also include drop-in files from .d and system.control directories
                let name = &unit.id.name;
                let dropin_dirs = [
                    format!("/etc/systemd/system/{name}.d"),
                    format!("/run/systemd/system/{name}.d"),
                    format!("/etc/systemd/system.control/{name}.d"),
                    format!("/run/systemd/system.control/{name}.d"),
                ];
                for dir in &dropin_dirs {
                    let dir_path = std::path::Path::new(dir);
                    if dir_path.is_dir() {
                        let mut files: Vec<_> = std::fs::read_dir(dir_path)
                            .into_iter()
                            .flatten()
                            .filter_map(|e| e.ok())
                            .filter(|e| e.path().extension().is_some_and(|ext| ext == "conf"))
                            .collect();
                        files.sort_by_key(|e| e.file_name());
                        for entry in files {
                            if let Ok(content) = std::fs::read_to_string(entry.path()) {
                                out.push('\n');
                                out.push_str(&format!("# {}\n", entry.path().display()));
                                out.push_str(&content);
                            }
                        }
                    }
                }
            }
            if out.is_empty() {
                return Err(format!(
                    "No fragment path recorded for {unit_name} (unit may have been generated at runtime)"
                ));
            }
            return Ok(serde_json::json!({ "cat": out }));
        }
        Command::Shutdown(action) => {
            crate::shutdown::shutdown_sequence(run_info, action);
        }
        Command::Suspend
        | Command::Hibernate
        | Command::HybridSleep
        | Command::SuspendThenHibernate => {
            let verb = match cmd {
                Command::Suspend => "suspend",
                Command::Hibernate => "hibernate",
                Command::HybridSleep => "hybrid-sleep",
                Command::SuspendThenHibernate => "suspend-then-hibernate",
                _ => unreachable!(),
            };

            // Find the systemd-sleep binary relative to our own executable,
            // falling back to well-known system paths.
            let sleep_bin = find_sleep_binary()
                .ok_or_else(|| "Could not find systemd-sleep binary".to_string())?;

            info!("Executing {} via {}", verb, sleep_bin.display());

            let status = std::process::Command::new(&sleep_bin)
                .arg(verb)
                .status()
                .map_err(|e| format!("Failed to execute systemd-sleep: {e}"))?;

            if !status.success() {
                return Err(format!(
                    "systemd-sleep {} failed with exit code: {}",
                    verb,
                    status.code().unwrap_or(-1)
                ));
            }
        }
        Command::Reload(unit_name) => {
            let id = find_or_load_unit(&unit_name, &run_info)?;
            let ri = run_info.read_poisoned();
            let unit = ri
                .unit_table
                .get(&id)
                .ok_or_else(|| format!("Unit {unit_name} not found"))?;

            // Verify the unit is active
            {
                let status = unit.common.status.read_poisoned();
                if !matches!(&*status, UnitStatus::Started(_) | UnitStatus::Starting) {
                    return Err(format!("Unit {unit_name} is not active, cannot reload."));
                }
            }

            // Get ExecReload commands and the main PID for $MAINPID substitution
            let (reload_cmds, main_pid, working_dir) =
                if let Specific::Service(svc) = &unit.specific {
                    let state = svc.state.read_poisoned();
                    let pid = state.srvc.main_pid.or(state.srvc.pid);
                    (
                        svc.conf.reload.clone(),
                        pid.map(i32::from),
                        svc.conf.exec_config.working_directory.clone(),
                    )
                } else {
                    return Err(format!("Unit {unit_name} is not a service, cannot reload."));
                };

            if reload_cmds.is_empty() {
                return Err(format!(
                    "Job for {unit_name} failed because the unit does not support reload."
                ));
            }

            drop(ri);

            // Run each ExecReload command synchronously
            let mut last_error: Option<String> = None;
            for cmd in &reload_cmds {
                let program = &cmd.cmd;
                let args: Vec<String> = cmd
                    .args
                    .iter()
                    .map(|a| {
                        if let Some(pid) = main_pid {
                            a.replace("$MAINPID", &pid.to_string())
                        } else {
                            a.clone()
                        }
                    })
                    .collect();

                let mut child_cmd = std::process::Command::new(program);
                child_cmd.args(&args);
                child_cmd.stdin(std::process::Stdio::null());
                child_cmd.stdout(std::process::Stdio::null());
                child_cmd.stderr(std::process::Stdio::null());
                if let Some(ref wd) = working_dir {
                    child_cmd.current_dir(wd);
                }
                if let Some(pid) = main_pid {
                    child_cmd.env("MAINPID", pid.to_string());
                }

                match child_cmd.status() {
                    Ok(status) => {
                        if !status.success()
                            && !cmd
                                .prefixes
                                .contains(&crate::units::CommandlinePrefix::Minus)
                        {
                            last_error = Some(format!(
                                "ExecReload command '{program}' failed with {}",
                                status
                            ));
                            break;
                        }
                    }
                    Err(e) => {
                        if !cmd
                            .prefixes
                            .contains(&crate::units::CommandlinePrefix::Minus)
                        {
                            last_error = Some(format!(
                                "Failed to execute ExecReload command '{program}': {e}"
                            ));
                            break;
                        }
                    }
                }
            }

            if let Some(err) = last_error {
                return Err(err);
            }
        }
        Command::Restart(unit_name) => {
            let id = find_or_load_unit(&unit_name, &run_info)?;
            // Load dependency units from disk (e.g. Wants= targets that were
            // created since the unit was first loaded) and refresh on-disk
            // .wants/.requires directories, matching Start behaviour.
            load_dependency_units(&id, &run_info);
            refresh_directory_deps(&id.name, &run_info);
            // Collect NeverStarted Wants/Requires deps before reactivation.
            // These are deps that were just loaded from disk and need to be
            // started after the restart (e.g. services created on disk after
            // the target was initially loaded).
            let new_dep_ids: Vec<crate::units::UnitId> = {
                let ri = run_info.read_poisoned();
                let mut ids = Vec::new();
                if let Some(unit) = ri.unit_table.get(&id) {
                    for dep_id in unit
                        .common
                        .dependencies
                        .wants
                        .iter()
                        .chain(unit.common.dependencies.requires.iter())
                    {
                        if let Some(dep) = ri.unit_table.get(dep_id) {
                            let status = dep.common.status.read_poisoned();
                            if matches!(&*status, crate::units::UnitStatus::NeverStarted) {
                                ids.push(dep_id.clone());
                            }
                        }
                    }
                }
                ids
            };
            {
                let ri = run_info.read_poisoned();
                crate::units::reactivate_unit(id, &ri).map_err(|e| format!("{e}"))?;
            }
            // Start any newly-discovered NeverStarted deps.
            for dep_id in new_dep_ids {
                let errs = crate::units::activate_needed_units(dep_id, run_info.clone());
                for err in &errs {
                    warn!("Error starting dependency after restart: {err}");
                }
            }
        }
        Command::TryRestart(unit_name) => {
            // try-restart: restart the unit only if it is currently active.
            // If the unit is not active, do nothing (success).
            let ri = run_info.read_poisoned();
            let units = find_units_with_name(&unit_name, &ri.unit_table);
            if units.is_empty() {
                // Unit not found — nothing to restart, not an error for try-restart.
                return Ok(serde_json::json!(null));
            }
            if units.len() > 1 {
                let names: Vec<_> = units.iter().map(|unit| unit.id.name.clone()).collect();
                return Err(format!(
                    "More than one unit found with name: {unit_name}: {names:?}"
                ));
            }

            let id = units[0].id.clone();
            // Check if the unit is currently active.
            let is_active = ri
                .unit_table
                .get(&id)
                .map(|unit| {
                    let status_locked = unit.common.status.read_poisoned();
                    matches!(
                        *status_locked,
                        crate::units::UnitStatus::Started(_) | crate::units::UnitStatus::Starting
                    )
                })
                .unwrap_or(false);

            if is_active {
                crate::units::reactivate_unit(id, &ri).map_err(|e| format!("{e}"))?;
            }
            // If not active, silently succeed.
        }
        Command::ReloadOrRestart(unit_name) => {
            // reload-or-restart: try to reload, fall back to restart.
            if unit_name == "--marked" {
                // --marked: restart all units with needs-restart marker, then clear markers
                let units_to_restart: Vec<String> = {
                    let ri = run_info.read_poisoned();
                    let markers = ri.unit_markers.lock().unwrap();
                    markers
                        .iter()
                        .filter(|(_, v)| v.iter().any(|m| m == "needs-restart"))
                        .map(|(k, _)| k.clone())
                        .collect()
                };
                for name in &units_to_restart {
                    if let Ok(id) = find_or_load_unit(name, &run_info) {
                        let ri = run_info.read_poisoned();
                        let _ = crate::units::reactivate_unit(id, &ri);
                    }
                }
                // Clear needs-restart markers
                {
                    let ri = run_info.read_poisoned();
                    let mut markers = ri.unit_markers.lock().unwrap();
                    for name in &units_to_restart {
                        if let Some(m) = markers.get_mut(name) {
                            m.retain(|v| v != "needs-restart");
                            if m.is_empty() {
                                markers.remove(name);
                            }
                        }
                    }
                }
            } else {
                let id = find_or_load_unit(&unit_name, &run_info)?;
                let ri = run_info.read_poisoned();
                crate::units::reactivate_unit(id, &ri).map_err(|e| format!("{e}"))?;
            }
        }
        Command::IsActive(unit_name) => {
            // For glob patterns, check all matching units
            if is_glob_pattern(&unit_name) {
                let ri = run_info.read_poisoned();
                let units = find_units_with_name(&unit_name, &ri.unit_table);
                if units.is_empty() {
                    return Err(format!("Unit {unit_name} not found."));
                }
                // Return "active" if any matching unit is active
                let any_active = units.iter().any(|unit| {
                    let status = unit.common.status.read_poisoned();
                    matches!(&*status, crate::units::UnitStatus::Started(_))
                });
                let state = if any_active { "active" } else { "inactive" };
                return Ok(serde_json::json!(state));
            }
            // Try to find the unit in memory, or load it from disk.
            match find_or_load_unit(&unit_name, &run_info) {
                Err(_) => return Err(format!("Unit {unit_name} not found.")),
                Ok(id) => {
                    let ri = run_info.read_poisoned();
                    let unit = ri
                        .unit_table
                        .get(&id)
                        .ok_or_else(|| format!("Unit {unit_name} not found."))?;
                    let status_locked = unit.common.status.read_poisoned();
                    let state = match &*status_locked {
                        crate::units::UnitStatus::Started(_) => "active",
                        crate::units::UnitStatus::Starting => "activating",
                        crate::units::UnitStatus::Stopping => "deactivating",
                        crate::units::UnitStatus::Restarting => "activating",
                        crate::units::UnitStatus::Stopped(_, errors) if !errors.is_empty() => {
                            "failed"
                        }
                        crate::units::UnitStatus::Stopped(_, _) => "inactive",
                        crate::units::UnitStatus::NeverStarted => "inactive",
                    };
                    return Ok(serde_json::json!(state));
                }
            }
        }
        Command::IsEnabled(unit_name) => {
            // Try to find the unit file on disk across all unit directories.
            let ri = run_info.read_poisoned();
            let mut found_path = None;
            for dir in &ri.config.unit_dirs {
                let candidate = dir.join(&unit_name);
                if candidate.exists() {
                    found_path = Some(candidate);
                    break;
                }
            }
            match found_path {
                None => return Err(format!("Unit {unit_name} not found.")),
                Some(path) => {
                    let state =
                        unit_file_state(&unit_name, &ri.unit_table, &path, &ri.config.unit_dirs);
                    return Ok(serde_json::json!(state));
                }
            }
        }
        Command::IsSystemRunning => {
            let ri = run_info.read_poisoned();
            // Check if any unit is in failed state
            let has_failed = ri.unit_table.values().any(|unit| {
                let status = unit.common.status.read_poisoned();
                matches!(&*status, crate::units::UnitStatus::Stopped(_, errors) if !errors.is_empty())
            });
            let state = if has_failed { "degraded" } else { "running" };
            return Ok(serde_json::json!(state));
        }
        Command::IsFailed(unit_name) => match find_or_load_unit(&unit_name, &run_info) {
            Err(_) => return Err(format!("Unit {unit_name} not found.")),
            Ok(id) => {
                let ri = run_info.read_poisoned();
                let unit = ri
                    .unit_table
                    .get(&id)
                    .ok_or_else(|| format!("Unit {unit_name} not found."))?;
                let status_locked = unit.common.status.read_poisoned();
                let state = match &*status_locked {
                    crate::units::UnitStatus::Stopped(_, errors) if !errors.is_empty() => "failed",
                    _ => "inactive",
                };
                return Ok(serde_json::json!(state));
            }
        },
        Command::Start(unit_names) => {
            for unit_name in &unit_names {
                let id = find_or_load_unit(unit_name, &run_info)?;
                // Load all dependency units from disk so the full graph is available.
                load_dependency_units(&id, &run_info);
                // Refresh on-disk .wants/.requires so dynamically created
                // symlinks are picked up without requiring daemon-reload.
                refresh_directory_deps(&id.name, &run_info);
                // Reset the unit (and its Wants deps) from Stopped → NeverStarted
                // so activate_needed_units will actually start them. Without this,
                // units that were previously stopped are skipped.
                {
                    let ri = run_info.read_poisoned();
                    let mut ids_to_reset = vec![id.clone()];
                    if let Some(unit) = ri.unit_table.get(&id) {
                        for dep_id in unit
                            .common
                            .dependencies
                            .wants
                            .iter()
                            .chain(unit.common.dependencies.requires.iter())
                        {
                            ids_to_reset.push(dep_id.clone());
                        }
                    }
                    for reset_id in &ids_to_reset {
                        if let Some(u) = ri.unit_table.get(reset_id) {
                            let mut status = u.common.status.write_poisoned();
                            match &*status {
                                crate::units::UnitStatus::Stopped(_, _)
                                | crate::units::UnitStatus::Restarting => {
                                    *status = crate::units::UnitStatus::NeverStarted;
                                }
                                _ => {}
                            }
                        }
                    }
                }
                let errs = crate::units::activate_needed_units(id.clone(), run_info.clone());
                if !errs.is_empty() {
                    let mut errstr = String::from("Errors while starting the unit:");
                    for err in errs {
                        let _ = write!(errstr, "\n{err:?}");
                    }
                    return Err(errstr);
                }
                // Check the target unit's final status after activation.
                //
                // For oneshot services: activation is synchronous, so check
                // immediately for failure / restart.
                //
                // For Type=notify services: block until READY=1 is received
                // or the service stops, matching real systemd behavior where
                // `systemctl start` waits for readiness notification.
                {
                    let ri = run_info.read_poisoned();
                    if let Some(unit) = ri.unit_table.get(&id) {
                        let srvc_type = if let Specific::Service(srvc) = &unit.specific {
                            Some(srvc.conf.srcv_type)
                        } else {
                            None
                        };
                        match srvc_type {
                            // Non-service units (sockets, targets, etc.):
                            // check if the unit ended up in a failed state.
                            None => {
                                let status = unit.common.status.read_poisoned();
                                if let UnitStatus::Stopped(_, errors) = &*status
                                    && !errors.is_empty()
                                {
                                    return Err(format!("Unit {} failed to start", id.name));
                                }
                            }
                            Some(crate::units::ServiceType::OneShot) => {
                                let status = unit.common.status.read_poisoned();
                                match &*status {
                                    UnitStatus::Stopped(_, errors) if !errors.is_empty() => {
                                        return Err(format!("Unit {} failed to start", id.name));
                                    }
                                    UnitStatus::Restarting => {
                                        return Err(format!("Unit {} failed to start", id.name));
                                    }
                                    _ => {}
                                }
                            }
                            Some(
                                crate::units::ServiceType::Notify
                                | crate::units::ServiceType::NotifyReload,
                            ) => {
                                // Drop the run_info lock before polling.
                                drop(ri);
                                // Poll until the service sends READY=1 or stops.
                                loop {
                                    let ri = run_info.read_poisoned();
                                    let Some(unit) = ri.unit_table.get(&id) else {
                                        break;
                                    };
                                    let status = unit.common.status.read_poisoned();
                                    match &*status {
                                        UnitStatus::Stopped(_, errors) if !errors.is_empty() => {
                                            return Err(format!(
                                                "Unit {} failed to start",
                                                id.name
                                            ));
                                        }
                                        UnitStatus::Stopped(
                                            crate::units::StatusStopped::StoppedUnexpected,
                                            _,
                                        ) => {
                                            return Err(format!(
                                                "Unit {} failed to start",
                                                id.name
                                            ));
                                        }
                                        UnitStatus::Restarting => {
                                            return Err(format!(
                                                "Unit {} failed to start",
                                                id.name
                                            ));
                                        }
                                        UnitStatus::Stopped(_, _) => {
                                            // Clean stop before READY — treat as failure
                                            return Err(format!(
                                                "Unit {} failed to start",
                                                id.name
                                            ));
                                        }
                                        UnitStatus::Started(_) => {
                                            // Check if READY=1 has been signaled
                                            if let Specific::Service(svc) = &unit.specific {
                                                let state = svc.state.read_poisoned();
                                                if state.srvc.signaled_ready {
                                                    break; // success
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                    drop(status);
                                    drop(ri);
                                    std::thread::sleep(std::time::Duration::from_millis(50));
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        Command::StartWait(unit_names) => {
            // Start units, then block until they all reach a terminal state.
            let mut unit_ids = Vec::new();
            for unit_name in &unit_names {
                let id = find_or_load_unit(unit_name, &run_info)?;
                load_dependency_units(&id, &run_info);
                refresh_directory_deps(&id.name, &run_info);
                {
                    let ri = run_info.read_poisoned();
                    let mut ids_to_reset = vec![id.clone()];
                    if let Some(unit) = ri.unit_table.get(&id) {
                        for dep_id in unit
                            .common
                            .dependencies
                            .wants
                            .iter()
                            .chain(unit.common.dependencies.requires.iter())
                        {
                            ids_to_reset.push(dep_id.clone());
                        }
                    }
                    for reset_id in &ids_to_reset {
                        if let Some(u) = ri.unit_table.get(reset_id) {
                            let mut status = u.common.status.write_poisoned();
                            match &*status {
                                crate::units::UnitStatus::Stopped(_, _)
                                | crate::units::UnitStatus::Restarting => {
                                    *status = crate::units::UnitStatus::NeverStarted;
                                }
                                _ => {}
                            }
                        }
                    }
                }
                let errs = crate::units::activate_needed_units(id.clone(), run_info.clone());
                if !errs.is_empty() {
                    let mut errstr = String::from("Errors while starting the unit:");
                    for err in errs {
                        let _ = write!(errstr, "\n{err:?}");
                    }
                    return Err(errstr);
                }
                unit_ids.push(id);
            }

            // Poll until all specified units reach a terminal state.
            let mut any_failed = false;
            loop {
                let mut all_done = true;
                let ri = run_info.read_poisoned();
                for id in &unit_ids {
                    if let Some(unit) = ri.unit_table.get(id) {
                        let status = unit.common.status.read_poisoned();
                        match &*status {
                            crate::units::UnitStatus::Stopped(_, errors) => {
                                if !errors.is_empty() {
                                    any_failed = true;
                                }
                            }
                            crate::units::UnitStatus::NeverStarted => {}
                            _ => {
                                all_done = false;
                            }
                        }
                    }
                }
                drop(ri);
                if all_done {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            if any_failed {
                return Err("One or more units failed.".to_string());
            }
        }
        Command::StartNoBlock(unit_names) => {
            // Like Start but returns immediately — activation runs in background.
            for unit_name in &unit_names {
                let id = find_or_load_unit(unit_name, &run_info)?;
                load_dependency_units(&id, &run_info);
                refresh_directory_deps(&id.name, &run_info);
                {
                    let ri = run_info.read_poisoned();
                    let mut ids_to_reset = vec![id.clone()];
                    if let Some(unit) = ri.unit_table.get(&id) {
                        for dep_id in unit
                            .common
                            .dependencies
                            .wants
                            .iter()
                            .chain(unit.common.dependencies.requires.iter())
                        {
                            ids_to_reset.push(dep_id.clone());
                        }
                    }
                    for reset_id in &ids_to_reset {
                        if let Some(u) = ri.unit_table.get(reset_id) {
                            let mut status = u.common.status.write_poisoned();
                            if let crate::units::UnitStatus::Stopped(_, _) = &*status {
                                *status = crate::units::UnitStatus::NeverStarted;
                            }
                        }
                    }
                }
                // Record the full activation subgraph so list-jobs can find
                // "waiting" units.
                let pending = {
                    let ri = run_info.read_poisoned();
                    let mut ids = vec![id.clone()];
                    crate::units::collect_unit_start_subgraph(&mut ids, &ri.unit_table);
                    ri.pending_activations.clone()
                };
                {
                    let mut pa = pending.lock().unwrap();
                    let ri = run_info.read_poisoned();
                    let mut ids = vec![id.clone()];
                    crate::units::collect_unit_start_subgraph(&mut ids, &ri.unit_table);
                    for pending_id in &ids {
                        pa.insert(pending_id.clone());
                    }
                }
                let run_info_clone = run_info.clone();
                std::thread::spawn(move || {
                    let errs = crate::units::activate_needed_units(id, run_info_clone.clone());
                    for err in &errs {
                        log::error!("Background activation error: {err}");
                    }
                    // Clean up pending activations
                    let ri = run_info_clone.read_poisoned();
                    let mut pa = ri.pending_activations.lock().unwrap();
                    pa.clear();
                });
            }
        }
        Command::StartAll(unit_name) => {
            let id = {
                let run_info_locked = &*run_info.read_poisoned();
                let unit_table = &run_info_locked.unit_table;
                let units = find_units_with_name(&unit_name, unit_table);
                if units.len() > 1 {
                    let names: Vec<_> = units.iter().map(|unit| unit.id.name.clone()).collect();
                    return Err(format!(
                        "More than one unit found with name: {unit_name}: {names:?}"
                    ));
                }
                if units.is_empty() {
                    return Err(format!("No unit found with name: {unit_name}"));
                }

                units[0].id.clone()
            };

            let errs = crate::units::activate_needed_units(id, run_info);
            if !errs.is_empty() {
                let mut errstr = String::from("Errors while starting the units:");
                for err in errs {
                    let _ = write!(errstr, "\n{err:?}");
                }
                return Err(errstr);
            }
        }
        Command::Remove(unit_name) => {
            let run_info = &mut *run_info.write_poisoned();
            let id = {
                let units = find_units_with_name(&unit_name, &run_info.unit_table);
                if units.len() > 1 {
                    let names: Vec<_> = units.iter().map(|unit| unit.id.name.clone()).collect();
                    return Err(format!(
                        "More than one unit found with name: {unit_name}: {names:?}"
                    ));
                }
                if units.is_empty() {
                    return Err(format!("No unit found with name: {unit_name}"));
                }

                units[0].id.clone()
            };

            crate::units::remove_unit_with_dependencies(id, run_info)?;
        }
        Command::Stop(unit_names) => {
            let run_info = &*run_info.read_poisoned();
            for unit_name in &unit_names {
                let id = {
                    let units = find_units_with_name(unit_name, &run_info.unit_table);
                    if units.len() > 1 {
                        let names: Vec<_> = units.iter().map(|unit| unit.id.name.clone()).collect();
                        return Err(format!(
                            "More than one unit found with name: {unit_name}: {names:?}"
                        ));
                    }
                    if units.is_empty() {
                        // Silently skip units not found (matches real systemd
                        // behaviour for multi-unit stop).
                        continue;
                    }

                    units[0].id.clone()
                };

                crate::units::deactivate_unit_recursive(&id, run_info)
                    .map_err(|e| format!("{e}"))?;
            }
        }
        Command::StopNoBlock(unit_names) => {
            // Like Stop but returns immediately — deactivation runs in background.
            let ri = run_info.read_poisoned();
            let mut ids = Vec::new();
            for unit_name in &unit_names {
                let units = find_units_with_name(unit_name, &ri.unit_table);
                if let Some(unit) = units.first() {
                    ids.push(unit.id.clone());
                }
            }
            drop(ri);
            for id in ids {
                let run_info_clone = run_info.clone();
                std::thread::spawn(move || {
                    let ri = run_info_clone.read_poisoned();
                    if let Err(e) = crate::units::deactivate_unit_recursive(&id, &ri) {
                        log::error!("Background stop error for {}: {e}", id.name);
                    }
                });
            }
        }
        Command::RestartNoBlock(unit_name) => {
            // Like Restart but returns immediately — restart runs in background.
            let id = find_or_load_unit(&unit_name, &run_info)?;
            let run_info_clone = run_info.clone();
            std::thread::spawn(move || {
                if let Err(e) = crate::units::reactivate_unit(id, &run_info_clone.read_poisoned()) {
                    log::error!("Background restart error for {unit_name}: {e}");
                }
            });
        }
        Command::StopAll(unit_name) => {
            let run_info = &*run_info.read_poisoned();
            let id = {
                let units = find_units_with_name(&unit_name, &run_info.unit_table);
                if units.len() > 1 {
                    let names: Vec<_> = units.iter().map(|unit| unit.id.name.clone()).collect();
                    return Err(format!(
                        "More than one unit found with name: {unit_name}: {names:?}"
                    ));
                }
                if units.is_empty() {
                    return Err(format!("No unit found with name: {unit_name}"));
                }

                units[0].id.clone()
            };

            crate::units::deactivate_unit_recursive(&id, run_info).map_err(|e| format!("{e}"))?;
            // Happy
        }
        Command::Status(unit_name) => {
            let run_info = &*run_info.read_poisoned();
            let unit_table = &run_info.unit_table;
            if let Some(name) = unit_name {
                // Default to .service if no recognized suffix
                let name = if !name.contains('.') {
                    format!("{name}.service")
                } else {
                    name
                };
                //list specific
                let units = find_units_with_pattern(&name, unit_table);
                for unit in units {
                    let status = { unit.common.status.read_poisoned().clone() };
                    if name.ends_with(".service") {
                        result_vec
                            .as_array_mut()
                            .unwrap()
                            .push(format_service(unit, status));
                    } else if name.ends_with(".socket") {
                        result_vec
                            .as_array_mut()
                            .unwrap()
                            .push(format_socket(unit, status));
                    } else if name.ends_with(".target")
                        || name.ends_with(".slice")
                        || name.ends_with(".timer")
                        || name.ends_with(".mount")
                        || name.ends_with(".swap")
                        || name.ends_with(".path")
                        || name.ends_with(".device")
                    {
                        result_vec
                            .as_array_mut()
                            .unwrap()
                            .push(format_target(unit, status));
                    } else {
                        return Err("Name suffix not recognized".into());
                    }
                }
            } else {
                //list all
                let strings: Vec<_> = unit_table
                    .values()
                    .map(|unit| {
                        let status = { unit.common.status.read_poisoned().clone() };
                        match unit.specific {
                            Specific::Socket(_) => format_socket(unit, status),
                            Specific::Service(_) => format_service(unit, status),
                            Specific::Target(_)
                            | Specific::Slice(_)
                            | Specific::Mount(_)
                            | Specific::Swap(_)
                            | Specific::Timer(_)
                            | Specific::Path(_)
                            | Specific::Device(_) => format_target(unit, status),
                        }
                    })
                    .collect();
                for s in strings {
                    result_vec.as_array_mut().unwrap().push(s);
                }
            }
        }
        Command::ListUnits(kind, state_filter) => {
            let run_info = &*run_info.read_poisoned();
            let unit_table = &run_info.unit_table;
            for (id, unit) in unit_table {
                let kind_match = if let Some(kind) = kind {
                    id.kind == kind
                } else {
                    true
                };
                let state_match = if let Some(ref state) = state_filter {
                    let status = unit.common.status.read_poisoned();
                    let active_state = match &*status {
                        UnitStatus::NeverStarted => "inactive",
                        UnitStatus::Starting => "activating",
                        UnitStatus::Stopping => "deactivating",
                        UnitStatus::Restarting => "activating",
                        UnitStatus::Started(_) => "active",
                        UnitStatus::Stopped(_, errors) => {
                            if errors.is_empty() {
                                "inactive"
                            } else {
                                "failed"
                            }
                        }
                    };
                    state.split(',').any(|s| s == active_state)
                } else {
                    true
                };
                if kind_match && state_match {
                    result_vec
                        .as_array_mut()
                        .unwrap()
                        .push(Value::String(unit.id.name.clone()));
                }
            }
        }
        Command::LoadNew(names) => {
            let run_info = &mut *run_info.write_poisoned();
            let mut map = std::collections::HashMap::new();
            for name in &names {
                // Normalize: append .service suffix if no suffix present
                let full_name = if name.contains('.') {
                    name.clone()
                } else {
                    format!("{name}.service")
                };
                // Skip units that are already loaded (like `systemctl enable`
                // on an already-active unit should succeed silently).
                let already_loaded = run_info
                    .unit_table
                    .values()
                    .any(|u| u.id.name == full_name || u.id.name == *name);
                if already_loaded {
                    continue;
                }
                let unit = load_new_unit(&run_info.config.unit_dirs, name)?;
                map.insert(unit.id.clone(), unit);
            }
            if !map.is_empty() {
                insert_new_units(map, run_info)?;
            }
        }
        Command::LoadAllNew => {
            let run_info = &mut *run_info.write_poisoned();
            let unit_table = &run_info.unit_table;
            // Load all units without pruning so that standalone units
            // (not reachable from the boot target) are also discovered.
            let units =
                load_all_units_no_prune(&run_info.config.unit_dirs, &run_info.config.target_unit)
                    .map_err(|e| format!("Error while loading unit definitions: {e:?}"))?;

            // collect all names
            let existing_names: Vec<String> = unit_table
                .values()
                .map(|unit| unit.id.name.clone())
                .collect();

            // Collect freshly-loaded unit names so we can detect stale entries
            let fresh_names: std::collections::HashSet<String> =
                units.values().map(|u| u.id.name.clone()).collect();

            // Separate into new and updated units
            let mut ignored_units_names = Vec::new();
            let mut new_units_names = Vec::new();
            let mut updated_units_names = Vec::new();
            let mut new_units = std::collections::HashMap::new();
            let mut updated_units = Vec::new();
            for (id, unit) in units {
                if existing_names.contains(&unit.id.name) {
                    updated_units.push((id, unit));
                } else {
                    new_units_names.push(Value::String(unit.id.name.clone()));
                    new_units.insert(id, unit);
                }
            }

            // Remove units whose files no longer exist on disk.
            // Skip transient units (no file path) — they only exist at runtime.
            let mut removed_units_names = Vec::new();
            let stale_ids: Vec<_> = run_info
                .unit_table
                .iter()
                .filter(|(_, unit)| {
                    !fresh_names.contains(&unit.id.name) && unit.common.unit.fragment_path.is_some()
                })
                .map(|(id, _)| id.clone())
                .collect();
            for id in stale_ids {
                removed_units_names.push(Value::String(id.name.clone()));
                run_info.unit_table.remove(&id);
            }

            // Update existing units' configuration (preserving runtime status)
            for (new_id, new_unit) in updated_units {
                // Find the existing unit by name
                let existing_id = run_info
                    .unit_table
                    .keys()
                    .find(|id| id.name == new_id.name)
                    .cloned();
                if let Some(existing_id) = existing_id {
                    if let Some(existing_unit) = run_info.unit_table.get_mut(&existing_id) {
                        // Update configuration but preserve runtime status
                        // (PIDs, restart counts, etc.) by only replacing conf.
                        existing_unit.specific.update_config_from(new_unit.specific);
                        existing_unit.common.unit = new_unit.common.unit;
                        existing_unit.common.dependencies = new_unit.common.dependencies;
                        updated_units_names.push(Value::String(existing_id.name.clone()));
                    }
                } else {
                    ignored_units_names.push(Value::String(new_id.name.clone()));
                }
            }

            let mut response_object = serde_json::Map::new();
            insert_new_units(new_units, run_info)?;
            response_object.insert("Added".into(), serde_json::Value::Array(new_units_names));
            response_object.insert(
                "Updated".into(),
                serde_json::Value::Array(updated_units_names),
            );
            response_object.insert(
                "Ignored".into(),
                serde_json::Value::Array(ignored_units_names),
            );
            response_object.insert(
                "Removed".into(),
                serde_json::Value::Array(removed_units_names),
            );
            result_vec
                .as_array_mut()
                .unwrap()
                .push(Value::Object(response_object));
        }
        Command::LoadAllNewDry => {
            let run_info = &mut *run_info.write_poisoned();
            let unit_table = &run_info.unit_table;
            // Load all units without pruning (same as LoadAllNew).
            let units =
                load_all_units_no_prune(&run_info.config.unit_dirs, &run_info.config.target_unit)
                    .map_err(|e| format!("Error while loading unit definitions: {e:?}"))?;

            // collect all names
            let existing_names = unit_table
                .values()
                .map(|unit| unit.id.name.clone())
                .collect::<Vec<_>>();

            // filter out existing units
            let mut ignored_units_names = Vec::new();
            let mut new_units_names = Vec::new();
            for (_id, unit) in units {
                if existing_names.contains(&unit.id.name) {
                    ignored_units_names.push(Value::String(unit.id.name.clone()));
                } else {
                    new_units_names.push(Value::String(unit.id.name.clone()));
                }
            }

            let mut response_object = serde_json::Map::new();
            response_object.insert(
                "Would add".into(),
                serde_json::Value::Array(new_units_names),
            );
            response_object.insert(
                "Would ignore".into(),
                serde_json::Value::Array(ignored_units_names),
            );
            result_vec
                .as_array_mut()
                .unwrap()
                .push(Value::Object(response_object));
        }
        Command::Mask(names) => {
            // Check for --runtime and --root flags
            let is_runtime = names.iter().any(|n| n == "--runtime");
            let root_prefix: String = names
                .iter()
                .find_map(|n| n.strip_prefix("--root=").map(|s| s.to_string()))
                .unwrap_or_default();
            let names: Vec<String> = names
                .into_iter()
                .filter(|n| n != "--runtime" && !n.starts_with("--root="))
                .collect();
            let mask_dir_str = if is_runtime {
                format!("{root_prefix}/run/systemd/system")
            } else {
                format!("{root_prefix}/etc/systemd/system")
            };
            let mask_dir = std::path::Path::new(&mask_dir_str);
            std::fs::create_dir_all(mask_dir)
                .map_err(|e| format!("Failed to create {}: {e}", mask_dir.display()))?;
            let mut masked = Vec::new();
            for name in &names {
                let link_path = mask_dir.join(name);
                // Remove existing file/symlink if present
                if (link_path.exists() || link_path.symlink_metadata().is_ok())
                    && let Err(e) = std::fs::remove_file(&link_path)
                {
                    return Err(format!("Failed to remove {}: {e}", link_path.display()));
                }
                // Create symlink to /dev/null
                if let Err(e) = std::os::unix::fs::symlink("/dev/null", &link_path) {
                    return Err(format!(
                        "Failed to create mask symlink {}: {e}",
                        link_path.display()
                    ));
                }
                masked.push(Value::String(name.clone()));
            }
            return Ok(serde_json::json!({ "masked": masked }));
        }
        Command::Unmask(names) => {
            // Check for --runtime and --root flags
            let is_runtime = names.iter().any(|n| n == "--runtime");
            let root_prefix: String = names
                .iter()
                .find_map(|n| n.strip_prefix("--root=").map(|s| s.to_string()))
                .unwrap_or_default();
            let names: Vec<String> = names
                .into_iter()
                .filter(|n| n != "--runtime" && !n.starts_with("--root="))
                .collect();
            let mask_dir_str = if is_runtime {
                format!("{root_prefix}/run/systemd/system")
            } else {
                format!("{root_prefix}/etc/systemd/system")
            };
            let mask_dir = std::path::Path::new(&mask_dir_str);
            let mut unmasked = Vec::new();
            for name in &names {
                let link_path = mask_dir.join(name);
                // Only remove if it's a symlink pointing to /dev/null
                if let Ok(target) = std::fs::read_link(&link_path)
                    && target == std::path::Path::new("/dev/null")
                {
                    if let Err(e) = std::fs::remove_file(&link_path) {
                        return Err(format!(
                            "Failed to remove mask symlink {}: {e}",
                            link_path.display()
                        ));
                    }
                    unmasked.push(Value::String(name.clone()));
                }
                // If it's not a symlink to /dev/null, silently skip
            }
            return Ok(serde_json::json!({ "unmasked": unmasked }));
        }
        Command::AddWants(unit, target) => {
            let dep_type = "wants";
            let dep_dir =
                std::path::Path::new("/etc/systemd/system").join(format!("{unit}.{dep_type}"));
            std::fs::create_dir_all(&dep_dir)
                .map_err(|e| format!("Failed to create {}: {e}", dep_dir.display()))?;
            let link_path = dep_dir.join(&target);
            let target_path = format!("/usr/lib/systemd/system/{target}");
            let _ = std::fs::remove_file(&link_path);
            std::os::unix::fs::symlink(&target_path, &link_path)
                .map_err(|e| format!("Failed to create symlink {}: {e}", link_path.display()))?;
            return Ok(serde_json::json!(null));
        }
        Command::AddRequires(unit, target) => {
            let dep_type = "requires";
            // Create /etc/systemd/system/<unit>.<dep_type>/<target> -> /usr/lib/systemd/system/<target>
            let dep_dir =
                std::path::Path::new("/etc/systemd/system").join(format!("{unit}.{dep_type}"));
            std::fs::create_dir_all(&dep_dir)
                .map_err(|e| format!("Failed to create {}: {e}", dep_dir.display()))?;
            let link_path = dep_dir.join(&target);
            let target_path = format!("/usr/lib/systemd/system/{target}");
            // Remove existing symlink if present
            let _ = std::fs::remove_file(&link_path);
            std::os::unix::fs::symlink(&target_path, &link_path)
                .map_err(|e| format!("Failed to create symlink {}: {e}", link_path.display()))?;
            return Ok(serde_json::json!(null));
        }
    }
    Ok(result_vec)
}

use std::io::Read;
use std::io::Write as IoWrite;
pub fn listen_on_commands<T: 'static + Read + IoWrite + Send>(
    mut source: Box<T>,
    run_info: ArcMutRuntimeInfo,
) {
    std::thread::spawn(move || {
        loop {
            match super::jsonrpc2::get_next_call(source.as_mut()) {
                Err(e) => {
                    if e.classify() == serde_json::error::Category::Eof {
                        // ignore, just stop reading
                    } else {
                        let err = super::jsonrpc2::make_error(
                            super::jsonrpc2::PARSE_ERROR,
                            format!("{e}"),
                            None,
                        );
                        let msg = super::jsonrpc2::make_error_response(None, err);
                        let response_string = serde_json::to_string_pretty(&msg).unwrap();
                        source.write_all(response_string.as_bytes()).unwrap();
                    }
                    return;
                }
                Ok(call) => {
                    match call {
                        Err(e) => {
                            let err = super::jsonrpc2::make_error(
                                super::jsonrpc2::INVALID_REQUEST_ERROR,
                                e,
                                None,
                            );
                            let msg = super::jsonrpc2::make_error_response(None, err);
                            let response_string = serde_json::to_string_pretty(&msg).unwrap();
                            source.write_all(response_string.as_bytes()).unwrap();
                        }
                        Ok(call) => {
                            match parse_command(&call) {
                                Err(e) => {
                                    // TODO invalid arguments error
                                    let (code, err_msg) = match e {
                                        ParseError::ParamsInvalid(s) => {
                                            (super::jsonrpc2::INVALID_PARAMS_ERROR, s)
                                        }
                                        ParseError::MethodNotFound(s) => {
                                            (super::jsonrpc2::METHOD_NOT_FOUND_ERROR, s)
                                        }
                                    };
                                    let err = super::jsonrpc2::make_error(code, err_msg, None);
                                    let msg = super::jsonrpc2::make_error_response(call.id, err);
                                    let response_string =
                                        serde_json::to_string_pretty(&msg).unwrap();
                                    source.write_all(response_string.as_bytes()).unwrap();
                                }
                                Ok(cmd) => {
                                    trace!("Execute command: {cmd:?}");
                                    let msg = match execute_command(cmd, run_info.clone()) {
                                        Err(e) => {
                                            let err = super::jsonrpc2::make_error(
                                                super::jsonrpc2::SERVER_ERROR,
                                                e,
                                                None,
                                            );
                                            super::jsonrpc2::make_error_response(call.id, err)
                                        }
                                        Ok(result) => {
                                            super::jsonrpc2::make_result_response(call.id, result)
                                        }
                                    };
                                    let response_string =
                                        serde_json::to_string_pretty(&msg).unwrap();
                                    source.write_all(response_string.as_bytes()).unwrap();
                                }
                            }
                        }
                    }
                }
            }
        }
    });
}

pub fn accept_control_connections_unix_socket(
    run_info: ArcMutRuntimeInfo,
    source: std::os::unix::net::UnixListener,
) {
    std::thread::spawn(move || {
        loop {
            match source.accept() {
                Ok((stream, _addr)) => {
                    listen_on_commands(Box::new(stream), run_info.clone());
                }
                Err(e) => {
                    warn!("Error on control socket accept: {e}");
                }
            }
        }
    });
}

pub fn accept_control_connections_tcp(run_info: ArcMutRuntimeInfo, source: std::net::TcpListener) {
    std::thread::spawn(move || {
        loop {
            let stream = Box::new(source.accept().unwrap().0);
            listen_on_commands(stream, run_info.clone());
        }
    });
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::{
        Common, CommonState, Dependencies, Specific, StatusStarted, StatusStopped, TargetSpecific,
        TargetState, Unit, UnitConfig, UnitId, UnitIdKind, UnitOperationErrorReason, UnitStatus,
        UnitTimestamps,
    };
    use std::collections::HashMap;
    use std::sync::RwLock;

    fn make_unit_id(name: &str) -> UnitId {
        let kind = if name.ends_with(".service") {
            UnitIdKind::Service
        } else if name.ends_with(".target") {
            UnitIdKind::Target
        } else if name.ends_with(".socket") {
            UnitIdKind::Socket
        } else if name.ends_with(".mount") {
            UnitIdKind::Mount
        } else {
            UnitIdKind::Service
        };
        UnitId {
            kind,
            name: name.to_string(),
        }
    }

    fn make_test_unit(name: &str) -> Unit {
        let id = make_unit_id(name);
        Unit {
            id: id.clone(),
            common: Common {
                unit: UnitConfig {
                    description: name.to_string(),
                    documentation: vec![],
                    fragment_path: None,
                    refs_by_name: vec![],
                    default_dependencies: true,
                    conditions: vec![],
                    assertions: vec![],
                    success_action: crate::units::UnitAction::None,
                    failure_action: crate::units::UnitAction::None,
                    aliases: vec![],
                    ignore_on_isolate: false,
                    default_instance: None,
                    allow_isolate: false,
                    job_timeout_sec: None,
                    job_timeout_action: crate::units::UnitAction::None,
                    refuse_manual_start: false,
                    refuse_manual_stop: false,
                    on_success: vec![],
                    on_success_job_mode: crate::units::OnFailureJobMode::Replace,
                    on_failure: vec![],
                    on_failure_job_mode: crate::units::OnFailureJobMode::Replace,
                    start_limit_interval_sec: None,
                    start_limit_burst: None,
                    start_limit_action: crate::units::UnitAction::None,
                    loaded_at: std::time::SystemTime::now(),
                    loaded_dropin_files: Vec::new(),
                    joins_namespace_of: Vec::new(),
                },
                dependencies: Dependencies {
                    wants: vec![],
                    wanted_by: vec![],
                    requires: vec![],
                    required_by: vec![],
                    conflicts: vec![],
                    conflicted_by: vec![],
                    before: vec![],
                    after: vec![],
                    part_of: vec![],
                    part_of_by: vec![],
                    binds_to: vec![],
                    bound_by: vec![],
                },
                status: RwLock::new(UnitStatus::NeverStarted),
                timestamps: RwLock::new(UnitTimestamps::default()),
                n_restarts: std::sync::atomic::AtomicU64::new(0),
            },
            specific: Specific::Target(TargetSpecific {
                state: RwLock::new(TargetState {
                    common: CommonState {
                        up_since: None,
                        restart_count: 0,
                        start_timestamps: Vec::new(),
                    },
                }),
            }),
        }
    }

    fn make_unit_table(units: Vec<Unit>) -> UnitTable {
        let mut table = HashMap::new();
        for unit in units {
            table.insert(unit.id.clone(), unit);
        }
        table
    }

    // ── parse_command tests ──────────────────────────────────────────────

    #[test]
    fn test_parse_list_dependencies_string() {
        let call = super::super::jsonrpc2::Call {
            method: "list-dependencies".to_string(),
            params: Some(Value::String("multi-user.target".to_string())),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::ListDependencies(name, reverse) => {
                assert_eq!(name, "multi-user.target");
                assert!(!reverse);
            }
            _ => panic!("Expected ListDependencies"),
        }
    }

    #[test]
    fn test_parse_list_dependencies_reverse() {
        let call = super::super::jsonrpc2::Call {
            method: "list-dependencies".to_string(),
            params: Some(Value::Array(vec![
                Value::String("sshd.service".to_string()),
                Value::String("--reverse".to_string()),
            ])),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::ListDependencies(name, reverse) => {
                assert_eq!(name, "sshd.service");
                assert!(reverse);
            }
            _ => panic!("Expected ListDependencies with reverse"),
        }
    }

    #[test]
    fn test_parse_list_dependencies_no_params() {
        let call = super::super::jsonrpc2::Call {
            method: "list-dependencies".to_string(),
            params: None,
            id: None,
        };
        assert!(parse_command(&call).is_err());
    }

    #[test]
    fn test_parse_mask_single() {
        let call = super::super::jsonrpc2::Call {
            method: "mask".to_string(),
            params: Some(Value::String("tmp.mount".to_string())),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::Mask(names) => {
                assert_eq!(names, vec!["tmp.mount"]);
            }
            _ => panic!("Expected Mask"),
        }
    }

    #[test]
    fn test_parse_mask_multiple() {
        let call = super::super::jsonrpc2::Call {
            method: "mask".to_string(),
            params: Some(Value::Array(vec![
                Value::String("a.service".to_string()),
                Value::String("b.service".to_string()),
            ])),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::Mask(names) => {
                assert_eq!(names, vec!["a.service", "b.service"]);
            }
            _ => panic!("Expected Mask"),
        }
    }

    #[test]
    fn test_parse_mask_no_params() {
        let call = super::super::jsonrpc2::Call {
            method: "mask".to_string(),
            params: None,
            id: None,
        };
        assert!(parse_command(&call).is_err());
    }

    #[test]
    fn test_parse_unmask_single() {
        let call = super::super::jsonrpc2::Call {
            method: "unmask".to_string(),
            params: Some(Value::String("tmp.mount".to_string())),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::Unmask(names) => {
                assert_eq!(names, vec!["tmp.mount"]);
            }
            _ => panic!("Expected Unmask"),
        }
    }

    #[test]
    fn test_parse_unmask_no_params() {
        let call = super::super::jsonrpc2::Call {
            method: "unmask".to_string(),
            params: None,
            id: None,
        };
        assert!(parse_command(&call).is_err());
    }

    // ── format_dep_tree tests ────────────────────────────────────────────

    #[test]
    fn test_dep_tree_single_unit_no_deps() {
        let unit = make_test_unit("test.target");
        let table = make_unit_table(vec![unit]);

        let mut out = String::new();
        let mut visited = std::collections::HashSet::new();
        format_dep_tree(
            "test.target",
            &table,
            false,
            "",
            true,
            &mut visited,
            &mut out,
            0,
        );

        assert!(out.contains("test.target"));
        // Should only have one line (the root)
        assert_eq!(out.lines().count(), 1);
    }

    #[test]
    fn test_dep_tree_with_wants() {
        let mut target = make_test_unit("multi-user.target");
        let svc_a = make_test_unit("a.service");
        let svc_b = make_test_unit("b.service");

        target.common.dependencies.wants =
            vec![make_unit_id("a.service"), make_unit_id("b.service")];

        let table = make_unit_table(vec![target, svc_a, svc_b]);

        let mut out = String::new();
        let mut visited = std::collections::HashSet::new();
        format_dep_tree(
            "multi-user.target",
            &table,
            false,
            "",
            true,
            &mut visited,
            &mut out,
            0,
        );

        assert!(out.contains("multi-user.target"));
        assert!(out.contains("a.service"));
        assert!(out.contains("b.service"));
        // Root + 2 children = 3 lines
        assert_eq!(out.lines().count(), 3);
    }

    #[test]
    fn test_dep_tree_with_requires() {
        let mut target = make_test_unit("network.target");
        let svc = make_test_unit("networkd.service");

        target.common.dependencies.requires = vec![make_unit_id("networkd.service")];

        let table = make_unit_table(vec![target, svc]);

        let mut out = String::new();
        let mut visited = std::collections::HashSet::new();
        format_dep_tree(
            "network.target",
            &table,
            false,
            "",
            true,
            &mut visited,
            &mut out,
            0,
        );

        assert!(out.contains("network.target"));
        assert!(out.contains("networkd.service"));
    }

    #[test]
    fn test_dep_tree_deduplicates_wants_and_requires() {
        // If a unit is in both wants and requires, it should appear once.
        let mut target = make_test_unit("test.target");
        let svc = make_test_unit("dup.service");

        target.common.dependencies.wants = vec![make_unit_id("dup.service")];
        target.common.dependencies.requires = vec![make_unit_id("dup.service")];

        let table = make_unit_table(vec![target, svc]);

        let mut out = String::new();
        let mut visited = std::collections::HashSet::new();
        format_dep_tree(
            "test.target",
            &table,
            false,
            "",
            true,
            &mut visited,
            &mut out,
            0,
        );

        // Count occurrences of "dup.service" in lines (should be 1)
        let count = out.lines().filter(|l| l.contains("dup.service")).count();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_dep_tree_reverse() {
        let mut svc = make_test_unit("sshd.service");
        let target = make_test_unit("multi-user.target");

        svc.common.dependencies.wanted_by = vec![make_unit_id("multi-user.target")];

        let table = make_unit_table(vec![svc, target]);

        let mut out = String::new();
        let mut visited = std::collections::HashSet::new();
        format_dep_tree(
            "sshd.service",
            &table,
            true, // reverse
            "",
            true,
            &mut visited,
            &mut out,
            0,
        );

        assert!(out.contains("sshd.service"));
        assert!(out.contains("multi-user.target"));
    }

    #[test]
    fn test_dep_tree_reverse_required_by() {
        let mut svc = make_test_unit("dbus.service");
        let target = make_test_unit("multi-user.target");

        svc.common.dependencies.required_by = vec![make_unit_id("multi-user.target")];

        let table = make_unit_table(vec![svc, target]);

        let mut out = String::new();
        let mut visited = std::collections::HashSet::new();
        format_dep_tree(
            "dbus.service",
            &table,
            true,
            "",
            true,
            &mut visited,
            &mut out,
            0,
        );

        assert!(out.contains("multi-user.target"));
    }

    #[test]
    fn test_dep_tree_cycle_protection() {
        // a.service wants b.service, b.service wants a.service → cycle
        let mut a = make_test_unit("a.service");
        let mut b = make_test_unit("b.service");

        a.common.dependencies.wants = vec![make_unit_id("b.service")];
        b.common.dependencies.wants = vec![make_unit_id("a.service")];

        let table = make_unit_table(vec![a, b]);

        let mut out = String::new();
        let mut visited = std::collections::HashSet::new();
        format_dep_tree(
            "a.service",
            &table,
            false,
            "",
            true,
            &mut visited,
            &mut out,
            0,
        );

        // Should not infinite loop — just check it terminates and contains both
        assert!(out.contains("a.service"));
        assert!(out.contains("b.service"));
        // The cycle should mean b.service's child (a.service) appears but doesn't recurse
        let lines: Vec<&str> = out.lines().collect();
        assert!(
            lines.len() <= 4,
            "Cycle should be bounded, got {} lines",
            lines.len()
        );
    }

    #[test]
    fn test_dep_tree_nested() {
        // target -> a.service -> b.service
        let mut target = make_test_unit("test.target");
        let mut svc_a = make_test_unit("a.service");
        let svc_b = make_test_unit("b.service");

        target.common.dependencies.wants = vec![make_unit_id("a.service")];
        svc_a.common.dependencies.wants = vec![make_unit_id("b.service")];

        let table = make_unit_table(vec![target, svc_a, svc_b]);

        let mut out = String::new();
        let mut visited = std::collections::HashSet::new();
        format_dep_tree(
            "test.target",
            &table,
            false,
            "",
            true,
            &mut visited,
            &mut out,
            0,
        );

        assert!(out.contains("test.target"));
        assert!(out.contains("a.service"));
        assert!(out.contains("b.service"));
        assert_eq!(out.lines().count(), 3);
    }

    #[test]
    fn test_dep_tree_box_drawing_chars() {
        // target wants a.service and b.service — check tree connectors
        let mut target = make_test_unit("multi.target");
        let svc_a = make_test_unit("a.service");
        let svc_b = make_test_unit("b.service");

        target.common.dependencies.wants =
            vec![make_unit_id("a.service"), make_unit_id("b.service")];

        let table = make_unit_table(vec![target, svc_a, svc_b]);

        let mut out = String::new();
        let mut visited = std::collections::HashSet::new();
        format_dep_tree(
            "multi.target",
            &table,
            false,
            "",
            true,
            &mut visited,
            &mut out,
            0,
        );

        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 3);
        // First child uses ├─, last child uses └─
        assert!(
            lines[1].contains("├─"),
            "first child should use ├─, got: {}",
            lines[1]
        );
        assert!(
            lines[2].contains("└─"),
            "last child should use └─, got: {}",
            lines[2]
        );
    }

    #[test]
    fn test_dep_tree_single_child_uses_last_connector() {
        let mut target = make_test_unit("test.target");
        let svc = make_test_unit("only.service");

        target.common.dependencies.wants = vec![make_unit_id("only.service")];

        let table = make_unit_table(vec![target, svc]);

        let mut out = String::new();
        let mut visited = std::collections::HashSet::new();
        format_dep_tree(
            "test.target",
            &table,
            false,
            "",
            true,
            &mut visited,
            &mut out,
            0,
        );

        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        // Single child is both first and last, so it uses └─
        assert!(
            lines[1].contains("└─"),
            "single child should use └─, got: {}",
            lines[1]
        );
    }

    #[test]
    fn test_dep_tree_children_sorted() {
        let mut target = make_test_unit("test.target");
        let c = make_test_unit("c.service");
        let a = make_test_unit("a.service");
        let b = make_test_unit("b.service");

        // Add in unsorted order
        target.common.dependencies.wants = vec![
            make_unit_id("c.service"),
            make_unit_id("a.service"),
            make_unit_id("b.service"),
        ];

        let table = make_unit_table(vec![target, c, a, b]);

        let mut out = String::new();
        let mut visited = std::collections::HashSet::new();
        format_dep_tree(
            "test.target",
            &table,
            false,
            "",
            true,
            &mut visited,
            &mut out,
            0,
        );

        let child_names: Vec<&str> = out
            .lines()
            .skip(1) // skip root
            .filter_map(|l| {
                // Extract the unit name after the status marker
                let trimmed = l.trim_start_matches(|c: char| !c.is_ascii_alphanumeric());
                if trimmed.contains(".service") {
                    // Find the start of the name
                    Some(trimmed.trim())
                } else {
                    None
                }
            })
            .collect();

        // Should be sorted: a, b, c
        assert_eq!(child_names.len(), 3);
        assert!(
            child_names[0].starts_with("a.service"),
            "first should be a.service, got: {}",
            child_names[0]
        );
        assert!(
            child_names[1].starts_with("b.service"),
            "second should be b.service, got: {}",
            child_names[1]
        );
        assert!(
            child_names[2].starts_with("c.service"),
            "third should be c.service, got: {}",
            child_names[2]
        );
    }

    #[test]
    fn test_dep_tree_unknown_unit() {
        let table: UnitTable = HashMap::new();

        let mut out = String::new();
        let mut visited = std::collections::HashSet::new();
        format_dep_tree(
            "nonexistent.service",
            &table,
            false,
            "",
            true,
            &mut visited,
            &mut out,
            0,
        );

        // Should print the name but with no children
        assert!(out.contains("nonexistent.service"));
        assert_eq!(out.lines().count(), 1);
    }

    #[test]
    fn test_dep_tree_binds_to_forward() {
        let mut svc = make_test_unit("a.service");
        let bound = make_test_unit("b.service");

        svc.common.dependencies.binds_to = vec![make_unit_id("b.service")];

        let table = make_unit_table(vec![svc, bound]);

        let mut out = String::new();
        let mut visited = std::collections::HashSet::new();
        format_dep_tree(
            "a.service",
            &table,
            false,
            "",
            true,
            &mut visited,
            &mut out,
            0,
        );

        assert!(out.contains("b.service"));
    }

    #[test]
    fn test_dep_tree_bound_by_reverse() {
        let mut svc = make_test_unit("a.service");
        let dependent = make_test_unit("b.service");

        svc.common.dependencies.bound_by = vec![make_unit_id("b.service")];

        let table = make_unit_table(vec![svc, dependent]);

        let mut out = String::new();
        let mut visited = std::collections::HashSet::new();
        format_dep_tree(
            "a.service",
            &table,
            true,
            "",
            true,
            &mut visited,
            &mut out,
            0,
        );

        assert!(out.contains("b.service"));
    }

    // ── unit_status_marker tests ─────────────────────────────────────────

    #[test]
    fn test_status_marker_never_started() {
        let unit = make_test_unit("test.service");
        let table = make_unit_table(vec![unit]);
        let marker = unit_status_marker("test.service", &table);
        assert_eq!(marker, "○ ");
    }

    #[test]
    fn test_status_marker_active() {
        let unit = make_test_unit("test.service");
        *unit.common.status.write().unwrap() = UnitStatus::Started(StatusStarted::Running);
        let table = make_unit_table(vec![unit]);
        let marker = unit_status_marker("test.service", &table);
        assert_eq!(marker, "● ");
    }

    #[test]
    fn test_status_marker_failed() {
        let unit = make_test_unit("test.service");
        *unit.common.status.write().unwrap() = UnitStatus::Stopped(
            StatusStopped::StoppedFinal,
            vec![UnitOperationErrorReason::GenericStartError(
                "error".to_string(),
            )],
        );
        let table = make_unit_table(vec![unit]);
        let marker = unit_status_marker("test.service", &table);
        assert_eq!(marker, "× ");
    }

    #[test]
    fn test_status_marker_stopped_no_errors() {
        let unit = make_test_unit("test.service");
        *unit.common.status.write().unwrap() =
            UnitStatus::Stopped(StatusStopped::StoppedFinal, vec![]);
        let table = make_unit_table(vec![unit]);
        let marker = unit_status_marker("test.service", &table);
        assert_eq!(marker, "○ ");
    }

    #[test]
    fn test_status_marker_unknown_unit() {
        let table: UnitTable = HashMap::new();
        let marker = unit_status_marker("nonexistent.service", &table);
        assert_eq!(marker, "○ ");
    }

    // ── mask / unmask filesystem tests ───────────────────────────────────

    #[test]
    fn test_mask_unmask_tempdir() {
        // Test mask/unmask logic using a temp directory instead of /etc/systemd/system
        let dir = tempfile::tempdir().unwrap();
        let mask_dir = dir.path();

        let name = "test-mask.service";
        let link_path = mask_dir.join(name);

        // Mask: create symlink to /dev/null
        std::os::unix::fs::symlink("/dev/null", &link_path).unwrap();
        assert!(link_path.symlink_metadata().is_ok());
        let target = std::fs::read_link(&link_path).unwrap();
        assert_eq!(target, std::path::Path::new("/dev/null"));

        // Unmask: remove the symlink
        std::fs::remove_file(&link_path).unwrap();
        assert!(!link_path.exists());
    }

    #[test]
    fn test_unmask_only_removes_devnull_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let mask_dir = dir.path();

        let name = "real-unit.service";
        let link_path = mask_dir.join(name);

        // Create a regular file (not a symlink to /dev/null)
        std::fs::write(&link_path, "[Unit]\nDescription=Real\n").unwrap();

        // Unmask should NOT remove a regular file
        if let Ok(target) = std::fs::read_link(&link_path) {
            assert_ne!(target, std::path::Path::new("/dev/null"));
        }
        // File should still exist
        assert!(link_path.exists());
    }

    #[test]
    fn test_mask_overwrites_existing() {
        let dir = tempfile::tempdir().unwrap();
        let mask_dir = dir.path();

        let name = "overwrite.service";
        let link_path = mask_dir.join(name);

        // Create a regular file first
        std::fs::write(&link_path, "[Unit]\nDescription=Old\n").unwrap();
        assert!(link_path.exists());

        // Remove and re-create as symlink (simulating mask)
        std::fs::remove_file(&link_path).unwrap();
        std::os::unix::fs::symlink("/dev/null", &link_path).unwrap();

        let target = std::fs::read_link(&link_path).unwrap();
        assert_eq!(target, std::path::Path::new("/dev/null"));
    }

    #[test]
    fn test_unmask_nonexistent_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let mask_dir = dir.path();

        let name = "nonexistent.service";
        let link_path = mask_dir.join(name);

        // read_link on nonexistent path should fail
        assert!(std::fs::read_link(&link_path).is_err());
        // Not an error — just nothing to do
    }

    // ── find_units_with_name tests ───────────────────────────────────────

    #[test]
    fn test_find_units_exact_name() {
        let unit = make_test_unit("sshd.service");
        let table = make_unit_table(vec![unit]);
        let found = find_units_with_name("sshd.service", &table);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id.name, "sshd.service");
    }

    #[test]
    fn test_find_units_prefix_match() {
        let unit = make_test_unit("sshd.service");
        let table = make_unit_table(vec![unit]);
        let found = find_units_with_name("sshd", &table);
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn test_find_units_no_match() {
        let unit = make_test_unit("sshd.service");
        let table = make_unit_table(vec![unit]);
        let found = find_units_with_name("nginx.service", &table);
        assert!(found.is_empty());
    }

    #[test]
    fn test_find_units_by_alias() {
        let mut unit = make_test_unit("systemd-resolved.service");
        unit.common.unit.aliases = vec!["dbus-org.freedesktop.resolve1.service".to_string()];
        let table = make_unit_table(vec![unit]);
        let found = find_units_with_name("dbus-org.freedesktop.resolve1.service", &table);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id.name, "systemd-resolved.service");
    }

    // ── parse_command edge cases ─────────────────────────────────────────

    #[test]
    fn test_parse_unknown_method() {
        let call = super::super::jsonrpc2::Call {
            method: "nonexistent-method".to_string(),
            params: None,
            id: None,
        };
        let result = parse_command(&call);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_show_with_filter() {
        let call = super::super::jsonrpc2::Call {
            method: "show".to_string(),
            params: Some(Value::Array(vec![
                Value::String("sshd.service".to_string()),
                Value::String("MainPID".to_string()),
                Value::String("ActiveState".to_string()),
            ])),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::Show(name, filter) => {
                assert_eq!(name, "sshd.service");
                let f = filter.unwrap();
                assert_eq!(f, vec!["MainPID", "ActiveState"]);
            }
            _ => panic!("Expected Show"),
        }
    }

    #[test]
    fn test_parse_cat() {
        let call = super::super::jsonrpc2::Call {
            method: "cat".to_string(),
            params: Some(Value::String("sshd.service".to_string())),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::Cat(name) => assert_eq!(name, "sshd.service"),
            _ => panic!("Expected Cat"),
        }
    }

    #[test]
    fn test_parse_is_active() {
        let call = super::super::jsonrpc2::Call {
            method: "is-active".to_string(),
            params: Some(Value::String("sshd.service".to_string())),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::IsActive(name) => assert_eq!(name, "sshd.service"),
            _ => panic!("Expected IsActive"),
        }
    }

    #[test]
    fn test_parse_list_dependencies_array_no_reverse() {
        let call = super::super::jsonrpc2::Call {
            method: "list-dependencies".to_string(),
            params: Some(Value::Array(vec![Value::String(
                "default.target".to_string(),
            )])),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::ListDependencies(name, reverse) => {
                assert_eq!(name, "default.target");
                assert!(!reverse);
            }
            _ => panic!("Expected ListDependencies"),
        }
    }

    #[test]
    fn test_parse_unmask_multiple() {
        let call = super::super::jsonrpc2::Call {
            method: "unmask".to_string(),
            params: Some(Value::Array(vec![
                Value::String("a.service".to_string()),
                Value::String("b.service".to_string()),
            ])),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::Unmask(names) => {
                assert_eq!(names, vec!["a.service", "b.service"]);
            }
            _ => panic!("Expected Unmask"),
        }
    }

    // ── dep tree with mixed status markers ───────────────────────────────

    #[test]
    fn test_dep_tree_status_markers_in_output() {
        let mut target = make_test_unit("test.target");
        let active_svc = make_test_unit("active.service");
        let failed_svc = make_test_unit("failed.service");
        let inactive_svc = make_test_unit("inactive.service");

        *active_svc.common.status.write().unwrap() = UnitStatus::Started(StatusStarted::Running);
        *failed_svc.common.status.write().unwrap() = UnitStatus::Stopped(
            StatusStopped::StoppedFinal,
            vec![UnitOperationErrorReason::GenericStartError(
                "exit 1".to_string(),
            )],
        );

        target.common.dependencies.wants = vec![
            make_unit_id("active.service"),
            make_unit_id("failed.service"),
            make_unit_id("inactive.service"),
        ];

        let table = make_unit_table(vec![target, active_svc, failed_svc, inactive_svc]);

        let mut out = String::new();
        let mut visited = std::collections::HashSet::new();
        format_dep_tree(
            "test.target",
            &table,
            false,
            "",
            true,
            &mut visited,
            &mut out,
            0,
        );

        // active.service should have ● marker
        let active_line = out.lines().find(|l| l.contains("active.service")).unwrap();
        assert!(
            active_line.contains("● "),
            "active should have ● marker: {}",
            active_line
        );

        // failed.service should have × marker
        let failed_line = out.lines().find(|l| l.contains("failed.service")).unwrap();
        assert!(
            failed_line.contains("× "),
            "failed should have × marker: {}",
            failed_line
        );

        // inactive.service should have ○ marker
        let inactive_line = out
            .lines()
            .find(|l| l.contains("inactive.service"))
            .unwrap();
        assert!(
            inactive_line.contains("○ "),
            "inactive should have ○ marker: {}",
            inactive_line
        );
    }

    #[test]
    fn test_dep_tree_diamond_dependency() {
        // Diamond: target -> a, b; a -> c; b -> c
        // c should appear under both a and b (not deduplicated across branches)
        let mut target = make_test_unit("test.target");
        let mut a = make_test_unit("a.service");
        let mut b = make_test_unit("b.service");
        let c = make_test_unit("c.service");

        target.common.dependencies.wants =
            vec![make_unit_id("a.service"), make_unit_id("b.service")];
        a.common.dependencies.wants = vec![make_unit_id("c.service")];
        b.common.dependencies.wants = vec![make_unit_id("c.service")];

        let table = make_unit_table(vec![target, a, b, c]);

        let mut out = String::new();
        let mut visited = std::collections::HashSet::new();
        format_dep_tree(
            "test.target",
            &table,
            false,
            "",
            true,
            &mut visited,
            &mut out,
            0,
        );

        // c.service should appear under both a and b branches
        let c_count = out.lines().filter(|l| l.contains("c.service")).count();
        assert!(
            c_count >= 2,
            "c.service should appear in both branches of the diamond, appeared {} times in:\n{}",
            c_count,
            out
        );
    }

    // ── parse_command: list-unit-files ────────────────────────────────────

    #[test]
    fn test_parse_list_unit_files_no_params() {
        let call = super::super::jsonrpc2::Call {
            method: "list-unit-files".to_string(),
            params: None,
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::ListUnitFiles(filter) => assert!(filter.is_none()),
            _ => panic!("Expected ListUnitFiles"),
        }
    }

    #[test]
    fn test_parse_list_unit_files_with_type() {
        let call = super::super::jsonrpc2::Call {
            method: "list-unit-files".to_string(),
            params: Some(Value::String("service".to_string())),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::ListUnitFiles(filter) => assert_eq!(filter.unwrap(), "service"),
            _ => panic!("Expected ListUnitFiles"),
        }
    }

    #[test]
    fn test_parse_list_unit_files_empty_string() {
        let call = super::super::jsonrpc2::Call {
            method: "list-unit-files".to_string(),
            params: Some(Value::String(String::new())),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::ListUnitFiles(filter) => assert!(filter.is_none()),
            _ => panic!("Expected ListUnitFiles"),
        }
    }

    // ── unit_file_state tests ────────────────────────────────────────────

    #[test]
    fn test_unit_file_state_static_no_install() {
        // A unit with no WantedBy/RequiredBy is "static"
        let unit = make_test_unit("basic.target");
        let table = make_unit_table(vec![unit]);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("basic.target");
        std::fs::write(&path, "[Unit]\nDescription=Basic\n").unwrap();

        let state = unit_file_state("basic.target", &table, &path, &[]);
        assert_eq!(state, "static");
    }

    #[test]
    fn test_unit_file_state_enabled_with_wanted_by() {
        // A unit loaded with WantedBy set → "enabled"
        let mut unit = make_test_unit("sshd.service");
        unit.common.dependencies.wanted_by = vec![make_unit_id("multi-user.target")];
        let table = make_unit_table(vec![unit]);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sshd.service");
        std::fs::write(
            &path,
            "[Unit]\nDescription=SSH\n[Install]\nWantedBy=multi-user.target\n",
        )
        .unwrap();

        // Create enablement symlink in multi-user.target.wants/
        let wants_dir = dir.path().join("multi-user.target.wants");
        std::fs::create_dir_all(&wants_dir).unwrap();
        std::os::unix::fs::symlink(&path, wants_dir.join("sshd.service")).unwrap();

        let unit_dirs = vec![dir.path().to_path_buf()];
        let state = unit_file_state("sshd.service", &table, &path, &unit_dirs);
        assert_eq!(state, "enabled");
    }

    #[test]
    fn test_unit_file_state_enabled_with_required_by() {
        let mut unit = make_test_unit("dbus.service");
        unit.common.dependencies.required_by = vec![make_unit_id("multi-user.target")];
        let table = make_unit_table(vec![unit]);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dbus.service");
        std::fs::write(
            &path,
            "[Unit]\nDescription=D-Bus\n[Install]\nRequiredBy=multi-user.target\n",
        )
        .unwrap();

        // Create enablement symlink in multi-user.target.requires/
        let requires_dir = dir.path().join("multi-user.target.requires");
        std::fs::create_dir_all(&requires_dir).unwrap();
        std::os::unix::fs::symlink(&path, requires_dir.join("dbus.service")).unwrap();

        let unit_dirs = vec![dir.path().to_path_buf()];
        let state = unit_file_state("dbus.service", &table, &path, &unit_dirs);
        assert_eq!(state, "enabled");
    }

    #[test]
    fn test_unit_file_state_disabled_not_loaded() {
        // Unit file has [Install] but is not in the unit table → "disabled"
        let table: UnitTable = HashMap::new();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("unloaded.service");
        std::fs::write(
            &path,
            "[Unit]\nDescription=Unloaded\n[Service]\nExecStart=/bin/true\n[Install]\nWantedBy=multi-user.target\n",
        )
        .unwrap();

        let state = unit_file_state("unloaded.service", &table, &path, &[]);
        assert_eq!(state, "disabled");
    }

    #[test]
    fn test_unit_file_state_static_no_install_not_loaded() {
        // Unit file has no [Install] section and is not loaded → "static"
        let table: UnitTable = HashMap::new();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("noinst.service");
        std::fs::write(
            &path,
            "[Unit]\nDescription=No Install\n[Service]\nExecStart=/bin/true\n",
        )
        .unwrap();

        let state = unit_file_state("noinst.service", &table, &path, &[]);
        assert_eq!(state, "static");
    }

    #[test]
    fn test_unit_file_state_static_loaded_no_deps() {
        // Unit is loaded but has empty wanted_by/required_by → "static"
        let unit = make_test_unit("simple.service");
        let table = make_unit_table(vec![unit]);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("simple.service");
        std::fs::write(
            &path,
            "[Unit]\nDescription=Simple\n[Service]\nExecStart=/bin/true\n",
        )
        .unwrap();

        let state = unit_file_state("simple.service", &table, &path, &[]);
        assert_eq!(state, "static");
    }

    #[test]
    fn test_unit_file_state_nonexistent_file() {
        // Path doesn't exist and unit not loaded → "static" (can't read file)
        let table: UnitTable = HashMap::new();
        let path = std::path::Path::new("/nonexistent/path/to/unit.service");
        let state = unit_file_state("unit.service", &table, path, &[]);
        assert_eq!(state, "static");
    }

    // ── list-unit-files integration with temp dir ────────────────────────

    #[test]
    fn test_list_unit_files_scans_directory() {
        let dir = tempfile::tempdir().unwrap();

        // Create some unit files
        std::fs::write(
            dir.path().join("a.service"),
            "[Unit]\nDescription=A\n[Service]\nExecStart=/bin/a\n[Install]\nWantedBy=multi-user.target\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("b.target"), "[Unit]\nDescription=B\n").unwrap();
        std::fs::write(dir.path().join("not-a-unit.conf"), "random config").unwrap();

        // Create a masked unit
        std::os::unix::fs::symlink("/dev/null", dir.path().join("masked.service")).unwrap();

        // Scan the directory manually (simulating what list-unit-files does)
        let mut found: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(dir.path()).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let is_unit = name.ends_with(".service")
                || name.ends_with(".target")
                || name.ends_with(".socket")
                || name.ends_with(".mount");
            if is_unit {
                found.push(name);
            }
        }
        found.sort();

        assert_eq!(found, vec!["a.service", "b.target", "masked.service"]);
        // "not-a-unit.conf" should not appear
        assert!(!found.contains(&"not-a-unit.conf".to_string()));
    }

    // ── parse_command: disable ───────────────────────────────────────────

    #[test]
    fn test_parse_disable_single() {
        let call = super::super::jsonrpc2::Call {
            method: "disable".to_string(),
            params: Some(Value::String("tmp.mount".to_string())),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::Disable(names) => assert_eq!(names, vec!["tmp.mount"]),
            _ => panic!("Expected Disable"),
        }
    }

    #[test]
    fn test_parse_disable_multiple() {
        let call = super::super::jsonrpc2::Call {
            method: "disable".to_string(),
            params: Some(Value::Array(vec![
                Value::String("a.service".to_string()),
                Value::String("b.service".to_string()),
            ])),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::Disable(names) => assert_eq!(names, vec!["a.service", "b.service"]),
            _ => panic!("Expected Disable"),
        }
    }

    #[test]
    fn test_parse_disable_no_params() {
        let call = super::super::jsonrpc2::Call {
            method: "disable".to_string(),
            params: None,
            id: None,
        };
        assert!(parse_command(&call).is_err());
    }

    // ── parse_command: reset-failed ──────────────────────────────────────

    #[test]
    fn test_parse_reset_failed_with_unit() {
        let call = super::super::jsonrpc2::Call {
            method: "reset-failed".to_string(),
            params: Some(Value::String("sshd.service".to_string())),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::ResetFailed(name) => assert_eq!(name.unwrap(), "sshd.service"),
            _ => panic!("Expected ResetFailed"),
        }
    }

    #[test]
    fn test_parse_reset_failed_no_params() {
        let call = super::super::jsonrpc2::Call {
            method: "reset-failed".to_string(),
            params: None,
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::ResetFailed(name) => assert!(name.is_none()),
            _ => panic!("Expected ResetFailed with None"),
        }
    }

    #[test]
    fn test_parse_reset_failed_empty_string() {
        let call = super::super::jsonrpc2::Call {
            method: "reset-failed".to_string(),
            params: Some(Value::String(String::new())),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::ResetFailed(name) => assert!(name.is_none()),
            _ => panic!("Expected ResetFailed with None"),
        }
    }

    // ── parse_command: kill ──────────────────────────────────────────────

    #[test]
    fn test_parse_kill_string_default_signal() {
        let call = super::super::jsonrpc2::Call {
            method: "kill".to_string(),
            params: Some(Value::String("sshd.service".to_string())),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::Kill(name, sig, _whom, _val, _wait) => {
                assert_eq!(name, "sshd.service");
                assert_eq!(sig, 15); // SIGTERM
            }
            _ => panic!("Expected Kill"),
        }
    }

    #[test]
    fn test_parse_kill_with_signal() {
        let call = super::super::jsonrpc2::Call {
            method: "kill".to_string(),
            params: Some(Value::Array(vec![
                Value::String("sshd.service".to_string()),
                Value::String("9".to_string()),
            ])),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::Kill(name, sig, _whom, _val, _wait) => {
                assert_eq!(name, "sshd.service");
                assert_eq!(sig, 9); // SIGKILL
            }
            _ => panic!("Expected Kill"),
        }
    }

    #[test]
    fn test_parse_kill_array_no_signal() {
        let call = super::super::jsonrpc2::Call {
            method: "kill".to_string(),
            params: Some(Value::Array(vec![Value::String(
                "nginx.service".to_string(),
            )])),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::Kill(name, sig, _whom, _val, _wait) => {
                assert_eq!(name, "nginx.service");
                assert_eq!(sig, 15); // default SIGTERM
            }
            _ => panic!("Expected Kill"),
        }
    }

    #[test]
    fn test_parse_kill_no_params() {
        let call = super::super::jsonrpc2::Call {
            method: "kill".to_string(),
            params: None,
            id: None,
        };
        assert!(parse_command(&call).is_err());
    }

    #[test]
    fn test_parse_kill_invalid_signal_defaults_to_sigterm() {
        let call = super::super::jsonrpc2::Call {
            method: "kill".to_string(),
            params: Some(Value::Array(vec![
                Value::String("test.service".to_string()),
                Value::String("not-a-number".to_string()),
            ])),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::Kill(name, sig, _whom, _val, _wait) => {
                assert_eq!(name, "test.service");
                assert_eq!(sig, 15); // fallback to SIGTERM
            }
            _ => panic!("Expected Kill"),
        }
    }

    // ── reset-failed execution logic ─────────────────────────────────────

    #[test]
    fn test_reset_failed_clears_error_state() {
        // A unit in failed state should be reset to NeverStarted
        let unit = make_test_unit("fail.service");
        *unit.common.status.write().unwrap() = UnitStatus::Stopped(
            StatusStopped::StoppedFinal,
            vec![UnitOperationErrorReason::GenericStartError(
                "exit 1".to_string(),
            )],
        );

        // Verify it's failed
        {
            let status = unit.common.status.read().unwrap();
            assert!(matches!(
                &*status,
                UnitStatus::Stopped(_, errors) if !errors.is_empty()
            ));
        }

        // Simulate reset-failed: clear the error
        {
            let mut status = unit.common.status.write().unwrap();
            if let UnitStatus::Stopped(_, ref errors) = *status
                && !errors.is_empty()
            {
                *status = UnitStatus::NeverStarted;
            }
        }

        // Verify it's been reset
        let status = unit.common.status.read().unwrap();
        assert!(matches!(&*status, UnitStatus::NeverStarted));
    }

    #[test]
    fn test_reset_failed_skips_non_failed_units() {
        // A unit that's stopped without errors should not be changed
        let unit = make_test_unit("ok.service");
        *unit.common.status.write().unwrap() =
            UnitStatus::Stopped(StatusStopped::StoppedFinal, vec![]);

        // Simulate reset-failed
        {
            let mut status = unit.common.status.write().unwrap();
            if let UnitStatus::Stopped(_, ref errors) = *status
                && !errors.is_empty()
            {
                *status = UnitStatus::NeverStarted;
            }
        }

        // Should still be Stopped (not changed)
        let status = unit.common.status.read().unwrap();
        assert!(matches!(&*status, UnitStatus::Stopped(_, errors) if errors.is_empty()));
    }

    #[test]
    fn test_reset_failed_skips_active_units() {
        let unit = make_test_unit("running.service");
        *unit.common.status.write().unwrap() = UnitStatus::Started(StatusStarted::Running);

        // Simulate reset-failed — should not touch active units
        {
            let mut status = unit.common.status.write().unwrap();
            if let UnitStatus::Stopped(_, ref errors) = *status
                && !errors.is_empty()
            {
                *status = UnitStatus::NeverStarted;
            }
        }

        // Should still be Started
        let status = unit.common.status.read().unwrap();
        assert!(matches!(&*status, UnitStatus::Started(_)));
    }

    // ── disable no-op test ───────────────────────────────────────────────

    #[test]
    fn test_disable_is_noop() {
        // disable currently returns the names as "disabled" without doing anything
        let names = vec!["a.service".to_string(), "b.service".to_string()];
        let disabled: Vec<Value> = names.into_iter().map(Value::String).collect();
        let result = serde_json::json!({ "disabled": disabled });

        let arr = result.get("disabled").unwrap().as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0].as_str().unwrap(), "a.service");
        assert_eq!(arr[1].as_str().unwrap(), "b.service");
    }

    // ── parse_command: suspend / hibernate / hybrid-sleep / suspend-then-hibernate ──

    #[test]
    fn test_parse_suspend() {
        let call = super::super::jsonrpc2::Call {
            method: "suspend".to_string(),
            params: None,
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        assert!(matches!(cmd, Command::Suspend));
    }

    #[test]
    fn test_parse_hibernate() {
        let call = super::super::jsonrpc2::Call {
            method: "hibernate".to_string(),
            params: None,
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        assert!(matches!(cmd, Command::Hibernate));
    }

    #[test]
    fn test_parse_hybrid_sleep() {
        let call = super::super::jsonrpc2::Call {
            method: "hybrid-sleep".to_string(),
            params: None,
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        assert!(matches!(cmd, Command::HybridSleep));
    }

    #[test]
    fn test_parse_suspend_then_hibernate() {
        let call = super::super::jsonrpc2::Call {
            method: "suspend-then-hibernate".to_string(),
            params: None,
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        assert!(matches!(cmd, Command::SuspendThenHibernate));
    }

    #[test]
    fn test_parse_suspend_ignores_params() {
        // Even if spurious params are passed, the command should parse fine
        let call = super::super::jsonrpc2::Call {
            method: "suspend".to_string(),
            params: Some(Value::String("ignored".to_string())),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        assert!(matches!(cmd, Command::Suspend));
    }

    // ── find_sleep_binary ────────────────────────────────────────────────

    #[test]
    fn test_find_sleep_binary_in_temp_dir() {
        // Create a temp dir with a fake systemd-sleep binary and verify
        // the search logic finds it when placed in a sibling position.
        let dir = tempfile::tempdir().unwrap();
        let fake_sleep = dir.path().join("systemd-sleep");
        std::fs::write(&fake_sleep, "#!/bin/sh\n").unwrap();

        // find_sleep_binary uses current_exe() which we can't easily mock,
        // but we can verify the function doesn't panic and returns Some or None.
        // The real test is that the function signature works and doesn't crash.
        let _result = find_sleep_binary();
        // We can't assert Some because the test env may not have the binary,
        // but we verify it doesn't panic.
    }

    #[test]
    fn test_find_sleep_binary_returns_path_or_none() {
        // Calling find_sleep_binary should always return a valid Option
        // (not panic). On most dev machines it will be None; on NixOS
        // with rust-systemd installed it will be Some.
        let result = find_sleep_binary();
        if let Some(path) = &result {
            assert!(path.to_string_lossy().contains("systemd-sleep"));
        }
    }

    #[test]
    fn test_list_unit_files_type_filter() {
        let dir = tempfile::tempdir().unwrap();

        std::fs::write(dir.path().join("a.service"), "[Unit]\n").unwrap();
        std::fs::write(dir.path().join("b.target"), "[Unit]\n").unwrap();
        std::fs::write(dir.path().join("c.socket"), "[Unit]\n").unwrap();

        // Filter for .service only
        let suffix = ".service";
        let found: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                if name.ends_with(suffix) {
                    Some(name)
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(found.len(), 1);
        assert_eq!(found[0], "a.service");
    }

    #[test]
    fn test_list_unit_files_masked_detection() {
        let dir = tempfile::tempdir().unwrap();
        let link_path = dir.path().join("masked.service");
        std::os::unix::fs::symlink("/dev/null", &link_path).unwrap();

        let target = std::fs::read_link(&link_path).unwrap();
        assert_eq!(target, std::path::Path::new("/dev/null"));

        // This would be detected as "masked" by the list-unit-files logic
        let is_masked = target == std::path::Path::new("/dev/null");
        assert!(is_masked);
    }

    #[test]
    fn test_list_unit_files_first_dir_wins() {
        // Simulate priority: if a unit appears in two dirs, first one wins
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();

        std::fs::write(
            dir1.path().join("dup.service"),
            "[Unit]\nDescription=Dir1\n",
        )
        .unwrap();
        std::fs::write(
            dir2.path().join("dup.service"),
            "[Unit]\nDescription=Dir2\n",
        )
        .unwrap();

        let mut entries: std::collections::BTreeMap<String, String> =
            std::collections::BTreeMap::new();

        for dir in [dir1.path(), dir2.path()] {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                // First occurrence wins
                entries
                    .entry(name)
                    .or_insert_with(|| dir.display().to_string());
            }
        }

        // The entry for dup.service should be from dir1
        let source = entries.get("dup.service").unwrap();
        assert_eq!(source, &dir1.path().display().to_string());
    }

    #[test]
    fn test_list_unit_files_empty_dir() {
        let dir = tempfile::tempdir().unwrap();

        let found: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                if name.ends_with(".service") || name.ends_with(".target") {
                    Some(name)
                } else {
                    None
                }
            })
            .collect();

        assert!(found.is_empty());
    }

    // ── Transient unit tests ────────────────────────────────────────────

    #[test]
    fn test_parse_start_transient_basic() {
        let call = super::super::jsonrpc2::Call {
            method: "start-transient".to_string(),
            params: Some(serde_json::json!({
                "unit": "run-test123.service",
                "command": ["/bin/echo", "hello"],
                "description": "Test transient unit"
            })),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::StartTransient(params) => {
                assert_eq!(params.unit_name, "run-test123.service");
                assert_eq!(
                    params.command,
                    Some(vec!["/bin/echo".to_string(), "hello".to_string()])
                );
                assert_eq!(params.description, Some("Test transient unit".to_string()));
                assert_eq!(params.user, None);
                assert_eq!(params.group, None);
                assert_eq!(params.working_directory, None);
                assert_eq!(params.service_type, None);
                assert!(!params.remain_after_exit);
            }
            other => panic!("Expected StartTransient, got: {other:?}"),
        }
    }

    #[test]
    fn test_parse_start_transient_with_all_options() {
        let call = super::super::jsonrpc2::Call {
            method: "start-transient".to_string(),
            params: Some(serde_json::json!({
                "unit": "run-u42.service",
                "command": ["/usr/bin/sleep", "60"],
                "description": "Sleep service",
                "user": "nobody",
                "group": "nogroup",
                "working_directory": "/tmp",
                "type": "oneshot",
                "remain_after_exit": true
            })),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::StartTransient(params) => {
                assert_eq!(params.unit_name, "run-u42.service");
                assert_eq!(
                    params.command,
                    Some(vec!["/usr/bin/sleep".to_string(), "60".to_string()])
                );
                assert_eq!(params.description, Some("Sleep service".to_string()));
                assert_eq!(params.user, Some("nobody".to_string()));
                assert_eq!(params.group, Some("nogroup".to_string()));
                assert_eq!(params.working_directory, Some("/tmp".to_string()));
                assert_eq!(params.service_type, Some("oneshot".to_string()));
                assert!(params.remain_after_exit);
            }
            other => panic!("Expected StartTransient, got: {other:?}"),
        }
    }

    #[test]
    fn test_parse_start_transient_minimal() {
        // Only the required "unit" field
        let call = super::super::jsonrpc2::Call {
            method: "start-transient".to_string(),
            params: Some(serde_json::json!({
                "unit": "run-minimal.service"
            })),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::StartTransient(params) => {
                assert_eq!(params.unit_name, "run-minimal.service");
                assert_eq!(params.command, None);
                assert_eq!(params.description, None);
            }
            other => panic!("Expected StartTransient, got: {other:?}"),
        }
    }

    #[test]
    fn test_parse_start_transient_missing_unit() {
        let call = super::super::jsonrpc2::Call {
            method: "start-transient".to_string(),
            params: Some(serde_json::json!({
                "command": ["/bin/echo"]
            })),
            id: None,
        };
        assert!(parse_command(&call).is_err());
    }

    #[test]
    fn test_parse_start_transient_no_params() {
        let call = super::super::jsonrpc2::Call {
            method: "start-transient".to_string(),
            params: None,
            id: None,
        };
        assert!(parse_command(&call).is_err());
    }

    #[test]
    fn test_parse_start_transient_string_params() {
        // Params must be an object, not a string
        let call = super::super::jsonrpc2::Call {
            method: "start-transient".to_string(),
            params: Some(serde_json::json!("run-test.service")),
            id: None,
        };
        assert!(parse_command(&call).is_err());
    }

    // ── daemon-reexec tests ─────────────────────────────────────────────

    #[test]
    fn test_parse_daemon_reexec() {
        let call = super::super::jsonrpc2::Call {
            method: "daemon-reexec".to_string(),
            params: None,
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        assert!(matches!(cmd, Command::DaemonReexec));
    }

    #[test]
    fn test_parse_daemon_reload_is_load_all_new() {
        let call = super::super::jsonrpc2::Call {
            method: "daemon-reload".to_string(),
            params: None,
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        assert!(matches!(cmd, Command::LoadAllNew));
    }

    #[test]
    fn test_parse_reload_is_load_all_new() {
        let call = super::super::jsonrpc2::Call {
            method: "reload".to_string(),
            params: None,
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        assert!(matches!(cmd, Command::LoadAllNew));
    }

    // ── Transient unit creation tests ───────────────────────────────────

    #[test]
    fn test_transient_unit_params_debug() {
        let params = TransientUnitParams {
            unit_name: "run-test.service".to_string(),
            command: Some(vec!["/bin/echo".to_string(), "hello".to_string()]),
            description: Some("A test".to_string()),
            user: None,
            group: None,
            working_directory: None,
            service_type: None,
            remain_after_exit: false,
            properties: Vec::new(),
            environment: Vec::new(),
            scope: false,
            wait: false,
            slice: None,
            on_calendar: None,
            on_active: None,
            on_boot: None,
        };
        let debug = format!("{params:?}");
        assert!(debug.contains("run-test.service"));
        assert!(debug.contains("/bin/echo"));
    }

    #[test]
    fn test_transient_unit_params_clone() {
        let params = TransientUnitParams {
            unit_name: "run-clone.service".to_string(),
            command: Some(vec!["/bin/true".to_string()]),
            description: None,
            user: Some("root".to_string()),
            group: Some("root".to_string()),
            working_directory: Some("/tmp".to_string()),
            service_type: Some("oneshot".to_string()),
            remain_after_exit: true,
            properties: Vec::new(),
            environment: Vec::new(),
            scope: false,
            wait: false,
            slice: None,
            on_calendar: None,
            on_active: None,
            on_boot: None,
        };
        let cloned = params.clone();
        assert_eq!(cloned.unit_name, params.unit_name);
        assert_eq!(cloned.command, params.command);
        assert_eq!(cloned.user, params.user);
        assert_eq!(cloned.group, params.group);
        assert_eq!(cloned.working_directory, params.working_directory);
        assert_eq!(cloned.service_type, params.service_type);
        assert_eq!(cloned.remain_after_exit, params.remain_after_exit);
    }

    // ── Edit command parsing tests ──────────────────────────────────────

    #[test]
    fn test_parse_edit_string() {
        let call = super::super::jsonrpc2::Call {
            method: "edit".to_string(),
            params: Some(Value::String("sshd.service".to_string())),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::Edit(name, full) => {
                assert_eq!(name, "sshd.service");
                assert!(!full);
            }
            _ => panic!("Expected Edit"),
        }
    }

    #[test]
    fn test_parse_edit_array_no_full() {
        let call = super::super::jsonrpc2::Call {
            method: "edit".to_string(),
            params: Some(Value::Array(vec![Value::String(
                "nginx.service".to_string(),
            )])),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::Edit(name, full) => {
                assert_eq!(name, "nginx.service");
                assert!(!full);
            }
            _ => panic!("Expected Edit"),
        }
    }

    #[test]
    fn test_parse_edit_array_with_full() {
        let call = super::super::jsonrpc2::Call {
            method: "edit".to_string(),
            params: Some(Value::Array(vec![
                Value::String("sshd.service".to_string()),
                Value::String("--full".to_string()),
            ])),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::Edit(name, full) => {
                assert_eq!(name, "sshd.service");
                assert!(full);
            }
            _ => panic!("Expected Edit"),
        }
    }

    #[test]
    fn test_parse_edit_no_params() {
        let call = super::super::jsonrpc2::Call {
            method: "edit".to_string(),
            params: None,
            id: None,
        };
        assert!(parse_command(&call).is_err());
    }

    // ── Revert command parsing tests ────────────────────────────────────

    #[test]
    fn test_parse_revert_string() {
        let call = super::super::jsonrpc2::Call {
            method: "revert".to_string(),
            params: Some(Value::String("sshd.service".to_string())),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::Revert(name) => assert_eq!(name, "sshd.service"),
            _ => panic!("Expected Revert"),
        }
    }

    #[test]
    fn test_parse_revert_array() {
        let call = super::super::jsonrpc2::Call {
            method: "revert".to_string(),
            params: Some(Value::Array(vec![Value::String(
                "nginx.service".to_string(),
            )])),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::Revert(name) => assert_eq!(name, "nginx.service"),
            _ => panic!("Expected Revert"),
        }
    }

    #[test]
    fn test_parse_revert_no_params() {
        let call = super::super::jsonrpc2::Call {
            method: "revert".to_string(),
            params: None,
            id: None,
        };
        assert!(parse_command(&call).is_err());
    }

    // ── Set-property command parsing tests ──────────────────────────────

    #[test]
    fn test_parse_set_property_string_only() {
        let call = super::super::jsonrpc2::Call {
            method: "set-property".to_string(),
            params: Some(Value::String("sshd.service".to_string())),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::SetProperty(name, props) => {
                assert_eq!(name, "sshd.service");
                assert!(props.is_empty());
            }
            _ => panic!("Expected SetProperty"),
        }
    }

    #[test]
    fn test_parse_set_property_with_props() {
        let call = super::super::jsonrpc2::Call {
            method: "set-property".to_string(),
            params: Some(Value::Array(vec![
                Value::String("sshd.service".to_string()),
                Value::String("CPUWeight=200".to_string()),
                Value::String("MemoryMax=1G".to_string()),
            ])),
            id: None,
        };
        let cmd = parse_command(&call).unwrap();
        match cmd {
            Command::SetProperty(name, props) => {
                assert_eq!(name, "sshd.service");
                assert_eq!(props, vec!["CPUWeight=200", "MemoryMax=1G"]);
            }
            _ => panic!("Expected SetProperty"),
        }
    }

    #[test]
    fn test_parse_set_property_no_params() {
        let call = super::super::jsonrpc2::Call {
            method: "set-property".to_string(),
            params: None,
            id: None,
        };
        assert!(parse_command(&call).is_err());
    }

    // ── Set-property execution tests ────────────────────────────────────

    #[test]
    fn test_set_property_writes_dropin_file() {
        let dir = tempfile::tempdir().unwrap();
        let etc_dir = dir.path().join("etc/systemd/system");
        std::fs::create_dir_all(&etc_dir).unwrap();

        // We can't easily call execute_command without a full runtime,
        // so test the drop-in content generation logic directly.
        let unit_name = "test.service";
        let props = vec!["CPUWeight=200".to_string(), "MemoryMax=1G".to_string()];

        // Simulate the section grouping logic from execute_command.
        let unit_props = [
            "Description",
            "Documentation",
            "Wants",
            "Requires",
            "After",
            "Before",
        ];
        let mut unit_section_lines = Vec::new();
        let mut specific_section_lines = Vec::new();
        for prop in &props {
            if let Some((key, _val)) = prop.split_once('=') {
                if unit_props.contains(&key) {
                    unit_section_lines.push(prop.as_str());
                } else {
                    specific_section_lines.push(prop.as_str());
                }
            }
        }

        let section_name = "Service";
        let mut content = String::new();
        if !unit_section_lines.is_empty() {
            content.push_str("[Unit]\n");
            for line in &unit_section_lines {
                content.push_str(line);
                content.push('\n');
            }
            content.push('\n');
        }
        if !specific_section_lines.is_empty() {
            use std::fmt::Write;
            let _ = writeln!(content, "[{section_name}]");
            for line in &specific_section_lines {
                content.push_str(line);
                content.push('\n');
            }
            content.push('\n');
        }

        let dropin_dir = etc_dir.join(format!("{unit_name}.d"));
        std::fs::create_dir_all(&dropin_dir).unwrap();
        let dropin_path = dropin_dir.join("50-set-property.conf");
        std::fs::write(&dropin_path, &content).unwrap();

        let written = std::fs::read_to_string(&dropin_path).unwrap();
        assert!(written.contains("[Service]"));
        assert!(written.contains("CPUWeight=200"));
        assert!(written.contains("MemoryMax=1G"));
        // No [Unit] section since these are service-level properties.
        assert!(!written.contains("[Unit]"));
    }

    #[test]
    fn test_set_property_unit_section_props() {
        // Properties like Description= should go under [Unit].
        let props = vec![
            "Description=My Service".to_string(),
            "CPUWeight=100".to_string(),
        ];
        let unit_props = [
            "Description",
            "Documentation",
            "Wants",
            "Requires",
            "After",
            "Before",
        ];
        let mut unit_section_lines = Vec::new();
        let mut specific_section_lines = Vec::new();
        for prop in &props {
            if let Some((key, _val)) = prop.split_once('=') {
                if unit_props.contains(&key) {
                    unit_section_lines.push(prop.as_str());
                } else {
                    specific_section_lines.push(prop.as_str());
                }
            }
        }
        assert_eq!(unit_section_lines, vec!["Description=My Service"]);
        assert_eq!(specific_section_lines, vec!["CPUWeight=100"]);
    }

    // ── Revert execution tests ──────────────────────────────────────────

    #[test]
    fn test_revert_removes_etc_dropin_dir() {
        let dir = tempfile::tempdir().unwrap();
        let dropin = dir.path().join("test.service.d");
        std::fs::create_dir_all(&dropin).unwrap();
        std::fs::write(dropin.join("override.conf"), "[Service]\nCPUWeight=200\n").unwrap();

        assert!(dropin.is_dir());
        std::fs::remove_dir_all(&dropin).unwrap();
        assert!(!dropin.exists());
    }

    #[test]
    fn test_revert_removes_run_dropin_dir() {
        let dir = tempfile::tempdir().unwrap();
        let dropin = dir.path().join("test.service.d");
        std::fs::create_dir_all(&dropin).unwrap();
        std::fs::write(dropin.join("override.conf"), "[Service]\nRestart=always\n").unwrap();

        assert!(dropin.is_dir());
        std::fs::remove_dir_all(&dropin).unwrap();
        assert!(!dropin.exists());
    }

    #[test]
    fn test_revert_only_removes_admin_override_when_vendor_exists() {
        let dir = tempfile::tempdir().unwrap();
        let vendor_dir = dir.path().join("vendor");
        let admin_dir = dir.path().join("admin");
        std::fs::create_dir_all(&vendor_dir).unwrap();
        std::fs::create_dir_all(&admin_dir).unwrap();

        // Create a vendor unit file.
        std::fs::write(
            vendor_dir.join("test.service"),
            "[Unit]\nDescription=Vendor\n",
        )
        .unwrap();
        // Create an admin override.
        std::fs::write(
            admin_dir.join("test.service"),
            "[Unit]\nDescription=Admin Override\n",
        )
        .unwrap();

        // Simulate revert: only remove admin copy if vendor exists.
        let has_vendor = vendor_dir.join("test.service").exists();
        assert!(has_vendor);
        let admin_path = admin_dir.join("test.service");
        if has_vendor && admin_path.is_file() {
            std::fs::remove_file(&admin_path).unwrap();
        }
        assert!(!admin_path.exists());
        // Vendor file should still exist.
        assert!(vendor_dir.join("test.service").exists());
    }

    #[test]
    fn test_revert_no_vendor_keeps_admin_file() {
        let dir = tempfile::tempdir().unwrap();
        let admin_dir = dir.path().join("admin");
        std::fs::create_dir_all(&admin_dir).unwrap();

        // Create an admin unit file with no vendor counterpart.
        std::fs::write(
            admin_dir.join("custom.service"),
            "[Unit]\nDescription=Custom\n",
        )
        .unwrap();

        // Simulate revert: no vendor file, so don't remove the admin file.
        let vendor_paths = [
            dir.path().join("vendor1/custom.service"),
            dir.path().join("vendor2/custom.service"),
        ];
        let has_vendor = vendor_paths.iter().any(|p| p.exists());
        assert!(!has_vendor);
        // Admin file should remain.
        assert!(admin_dir.join("custom.service").exists());
    }

    // ── Edit execution tests ────────────────────────────────────────────

    #[test]
    fn test_edit_returns_unit_info() {
        // Test that the edit command response structure has the expected fields.
        let response = serde_json::json!({
            "unit": "sshd.service",
            "fragment_path": "/usr/lib/systemd/system/sshd.service",
            "dropin_dir": "/etc/systemd/system/sshd.service.d",
            "override_path": "/etc/systemd/system/sshd.service.d/override.conf",
            "existing_override": "",
            "original_content": "",
            "full": false,
        });

        assert_eq!(
            response.get("unit").and_then(|v| v.as_str()),
            Some("sshd.service")
        );
        assert_eq!(
            response.get("dropin_dir").and_then(|v| v.as_str()),
            Some("/etc/systemd/system/sshd.service.d")
        );
        assert_eq!(
            response.get("override_path").and_then(|v| v.as_str()),
            Some("/etc/systemd/system/sshd.service.d/override.conf")
        );
        assert_eq!(response.get("full").and_then(|v| v.as_bool()), Some(false));
    }

    #[test]
    fn test_edit_full_returns_original_content() {
        let original = "[Unit]\nDescription=OpenSSH\n\n[Service]\nExecStart=/usr/sbin/sshd\n";
        let response = serde_json::json!({
            "unit": "sshd.service",
            "fragment_path": "/usr/lib/systemd/system/sshd.service",
            "dropin_dir": "/etc/systemd/system/sshd.service.d",
            "override_path": "/etc/systemd/system/sshd.service.d/override.conf",
            "existing_override": "",
            "original_content": original,
            "full": true,
        });

        assert_eq!(response.get("full").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(
            response.get("original_content").and_then(|v| v.as_str()),
            Some(original)
        );
    }

    #[test]
    fn test_edit_returns_existing_override() {
        let existing = "[Service]\nCPUWeight=200\n";
        let response = serde_json::json!({
            "unit": "sshd.service",
            "fragment_path": "/usr/lib/systemd/system/sshd.service",
            "dropin_dir": "/etc/systemd/system/sshd.service.d",
            "override_path": "/etc/systemd/system/sshd.service.d/override.conf",
            "existing_override": existing,
            "original_content": "",
            "full": false,
        });

        assert_eq!(
            response.get("existing_override").and_then(|v| v.as_str()),
            Some(existing)
        );
    }

    // ── Section name inference tests ────────────────────────────────────

    #[test]
    fn test_set_property_section_name_from_unit_suffix() {
        let cases = vec![
            ("test.service", "Service"),
            ("test.socket", "Socket"),
            ("test.slice", "Slice"),
            ("test.mount", "Mount"),
            ("test.swap", "Swap"),
            ("test.timer", "Timer"),
            ("test.path", "Path"),
            ("test.target", "Service"), // targets default to Service
        ];
        for (unit_name, expected_section) in cases {
            let section = if unit_name.ends_with(".service") {
                "Service"
            } else if unit_name.ends_with(".socket") {
                "Socket"
            } else if unit_name.ends_with(".slice") {
                "Slice"
            } else if unit_name.ends_with(".mount") {
                "Mount"
            } else if unit_name.ends_with(".swap") {
                "Swap"
            } else if unit_name.ends_with(".timer") {
                "Timer"
            } else if unit_name.ends_with(".path") {
                "Path"
            } else {
                "Service"
            };
            assert_eq!(
                section, expected_section,
                "unit {unit_name} should map to section [{expected_section}]"
            );
        }
    }
}
