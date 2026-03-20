use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process;

use bzip2::read::BzDecoder;
use bzip2::write::BzEncoder;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use tar::{Archive, Builder, EntryType, Header};
use walkdir::WalkDir;
use xz2::read::XzDecoder;
use xz2::write::XzEncoder;

// ---------------------------------------------------------------------------
// Compression helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Compression {
    None,
    Gzip,
    Bzip2,
    Xz,
}

fn detect_from_extension(path: &str) -> Compression {
    if path.ends_with(".tar.gz") || path.ends_with(".tgz") {
        Compression::Gzip
    } else if path.ends_with(".tar.bz2") || path.ends_with(".tbz2") || path.ends_with(".tbz") {
        Compression::Bzip2
    } else if path.ends_with(".tar.xz") || path.ends_with(".txz") {
        Compression::Xz
    } else {
        Compression::None
    }
}

fn detect_from_magic(buf: &[u8]) -> Compression {
    if buf.len() >= 2 && buf[0] == 0x1f && buf[1] == 0x8b {
        Compression::Gzip
    } else if buf.len() >= 3 && &buf[..3] == b"BZh" {
        Compression::Bzip2
    } else if buf.len() >= 6 && buf[..6] == [0xFD, b'7', b'z', b'X', b'Z', 0x00] {
        Compression::Xz
    } else {
        Compression::None
    }
}

// ---------------------------------------------------------------------------
// Transform (--transform / --xform)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Transform {
    pattern: String,
    replacement: String,
    global: bool,
}

fn parse_transform(expr: &str) -> Result<Transform, String> {
    // Supports s/PATTERN/REPLACEMENT/[g]
    if !expr.starts_with("s") || expr.len() < 4 {
        return Err(format!("unsupported transform expression: {expr}"));
    }
    let sep = expr.as_bytes()[1] as char;
    let rest = &expr[2..];
    let parts: Vec<&str> = rest.splitn(3, sep).collect();
    if parts.len() < 2 {
        return Err(format!("bad transform expression: {expr}"));
    }
    let pattern = parts[0].to_string();
    let replacement = parts[1].to_string();
    let flags = if parts.len() > 2 { parts[2] } else { "" };
    let global = flags.contains('g');
    Ok(Transform {
        pattern,
        replacement,
        global,
    })
}

