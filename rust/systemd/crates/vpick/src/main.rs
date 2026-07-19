//! `systemd-vpick` - pick an entry from a versioned (`.v/`) directory.
//!
//! A faithful port of upstream `src/vpick/vpick-tool.c` + the `path_pick`
//! selection logic in `src/shared/vpick.c`. Given a path to a `NAME.v/`
//! directory (or an explicit `--basename=` pattern), it enumerates the
//! versioned entries inside, whose filenames follow the `BVAS` convention
//!
//!     <basename>_<version>_<architecture>+<tries-left>-<tries-done><suffix>
//!
//! (each field optional), filters them by the requested version / architecture
//! / suffix / inode type, and prints the "best" (newest, native-arch,
//! most-tries-left) match.

use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

// ── Architecture ──────────────────────────────────────────────────────────

/// The subset of systemd architectures relevant here. `to_string` values match
/// systemd's `architecture_to_string`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Arch {
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

fn arch_from_string(s: &str) -> Option<Arch> {
    ARCH_TABLE.iter().find(|(n, _)| *n == s).map(|(_, a)| *a)
}

fn arch_to_string(a: Arch) -> &'static str {
    ARCH_TABLE.iter().find(|(_, x)| *x == a).map(|(n, _)| *n).unwrap_or("")
}

