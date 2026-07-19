//! Shared core for systemd's versioned-directory (`.v/`) resource picking.
//!
//! A faithful port of the `path_pick` selection logic in upstream
//! `src/shared/vpick.c`. Given a path to a `NAME.v/` directory (or an explicit
//! basename pattern), it enumerates versioned entries named
//!
//! ```text
//! <basename>_<version>_<architecture>+<tries-left>-<tries-done><suffix>
//! ```
//!
//! (each field optional) and returns the "best" match: newest version,
//! native architecture, most boot-tries left.
//!
//! Used by the `systemd-vpick` tool, `systemd-dissect --discover`, and PID 1's
//! `RootDirectory=`/`RootImage=` resolution.

use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

// ── Architecture ──────────────────────────────────────────────────────────

/// The subset of systemd architectures relevant to `.v` resolution. The
/// `to_string` values match systemd's `architecture_to_string`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Arch {
    X86_64,
    X86,
    Arm64,
    Arm,
    Ia64,
    Ppc64Le,
    Ppc64,
    Ppc,
    S390x,
    S390,
    Riscv64,
    Riscv32,
    Mips64Le,
    MipsLe,
    Loongarch64,
}

const ARCH_TABLE: &[(&str, Arch)] = &[
    ("x86-64", Arch::X86_64),
    ("x86", Arch::X86),
    ("arm64", Arch::Arm64),
    ("arm", Arch::Arm),
    ("ia64", Arch::Ia64),
    ("ppc64-le", Arch::Ppc64Le),
    ("ppc64", Arch::Ppc64),
    ("ppc", Arch::Ppc),
    ("s390x", Arch::S390x),
    ("s390", Arch::S390),
    ("riscv64", Arch::Riscv64),
    ("riscv32", Arch::Riscv32),
    ("mips64-le", Arch::Mips64Le),
    ("mips-le", Arch::MipsLe),
    ("loongarch64", Arch::Loongarch64),
];

pub fn arch_from_string(s: &str) -> Option<Arch> {
    ARCH_TABLE.iter().find(|(n, _)| *n == s).map(|(_, a)| *a)
}

pub fn arch_to_string(a: Arch) -> &'static str {
    ARCH_TABLE.iter().find(|(_, x)| *x == a).map(|(n, _)| *n).unwrap_or("")
}

/// The native architecture, derived from `uname(2)`, mapped the same way
/// systemd's `uname_architecture()` does.
pub fn native_arch() -> Option<Arch> {
    let mut uts: libc::utsname = unsafe { std::mem::zeroed() };
    if unsafe { libc::uname(&mut uts) } != 0 {
        return None;
    }
    let machine: Vec<u8> = uts
        .machine
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    let m = String::from_utf8_lossy(&machine);
    Some(match m.as_ref() {
        "x86_64" => Arch::X86_64,
        "i686" | "i586" | "i486" | "i386" => Arch::X86,
        "aarch64" | "aarch64_be" => Arch::Arm64,
        m if m.starts_with("arm") => Arch::Arm,
        "ia64" => Arch::Ia64,
        "ppc64le" => Arch::Ppc64Le,
        "ppc64" => Arch::Ppc64,
        "ppc" => Arch::Ppc,
        "s390x" => Arch::S390x,
        "s390" => Arch::S390,
        "riscv64" => Arch::Riscv64,
        "riscv32" => Arch::Riscv32,
        "mips64" => Arch::Mips64Le,
        "mips" => Arch::MipsLe,
        "loongarch64" => Arch::Loongarch64,
        _ => return None,
    })
}

/// The secondary architecture for the native one (e.g. x86 on x86-64), if any.
pub fn secondary_arch() -> Option<Arch> {
    match native_arch() {
        Some(Arch::X86_64) => Some(Arch::X86),
        Some(Arch::Arm64) => Some(Arch::Arm),
        Some(Arch::Ppc64) => Some(Arch::Ppc),
        _ => None,
    }
}

// ── Version comparison (matches the crate's analyze `compare-versions`) ─────

pub fn compare_versions(a: &str, b: &str) -> i32 {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => 0,
        (true, false) => -1,
        (false, true) => 1,
        (false, false) => compare_kernel_versions(a, b),
    }
}

