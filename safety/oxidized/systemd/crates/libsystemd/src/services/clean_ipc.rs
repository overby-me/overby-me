//! `RemoveIPC=` — remove IPC objects owned by a service's UID when it stops.
//!
//! Ports systemd's `clean_ipc_by_uid` (src/shared/clean-ipc.c): SysV shared
//! memory / semaphores / message queues (enumerated from `/proc/sysvipc/*` and
//! removed with `IPC_RMID`) plus POSIX shared memory (`/dev/shm`) and POSIX
//! message queues (`/dev/mqueue`). Only objects owned by `uid` are removed.
//!
//! systemd matches on the service UID *or* GID; this port matches on UID only
//! (the common case — services own their IPC by UID). It is also currently
//! wired only for a static `User=` (whose UID is re-derivable at stop);
//! `DynamicUser=` support needs the allocated UID stored on the service state.

use std::io::BufRead;
use std::os::unix::fs::MetadataExt;

/// Remove every IPC object owned by `uid`. Best-effort: individual failures are
/// logged and skipped. The caller must not pass uid 0 (never clean root's IPC).
pub(crate) fn clean_ipc_by_uid(uid: u32) {
    // /proc/sysvipc/<type> columns (whitespace-separated, after a header line):
    //   shm: key shmid perms size cpid lpid nattch UID gid ...   -> id=1, uid=7
    //   sem: key semid perms nsems UID gid ...                   -> id=1, uid=4
    //   msg: key msqid perms cbytes qnum lspid lrpid UID gid ... -> id=1, uid=7
    clean_sysvipc(uid, "/proc/sysvipc/shm", 7, remove_sysv_shm);
    clean_sysvipc(uid, "/proc/sysvipc/sem", 4, remove_sysv_sem);
    clean_sysvipc(uid, "/proc/sysvipc/msg", 7, remove_sysv_msg);
    clean_posix_dir(uid, "/dev/shm", false);
    clean_posix_dir(uid, "/dev/mqueue", true);
}

fn clean_sysvipc(uid: u32, path: &str, uid_col: usize, remove: fn(i32)) {
    let Ok(file) = std::fs::File::open(path) else {
        return;
    };
    for (i, line) in std::io::BufReader::new(file).lines().enumerate() {
        if i == 0 {
            continue; // header row
        }
        let Ok(line) = line else { continue };
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() <= uid_col {
            continue;
        }
        let (Ok(id), Ok(owner)) = (fields[1].parse::<i32>(), fields[uid_col].parse::<u32>()) else {
            continue;
        };
        if owner == uid {
            remove(id);
        }
    }
}

fn log_rmid_err(kind: &str, id: i32) {
    let e = std::io::Error::last_os_error();
    // EIDRM/EINVAL mean the object is already gone — not worth a warning.
    if !matches!(e.raw_os_error(), Some(libc::EIDRM) | Some(libc::EINVAL)) {
        log::warn!("RemoveIPC: failed to remove SysV {kind} {id}: {e}");
    }
}

fn remove_sysv_shm(id: i32) {
    if unsafe { libc::shmctl(id, libc::IPC_RMID, std::ptr::null_mut()) } < 0 {
        log_rmid_err("shared memory segment", id);
    }
}

fn remove_sysv_sem(id: i32) {
    if unsafe { libc::semctl(id, 0, libc::IPC_RMID) } < 0 {
        log_rmid_err("semaphore", id);
    }
}

fn remove_sysv_msg(id: i32) {
    if unsafe { libc::msgctl(id, libc::IPC_RMID, std::ptr::null_mut()) } < 0 {
        log_rmid_err("message queue", id);
    }
}

/// Remove entries under `dir` owned by `uid`. For `/dev/mqueue` a matching file
/// is a POSIX message queue and is removed with `mq_unlink`; elsewhere
/// (`/dev/shm`) a matching file is unlinked and a matching directory removed
/// recursively.
fn clean_posix_dir(uid: u32, dir: &str, is_mqueue: bool) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(md) = entry.path().symlink_metadata() else {
            continue;
        };
        if md.uid() != uid {
            continue;
        }
        let path = entry.path();
        if md.is_dir() {
            let _ = std::fs::remove_dir_all(&path);
        } else if is_mqueue {
            let name = format!("/{}", entry.file_name().to_string_lossy());
            if let Ok(cname) = std::ffi::CString::new(name) {
                unsafe { libc::mq_unlink(cname.as_ptr()) };
            }
        } else {
            let _ = std::fs::remove_file(&path);
        }
    }
}
