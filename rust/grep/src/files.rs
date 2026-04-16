use std::path::PathBuf;

use walkdir::WalkDir;

use crate::args::Options;

pub(crate) fn matches_glob(name: &str, pattern: &str) -> bool {
    let name_chars: Vec<char> = name.chars().collect();
    let pat_chars: Vec<char> = pattern.chars().collect();
    glob_match(&name_chars, &pat_chars, 0, 0)
}

fn glob_match(name: &[char], pat: &[char], mut ni: usize, mut pi: usize) -> bool {
    while pi < pat.len() {
        if pat[pi] == '*' {
            pi += 1;
            // Skip consecutive *
            while pi < pat.len() && pat[pi] == '*' {
                pi += 1;
            }
            if pi == pat.len() {
                return true; // trailing * matches everything
            }
            // Try matching rest of pattern at each position
            while ni <= name.len() {
                if glob_match(name, pat, ni, pi) {
                    return true;
                }
                ni += 1;
            }
            return false;
        } else if ni >= name.len() {
            return false;
        } else if pat[pi] == '?' {
            ni += 1;
            pi += 1;
        } else if pat[pi] == '[' {
            pi += 1;
            let negate = pi < pat.len() && (pat[pi] == '!' || pat[pi] == '^');
            if negate {
                pi += 1;
            }
            let mut matched = false;
            let mut first = true;
            while pi < pat.len() && (pat[pi] != ']' || first) {
                first = false;
                if pi + 2 < pat.len() && pat[pi + 1] == '-' {
                    if name[ni] >= pat[pi] && name[ni] <= pat[pi + 2] {
                        matched = true;
                    }
                    pi += 3;
                } else {
                    if name[ni] == pat[pi] {
                        matched = true;
                    }
                    pi += 1;
                }
            }
            if pi < pat.len() {
                pi += 1; // skip ]
            }
            if matched == negate {
                return false;
            }
            ni += 1;
        } else if pat[pi] == name[ni] {
            ni += 1;
            pi += 1;
        } else {
            return false;
        }
    }
    ni == name.len()
}

pub(crate) fn collect_files(opts: &Options, default_dir: bool) -> Vec<PathBuf> {
    let mut files = Vec::new();

    if opts.files.is_empty() {
        return files; // stdin mode
    }

    for path in &opts.files {
        if path.as_os_str() == "-" {
            files.push(PathBuf::from("-"));
            continue;
        }

        if opts.recursive && path.is_dir() {
            let walker = WalkDir::new(path).into_iter();
            for entry in walker.filter_entry(|e| {
                // Filter out excluded directories
                // Don't exclude the root directory when it's the implicit default "."
                let is_default_root = e.depth() == 0 && default_dir;
                if e.file_type().is_dir() && !opts.exclude_dir_glob.is_empty() && !is_default_root {
                    let name = e.file_name().to_string_lossy();
                    let full_path = e.path().to_string_lossy();
                    let clean_path = full_path.strip_prefix("./").unwrap_or(&full_path);
                    if opts.exclude_dir_glob.iter().any(|g| {
                        let clean_g = g.strip_prefix("./").unwrap_or(g);
                        matches_glob(&name, g)
                            || matches_glob(&name, clean_g)
                            || matches_glob(clean_path, g)
                            || matches_glob(clean_path, clean_g)
                    }) {
                        return false;
                    }
                }
                true
            }) {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                if entry.file_type().is_file() {
                    let name = entry.file_name().to_string_lossy();

                    // Apply include/exclude filters (match against both name and path)
                    let entry_path_str = entry.path().to_string_lossy();
                    let matches_exclude = opts.exclude_glob.iter().any(|g| {
                        matches_glob(&name, g) || matches_glob(&entry_path_str, g)
                    });

                    if matches_exclude {
                        continue;
                    }

                    if !opts.include_glob.is_empty() && opts.include_is_strict {
                        // Strict whitelist: only files matching include are considered
                        let matches_include = opts.include_glob.iter().any(|g| {
                            matches_glob(&name, g) || matches_glob(&entry_path_str, g)
                        });
                        if !matches_include {
                            continue;
                        }
                    }
                    // Non-strict: exclude-only mode, include patterns are ignored

                    let entry_path = entry.into_path();
                    // Strip leading ./ for cleaner output
                    let clean_path = entry_path
                        .strip_prefix("./")
                        .unwrap_or(&entry_path)
                        .to_path_buf();
                    files.push(clean_path);
                }
            }
        } else {
            let name = path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
            let path_str = path.to_string_lossy();

            // Apply --include filter for non-recursive files
            if !opts.include_glob.is_empty() && opts.include_is_strict {
                if !opts.include_glob.iter().any(|g| {
                    matches_glob(&name, g) || matches_glob(&path_str, g)
                }) {
                    continue;
                }
            }

            // Apply --exclude to non-recursive file arguments too
            if opts.exclude_glob.iter().any(|g| {
                matches_glob(&name, g) || matches_glob(&path_str, g)
            }) {
                continue;
            }

            // Skip directories when --directories=skip
            if opts.skip_directories && path.is_dir() {
                continue;
            }

            files.push(path.clone());
        }
    }

    files
}
