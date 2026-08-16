//! System operations for the tramp-agent RPC server.
//!
//! Implements the following RPC methods:
//!
//! | Method           | Description                                          |
//! |------------------|------------------------------------------------------|
//! | `system.info`    | Gather system info (OS, arch, hostname, user, uptime)|
//! | `system.getenv`  | Read an environment variable on the remote           |
//! | `system.statvfs` | Get filesystem usage statistics for a mount point    |

use rmpv::Value;

use crate::rpc::{Response, error_code};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract a required string parameter from a MsgPack map by key.
fn get_str_param<'a>(params: &'a Value, key: &str) -> Result<&'a str, Response> {
    params
        .as_map()
        .and_then(|m| {
            m.iter()
                .find(|(k, _)| k.as_str() == Some(key))
                .and_then(|(_, v)| v.as_str())
        })
        .ok_or_else(|| {
            Response::err(
                0, // caller must fix up the id
                error_code::INVALID_PARAMS,
                format!("missing or invalid parameter: {key}"),
            )
        })
}

// ---------------------------------------------------------------------------
// RPC method handlers
// ---------------------------------------------------------------------------

/// `system.info` — gather system information in one call.
///
/// Params: `{}` (empty map, no parameters required)
///
/// Result: a map with the following fields (all strings unless noted):
///
/// | Field      | Description                                   |
/// |------------|-----------------------------------------------|
/// | `os`       | Operating system name (e.g. `"Linux"`)        |
/// | `arch`     | CPU architecture (e.g. `"x86_64"`)            |
/// | `hostname` | Machine hostname                               |
/// | `user`     | Current username                               |
/// | `home`     | Home directory path                            |
/// | `pid`      | Agent process ID (u64)                         |
/// | `version`  | Agent version string                           |
/// | `uptime`   | System uptime in seconds (u64, Linux only)     |
pub async fn info(id: u64, _params: &Value) -> Response {
    let mut fields: Vec<(Value, Value)> = Vec::with_capacity(10);

    // OS and architecture via uname(2).
    let (os, arch) = get_uname();
    fields.push((Value::String("os".into()), Value::String(os.into())));
    fields.push((Value::String("arch".into()), Value::String(arch.into())));

    // Hostname.
    if let Some(hostname) = get_hostname() {
        fields.push((
            Value::String("hostname".into()),
            Value::String(hostname.into()),
        ));
    }

    // Current user (from $USER or getuid → getpwuid).
    if let Some(user) = get_current_user() {
        fields.push((Value::String("user".into()), Value::String(user.into())));
    }

    // Home directory.
    if let Some(home) = get_home_dir() {
        fields.push((Value::String("home".into()), Value::String(home.into())));
    }

    // Agent PID.
    fields.push((
        Value::String("pid".into()),
        Value::Integer((std::process::id() as u64).into()),
    ));

    // Agent version.
    fields.push((
        Value::String("version".into()),
        Value::String(env!("CARGO_PKG_VERSION").into()),
    ));

    // System uptime (Linux: /proc/uptime, fallback: sysinfo).
    if let Some(uptime_secs) = get_uptime() {
        fields.push((
            Value::String("uptime".into()),
            Value::Integer(uptime_secs.into()),
        ));
    }

    Response::ok(id, Value::Map(fields))
}

