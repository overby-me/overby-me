//! Instance tracking for running Flatpak sandboxes.
//!
//! Manages `/run/user/<uid>/.flatpak/<instance-id>/` directories that track
//! running sandbox instances. Used by `flatpak ps`, `flatpak enter`, and
//! `flatpak kill`.

use std::fs;
use std::path::PathBuf;

use crate::metadata::Metadata;

/// A running Flatpak instance.
#[derive(Debug)]
#[allow(dead_code)]
pub struct Instance {
    pub id: String,
    pub path: PathBuf,
    pub pid: Option<u32>,
    pub app_id: Option<String>,
    pub info: Option<Metadata>,
}

/// Get the base directory for instance tracking.
pub fn instances_dir() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{uid}/.flatpak"))
}

/// Create a new instance directory and return the instance ID.
pub fn create_instance(flatpak_info: &str) -> Result<String, String> {
    let base = instances_dir();
    fs::create_dir_all(&base).map_err(|e| format!("create instances dir: {e}"))?;

    let id = generate_instance_id();
    let dir = base.join(&id);
    fs::create_dir_all(&dir).map_err(|e| format!("create instance dir: {e}"))?;

    // Write info file.
    fs::write(dir.join("info"), flatpak_info).map_err(|e| format!("write instance info: {e}"))?;

    Ok(id)
}

/// Record the PID of a running instance.
#[allow(dead_code)]
pub fn write_pid(instance_id: &str, pid: u32) -> Result<(), String> {
    let path = instances_dir().join(instance_id).join("pid");
    fs::write(&path, pid.to_string()).map_err(|e| format!("write instance pid: {e}"))
}

/// Record bwrap info JSON.
#[allow(dead_code)]
pub fn write_bwrap_info(instance_id: &str, info_json: &str) -> Result<(), String> {
    let path = instances_dir().join(instance_id).join("bwrapinfo.json");
    fs::write(&path, info_json).map_err(|e| format!("write bwrapinfo: {e}"))
}

/// Remove an instance directory.
pub fn cleanup_instance(instance_id: &str) {
    let dir = instances_dir().join(instance_id);
    let _ = fs::remove_dir_all(&dir);
}

/// List all running instances.
pub fn list_instances() -> Vec<Instance> {
    let base = instances_dir();
    if !base.exists() {
        return Vec::new();
    }

    let mut instances = Vec::new();
    if let Ok(entries) = fs::read_dir(&base) {
        for entry in entries.flatten() {
            let id = entry.file_name().to_string_lossy().to_string();
            let dir = entry.path();

            let pid = fs::read_to_string(dir.join("pid"))
                .ok()
                .and_then(|s| s.trim().parse().ok());

            // Check if the process is still alive.
            if let Some(p) = pid
                && !process_alive(p)
            {
                // Stale instance — clean it up.
                let _ = fs::remove_dir_all(&dir);
                continue;
            }

            let info = Metadata::from_file(&dir.join("info")).ok();
            let app_id = info.as_ref().and_then(|m| m.app_name().map(String::from));

            instances.push(Instance {
                id,
                path: dir,
                pid,
                app_id,
                info,
            });
        }
    }

    instances
}

/// Find an instance by app ID.
pub fn find_instance_by_app(app_id: &str) -> Option<Instance> {
    list_instances()
        .into_iter()
        .find(|i| i.app_id.as_deref() == Some(app_id))
}

/// Send a signal to an instance's process.
pub fn kill_instance(instance_id: &str, signal: i32) -> Result<(), String> {
    let pid_path = instances_dir().join(instance_id).join("pid");
    let pid: i32 = fs::read_to_string(&pid_path)
        .map_err(|e| format!("read pid: {e}"))?
        .trim()
        .parse()
        .map_err(|e| format!("parse pid: {e}"))?;

    let ret = unsafe { libc::kill(pid, signal) };
    if ret == -1 {
        Err(format!("kill: {}", std::io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

fn process_alive(pid: u32) -> bool {
    let ret = unsafe { libc::kill(pid as i32, 0) };
    ret == 0
}

fn generate_instance_id() -> String {
    // Use PID + timestamp for a simple unique ID.
    let pid = std::process::id();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{pid}-{ts:x}")
}

/// Clean up temp files created during sandbox setup.
pub fn cleanup_temp_files() {
    let pid = std::process::id();
    let patterns = [
        format!("/tmp/.flatpak-info-{pid}"),
        format!("/tmp/.flatpak-passwd-{pid}"),
        format!("/tmp/.flatpak-group-{pid}"),
    ];
    for path in &patterns {
        let _ = fs::remove_file(path);
    }
}