fn compare_kernel_versions(a: &str, b: &str) -> i32 {
    fn split(s: &str) -> Vec<u32> {
        s.split(|c: char| !c.is_ascii_digit() && c != '.')
            .next()
            .unwrap_or("")
            .split('.')
            .filter_map(|p| p.parse::<u32>().ok())
            .collect()
    }
    let av = split(a);
    let bv = split(b);
    let len = av.len().max(bv.len());
    for i in 0..len {
        let x = av.get(i).copied().unwrap_or(0);
        let y = bv.get(i).copied().unwrap_or(0);
        if x < y {
            return -1;
        }
        if x > y {
            return 1;
        }
    }
    0
}

fn cmp<T: Ord>(a: T, b: T) -> i32 {
    match a.cmp(&b) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

// ── Validity helpers ────────────────────────────────────────────────────────

const VERSION_CHARS: &str =
    "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ.~_-+^";

pub fn version_is_valid(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| VERSION_CHARS.contains(c))
}

pub fn filename_part_is_valid(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && !s.contains('/') && !s.contains('\0')
}

// ── Inode type ──────────────────────────────────────────────────────────────

pub fn dt_bit(dt: u32) -> u32 {
    1u32 << dt
}

pub fn mode_to_dt(mode: u32) -> u32 {
    match mode & libc::S_IFMT {
        libc::S_IFREG => libc::DT_REG as u32,
        libc::S_IFDIR => libc::DT_DIR as u32,
        libc::S_IFLNK => libc::DT_LNK as u32,
        libc::S_IFBLK => libc::DT_BLK as u32,
        libc::S_IFCHR => libc::DT_CHR as u32,
        libc::S_IFIFO => libc::DT_FIFO as u32,
        libc::S_IFSOCK => libc::DT_SOCK as u32,
        _ => libc::DT_UNKNOWN as u32,
    }
}

pub fn inode_type_to_string(mode: u32) -> Option<&'static str> {
    Some(match mode & libc::S_IFMT {
        libc::S_IFREG => "reg",
        libc::S_IFDIR => "dir",
        libc::S_IFLNK => "lnk",
        libc::S_IFBLK => "blk",
        libc::S_IFCHR => "chr",
        libc::S_IFIFO => "fifo",
        libc::S_IFSOCK => "sock",
        _ => return None,
    })
}

pub fn inode_type_from_string(s: &str) -> Option<u32> {
    Some(match s {
        "reg" => libc::S_IFREG,
        "dir" => libc::S_IFDIR,
        "lnk" => libc::S_IFLNK,
        "blk" => libc::S_IFBLK,
        "chr" => libc::S_IFCHR,
        "fifo" => libc::S_IFIFO,
        "sock" => libc::S_IFSOCK,
        _ => return None,
    })
}

// ── Pick data ───────────────────────────────────────────────────────────────

pub const PICK_ARCHITECTURE: u32 = 1 << 0;
pub const PICK_TRIES: u32 = 1 << 1;
pub const PICK_RESOLVE: u32 = 1 << 2;

/// Default flags used by non-interactive callers (dissect, PID 1): honor the
/// architecture suffix and the boot-tries counter.
pub const PICK_DEFAULT: u32 = PICK_ARCHITECTURE | PICK_TRIES;

#[derive(Default, Clone)]
pub struct PickFilter {
    pub type_mask: u32,
    pub basename: Option<String>,
    pub version: Option<String>,
    /// `None` means "auto" (match native / secondary / arch-less entries).
    pub architecture: Option<Arch>,
    pub suffix: Option<String>,
}

pub struct PickResult {
    pub path: PathBuf,
    pub mode: u32,
    pub version: Option<String>,
    pub architecture: Option<Arch>,
    /// `None` == UINT_MAX (no tries counter present).
    pub tries_left: Option<u32>,
    pub tries_done: Option<u32>,
}

impl PickResult {
    pub fn is_dir(&self) -> bool {
        self.mode & libc::S_IFMT == libc::S_IFDIR
    }
}

/// Returns > 0 if `a` is the better pick, < 0 if `b` is better.
pub fn pick_result_compare(a: &PickResult, b: &PickResult, flags: u32) -> i32 {
    let mut d;

    // Prefer entries with tries left over those without.
    if flags & PICK_TRIES != 0 {
        d = cmp(a.tries_left != Some(0), b.tries_left != Some(0));
    } else {
        d = 0;
    }

    if d == 0 {
        d = compare_versions(
            a.version.as_deref().unwrap_or(""),
            b.version.as_deref().unwrap_or(""),
        );
    }

    if flags & PICK_ARCHITECTURE != 0 {
        let native = native_arch();
        if d == 0 {
            d = cmp(a.architecture == native, b.architecture == native);
        }
        let secondary = secondary_arch();
        if d == 0 && secondary.is_some() {
            d = cmp(a.architecture == secondary, b.architecture == secondary);
        }
    }

    if flags & PICK_TRIES != 0 {
        if d == 0 {
            d = cmp(a.tries_left.unwrap_or(u32::MAX), b.tries_left.unwrap_or(u32::MAX));
        }
        if d == 0 {
            d = -cmp(a.tries_done.unwrap_or(u32::MAX), b.tries_done.unwrap_or(u32::MAX));
        }
    }

    if d == 0 {
        d = cmp(a.path.as_os_str().as_bytes(), b.path.as_os_str().as_bytes());
    }

    d
}

