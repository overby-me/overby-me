//! NUMA memory policy support (`NUMAPolicy=` / `NUMAMask=`).
//!
//! Ports systemd's `src/shared/numa-util.c`. A [`NumaPolicy`] is a memory
//! policy type (one of the `MPOL_*` constants) plus an optional set of NUMA
//! nodes. [`apply_numa_policy`] installs it via `set_mempolicy(2)`. The same
//! helper is used by PID 1 for its own policy (from `[Manager] NUMAPolicy=`)
//! and by services at exec (from the service's `NUMAPolicy=`).
//!
//! `set_mempolicy(2)` sets the policy of the *calling task*, so the caller must
//! run on the thread whose policy should change (for the Manager policy that is
//! PID 1's main thread / TID 1).

/// Memory policy modes, matching `linux/mempolicy.h`.
pub const MPOL_DEFAULT: i32 = 0;
pub const MPOL_PREFERRED: i32 = 1;
pub const MPOL_BIND: i32 = 2;
pub const MPOL_INTERLEAVE: i32 = 3;
pub const MPOL_LOCAL: i32 = 4;

/// Parse a `NUMAPolicy=` value into an `MPOL_*` constant.
pub fn mpol_from_string(s: &str) -> Option<i32> {
    match s.trim() {
        "default" => Some(MPOL_DEFAULT),
        "preferred" => Some(MPOL_PREFERRED),
        "bind" => Some(MPOL_BIND),
        "interleave" => Some(MPOL_INTERLEAVE),
        "local" => Some(MPOL_LOCAL),
        _ => None,
    }
}

/// Render an `MPOL_*` constant back to its `NUMAPolicy=` string (for
/// `systemctl show -p NUMAPolicy`).
pub fn mpol_to_string(t: i32) -> Option<&'static str> {
    match t {
        MPOL_DEFAULT => Some("default"),
        MPOL_PREFERRED => Some("preferred"),
        MPOL_BIND => Some("bind"),
        MPOL_INTERLEAVE => Some("interleave"),
        MPOL_LOCAL => Some("local"),
        _ => None,
    }
}

/// Parse a `NUMAMask=` value: a whitespace/comma separated list of node
/// indices and inclusive `a-b` ranges (e.g. `"0"`, `"0-3"`, `"0 2 4"`).
/// Returns the node indices, or `None` on a malformed token.
pub fn parse_numa_mask(s: &str) -> Option<Vec<usize>> {
    let mut nodes = Vec::new();
    for tok in s.split([',', ' ', '\t', '\n']).filter(|t| !t.is_empty()) {
        if let Some((a, b)) = tok.split_once('-') {
            let a: usize = a.trim().parse().ok()?;
            let b: usize = b.trim().parse().ok()?;
            for n in a..=b {
                nodes.push(n);
            }
        } else {
            nodes.push(tok.trim().parse().ok()?);
        }
    }
    Some(nodes)
}

/// A parsed NUMA memory policy: a mode plus its node mask.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NumaPolicy {
    /// `MPOL_*` constant, or `-1` when unset (see [`NumaPolicy::get_type`]).
    pub type_: i32,
    /// Node indices in the mask (empty = no mask).
    pub nodes: Vec<usize>,
}

impl NumaPolicy {
    /// Mirror systemd's `numa_policy_get_type()`: an unset type (`< 0`) with a
    /// node mask is treated as `MPOL_PREFERRED`; unset with no mask stays
    /// invalid (`-1`).
    pub fn get_type(&self) -> i32 {
        if self.type_ < 0 {
            if self.nodes.is_empty() {
                -1
            } else {
                MPOL_PREFERRED
            }
        } else {
            self.type_
        }
    }

    /// Mirror `numa_policy_is_valid()`.
    pub fn is_valid(&self) -> bool {
        let t = self.get_type();
        if !(MPOL_DEFAULT..=MPOL_LOCAL).contains(&t) {
            return false;
        }
        // BIND and INTERLEAVE require a node mask.
        if self.nodes.is_empty() && !matches!(t, MPOL_DEFAULT | MPOL_LOCAL | MPOL_PREFERRED) {
            return false;
        }
        // PREFERRED accepts at most one node.
        if !self.nodes.is_empty() && t == MPOL_PREFERRED && self.nodes.len() != 1 {
            return false;
        }
        true
    }