/// `system.getenv` — read an environment variable.
///
/// Params: `{ name: "<VARIABLE_NAME>" }`
///
/// Result: `{ value: "<value>" }` or `{ value: null }` if the variable is
/// not set.
pub async fn getenv(id: u64, params: &Value) -> Response {
    let name = match get_str_param(params, "name") {
        Ok(n) => n,
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    let value = std::env::var(name).ok();

    let val = match value {
        Some(v) => Value::String(v.into()),
        None => Value::Nil,
    };

    Response::ok(id, Value::Map(vec![(Value::String("value".into()), val)]))
}

/// `system.statvfs` — get filesystem usage statistics for a path.
///
/// Params: `{ path: "<mount_point_or_path>" }`
///
/// Result: a map with:
///
/// | Field          | Type | Description                               |
/// |----------------|------|-------------------------------------------|
/// | `total_bytes`  | u64  | Total filesystem size in bytes             |
/// | `free_bytes`   | u64  | Free space in bytes (for superuser)        |
/// | `avail_bytes`  | u64  | Available space in bytes (for normal user) |
/// | `total_inodes` | u64  | Total number of inodes                     |
/// | `free_inodes`  | u64  | Free inodes                                |
/// | `block_size`   | u64  | Filesystem block size                      |
pub async fn statvfs(id: u64, params: &Value) -> Response {
    let path = match get_str_param(params, "path") {
        Ok(p) => p,
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    match do_statvfs(path) {
        Ok(fields) => Response::ok(id, Value::Map(fields)),
        Err(e) => {
            let (code, msg) = match e.kind() {
                std::io::ErrorKind::NotFound => (
                    error_code::NOT_FOUND,
                    format!("no such file or directory: {path}"),
                ),
                std::io::ErrorKind::PermissionDenied => (
                    error_code::PERMISSION_DENIED,
                    format!("permission denied: {path}"),
                ),
                _ => (error_code::IO_ERROR, format!("{path}: {e}")),
            };
            Response::err(id, code, msg)
        }
    }
}

// ---------------------------------------------------------------------------
// Platform-specific helpers
// ---------------------------------------------------------------------------

/// Read uname fields (sysname, machine) via libc.
fn get_uname() -> (String, String) {
    // SAFETY: uname is a standard POSIX call.  We initialise the struct to
    // zeroes so it's safe even if the call fails.
    unsafe {
        let mut buf: libc::utsname = std::mem::zeroed();
        if libc::uname(&mut buf) == 0 {
            let os = std::ffi::CStr::from_ptr(buf.sysname.as_ptr())
                .to_string_lossy()
                .into_owned();
            let arch = std::ffi::CStr::from_ptr(buf.machine.as_ptr())
                .to_string_lossy()
                .into_owned();
            (os, arch)
        } else {
            ("unknown".into(), "unknown".into())
        }
    }
}

/// Get the hostname from uname.
fn get_hostname() -> Option<String> {
    // SAFETY: same as get_uname.
    unsafe {
        let mut buf: libc::utsname = std::mem::zeroed();
        if libc::uname(&mut buf) == 0 {
            let hostname = std::ffi::CStr::from_ptr(buf.nodename.as_ptr())
                .to_string_lossy()
                .into_owned();
            Some(hostname)
        } else {
            None
        }
    }
}

/// Get the current username from the environment or getpwuid.
fn get_current_user() -> Option<String> {
    // Try $USER first (fast path).
    if let Ok(user) = std::env::var("USER")
        && !user.is_empty()
    {
        return Some(user);
    }

    // Fall back to getpwuid.
    // SAFETY: getuid + getpwuid are standard POSIX calls.
    unsafe {
        let uid = libc::getuid();
        let pw = libc::getpwuid(uid);
        if pw.is_null() {
            return None;
        }
        let name = std::ffi::CStr::from_ptr((*pw).pw_name);
        Some(name.to_string_lossy().into_owned())
    }
}

/// Get the home directory from the environment or getpwuid.
fn get_home_dir() -> Option<String> {
    if let Ok(home) = std::env::var("HOME")
        && !home.is_empty()
    {
        return Some(home);
    }

    // Fall back to getpwuid.
    unsafe {
        let uid = libc::getuid();
        let pw = libc::getpwuid(uid);
        if pw.is_null() {
            return None;
        }
        let dir = std::ffi::CStr::from_ptr((*pw).pw_dir);
        Some(dir.to_string_lossy().into_owned())
    }
}

/// Get system uptime in seconds.
///
/// On Linux, reads `/proc/uptime`.  Returns `None` on failure.
fn get_uptime() -> Option<u64> {
    // Try /proc/uptime (Linux).
    if let Ok(contents) = std::fs::read_to_string("/proc/uptime")
        && let Some(first_field) = contents.split_whitespace().next()
        && let Ok(secs) = first_field.parse::<f64>()
    {
        return Some(secs as u64);
    }

    // Try clock_gettime with CLOCK_BOOTTIME (Linux) or CLOCK_MONOTONIC.
    #[cfg(target_os = "linux")]
    {
        let mut ts: libc::timespec = unsafe { std::mem::zeroed() };
        // CLOCK_BOOTTIME (7) includes time spent in suspend.
        if unsafe { libc::clock_gettime(7, &mut ts) } == 0 {
            return Some(ts.tv_sec as u64);
        }
    }

    // Fallback: CLOCK_MONOTONIC (not exactly uptime but close enough).
    {
        let mut ts: libc::timespec = unsafe { std::mem::zeroed() };
        if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) } == 0 {
            return Some(ts.tv_sec as u64);
        }
    }

    None
}