fn apply_transforms(path: &str, transforms: &[Transform]) -> String {
    let mut result = path.to_string();
    for t in transforms {
        if t.global {
            result = result.replace(&t.pattern, &t.replacement);
        } else {
            result = result.replacen(&t.pattern, &t.replacement, 1);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Exclude matching (simple glob: * matches anything, ? matches one char)
// ---------------------------------------------------------------------------

fn matches_exclude(path: &str, pattern: &str) -> bool {
    glob_match(pattern, path)
}

fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    glob_match_inner(&p, &t)
}

fn glob_match_inner(pattern: &[char], text: &[char]) -> bool {
    let (mut pi, mut ti) = (0, 0);
    let (mut star_pi, mut star_ti) = (usize::MAX, 0);

    while ti < text.len() {
        if pi < pattern.len() && (pattern[pi] == '?' || pattern[pi] == text[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < pattern.len() && pattern[pi] == '*' {
            star_pi = pi;
            star_ti = ti;
            pi += 1;
        } else if star_pi != usize::MAX {
            pi = star_pi + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }
    while pi < pattern.len() && pattern[pi] == '*' {
        pi += 1;
    }
    pi == pattern.len()
}

fn is_excluded(path: &str, excludes: &[String]) -> bool {
    for exc in excludes {
        if matches_exclude(path, exc) {
            return true;
        }
        // Also check basename
        if let Some(name) = Path::new(path).file_name().and_then(|n| n.to_str())
            && matches_exclude(name, exc)
        {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Argument parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct Args {
    create: bool,
    extract: bool,
    list: bool,
    file: Option<String>,
    directory: Option<String>,
    verbose: bool,
    compression: Option<Compression>,
    strip_components: usize,
    transforms: Vec<Transform>,
    excludes: Vec<String>,
    owner: Option<String>,
    group: Option<String>,
    sort_name: bool,
    no_same_owner: bool,
    no_same_permissions: bool,
    preserve_permissions: bool,
    paths: Vec<String>,
}

fn parse_args() -> Args {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut args = Args::default();
    let mut queue: VecDeque<String> = raw.into_iter().collect();

    // Handle combined short flags in first argument (e.g. -czf, czf, xzf)
    // GNU tar allows the first argument without a dash prefix.
    if let Some(first) = queue.front() {
        let first = first.clone();
        if !first.starts_with("--")
            && !first.is_empty()
            && first
                .trim_start_matches('-')
                .chars()
                .all(|c| "cxtzvjJfp".contains(c))
        {
            queue.pop_front();
            let flags = first.trim_start_matches('-');
            let mut expanded = Vec::new();
            for ch in flags.chars() {
                expanded.push(format!("-{ch}"));
            }
            // Re-insert in reverse so order is preserved
            for e in expanded.into_iter().rev() {
                queue.push_front(e);
            }
        }
    }

    while let Some(arg) = queue.pop_front() {
        match arg.as_str() {
            "--version" | "-V" => {
                println!("tar (rust-tar) {}", env!("CARGO_PKG_VERSION"));
                process::exit(0);
            }
            "-c" | "--create" => args.create = true,
            "-x" | "--extract" | "--get" => args.extract = true,
            "-t" | "--list" => args.list = true,
            "-f" => {
                args.file = queue.pop_front();
            }
            "-C" | "--directory" => {
                args.directory = queue.pop_front();
            }
            "-v" | "--verbose" => args.verbose = true,
            "-z" | "--gzip" | "--gunzip" => args.compression = Some(Compression::Gzip),
            "-j" | "--bzip2" => args.compression = Some(Compression::Bzip2),
            "-J" | "--xz" => args.compression = Some(Compression::Xz),
            "-p" | "--preserve-permissions" | "--same-permissions" => {
                args.preserve_permissions = true;
            }
            "--no-same-owner" => args.no_same_owner = true,
            "--no-same-permissions" => args.no_same_permissions = true,
            other => {
                if let Some(val) = other.strip_prefix("--file=") {
                    args.file = Some(val.to_string());
                } else if let Some(val) = other.strip_prefix("--directory=") {
                    args.directory = Some(val.to_string());
                } else if let Some(val) = other.strip_prefix("--strip-components=") {
                    args.strip_components = val.parse().unwrap_or(0);
                } else if let Some(val) = other.strip_prefix("--transform=") {
                    match parse_transform(val) {
                        Ok(t) => args.transforms.push(t),
                        Err(e) => {
                            eprintln!("tar: {e}");
                            process::exit(2);
                        }
                    }
                } else if let Some(val) = other.strip_prefix("--xform=") {
                    match parse_transform(val) {
                        Ok(t) => args.transforms.push(t),
                        Err(e) => {
                            eprintln!("tar: {e}");
                            process::exit(2);
                        }
                    }
                } else if let Some(val) = other.strip_prefix("--exclude=") {
                    args.excludes.push(val.to_string());
                } else if let Some(val) = other.strip_prefix("--owner=") {
                    args.owner = Some(val.to_string());
                } else if let Some(val) = other.strip_prefix("--group=") {
                    args.group = Some(val.to_string());
                } else if let Some(val) = other.strip_prefix("--sort=") {
                    args.sort_name = val == "name";
                } else if other.starts_with('-') && !other.starts_with("--") && other.len() > 1 {
                    // Potentially bundled short options like -xvf
                    let chars: Vec<char> = other[1..].chars().collect();
                    let mut i = 0;
                    while i < chars.len() {
                        match chars[i] {
                            'c' => args.create = true,
                            'x' => args.extract = true,
                            't' => args.list = true,
                            'v' => args.verbose = true,
                            'z' => args.compression = Some(Compression::Gzip),
                            'j' => args.compression = Some(Compression::Bzip2),
                            'J' => args.compression = Some(Compression::Xz),
                            'p' => args.preserve_permissions = true,
                            'f' => {
                                // Rest of chars is the filename, or next arg
                                let rest: String = chars[i + 1..].iter().collect();
                                if rest.is_empty() {
                                    args.file = queue.pop_front();
                                } else {
                                    args.file = Some(rest);
                                }
                                i = chars.len(); // break
                                continue;
                            }
                            'C' => {
                                let rest: String = chars[i + 1..].iter().collect();
                                if rest.is_empty() {
                                    args.directory = queue.pop_front();
                                } else {
                                    args.directory = Some(rest);
                                }
                                i = chars.len();
                                continue;
                            }
                            _ => {
                                eprintln!("tar: unknown option: -{}", chars[i]);
                                process::exit(2);
                            }
                        }
                        i += 1;
                    }
                } else if other.starts_with('-') {
                    eprintln!("tar: unrecognized option: {other}");
                    process::exit(2);
                } else {
                    args.paths.push(other.to_string());
                }
            }
        }
    }

    args
}

// ---------------------------------------------------------------------------
// Create
// ---------------------------------------------------------------------------

fn do_create(args: &Args) -> io::Result<()> {
    let compression = args.compression.unwrap_or_else(|| {
        args.file
            .as_deref()
            .map(detect_from_extension)
            .unwrap_or(Compression::None)
    });

    let writer: Box<dyn Write> = match args.file.as_deref() {
        Some("-") | None => Box::new(io::stdout().lock()),
        Some(path) => Box::new(File::create(path)?),
    };

    let compressed_writer: Box<dyn Write> = match compression {
        Compression::None => writer,
        Compression::Gzip => Box::new(GzEncoder::new(writer, flate2::Compression::default())),
        Compression::Bzip2 => Box::new(BzEncoder::new(writer, bzip2::Compression::default())),
        Compression::Xz => Box::new(XzEncoder::new(writer, 6)),
    };

    let mut builder = Builder::new(compressed_writer);

    if let Some(dir) = &args.directory {
        std::env::set_current_dir(dir)?;
    }

    if args.paths.is_empty() {
        eprintln!("tar: cowardly refusing to create an empty archive");
        process::exit(2);
    }

    for src in &args.paths {
        let src_path = Path::new(src);

        // Collect entries (for optional sorting)
        let mut entries: Vec<PathBuf> = Vec::new();

        if src_path.is_dir() {
            for entry in WalkDir::new(src).follow_links(false) {
                let entry = entry.map_err(io::Error::other)?;
                entries.push(entry.into_path());
            }
        } else {
            entries.push(src_path.to_path_buf());
        }

        if args.sort_name {
            entries.sort();
        }

        for path in &entries {
            let path_str = path.to_string_lossy();

            if is_excluded(&path_str, &args.excludes) {
                continue;
            }

            let archive_name = if !args.transforms.is_empty() {
                apply_transforms(&path_str, &args.transforms)
            } else {
                path_str.to_string()
            };

            // Strip leading / for safety
            let archive_name = archive_name.trim_start_matches('/');

            if args.verbose {
                eprintln!("{archive_name}");
            }

            if path.is_dir() {
                let mut header = Header::new_gnu();
                header.set_entry_type(EntryType::Directory);
                header.set_size(0);
                header.set_mode(0o755);
                set_owner_group(&mut header, args);
                let dir_name = if archive_name.ends_with('/') {
                    archive_name.to_string()
                } else {
                    format!("{archive_name}/")
                };
                header.set_cksum();
                builder.append_data(&mut header, &dir_name, io::empty())?;
            } else if path.is_symlink() {
                let target = fs::read_link(path)?;
                let mut header = Header::new_gnu();
                header.set_entry_type(EntryType::Symlink);
                header.set_size(0);
                set_owner_group(&mut header, args);
                header.set_cksum();
                builder.append_link(&mut header, archive_name, &target)?;
            } else if path.is_file() {
                let metadata = fs::metadata(path)?;
                let mut header = Header::new_gnu();
                header.set_size(metadata.len());
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    header.set_mode(metadata.mode());
                    header.set_mtime(metadata.mtime() as u64);
                    header.set_uid(metadata.uid() as u64);
                    header.set_gid(metadata.gid() as u64);
                }
                set_owner_group(&mut header, args);
                header.set_cksum();
                let file = File::open(path)?;
                builder.append_data(&mut header, archive_name, file)?;
            }
        }
    }

    builder.into_inner()?.flush()?;
    Ok(())
}

fn set_owner_group(header: &mut Header, args: &Args) {
    if let Some(owner) = &args.owner {
        if let Ok(uid) = owner.parse::<u64>() {
            header.set_uid(uid);
        }
        header.set_username(owner).ok();
    }
    if let Some(group) = &args.group {
        if let Ok(gid) = group.parse::<u64>() {
            header.set_gid(gid);
        }
        header.set_groupname(group).ok();
    }
}

// ---------------------------------------------------------------------------
// Extract / List
// ---------------------------------------------------------------------------

fn do_extract_or_list(args: &Args) -> io::Result<()> {
    let explicit_compression = args.compression;

    let (reader, detected_compression): (Box<dyn Read>, Compression) = match args.file.as_deref() {
        Some("-") | None => {
            // stdin – need to buffer for magic detection
            let mut buf = [0u8; 6];
            let mut stdin = io::stdin().lock();
            let n = stdin.read(&mut buf)?;
            let magic_comp = detect_from_magic(&buf[..n]);
            let chain: Box<dyn Read> = Box::new(io::Cursor::new(buf[..n].to_vec()).chain(stdin));
            (chain, magic_comp)
        }
        Some(path) => {
            let file = File::open(path)?;
            let mut buf = [0u8; 6];
            let mut reader = BufReader::new(file);
            let n = reader.read(&mut buf)?;
            let magic_comp = detect_from_magic(&buf[..n]);
            let ext_comp = detect_from_extension(path);
            let chain: Box<dyn Read> = Box::new(io::Cursor::new(buf[..n].to_vec()).chain(reader));
            // Prefer magic bytes, fall back to extension
            let comp = if magic_comp != Compression::None {
                magic_comp
            } else {
                ext_comp
            };
            (chain, comp)
        }
    };

    let compression = explicit_compression.unwrap_or(detected_compression);

    let decompressed: Box<dyn Read> = match compression {
        Compression::None => reader,
        Compression::Gzip => Box::new(GzDecoder::new(reader)),
        Compression::Bzip2 => Box::new(BzDecoder::new(reader)),
        Compression::Xz => Box::new(XzDecoder::new(reader)),
    };

    let mut archive = Archive::new(decompressed);
    archive.set_preserve_permissions(args.preserve_permissions && !args.no_same_permissions);
    archive.set_unpack_xattrs(false);
    archive.set_overwrite(true);

    let entries = archive.entries()?;

    for entry in entries {
        let mut entry = entry?;
        let orig_path = entry.path()?.to_path_buf();
        let path_str = orig_path.to_string_lossy().to_string();

        // Apply strip-components
        let stripped = if args.strip_components > 0 {
            let components: Vec<&str> = path_str.split('/').collect();
            if components.len() <= args.strip_components {
                continue;
            }
            components[args.strip_components..].join("/")
        } else {
            path_str.clone()
        };

        if stripped.is_empty() {
            continue;
        }

        // Apply transforms
        let final_path = if !args.transforms.is_empty() {
            apply_transforms(&stripped, &args.transforms)
        } else {
            stripped
        };

        // Check excludes
        if is_excluded(&final_path, &args.excludes) {
            continue;
        }

        // Filter by explicitly listed paths
        if !args.paths.is_empty()
            && !args.paths.iter().any(|p| {
                final_path.starts_with(p.as_str())
                    || final_path.trim_end_matches('/') == p.trim_end_matches('/')
            })
        {
            continue;
        }

        if args.list {
            println!("{final_path}");
            continue;
        }

        // Extract
        if args.verbose {
            eprintln!("{final_path}");
        }

        let dest = match &args.directory {
            Some(dir) => PathBuf::from(dir).join(&final_path),
            None => PathBuf::from(&final_path),
        };

        let entry_type = entry.header().entry_type();
        match entry_type {
            EntryType::Directory => {
                fs::create_dir_all(&dest)?;
                #[cfg(unix)]
                if args.preserve_permissions && !args.no_same_permissions {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(mode) = entry.header().mode() {
                        fs::set_permissions(&dest, fs::Permissions::from_mode(mode))?;
                    }
                }
            }
            EntryType::Regular | EntryType::GNUSparse => {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut file = File::create(&dest)?;
                io::copy(&mut entry, &mut file)?;
                #[cfg(unix)]
                if args.preserve_permissions && !args.no_same_permissions {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(mode) = entry.header().mode() {
                        fs::set_permissions(&dest, fs::Permissions::from_mode(mode))?;
                    }
                }
            }
            EntryType::Symlink => {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)?;
                }
                if let Some(link) = entry.link_name()? {
                    // Remove existing if present
                    let _ = fs::remove_file(&dest);
                    #[cfg(unix)]
                    std::os::unix::fs::symlink(link.as_ref(), &dest)?;
                }
            }
            EntryType::Link => {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)?;
                }
                if let Some(link) = entry.link_name()? {
                    let link_target = match &args.directory {
                        Some(dir) => PathBuf::from(dir).join(link.as_ref()),
                        None => link.into_owned(),
                    };
                    let _ = fs::remove_file(&dest);
                    fs::hard_link(link_target, &dest)?;
                }
            }
            _ => {
                // Skip other entry types (char devices, etc.)
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let args = parse_args();

    let op_count = args.create as u8 + args.extract as u8 + args.list as u8;
    if op_count == 0 {
        eprintln!("tar: you must specify one of -c, -x, or -t");
        process::exit(2);
    }
    if op_count > 1 {
        eprintln!("tar: only one of -c, -x, or -t may be specified");
        process::exit(2);
    }

    let result = if args.create {
        do_create(&args)
    } else {
        do_extract_or_list(&args)
    };

    if let Err(e) = result {
        eprintln!("tar: {e}");
        process::exit(1);
    }
}
