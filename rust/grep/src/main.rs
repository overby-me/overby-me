mod args;
mod files;
mod grep;
mod matcher;
mod pattern;

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process;

use args::parse_args;
use files::collect_files;
use grep::grep_file;
use matcher::build_matcher;

/// Exit with proper write error handling — flush stdout and detect errors.
fn safe_exit(code: i32) -> ! {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    if out.flush().is_err() {
        drop(out);
        eprintln!("grep: write error: {}", io::Error::last_os_error());
        process::exit(2);
    }
    process::exit(code);
}

fn main() {
    let mut opts = parse_args();

    // With -r and no files, default to current directory
    let default_dir = opts.recursive && opts.files.is_empty();
    if default_dir {
        opts.files.push(PathBuf::from("."));
    }

    let matcher = build_matcher(&opts);

    let had_file_args = !opts.files.is_empty();
    let files = collect_files(&opts, default_dir);

    let mut any_match = false;
    let mut had_error = false;

    if files.is_empty() && !had_file_args {
        // Read from stdin (only if no file args were given)
        let (count, matched, errored) = grep_file(Path::new("-"), &matcher, &opts);
        if opts.files_without_match {
            if !matched {
                any_match = true;
            }
        } else if matched {
            any_match = true;
        }
        if errored {
            had_error = true;
        }
        if opts.count {
            let stdout = io::stdout();
            let mut out = stdout.lock();
            let _ = writeln!(out, "{count}");
        }
    } else {
        for path in &files {
            let (count, matched, errored) = grep_file(path, &matcher, &opts);
            if opts.files_without_match {
                // For -L, "success" means finding a file WITHOUT matches
                if !matched {
                    any_match = true;
                }
            } else if matched {
                any_match = true;
            }
            if errored {
                had_error = true;
            }

            let filename = if path.as_os_str() == "-" {
                opts.label.clone()
            } else {
                path.to_string_lossy().to_string()
            };

            let stdout = io::stdout();
            let mut out = stdout.lock();

            if opts.count {
                if opts.with_filename && !opts.no_filename {
                    let _ = writeln!(out, "{filename}:{count}");
                } else {
                    let _ = writeln!(out, "{count}");
                }
            } else if (opts.files_with_matches && matched) || (opts.files_without_match && !matched)
            {
                if opts.null_separator {
                    let _ = write!(out, "{filename}\0");
                } else {
                    let _ = writeln!(out, "{filename}");
                }
            }
        }
    }

    if any_match {
        safe_exit(0);
    } else if had_error {
        safe_exit(2);
    } else {
        safe_exit(1);
    }
}