/// The native architecture, derived from `uname(2)`'s machine field, mapped the
/// same way systemd's `uname_architecture()` does.
fn native_arch() -> Option<Arch> {
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
fn secondary_arch() -> Option<Arch> {
    match native_arch() {
        Some(Arch::X86_64) => Some(Arch::X86),
        Some(Arch::Arm64) => Some(Arch::Arm),
        Some(Arch::Ppc64) => Some(Arch::Ppc),
        _ => None,
    }
}

// ── Version comparison (matches the crate's analyze `compare-versions`) ─────

fn compare_versions(a: &str, b: &str) -> i32 {
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

fn version_is_valid(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| VERSION_CHARS.contains(c))
}

fn filename_part_is_valid(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && !s.contains('/') && !s.contains('\0')
}

// ── Inode type ──────────────────────────────────────────────────────────────

/// Bits in a type mask, indexed by DT_* values.
fn dt_bit(dt: u32) -> u32 {
    1u32 << dt
}

fn mode_to_dt(mode: u32) -> u32 {
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

fn inode_type_to_string(mode: u32) -> Option<&'static str> {
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

fn inode_type_from_string(s: &str) -> Option<u32> {
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

const PICK_ARCHITECTURE: u32 = 1 << 0;
const PICK_TRIES: u32 = 1 << 1;
const PICK_RESOLVE: u32 = 1 << 2;

#[derive(Default, Clone)]
struct Filter {
    type_mask: u32,
    basename: Option<String>,
    version: Option<String>,
    /// `None` means "auto" (match native / secondary / arch-less entries).
    architecture: Option<Arch>,
    suffix: Option<String>,
}

struct Pick {
    path: PathBuf,
    mode: u32,
    version: Option<String>,
    architecture: Option<Arch>,
    /// `None` == UINT_MAX (no tries counter present).
    tries_left: Option<u32>,
    tries_done: Option<u32>,
}

/// Returns > 0 if `a` is the better pick, < 0 if `b` is better.
fn pick_result_compare(a: &Pick, b: &Pick, flags: u32) -> i32 {
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
        // Prefer more tries left (None == UINT_MAX).
        if d == 0 {
            d = cmp(a.tries_left.unwrap_or(u32::MAX), b.tries_left.unwrap_or(u32::MAX));
        }
        // Prefer fewer attempts done so far.
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

fn architecture_matches(filter: &Filter, a: Option<Arch>) -> bool {
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

// ── Selection ───────────────────────────────────────────────────────────────

/// Stat a path (following symlinks) and, when `resolve` is set, canonicalize it.
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

/// Enumerate `dir` for versioned entries matching `filter`, and return the best
/// one. Returns `Ok(None)` when nothing matched.
fn make_choice(dir: &Path, filter: &Filter, flags: u32) -> Result<Option<Pick>, String> {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => return Err(format!("Failed to read directory {}: {e}", dir.display())),
    };

    let mut best: Option<Pick> = None;

    for ent in rd.flatten() {
        let dname = ent.file_name();
        let dname = dname.to_string_lossy().into_owned();

        // Strip basename prefix (+ the mandatory '_') if a basename is set.
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

        // Strip suffix.
        if let Some(sfx) = filter.suffix.as_deref().filter(|s| !s.is_empty()) {
            match e.strip_suffix(sfx) {
                Some(r) => e = r,
                None => continue,
            }
        }

        let mut owned = e.to_string();

        // Tries counter (`+left-done`).
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

        // Architecture (last `_`-separated field).
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

        // Inode type filter (against the followed type).
        if filter.type_mask != 0 && filter.type_mask & dt_bit(mode_to_dt(mode)) == 0 {
            continue;
        }

        let found = Pick {
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

/// Resolve one path argument into a pick, handling `.v/` directory logic.
fn path_pick_one(path: &Path, filter: &Filter, flags: u32) -> Result<Option<Pick>, String> {
    // Explicit basename shortcut: treat `path` as a `.v` dir regardless of name.
    if filter.basename.is_some() {
        return make_choice(path, filter, flags);
    }

    let fname = match path.file_name().and_then(|n| n.to_str()) {
        Some(f) => f.to_string(),
        None => return bypass(path, filter, flags),
    };

    if let Some(stem) = fname.strip_suffix(".v") {
        // `/foo/bar/baz.v` -> search `baz` (minus suffix) inside it.
        let mut filter_bname = stem.to_string();
        if let Some(sfx) = filter.suffix.as_deref().filter(|s| !s.is_empty())
            && let Some(b) = filter_bname.strip_suffix(sfx)
        {
            filter_bname = b.to_string();
        }
        let sub = Filter {
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
        let sub = Filter {
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
fn bypass(path: &Path, filter: &Filter, flags: u32) -> Result<Option<Pick>, String> {
    let (mode, resolved) = match stat_and_maybe_resolve(path, flags & PICK_RESOLVE != 0) {
        Ok(v) => v,
        Err(e) => return Err(format!("Failed to open {}: {e}", path.display())),
    };
    if filter.type_mask != 0 && filter.type_mask & dt_bit(mode_to_dt(mode)) == 0 {
        return Ok(None);
    }
    Ok(Some(Pick {
        path: resolved,
        mode,
        version: filter.version.clone(),
        architecture: filter.architecture,
        tries_left: None,
        tries_done: None,
    }))
}

// ── CLI ─────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum Print {
    Path,
    Filename,
    Version,
    Type,
    Architecture,
    Tries,
    All,
}

fn print_from_string(s: &str) -> Option<Print> {
    Some(match s {
        "path" => Print::Path,
        "filename" => Print::Filename,
        "version" => Print::Version,
        "type" => Print::Type,
        "architecture" | "arch" => Print::Architecture,
        "tries" => Print::Tries,
        "all" => Print::All,
        _ => return None,
    })
}

fn help() {
    println!(
        "systemd-vpick [OPTIONS...] PATH...\n\n\
         Pick entry from versioned directory.\n\n  \
         -h --help            Show this help\n     \
         --version         Show package version\n\n\
         Lookup Keys:\n  \
         -B --basename=BASENAME\n                       Look for specified basename\n  \
         -V VERSION           Look for specified version\n  \
         -A ARCH              Look for specified architecture\n  \
         -S --suffix=SUFFIX   Look for specified suffix\n  \
         -t --type=TYPE       Look for specified inode type\n\n\
         Output:\n  \
         -p --print=filename  Print selected filename rather than path\n     \
         --resolve=yes     Canonicalize the result path"
    );
}

struct Options {
    filter_basename: Option<String>,
    filter_version: Option<String>,
    filter_architecture: Option<Arch>,
    filter_suffix: Option<String>,
    filter_type_mask: u32,
    print: Option<Print>,
    flags: u32,
    paths: Vec<String>,
}

enum ParseOutcome {
    Ok(Options),
    ExitOk,
    ExitErr,
}

fn parse_argv(args: &[String]) -> ParseOutcome {
    let mut o = Options {
        filter_basename: None,
        filter_version: None,
        filter_architecture: None,
        filter_suffix: None,
        filter_type_mask: 0,
        print: None,
        flags: PICK_ARCHITECTURE | PICK_TRIES,
        paths: Vec::new(),
    };

    // Fetch the value for an option that takes an argument, from either the
    // inline `=` form or the following token.
    let mut i = 0;
    let mut positional_only = false;
    while i < args.len() {
        let a = &args[i];
        if positional_only || a == "-" || !a.starts_with('-') {
            o.paths.push(a.clone());
            i += 1;
            continue;
        }
        if a == "--" {
            positional_only = true;
            i += 1;
            continue;
        }

        // Split into (name, inline value) for long opts.
        let take_value = |i: &mut usize, inline: Option<&str>| -> Option<String> {
            if let Some(v) = inline {
                return Some(v.to_string());
            }
            *i += 1;
            args.get(*i).cloned()
        };

        if let Some(rest) = a.strip_prefix("--") {
            let (name, inline) = match rest.split_once('=') {
                Some((n, v)) => (n, Some(v)),
                None => (rest, None),
            };
            match name {
                "help" => {
                    help();
                    return ParseOutcome::ExitOk;
                }
                "version" => {
                    println!("systemd {} (systemd-vpick)", env!("CARGO_PKG_VERSION"));
                    return ParseOutcome::ExitOk;
                }
                "basename" => match take_value(&mut i, inline) {
                    Some(v) if filename_part_is_valid(&v) => o.filter_basename = Some(v),
                    Some(v) => {
                        eprintln!("Invalid basename string: {v}");
                        return ParseOutcome::ExitErr;
                    }
                    None => return ParseOutcome::ExitErr,
                },
                "suffix" => match take_value(&mut i, inline) {
                    Some(v) if filename_part_is_valid(&v) => o.filter_suffix = Some(v),
                    Some(v) => {
                        eprintln!("Invalid suffix string: {v}");
                        return ParseOutcome::ExitErr;
                    }
                    None => return ParseOutcome::ExitErr,
                },
                "type" => match take_value(&mut i, inline) {
                    Some(v) => {
                        if !set_type(&mut o, &v) {
                            return ParseOutcome::ExitErr;
                        }
                    }
                    None => return ParseOutcome::ExitErr,
                },
                "print" => match take_value(&mut i, inline) {
                    Some(v) => match print_from_string(&v) {
                        Some(p) => o.print = Some(p),
                        None => {
                            eprintln!("Unknown --print= argument: {v}");
                            return ParseOutcome::ExitErr;
                        }
                    },
                    None => return ParseOutcome::ExitErr,
                },
                "resolve" => match take_value(&mut i, inline) {
                    Some(v) => match parse_boolean(&v) {
                        Some(true) => o.flags |= PICK_RESOLVE,
                        Some(false) => o.flags &= !PICK_RESOLVE,
                        None => {
                            eprintln!("Failed to parse --resolve= value: {v}");
                            return ParseOutcome::ExitErr;
                        }
                    },
                    None => return ParseOutcome::ExitErr,
                },
                _ => {
                    eprintln!("Unknown option --{name}");
                    return ParseOutcome::ExitErr;
                }
            }
            i += 1;
            continue;
        }

        // Short options. `-h` exits; the rest take a value from the remainder
        // of this arg (`-V7.5`) or the next token (`-V 7.5`), so there is no
        // meaningful bundling to loop over.
        let chars: Vec<char> = a[1..].chars().collect();
        let c = chars[0];
        let inline: Option<String> = if chars.len() > 1 {
            Some(chars[1..].iter().collect())
        } else {
            None
        };
        // Resolve the value for a value-taking short option.
        let value = |i: &mut usize| -> Option<String> {
            if let Some(v) = &inline {
                return Some(v.clone());
            }
            *i += 1;
            args.get(*i).cloned()
        };
        match c {
            'h' => {
                help();
                return ParseOutcome::ExitOk;
            }
            'B' => match value(&mut i) {
                Some(v) if filename_part_is_valid(&v) => o.filter_basename = Some(v),
                Some(v) => {
                    eprintln!("Invalid basename string: {v}");
                    return ParseOutcome::ExitErr;
                }
                None => return ParseOutcome::ExitErr,
            },
            'V' => match value(&mut i) {
                Some(v) if version_is_valid(&v) => o.filter_version = Some(v),
                Some(v) => {
                    eprintln!("Invalid version string: {v}");
                    return ParseOutcome::ExitErr;
                }
                None => return ParseOutcome::ExitErr,
            },
            'A' => match value(&mut i) {
                Some(v) => {
                    if !set_arch(&mut o, &v) {
                        return ParseOutcome::ExitErr;
                    }
                }
                None => return ParseOutcome::ExitErr,
            },
            'S' => match value(&mut i) {
                Some(v) if filename_part_is_valid(&v) => o.filter_suffix = Some(v),
                Some(v) => {
                    eprintln!("Invalid suffix string: {v}");
                    return ParseOutcome::ExitErr;
                }
                None => return ParseOutcome::ExitErr,
            },
            't' => match value(&mut i) {
                Some(v) => {
                    if !set_type(&mut o, &v) {
                        return ParseOutcome::ExitErr;
                    }
                }
                None => return ParseOutcome::ExitErr,
            },
            'p' => match value(&mut i) {
                Some(v) => match print_from_string(&v) {
                    Some(p) => o.print = Some(p),
                    None => {
                        eprintln!("Unknown --print= argument: {v}");
                        return ParseOutcome::ExitErr;
                    }
                },
                None => return ParseOutcome::ExitErr,
            },
            other => {
                eprintln!("Invalid option -- '{other}'");
                return ParseOutcome::ExitErr;
            }
        }
        i += 1;
    }

    ParseOutcome::Ok(o)
}

fn set_arch(o: &mut Options, v: &str) -> bool {
    match v {
        "native" => o.filter_architecture = native_arch(),
        "secondary" => match secondary_arch() {
            Some(a) => o.filter_architecture = Some(a),
            None => {
                eprintln!("Local architecture has no secondary architecture.");
                return false;
            }
        },
        "auto" => o.filter_architecture = None,
        _ => match arch_from_string(v) {
            Some(a) => o.filter_architecture = Some(a),
            None => {
                eprintln!("Unknown architecture: {v}");
                return false;
            }
        },
    }
    true
}

fn set_type(o: &mut Options, v: &str) -> bool {
    if v.is_empty() {
        o.filter_type_mask = 0;
        return true;
    }
    match inode_type_from_string(v) {
        Some(m) => {
            o.filter_type_mask |= dt_bit(mode_to_dt(m));
            true
        }
        None => {
            eprintln!("Unknown inode type: {v}");
            false
        }
    }
}

fn parse_boolean(s: &str) -> Option<bool> {
    match s {
        "1" | "yes" | "y" | "true" | "t" | "on" => Some(true),
        "0" | "no" | "n" | "false" | "f" | "off" => Some(false),
        _ => None,
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut opts = match parse_argv(&args) {
        ParseOutcome::Ok(o) => o,
        ParseOutcome::ExitOk => return ExitCode::SUCCESS,
        ParseOutcome::ExitErr => return ExitCode::from(1),
    };

    if opts.paths.is_empty() {
        eprintln!("Path to resolve must be specified.");
        return ExitCode::from(1);
    }
    let print = opts.print.unwrap_or(Print::Path);
    let flags = opts.flags;

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));

    for arg in std::mem::take(&mut opts.paths) {
        let p = {
            let ap = Path::new(&arg);
            if ap.is_absolute() {
                ap.to_path_buf()
            } else {
                cwd.join(ap)
            }
        };

        let filter = Filter {
            type_mask: opts.filter_type_mask,
            basename: opts.filter_basename.clone(),
            version: opts.filter_version.clone(),
            architecture: opts.filter_architecture,
            suffix: opts.filter_suffix.clone(),
        };

        let result = match path_pick_one(&p, &filter, flags) {
            Ok(Some(r)) => r,
            Ok(None) => {
                eprintln!("No matching version for '{}' found.", p.display());
                return ExitCode::from(1);
            }
            Err(e) => {
                eprintln!("Failed to pick version for '{}': {e}", p.display());
                return ExitCode::from(1);
            }
        };

        if !print_result(&result, print) {
            return ExitCode::from(1);
        }
    }

    ExitCode::SUCCESS
}

fn print_result(result: &Pick, print: Print) -> bool {
    match print {
        Print::Path => {
            let mut s = result.path.to_string_lossy().into_owned();
            if result.mode & libc::S_IFMT == libc::S_IFDIR && !s.ends_with('/') {
                s.push('/');
            }
            println!("{s}");
        }
        Print::Filename => match result.path.file_name() {
            Some(f) => println!("{}", f.to_string_lossy()),
            None => {
                eprintln!("Failed to extract filename from path '{}'.", result.path.display());
                return false;
            }
        },
        Print::Version => match &result.version {
            Some(v) => println!("{v}"),
            None => {
                eprintln!("No version information discovered.");
                return false;
            }
        },
        Print::Type => match inode_type_to_string(result.mode) {
            Some(t) => println!("{t}"),
            None => {
                eprintln!("No inode type information discovered.");
                return false;
            }
        },
        Print::Architecture => match result.architecture {
            Some(a) => println!("{}", arch_to_string(a)),
            None => {
                eprintln!("No architecture information discovered.");
                return false;
            }
        },
        Print::Tries => match (result.tries_left, result.tries_done) {
            (Some(l), Some(d)) => println!("+{l}-{d}"),
            _ => {
                eprintln!("No tries left/tries done information discovered.");
                return false;
            }
        },
        Print::All => {
            println!("     Path: {}", result.path.display());
            if let Some(v) = &result.version {
                println!("  Version: {v}");
            }
            if let Some(t) = inode_type_to_string(result.mode) {
                println!("     Type: {t}");
            }
            if let Some(a) = result.architecture {
                println!("     Arch: {}", arch_to_string(a));
            }
            if let (Some(l), Some(d)) = (result.tries_left, result.tries_done) {
                println!("Tries left: {l}");
                println!("Tries done: {d}");
            }
        }
    }
    true
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
        assert_eq!(parse_tries("0-5"), None); // no leading '+'
        assert_eq!(parse_tries("+a-b"), None);
    }

    #[test]
    fn version_compare_order() {
        assert!(compare_versions("7.5.13", "7.5.14") < 0);
        assert!(compare_versions("7.7.0", "7.6.0") > 0);
        assert_eq!(compare_versions("33.5", "33.5"), 0);
        assert!(compare_versions("", "1.0") < 0);
        assert!(compare_versions("38.0", "37.0") > 0);
    }

    #[test]
    fn arch_roundtrip() {
        assert_eq!(arch_from_string("x86-64"), Some(Arch::X86_64));
        assert_eq!(arch_from_string("arm64"), Some(Arch::Arm64));
        assert_eq!(arch_from_string("ia64"), Some(Arch::Ia64));
        assert_eq!(arch_from_string("nonsense"), None);
        assert_eq!(arch_to_string(Arch::Arm64), "arm64");
    }

    #[test]
    fn validity() {
        assert!(version_is_valid("7.5.14"));
        assert!(!version_is_valid(""));
        assert!(!version_is_valid("has/slash"));
        assert!(filename_part_is_valid("foo.raw"));
        assert!(!filename_part_is_valid(".."));
        assert!(!filename_part_is_valid("a/b"));
    }

    #[test]
    fn type_mask_roundtrip() {
        assert_eq!(inode_type_from_string("reg"), Some(libc::S_IFREG));
        assert_eq!(inode_type_from_string("dir"), Some(libc::S_IFDIR));
        assert_eq!(inode_type_from_string("bogus"), None);
        assert_eq!(inode_type_to_string(libc::S_IFDIR), Some("dir"));
    }

    #[test]
    fn tries_left_zero_deprioritized() {
        // An entry with 0 tries left loses to one with tries, even if newer.
        let newer_dead = Pick {
            path: PathBuf::from("/x/a_7.7.0"),
            mode: libc::S_IFREG,
            version: Some("7.7.0".into()),
            architecture: None,
            tries_left: Some(0),
            tries_done: Some(5),
        };
        let older_live = Pick {
            path: PathBuf::from("/x/a_7.5.14"),
            mode: libc::S_IFREG,
            version: Some("7.5.14".into()),
            architecture: None,
            tries_left: None,
            tries_done: None,
        };
        assert!(pick_result_compare(&older_live, &newer_dead, PICK_TRIES) > 0);
    }
}
