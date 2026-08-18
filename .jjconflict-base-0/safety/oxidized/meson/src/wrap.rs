/// Wrap file handling: download, extract, and patch subproject dependencies.
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

/// Download and extract a wrap dependency.
pub fn download_wrap(wrap_file: &str, dest_dir: &str) -> Result<(), String> {
    let config = parse_wrap_file(wrap_file)?;

    // Determine wrap type from section name ([wrap-file], [wrap-git], etc.)
    // or from explicit "type" key in [wrap] section
    let wrap_type = if config.contains_key("wrap-file") {
        "file"
    } else if config.contains_key("wrap-git") {
        "git"
    } else if config.contains_key("wrap-hg") {
        "hg"
    } else if config.contains_key("wrap-svn") {
        "svn"
    } else {
        config
            .get("wrap")
            .and_then(|s| s.get("type"))
            .map(|s| s.as_str())
            .unwrap_or("file")
    };

    // Normalize: get the wrap section data regardless of section naming style
    let normalized = normalize_wrap_config(&config);

    match wrap_type {
        "file" => download_file_wrap(&normalized, dest_dir),
        "git" => download_git_wrap(&normalized, dest_dir),
        "hg" => download_hg_wrap(&normalized, dest_dir),
        "svn" => download_svn_wrap(&normalized, dest_dir),
        _ => Err(format!("Unknown wrap type: {}", wrap_type)),
    }
}

/// Normalize wrap config so that the main section is always accessible as "wrap"
fn normalize_wrap_config(
    config: &HashMap<String, HashMap<String, String>>,
) -> HashMap<String, HashMap<String, String>> {
    let mut normalized = config.clone();
    // If there's a [wrap-file], [wrap-git], etc. section, merge it into "wrap"
    for key in &["wrap-file", "wrap-git", "wrap-hg", "wrap-svn"] {
        if let Some(section) = config.get(*key) {
            let wrap = normalized.entry("wrap".to_string()).or_default();
            for (k, v) in section {
                wrap.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
    }
    normalized
}

fn parse_wrap_file(path: &str) -> Result<HashMap<String, HashMap<String, String>>, String> {
    let content =
        std::fs::read_to_string(path).map_err(|e| format!("Cannot read {}: {}", path, e))?;

    let mut sections: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut current_section = "wrap".to_string();
    sections.insert(current_section.clone(), HashMap::new());

    let mut last_key: Option<String> = None;
    for raw_line in content.lines() {
        // Preserve original to detect indentation
        let is_indented = raw_line.starts_with(' ') || raw_line.starts_with('\t');
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            // blank/comment lines also break a continuation only if blank
            if line.is_empty() {
                last_key = None;
            }
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            current_section = line[1..line.len() - 1].to_string();
            sections.entry(current_section.clone()).or_default();
            last_key = None;
        } else if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim().to_string();
            let value = line[eq_pos + 1..].trim().to_string();
            sections
                .entry(current_section.clone())
                .or_default()
                .insert(key.clone(), value);
            last_key = Some(key);
        } else if is_indented {
            if let Some(ref k) = last_key {
                let sec = sections.entry(current_section.clone()).or_default();
                if let Some(existing) = sec.get_mut(k) {
                    if !existing.is_empty() && !existing.ends_with(',') {
                        existing.push(',');
                    }
                    existing.push_str(line);
                }
            }
        }
    }

    Ok(sections)
}

