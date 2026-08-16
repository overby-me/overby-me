//! `systemd-vpick` - pick an entry from a versioned (`.v/`) directory.
//!
//! A faithful port of upstream `src/vpick/vpick-tool.c`. The selection logic
//! lives in the shared [`vpick_core`] crate (also used by
//! `systemd-dissect --discover` and PID 1's `RootDirectory=` resolution); this
//! binary is the CLI wrapper around it.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use vpick_core::{
    Arch, PickFilter, PickResult, arch_from_string, arch_to_string, dt_bit, filename_part_is_valid,
    inode_type_from_string, inode_type_to_string, mode_to_dt, native_arch, path_pick, secondary_arch,
    version_is_valid, PICK_ARCHITECTURE, PICK_RESOLVE, PICK_TRIES,
};

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

        // Short options.
        let chars: Vec<char> = a[1..].chars().collect();
        let c = chars[0];
        let inline: Option<String> = if chars.len() > 1 {
            Some(chars[1..].iter().collect())
        } else {
            None
        };
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
        let ap = Path::new(&arg);
        let p = if ap.is_absolute() { ap.to_path_buf() } else { cwd.join(ap) };

        let filter = PickFilter {
            type_mask: opts.filter_type_mask,
            basename: opts.filter_basename.clone(),
            version: opts.filter_version.clone(),
            architecture: opts.filter_architecture,
            suffix: opts.filter_suffix.clone(),
        };

        let result = match path_pick(&p, &filter, flags) {
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

fn print_result(result: &PickResult, print: Print) -> bool {
    match print {
        Print::Path => {
            let mut s = result.path.to_string_lossy().into_owned();
            if result.is_dir() && !s.ends_with('/') {
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