    /// Build the `(nodemask, maxnode)` pair for `set_mempolicy(2)`.
    ///
    /// `MPOL_DEFAULT`/`MPOL_LOCAL`, and `MPOL_PREFERRED` with no mask, use a
    /// NULL nodemask (empty vec, maxnode 0) — the kernel requires this.
    /// Otherwise the nodes are packed into an array of `c_ulong`, and `maxnode`
    /// is the number of bits in that array (a whole number of `c_ulong`s, so
    /// the kernel never reads past the buffer).
    fn to_mempolicy(&self) -> (Vec<libc::c_ulong>, libc::c_ulong) {
        let t = self.get_type();
        if matches!(t, MPOL_DEFAULT | MPOL_LOCAL) || (t == MPOL_PREFERRED && self.nodes.is_empty())
        {
            return (Vec::new(), 0);
        }
        const BITS: usize = libc::c_ulong::BITS as usize;
        let max_node = self.nodes.iter().copied().max().unwrap_or(0);
        let n_ulongs = max_node / BITS + 1;
        let mut mask = vec![0 as libc::c_ulong; n_ulongs];
        for &n in &self.nodes {
            mask[n / BITS] |= (1 as libc::c_ulong) << (n % BITS);
        }
        (mask, (n_ulongs * BITS) as libc::c_ulong)
    }
}

/// Result of [`apply_numa_policy`]: the raw errno on failure.
pub type NumaResult = Result<(), i32>;

/// Apply a NUMA memory policy to the calling task via `set_mempolicy(2)`.
///
/// Returns `Err(EOPNOTSUPP)` when the kernel has no NUMA support, `Err(EINVAL)`
/// for an invalid policy (e.g. `bind`/`interleave` with no mask), or the raw
/// errno from a failed `set_mempolicy`.
pub fn apply_numa_policy(policy: &NumaPolicy) -> NumaResult {
    // Probe for NUMA support: get_mempolicy fails with ENOSYS on non-NUMA
    // kernels (systemd does the same before applying).
    let probe = unsafe { libc::syscall(libc::SYS_get_mempolicy, 0, 0, 0, 0, 0) };
    if probe < 0 {
        let e = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if e == libc::ENOSYS {
            return Err(libc::EOPNOTSUPP);
        }
    }

    if !policy.is_valid() {
        return Err(libc::EINVAL);
    }

    let t = policy.get_type();
    let (mask, maxnode) = policy.to_mempolicy();
    let nodes_ptr = if mask.is_empty() {
        std::ptr::null::<libc::c_ulong>()
    } else {
        mask.as_ptr()
    };
    let r = unsafe { libc::syscall(libc::SYS_set_mempolicy, t, nodes_ptr, maxnode) };
    if r < 0 {
        return Err(std::io::Error::last_os_error()
            .raw_os_error()
            .unwrap_or(libc::EINVAL));
    }
    Ok(())
}

/// Resolve `CPUAffinity=numa` to the CPU list of the given NUMA nodes, read
/// from `/sys/devices/system/node/nodeN/cpulist`. For a single node the sysfs
/// content is returned verbatim (an exact match for the kernel's formatting);
/// multiple nodes are unioned and re-formatted as a compressed range list. An
/// empty `nodes` slice means "all online nodes".
pub fn numa_cpu_list(nodes: &[usize]) -> Option<String> {
    let node_list: Vec<usize> = if nodes.is_empty() {
        online_numa_nodes()
    } else {
        nodes.to_vec()
    };
    if node_list.is_empty() {
        return None;
    }
    if node_list.len() == 1 {
        let path = format!("/sys/devices/system/node/node{}/cpulist", node_list[0]);
        return std::fs::read_to_string(path)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
    }
    let mut cpus = std::collections::BTreeSet::new();
    for n in node_list {
        let path = format!("/sys/devices/system/node/node{n}/cpulist");
        if let Ok(content) = std::fs::read_to_string(&path) {
            for cpu in parse_cpu_range_list(content.trim()) {
                cpus.insert(cpu);
            }
        }
    }
    if cpus.is_empty() {
        None
    } else {
        Some(format_cpu_range_list(&cpus))
    }
}

/// List online NUMA node indices from `/sys/devices/system/node`.
fn online_numa_nodes() -> Vec<usize> {
    let mut nodes = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/sys/devices/system/node") {
        for e in entries.flatten() {
            if let Some(idx) = e
                .file_name()
                .to_str()
                .and_then(|name| name.strip_prefix("node"))
                .and_then(|i| i.parse::<usize>().ok())
            {
                nodes.push(idx);
            }
        }
    }
    nodes.sort_unstable();
    nodes
}

/// Parse a kernel cpulist string (`"0-3"`, `"0,2-4"`) into CPU indices.
fn parse_cpu_range_list(s: &str) -> Vec<usize> {
    let mut cpus = Vec::new();
    for tok in s.split(',').filter(|t| !t.is_empty()) {
        if let Some((a, b)) = tok.split_once('-') {
            if let (Ok(a), Ok(b)) = (a.trim().parse::<usize>(), b.trim().parse::<usize>()) {
                cpus.extend(a..=b);
            }
        } else if let Ok(c) = tok.trim().parse::<usize>() {
            cpus.push(c);
        }
    }
    cpus
}