fn download_file_wrap(
    config: &HashMap<String, HashMap<String, String>>,
    dest_dir: &str,
) -> Result<(), String> {
    let wrap = config.get("wrap").ok_or("No [wrap] section")?;

    let url = wrap.get("source_url");
    let filename = wrap
        .get("source_filename")
        .ok_or("No source_filename in wrap file")?;
    let directory = wrap.get("directory").map(|s| s.as_str()).unwrap_or("");

    let parent = Path::new(dest_dir).parent().unwrap_or(Path::new("."));
    let archive_path = parent.join(filename);

    // Check if the file already exists locally (e.g., in packagefiles/ or packagecache/)
    if !archive_path.exists() {
        // Check in packagefiles/ directory next to the wrap file
        let packagefiles_path = parent.join("packagefiles").join(filename);
        let packagecache_path = parent.join("packagecache").join(filename);
        if packagefiles_path.exists() {
            std::fs::copy(&packagefiles_path, &archive_path)
                .map_err(|e| format!("Failed to copy local archive: {}", e))?;
        } else if packagecache_path.exists() {
            std::fs::copy(&packagecache_path, &archive_path)
                .map_err(|e| format!("Failed to copy local archive: {}", e))?;
        } else if let Some(url) = url {
            // Download from URL
            eprintln!("Downloading {}...", filename);
            let status = Command::new("curl")
                .args(["-L", "-o"])
                .arg(&archive_path)
                .arg(url)
                .status()
                .map_err(|e| format!("Failed to download: {}", e))?;

            if !status.success() {
                return Err("Download failed".to_string());
            }
        } else {
            return Err(format!(
                "No source_url in wrap file and local file not found: {}",
                archive_path.display()
            ));
        }
    }

    // Extract
    let _extract_dir = if directory.is_empty() {
        dest_dir
    } else {
        directory
    };
    let lead_missing = wrap
        .get("lead_directory_missing")
        .map(|s| s == "true")
        .unwrap_or(false);
    // If lead_directory_missing=true, extract into the wrap-declared directory itself
    // rather than the parent (since the archive has no top-level dir).
    let extract_target: std::path::PathBuf = if lead_missing && !directory.is_empty() {
        let p = parent.join(directory);
        std::fs::create_dir_all(&p).ok();
        p
    } else {
        // Create dest_dir as well (for archives that include their own top dir
        // matching directory= or otherwise).
        let _ = std::fs::create_dir_all(dest_dir);
        parent.to_path_buf()
    };

    eprintln!("Extracting {}...", filename);
    let fname = filename.to_lowercase();
    if fname.ends_with(".tar.gz") || fname.ends_with(".tgz") {
        Command::new("tar")
            .args(["xzf"])
            .arg(&archive_path)
            .arg("-C")
            .arg(&extract_target)
            .status()
            .map_err(|e| format!("Extract failed: {}", e))?;
    } else if fname.ends_with(".tar.xz") || fname.ends_with(".txz") {
        Command::new("tar")
            .args(["xJf"])
            .arg(&archive_path)
            .arg("-C")
            .arg(&extract_target)
            .status()
            .map_err(|e| format!("Extract failed: {}", e))?;
    } else if fname.ends_with(".tar.bz2") {
        Command::new("tar")
            .args(["xjf"])
            .arg(&archive_path)
            .arg("-C")
            .arg(&extract_target)
            .status()
            .map_err(|e| format!("Extract failed: {}", e))?;
    } else if fname.ends_with(".zip") {
        Command::new("unzip")
            .arg("-o")
            .arg(&archive_path)
            .arg("-d")
            .arg(&extract_target)
            .status()
            .map_err(|e| format!("Extract failed: {}", e))?;
    }

    // Apply patch_directory: copy contents of packagefiles/<patch_directory>/ into the extracted dir
    if let Some(patch_dir) = wrap.get("patch_directory") {
        let src = parent.join("packagefiles").join(patch_dir);
        if src.is_dir() {
            fn copy_dir_recursive(
                src: &std::path::Path,
                dst: &std::path::Path,
            ) -> std::io::Result<()> {
                std::fs::create_dir_all(dst)?;
                for entry in std::fs::read_dir(src)? {
                    let e = entry?;
                    let to = dst.join(e.file_name());
                    if e.file_type()?.is_dir() {
                        copy_dir_recursive(&e.path(), &to)?;
                    } else {
                        std::fs::copy(e.path(), &to)?;
                    }
                }
                Ok(())
            }
            let _ = copy_dir_recursive(&src, &extract_target);
        }
    }

    // Apply patch if present
    let patch_filename_explicit = wrap.get("patch_filename").map(|s| s.as_str());
    if wrap.get("patch_url").is_some() || patch_filename_explicit.is_some() {
        let patch_filename = patch_filename_explicit.unwrap_or("patch.tar.gz");
        let mut patch_path = parent.join(patch_filename);

        // Look for cached patch first
        let pf_pc = parent.join("packagecache").join(patch_filename);
        let pf_pf = parent.join("packagefiles").join(patch_filename);
        let mut cleanup = false;
        if !patch_path.exists() {
            if pf_pf.exists() {
                std::fs::copy(&pf_pf, &patch_path).ok();
                cleanup = true;
            } else if pf_pc.exists() {
                std::fs::copy(&pf_pc, &patch_path).ok();
                cleanup = true;
            } else if let Some(patch_url) = wrap.get("patch_url") {
                Command::new("curl")
                    .args(["-L", "-o"])
                    .arg(&patch_path)
                    .arg(patch_url)
                    .status()
                    .map_err(|e| format!("Failed to download patch: {}", e))?;
                cleanup = true;
            } else {
                patch_path = std::path::PathBuf::new();
            }
        }

        if patch_path.as_os_str().len() > 0 && patch_path.exists() {
            let pname = patch_path.to_string_lossy().to_lowercase();
            let extract_dir_arg =
                if wrap.get("patch_directory").is_some() || patch_filename_explicit.is_some() {
                    // patches typically extract one level above
                    parent.to_path_buf()
                } else {
                    std::path::PathBuf::from(dest_dir)
                };
            if pname.ends_with(".tar.xz") || pname.ends_with(".txz") {
                Command::new("tar")
                    .arg("xJf")
                    .arg(&patch_path)
                    .arg("-C")
                    .arg(&extract_dir_arg)
                    .status()
                    .ok();
            } else if pname.ends_with(".tar.bz2") {
                Command::new("tar")
                    .arg("xjf")
                    .arg(&patch_path)
                    .arg("-C")
                    .arg(&extract_dir_arg)
                    .status()
                    .ok();
            } else if pname.ends_with(".zip") {
                Command::new("unzip")
                    .arg("-o")
                    .arg(&patch_path)
                    .arg("-d")
                    .arg(&extract_dir_arg)
                    .status()
                    .ok();
            } else {
                Command::new("tar")
                    .arg("xzf")
                    .arg(&patch_path)
                    .arg("-C")
                    .arg(&extract_dir_arg)
                    .status()
                    .ok();
            }
            if cleanup {
                let _ = std::fs::remove_file(&patch_path);
            }
        }
    }

    // Apply diff files. Each entry is resolved relative to (in order):
    //   subprojects/packagefiles/<entry>
    //   subprojects/<entry>
    if let Some(diff_files) = wrap.get("diff_files") {
        for diff in diff_files.split([',', '\n']) {
            let diff = diff.trim();
            if diff.is_empty() {
                continue;
            }
            let candidate1 = parent.join("packagefiles").join(diff);
            let candidate2 = parent.join(diff);
            let diff_path = if candidate1.is_file() {
                candidate1
            } else if candidate2.is_file() {
                candidate2
            } else {
                continue;
            };
            Command::new("patch")
                .args(["-p1", "-i"])
                .arg(&diff_path)
                .current_dir(&extract_target)
                .status()
                .map_err(|e| format!("Patch failed: {}", e))?;
        }
    }

    let _ = std::fs::remove_file(&archive_path);
    Ok(())
}

