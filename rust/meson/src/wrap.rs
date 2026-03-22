/// Wrap file handling: download, extract, and patch subproject dependencies.
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

/// Download and extract a wrap dependency.
pub fn download_wrap(wrap_file: &str, dest_dir: &str) -> Result<(), String> {
    let config = parse_wrap_file(wrap_file)?;

    let wrap_type = config
        .get("wrap")
        .and_then(|s| s.get("type"))
        .map(|s| s.as_str())
        .unwrap_or("file");

    match wrap_type {
        "file" => download_file_wrap(&config, dest_dir),
        "git" => download_git_wrap(&config, dest_dir),
        "hg" => download_hg_wrap(&config, dest_dir),
        "svn" => download_svn_wrap(&config, dest_dir),
        _ => Err(format!("Unknown wrap type: {}", wrap_type)),
    }
}

fn parse_wrap_file(path: &str) -> Result<HashMap<String, HashMap<String, String>>, String> {
    let content =
        std::fs::read_to_string(path).map_err(|e| format!("Cannot read {}: {}", path, e))?;

    let mut sections: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut current_section = "wrap".to_string();
    sections.insert(current_section.clone(), HashMap::new());

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            current_section = line[1..line.len() - 1].to_string();
            sections.entry(current_section.clone()).or_default();
        } else if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim().to_string();
            let value = line[eq_pos + 1..].trim().to_string();
            sections
                .entry(current_section.clone())
                .or_default()
                .insert(key, value);
        }
    }

    Ok(sections)
}

fn download_file_wrap(
    config: &HashMap<String, HashMap<String, String>>,
    dest_dir: &str,
) -> Result<(), String> {
    let wrap = config.get("wrap").ok_or("No [wrap] section")?;

    let url = wrap.get("source_url").ok_or("No source_url in wrap file")?;
    let filename = wrap
        .get("source_filename")
        .ok_or("No source_filename in wrap file")?;
    let directory = wrap.get("directory").map(|s| s.as_str()).unwrap_or("");

    let parent = Path::new(dest_dir).parent().unwrap_or(Path::new("."));
    let archive_path = parent.join(filename);

    // Download
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

    // Extract
    let extract_dir = if directory.is_empty() {
        dest_dir
    } else {
        directory
    };
    std::fs::create_dir_all(dest_dir).map_err(|e| format!("Cannot create dir: {}", e))?;

    eprintln!("Extracting {}...", filename);
    let fname = filename.to_lowercase();
    if fname.ends_with(".tar.gz") || fname.ends_with(".tgz") {
        Command::new("tar")
            .args(["xzf"])
            .arg(&archive_path)
            .arg("-C")
            .arg(parent)
            .status()
            .map_err(|e| format!("Extract failed: {}", e))?;
    } else if fname.ends_with(".tar.xz") || fname.ends_with(".txz") {
        Command::new("tar")
            .args(["xJf"])
            .arg(&archive_path)
            .arg("-C")
            .arg(parent)
            .status()
            .map_err(|e| format!("Extract failed: {}", e))?;
    } else if fname.ends_with(".tar.bz2") {
        Command::new("tar")
            .args(["xjf"])
            .arg(&archive_path)
            .arg("-C")
            .arg(parent)
            .status()
            .map_err(|e| format!("Extract failed: {}", e))?;
    } else if fname.ends_with(".zip") {
        Command::new("unzip")
            .arg(&archive_path)
            .arg("-d")
            .arg(parent)
            .status()
            .map_err(|e| format!("Extract failed: {}", e))?;
    }

    // Apply patch if present
    if let Some(patch_url) = wrap.get("patch_url") {
        let patch_filename = wrap
            .get("patch_filename")
            .map(|s| s.as_str())
            .unwrap_or("patch.tar.gz");
        let patch_path = parent.join(patch_filename);

        Command::new("curl")
            .args(["-L", "-o"])
            .arg(&patch_path)
            .arg(patch_url)
            .status()
            .map_err(|e| format!("Failed to download patch: {}", e))?;

        Command::new("tar")
            .args(["xzf"])
            .arg(&patch_path)
            .arg("-C")
            .arg(dest_dir)
            .status()
            .map_err(|e| format!("Patch extract failed: {}", e))?;

        let _ = std::fs::remove_file(&patch_path);
    }

    // Apply diff files
    if let Some(diff_files) = wrap.get("diff_files") {
        let wrap_dir = Path::new(dest_dir).parent().unwrap_or(Path::new("."));
        for diff in diff_files.split(',') {
            let diff = diff.trim();
            let diff_path = wrap_dir.join(diff);
            if diff_path.exists() {
                Command::new("patch")
                    .args(["-p1", "-i"])
                    .arg(&diff_path)
                    .current_dir(dest_dir)
                    .status()
                    .map_err(|e| format!("Patch failed: {}", e))?;
            }
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