/// Format sorted CPU indices as a compressed kernel-style range list (`"0-3"`).
fn format_cpu_range_list(cpus: &std::collections::BTreeSet<usize>) -> String {
    let mut out = String::new();
    let mut iter = cpus.iter().copied().peekable();
    while let Some(start) = iter.next() {
        let mut end = start;
        while iter.peek() == Some(&(end + 1)) {
            end = iter.next().unwrap();
        }
        if !out.is_empty() {
            out.push(',');
        }
        if start == end {
            out.push_str(&start.to_string());
        } else {
            out.push_str(&format!("{start}-{end}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mpol_roundtrip() {
        for s in ["default", "preferred", "bind", "interleave", "local"] {
            let t = mpol_from_string(s).unwrap();
            assert_eq!(mpol_to_string(t), Some(s));
        }
        assert_eq!(mpol_from_string("bogus"), None);
        assert_eq!(mpol_from_string(" bind "), Some(MPOL_BIND));
    }

    #[test]
    fn test_parse_numa_mask() {
        assert_eq!(parse_numa_mask("0"), Some(vec![0]));
        assert_eq!(parse_numa_mask("0-3"), Some(vec![0, 1, 2, 3]));
        assert_eq!(parse_numa_mask("0 2 4"), Some(vec![0, 2, 4]));
        assert_eq!(parse_numa_mask("0,2"), Some(vec![0, 2]));
        assert_eq!(parse_numa_mask(""), Some(vec![]));
        assert_eq!(parse_numa_mask("x"), None);
    }

    #[test]
    fn test_get_type() {
        // Unset type with a node mask => PREFERRED.
        let p = NumaPolicy {
            type_: -1,
            nodes: vec![0],
        };
        assert_eq!(p.get_type(), MPOL_PREFERRED);
        // Unset with no mask => invalid.
        let p = NumaPolicy {
            type_: -1,
            nodes: vec![],
        };
        assert_eq!(p.get_type(), -1);
        // Explicit type wins.
        let p = NumaPolicy {
            type_: MPOL_BIND,
            nodes: vec![0],
        };
        assert_eq!(p.get_type(), MPOL_BIND);
    }

    #[test]
    fn test_is_valid() {
        // default/local without mask are valid.
        assert!(
            NumaPolicy {
                type_: MPOL_DEFAULT,
                nodes: vec![]
            }
            .is_valid()
        );
        assert!(
            NumaPolicy {
                type_: MPOL_LOCAL,
                nodes: vec![]
            }
            .is_valid()
        );
        // bind/interleave require a mask.
        assert!(
            !NumaPolicy {
                type_: MPOL_BIND,
                nodes: vec![]
            }
            .is_valid()
        );
        assert!(
            !NumaPolicy {
                type_: MPOL_INTERLEAVE,
                nodes: vec![]
            }
            .is_valid()
        );
        assert!(
            NumaPolicy {
                type_: MPOL_BIND,
                nodes: vec![0]
            }
            .is_valid()
        );
        // preferred accepts exactly one node when a mask is given.
        assert!(
            NumaPolicy {
                type_: MPOL_PREFERRED,
                nodes: vec![0]
            }
            .is_valid()
        );
        assert!(
            !NumaPolicy {
                type_: MPOL_PREFERRED,
                nodes: vec![0, 1]
            }
            .is_valid()
        );
        // preferred with no mask is valid (resets to default).
        assert!(
            NumaPolicy {
                type_: MPOL_PREFERRED,
                nodes: vec![]
            }
            .is_valid()
        );
    }

    #[test]
    fn test_to_mempolicy() {
        // DEFAULT/LOCAL => NULL mask.
        let (m, mn) = NumaPolicy {
            type_: MPOL_DEFAULT,
            nodes: vec![],
        }
        .to_mempolicy();
        assert!(m.is_empty() && mn == 0);
        let (m, mn) = NumaPolicy {
            type_: MPOL_LOCAL,
            nodes: vec![0],
        }
        .to_mempolicy();
        assert!(m.is_empty() && mn == 0);
        // PREFERRED with no mask => NULL.
        let (m, mn) = NumaPolicy {
            type_: MPOL_PREFERRED,
            nodes: vec![],
        }
        .to_mempolicy();
        assert!(m.is_empty() && mn == 0);
        // BIND node 0 => single c_ulong [0x1], maxnode = one word of bits.
        let (m, mn) = NumaPolicy {
            type_: MPOL_BIND,
            nodes: vec![0],
        }
        .to_mempolicy();
        assert_eq!(m, vec![1 as libc::c_ulong]);
        assert_eq!(mn, libc::c_ulong::BITS as libc::c_ulong);
        // BIND node 65 => two words, bit set in the second.
        let (m, _mn) = NumaPolicy {
            type_: MPOL_BIND,
            nodes: vec![65],
        }
        .to_mempolicy();
        assert_eq!(m.len(), 2);
        assert_eq!(
            m[1],
            (1 as libc::c_ulong) << (65 % libc::c_ulong::BITS as usize)
        );
    }
}