fn download_git_wrap(
    config: &HashMap<String, HashMap<String, String>>,
    dest_dir: &str,
) -> Result<(), String> {
    let wrap = config.get("wrap").ok_or("No [wrap] section")?;

    let url = wrap.get("url").ok_or("No url in git wrap")?;
    let revision = wrap.get("revision").unwrap_or(&"HEAD".to_string()).clone();
    let depth = wrap.get("depth");

    eprintln!("Cloning {}...", url);
    let mut cmd = Command::new("git");
    cmd.arg("clone").arg(url).arg(dest_dir);
    if let Some(depth) = depth {
        cmd.arg("--depth").arg(depth);
    }

    let status = cmd
        .status()
        .map_err(|e| format!("Git clone failed: {}", e))?;

    if !status.success() {
        return Err("Git clone failed".to_string());
    }

    // Checkout specific revision
    if revision != "HEAD" {
        Command::new("git")
            .args(["checkout", &revision])
            .current_dir(dest_dir)
            .status()
            .map_err(|e| format!("Git checkout failed: {}", e))?;
    }

    Ok(())
}

fn download_hg_wrap(
    config: &HashMap<String, HashMap<String, String>>,
    dest_dir: &str,
) -> Result<(), String> {
    let wrap = config.get("wrap").ok_or("No [wrap] section")?;
    let url = wrap.get("url").ok_or("No url in hg wrap")?;
    let revision = wrap.get("revision");

    let mut cmd = Command::new("hg");
    cmd.arg("clone").arg(url).arg(dest_dir);
    if let Some(rev) = revision {
        cmd.arg("-r").arg(rev);
    }

    cmd.status()
        .map_err(|e| format!("Hg clone failed: {}", e))?;
    Ok(())
}

fn download_svn_wrap(
    config: &HashMap<String, HashMap<String, String>>,
    dest_dir: &str,
) -> Result<(), String> {
    let wrap = config.get("wrap").ok_or("No [wrap] section")?;
    let url = wrap.get("url").ok_or("No url in svn wrap")?;
    let revision = wrap.get("revision");

    let mut cmd = Command::new("svn");
    cmd.arg("checkout").arg(url).arg(dest_dir);
    if let Some(rev) = revision {
        cmd.arg("-r").arg(rev);
    }

    cmd.status()
        .map_err(|e| format!("SVN checkout failed: {}", e))?;
    Ok(())
}