/// Perform `statvfs(2)` on the given path and return a list of MsgPack
/// key-value pairs.
fn do_statvfs(path: &str) -> Result<Vec<(Value, Value)>, std::io::Error> {
    use std::ffi::CString;

    let c_path =
        CString::new(path).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

    // SAFETY: statvfs is a standard POSIX call.  We zero-initialise the
    // output struct.
    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        let ret = libc::statvfs(c_path.as_ptr(), &mut stat);
        if ret != 0 {
            return Err(std::io::Error::last_os_error());
        }

        let block_size = stat.f_frsize;

        Ok(vec![
            (
                Value::String("total_bytes".into()),
                Value::Integer((stat.f_blocks * block_size).into()),
            ),
            (
                Value::String("free_bytes".into()),
                Value::Integer((stat.f_bfree * block_size).into()),
            ),
            (
                Value::String("avail_bytes".into()),
                Value::Integer((stat.f_bavail * block_size).into()),
            ),
            (
                Value::String("total_inodes".into()),
                Value::Integer(stat.f_files.into()),
            ),
            (
                Value::String("free_inodes".into()),
                Value::Integer(stat.f_ffree.into()),
            ),
            (
                Value::String("block_size".into()),
                Value::Integer(block_size.into()),
            ),
        ])
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rmpv::Value;

    fn make_params(pairs: Vec<(&str, Value)>) -> Value {
        Value::Map(
            pairs
                .into_iter()
                .map(|(k, v)| (Value::String(k.into()), v))
                .collect(),
        )
    }

    #[tokio::test]
    async fn info_returns_expected_fields() {
        let params = Value::Map(vec![]);
        let resp = info(1, &params).await;
        assert!(resp.error.is_none(), "expected ok, got: {:?}", resp.error);

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();

        // Should have at least os, arch, pid, version.
        let os = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("os"))
            .unwrap()
            .1
            .as_str()
            .unwrap();
        assert!(!os.is_empty(), "os should not be empty");

        let arch = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("arch"))
            .unwrap()
            .1
            .as_str()
            .unwrap();
        assert!(!arch.is_empty(), "arch should not be empty");

        let pid = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("pid"))
            .unwrap()
            .1
            .as_u64()
            .unwrap();
        assert!(pid > 0);

        let version = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("version"))
            .unwrap()
            .1
            .as_str()
            .unwrap();
        assert!(!version.is_empty());
    }

    #[tokio::test]
    async fn info_has_hostname() {
        let params = Value::Map(vec![]);
        let resp = info(2, &params).await;
        assert!(resp.error.is_none());

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();

        let hostname = map.iter().find(|(k, _)| k.as_str() == Some("hostname"));
        assert!(hostname.is_some(), "info should include hostname");
    }

    #[tokio::test]
    async fn info_has_user() {
        let params = Value::Map(vec![]);
        let resp = info(3, &params).await;
        assert!(resp.error.is_none());

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();

        let user = map.iter().find(|(k, _)| k.as_str() == Some("user"));
        assert!(user.is_some(), "info should include user");
    }

    #[tokio::test]
    async fn getenv_existing_variable() {
        // $HOME should be set on any POSIX system.
        let params = make_params(vec![("name", Value::String("HOME".into()))]);
        let resp = getenv(4, &params).await;
        assert!(resp.error.is_none(), "expected ok, got: {:?}", resp.error);

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();

        let value = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("value"))
            .unwrap()
            .1
            .as_str()
            .unwrap();
        assert!(!value.is_empty(), "HOME should not be empty");
    }

    #[tokio::test]
    async fn getenv_nonexistent_variable() {
        let params = make_params(vec![(
            "name",
            Value::String("__TRAMP_AGENT_TEST_NONEXISTENT_VAR_12345__".into()),
        )]);
        let resp = getenv(5, &params).await;
        assert!(resp.error.is_none());

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();

        let value = &map
            .iter()
            .find(|(k, _)| k.as_str() == Some("value"))
            .unwrap()
            .1;
        assert!(value.is_nil(), "expected null for nonexistent variable");
    }

    #[tokio::test]
    async fn getenv_missing_param() {
        let params = Value::Map(vec![]);
        let resp = getenv(99, &params).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, error_code::INVALID_PARAMS);
    }

    #[tokio::test]
    async fn statvfs_root() {
        let params = make_params(vec![("path", Value::String("/".into()))]);
        let resp = statvfs(6, &params).await;
        assert!(resp.error.is_none(), "expected ok, got: {:?}", resp.error);

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();

        let total = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("total_bytes"))
            .unwrap()
            .1
            .as_u64()
            .unwrap();
        assert!(total > 0, "total_bytes should be > 0 for /");

        let avail = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("avail_bytes"))
            .unwrap()
            .1
            .as_u64()
            .unwrap();
        // avail should be <= total.
        assert!(avail <= total);

        let block_size = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("block_size"))
            .unwrap()
            .1
            .as_u64()
            .unwrap();
        assert!(block_size > 0, "block_size should be > 0");
    }

    #[tokio::test]
    async fn statvfs_nonexistent_path() {
        let params = make_params(vec![(
            "path",
            Value::String("/tmp/__tramp_agent_statvfs_nonexistent_12345__".into()),
        )]);
        let resp = statvfs(7, &params).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, error_code::NOT_FOUND);
    }

    #[tokio::test]
    async fn statvfs_missing_param() {
        let params = Value::Map(vec![]);
        let resp = statvfs(98, &params).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, error_code::INVALID_PARAMS);
    }

    #[test]
    fn uname_returns_nonempty() {
        let (os, arch) = get_uname();
        assert!(!os.is_empty());
        assert!(!arch.is_empty());
    }

    #[test]
    fn hostname_returns_some() {
        let hostname = get_hostname();
        assert!(hostname.is_some());
        assert!(!hostname.unwrap().is_empty());
    }

    #[test]
    fn current_user_returns_some() {
        let user = get_current_user();
        assert!(user.is_some());
        assert!(!user.unwrap().is_empty());
    }

    #[test]
    fn home_dir_returns_some() {
        let home = get_home_dir();
        assert!(home.is_some());
        assert!(!home.unwrap().is_empty());
    }

    #[test]
    fn uptime_returns_some() {
        let uptime = get_uptime();
        // Uptime should always be available on Linux; may be None on
        // exotic platforms but we expect it in CI.
        assert!(uptime.is_some(), "expected uptime to be available");
        assert!(uptime.unwrap() > 0);
    }

    #[test]
    fn do_statvfs_root_succeeds() {
        let result = do_statvfs("/");
        assert!(result.is_ok());
        let fields = result.unwrap();
        assert!(!fields.is_empty());
    }

    #[test]
    fn do_statvfs_nonexistent_fails() {
        let result = do_statvfs("/tmp/__tramp_agent_statvfs_noexist_54321__");
        assert!(result.is_err());
    }
}