/// Parse a `+left-done` (or `+left`) tries suffix. Returns `Some((left, done))`.
fn parse_tries(s: &str) -> Option<(u32, u32)> {
    let rest = s.strip_prefix('+')?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let left: u32 = digits.parse().ok()?;
    let after = &rest[digits.len()..];
    if after.is_empty() {
        Some((left, 0))
    } else if let Some(done_str) = after.strip_prefix('-') {
        if done_str.is_empty() || !done_str.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let done: u32 = done_str.parse().ok()?;
        Some((left, done))
    } else {
        None
    }
}

fn architecture_matches(filter: &PickFilter, a: Option<Arch>) -> bool {
    if let Some(fa) = filter.architecture {
        return a == Some(fa);
    }
    if a == native_arch() {
        return true;
    }
    if secondary_arch().is_some() && a == secondary_arch() {
        return true;
    }
    a.is_none()
}

fn stat_and_maybe_resolve(path: &Path, resolve: bool) -> std::io::Result<(u32, PathBuf)> {
    let md = std::fs::metadata(path)?; // follows symlinks
    let mode = md.mode();
    let out = if resolve {
        std::fs::canonicalize(path)?
    } else {
        path.to_path_buf()
    };
    Ok((mode, out))
}

/// Enumerate `dir` for versioned entries matching `filter`; return the best one.
fn make_choice(dir: &Path, filter: &PickFilter, flags: u32) -> Result<Option<PickResult>, String> {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => return Err(format!("Failed to read directory {}: {e}", dir.display())),
    };

    let mut best: Option<PickResult> = None;

    for ent in rd.flatten() {
        let dname = ent.file_name();
        let dname = dname.to_string_lossy().into_owned();

        let mut e: &str = if let Some(bn) = filter.basename.as_deref().filter(|s| !s.is_empty()) {
            match dname.strip_prefix(bn) {
                Some(rest) => match rest.strip_prefix('_') {
                    Some(r) => r,
                    None => continue,
                },
                None => continue,
            }
        } else {
            &dname
        };

        if let Some(sfx) = filter.suffix.as_deref().filter(|s| !s.is_empty()) {
            match e.strip_suffix(sfx) {
                Some(r) => e = r,
                None => continue,
            }
        }

        let mut owned = e.to_string();

        let mut tries_left = None;
        let mut tries_done = None;
        if flags & PICK_TRIES != 0
            && let Some(pos) = owned.rfind('+')
            && let Some((l, d)) = parse_tries(&owned[pos..])
        {
            tries_left = Some(l);
            tries_done = Some(d);
            owned.truncate(pos);
        }

        let mut arch = None;
        if flags & PICK_ARCHITECTURE != 0 {
            if let Some(pos) = owned.rfind('_') {
                arch = arch_from_string(&owned[pos + 1..]);
                if !architecture_matches(filter, arch) {
                    continue;
                }
                owned.truncate(pos);
            } else if !architecture_matches(filter, None) {
                continue;
            }
        }

        if !version_is_valid(&owned) {
            continue;
        }
        if let Some(v) = filter.version.as_deref()
            && v != owned
        {
            continue;
        }

        let entry_path = dir.join(&dname);
        let (mode, resolved) = match stat_and_maybe_resolve(&entry_path, flags & PICK_RESOLVE != 0) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if filter.type_mask != 0 && filter.type_mask & dt_bit(mode_to_dt(mode)) == 0 {
            continue;
        }

        let found = PickResult {
            path: resolved,
            mode,
            version: if owned.is_empty() { None } else { Some(owned) },
            architecture: arch,
            tries_left,
            tries_done,
        };

        match &best {
            None => best = Some(found),
            Some(b) => {
                if pick_result_compare(&found, b, flags) > 0 {
                    best = Some(found);
                }
            }
        }
    }

    Ok(best)
}

