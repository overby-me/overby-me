use std::ffi::CString;

pub struct GroupEntry {
    pub name: String,
    pub pw: Option<Vec<u8>>,
    pub gid: nix::unistd::Gid,
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn make_group_from_libc(groupname: &str, group: &libc::group) -> Result<GroupEntry, String> {
    let gid = nix::unistd::Gid::from_raw(group.gr_gid);
    let pw = if group.gr_passwd.is_null() {
        None
    } else {
        let mut vec = Vec::new();
        let mut ptr = group.gr_passwd;
        loop {
            let byte = unsafe { *ptr } as u8;
            if byte == b'\0' {
                break;
            }
            vec.push(byte);
            unsafe { ptr = ptr.add(1) };
        }
        Some(vec)
    };
    Ok(GroupEntry {
        name: groupname.to_string(),
        gid,
        pw,
    })
}

#[cfg(target_os = "linux")]
#[allow(dead_code)]
// keep around for a PR to the nix crate
fn getgrnam(groupname: &str) -> Result<GroupEntry, String> {
    let c_groupname =
        CString::new(groupname).map_err(|e| format!("Invalid groupname '{groupname}': {e}"))?;
    // TODO check errno
    let res = unsafe { libc::getgrnam(c_groupname.as_ptr()) };
    if res.is_null() {
        return Err(format!("No entry found for groupname: {groupname}"));
    }
    let res = unsafe { *res };
    make_group_from_libc(groupname, &res)
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub fn getgrnam_r(groupname: &str) -> Result<GroupEntry, String> {
    let c_groupname =
        CString::new(groupname).map_err(|e| format!("Invalid groupname '{groupname}': {e}"))?;
    let mut buf_size = 32;
    let mut group: libc::group = libc::group {
        gr_name: std::ptr::null_mut(),
        gr_passwd: std::ptr::null_mut(),
        gr_gid: 0,
        gr_mem: std::ptr::null_mut(),
    };

    loop {
        let mut buf = vec![0i8; buf_size];
        let mut result: *mut libc::group = std::ptr::null_mut();

        let rc = unsafe {
            libc::getgrnam_r(
                c_groupname.as_ptr(),
                &mut group,
                buf.as_mut_ptr(),
                buf_size,
                &mut result,
            )
        };

        if rc == libc::ERANGE {
            // Buffer too small, retry with a larger one
            buf_size *= 2;
            continue;
        }

        if rc != 0 {
            return Err(format!(
                "Error calling getgrnam_r for groupname '{groupname}': errno {rc}"
            ));
        }

        if result.is_null() {
            return Err(format!("No entry found for groupname: {groupname}"));
        }

        return make_group_from_libc(groupname, &group);
    }
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
pub fn getgrnam_r(_groupname: &str) -> Result<GroupEntry, String> {
    compile_error!("getgrnam_r is not yet implemented for this platform");
}
