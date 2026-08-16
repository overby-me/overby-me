use nix::unistd::Gid;
use nix::unistd::Uid;
use nix::unistd::setresgid;
use nix::unistd::setresuid;
use std::io::Read;

/// Drop all privileges from root to the specified uid/gid.
///
/// Sequence:  setresgid -> setgroups -> setresuid
/// This matches systemd's privilege-drop ordering.
pub fn drop_privileges(gid: Gid, supp_gids: &[Gid], uid: Uid) -> Result<(), String> {
    setresgid(gid, gid, gid).map_err(|e| format!("Error while setting groupid: {e}"))?;
    maybe_set_groups(supp_gids)?;
    setresuid(uid, uid, uid).map_err(|e| format!("Error while setting userid: {e}"))?;
    Ok(())
}

const ALLOW_READ: [u8; 5] = [b'a', b'l', b'l', b'o', b'w'];

fn maybe_set_groups(supp_gids: &[Gid]) -> Result<(), String> {
    if can_drop_groups()? {
        nix::unistd::setgroups(supp_gids).map_err(|e| format!("Error while calling setgroups: {e}"))
    } else {
        // We just ignore groups if the kernel says we can't drop them.
        // systemd seems to do it like this:
        // https://github.com/systemd/systemd/blob/master/src/basic/user-util.c
        Ok(())
    }
}

fn can_drop_groups() -> Result<bool, String> {
    let kernel_iface_path = std::path::PathBuf::from("/proc/self/setgroups");

    if kernel_iface_path.exists() {
        let mut buf = [0u8; 5];
        let mut file = std::fs::File::open(&kernel_iface_path).map_err(|e| {
            format!(
                "Error while opening file: {kernel_iface_path:?} to check if we can call setgroups: {e}"
            )
        })?;
        // Use read() instead of read_exact() to avoid panicking if the
        // file is shorter than expected.  /proc/self/setgroups typically
        // contains "allow\n" (6 bytes) or "deny\n" (5 bytes), but we
        // only need the first 5 bytes to distinguish between them.
        match file.read(&mut buf[..]) {
            Ok(n) if n >= 5 && buf[..5].eq(&ALLOW_READ) => Ok(true),
            Ok(_) => Ok(false),
            Err(e) => Err(format!("Error while reading {kernel_iface_path:?}: {e}")),
        }
    } else {
        // assume true since we can't check
        Ok(true)
    }
}