/// Resolve one path, handling `.v/` directory logic. Returns `Ok(None)` when
/// nothing matched. `path` should be absolute.
pub fn path_pick(path: &Path, filter: &PickFilter, flags: u32) -> Result<Option<PickResult>, String> {
    // Explicit basename: treat `path` as a `.v` dir regardless of its name.
    if filter.basename.is_some() {
        return make_choice(path, filter, flags);
    }

    let fname = match path.file_name().and_then(|n| n.to_str()) {
        Some(f) => f.to_string(),
        None => return bypass(path, filter, flags),
    };

    if let Some(stem) = fname.strip_suffix(".v") {
        let mut filter_bname = stem.to_string();
        if let Some(sfx) = filter.suffix.as_deref().filter(|s| !s.is_empty())
            && let Some(b) = filter_bname.strip_suffix(sfx)
        {
            filter_bname = b.to_string();
        }
        let sub = PickFilter {
            type_mask: filter.type_mask,
            basename: Some(filter_bname),
            version: filter.version.clone(),
            architecture: filter.architecture,
            suffix: filter.suffix.clone(),
        };
        return make_choice(path, &sub, flags);
    }

    // `<basename>___<suffix>` wildcard whose parent dir ends in `.v`.
    if let Some(pos) = fname.rfind("___") {
        let filter_bname = fname[..pos].to_string();
        let filter_suffix = &fname[pos + 3..];
        let dir = match path.parent() {
            Some(d) if !d.as_os_str().is_empty() => d,
            _ => return bypass(path, filter, flags),
        };
        let parent_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !parent_name.ends_with(".v") {
            return bypass(path, filter, flags);
        }
        let sub = PickFilter {
            type_mask: filter.type_mask,
            basename: Some(filter_bname),
            version: filter.version.clone(),
            architecture: filter.architecture,
            suffix: if filter_suffix.is_empty() {
                None
            } else {
                Some(filter_suffix.to_string())
            },
        };
        return make_choice(dir, &sub, flags);
    }

    bypass(path, filter, flags)
}

/// No `.v` logic applies: use the path literally.
fn bypass(path: &Path, filter: &PickFilter, flags: u32) -> Result<Option<PickResult>, String> {
    let (mode, resolved) = match stat_and_maybe_resolve(path, flags & PICK_RESOLVE != 0) {
        Ok(v) => v,
        Err(e) => return Err(format!("Failed to open {}: {e}", path.display())),
    };
    if filter.type_mask != 0 && filter.type_mask & dt_bit(mode_to_dt(mode)) == 0 {
        return Ok(None);
    }
    Ok(Some(PickResult {
        path: resolved,
        mode,
        version: filter.version.clone(),
        architecture: filter.architecture,
        tries_left: None,
        tries_done: None,
    }))
}

/// Whether a path participates in `.v` resolution: it (or, for a `___` pattern,
/// its parent) ends in `.v`.
pub fn path_uses_vpick(path: &Path) -> bool {
    let fname = match path.file_name().and_then(|n| n.to_str()) {
        Some(f) => f,
        None => return false,
    };
    if fname.ends_with(".v") {
        return true;
    }
    if fname.contains("___") {
        return path
            .parent()
            .and_then(|d| d.file_name())
            .and_then(|n| n.to_str())
            .map(|p| p.ends_with(".v"))
            .unwrap_or(false);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tries_parsing() {
        assert_eq!(parse_tries("+0-5"), Some((0, 5)));
        assert_eq!(parse_tries("+2-3"), Some((2, 3)));
        assert_eq!(parse_tries("+7"), Some((7, 0)));
        assert_eq!(parse_tries("+"), None);
        assert_eq!(parse_tries("0-5"), None);
    }

    #[test]
    fn version_compare_order() {
        assert!(compare_versions("7.5.13", "7.5.14") < 0);
        assert!(compare_versions("7.7.0", "7.6.0") > 0);
        assert_eq!(compare_versions("33.5", "33.5"), 0);
        assert!(compare_versions("", "1.0") < 0);
    }

    #[test]
    fn arch_roundtrip() {
        assert_eq!(arch_from_string("x86-64"), Some(Arch::X86_64));
        assert_eq!(arch_from_string("arm64"), Some(Arch::Arm64));
        assert_eq!(arch_from_string("nonsense"), None);
        assert_eq!(arch_to_string(Arch::Arm64), "arm64");
    }

    #[test]
    fn uses_vpick() {
        assert!(path_uses_vpick(Path::new("/x/foo.raw.v")));
        assert!(!path_uses_vpick(Path::new("/x/foo.raw")));
    }
}
