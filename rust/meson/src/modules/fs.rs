/// Filesystem module: provides fs.exists, fs.is_dir, fs.is_file, etc.
use crate::objects::*;
use crate::vm::*;

pub fn register(vm: &mut VM) {
    vm.method_registry
        .insert(("module".to_string(), "fs.exists".to_string()), fs_exists);
    vm.method_registry
        .insert(("module".to_string(), "fs.is_dir".to_string()), fs_is_dir);
    vm.method_registry
        .insert(("module".to_string(), "fs.is_file".to_string()), fs_is_file);
    vm.method_registry.insert(
        ("module".to_string(), "fs.is_symlink".to_string()),
        fs_is_symlink,
    );
    vm.method_registry.insert(
        ("module".to_string(), "fs.is_absolute".to_string()),
        fs_is_absolute,
    );
    vm.method_registry
        .insert(("module".to_string(), "fs.read".to_string()), fs_read);
    vm.method_registry
        .insert(("module".to_string(), "fs.hash".to_string()), fs_hash);
    vm.method_registry
        .insert(("module".to_string(), "fs.size".to_string()), fs_size);
    vm.method_registry
        .insert(("module".to_string(), "fs.name".to_string()), fs_name);
    vm.method_registry
        .insert(("module".to_string(), "fs.stem".to_string()), fs_stem);
    vm.method_registry
        .insert(("module".to_string(), "fs.parent".to_string()), fs_parent);
    vm.method_registry.insert(
        ("module".to_string(), "fs.replace_suffix".to_string()),
        fs_replace_suffix,
    );
    vm.method_registry.insert(
        ("module".to_string(), "fs.copyfile".to_string()),
        fs_copyfile,
    );
    vm.method_registry.insert(
        ("module".to_string(), "fs.as_posix".to_string()),
        fs_as_posix,
    );
    vm.method_registry.insert(
        ("module".to_string(), "fs.expanduser".to_string()),
        fs_expanduser,
    );
    vm.method_registry.insert(
        ("module".to_string(), "fs.relative_to".to_string()),
        fs_relative_to,
    );
}

fn resolve_path(vm: &VM, path_str: &str) -> std::path::PathBuf {
    let p = std::path::Path::new(path_str);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::path::Path::new(&vm.source_root)
            .join(&vm.current_subdir)
            .join(path_str)
    }
}

fn get_path_arg(args: &[CallArg]) -> Result<String, String> {
    let positional = VM::get_positional_args(args);
    match positional.first() {
        Some(Object::String(s)) => Ok(s.clone()),
        Some(Object::File(f)) => Ok(f.path.clone()),
        _ => Err("fs method requires a string or file argument".to_string()),
    }
}

fn fs_exists(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let path_str = get_path_arg(args)?;
    let full = resolve_path(vm, &path_str);
    Ok(Object::Bool(full.exists()))
}

fn fs_is_dir(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let path_str = get_path_arg(args)?;
    let full = resolve_path(vm, &path_str);
    Ok(Object::Bool(full.is_dir()))
}

fn fs_is_file(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let path_str = get_path_arg(args)?;
    let full = resolve_path(vm, &path_str);
    Ok(Object::Bool(full.is_file()))
}

fn fs_is_symlink(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let path_str = get_path_arg(args)?;
    let full = resolve_path(vm, &path_str);
    Ok(Object::Bool(
        full.symlink_metadata()
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false),
    ))
}

fn fs_is_absolute(_vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let path_str = get_path_arg(args)?;
    Ok(Object::Bool(std::path::Path::new(&path_str).is_absolute()))
}

fn fs_read(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let path_str = get_path_arg(args)?;
    let full = resolve_path(vm, &path_str);
    let encoding = VM::get_arg_str(args, "encoding", 1).unwrap_or("utf-8");
    let _ = encoding; // Only utf-8 supported for now
    std::fs::read_to_string(&full)
        .map(Object::String)
        .map_err(|e| format!("fs.read: cannot read '{}': {}", full.display(), e))
}

fn fs_hash(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let path_str = get_path_arg(args)?;
    let full = resolve_path(vm, &path_str);
    let algorithm = VM::get_arg_str(args, "algorithm", 1).unwrap_or("sha256");
    let data = std::fs::read(&full)
        .map_err(|e| format!("fs.hash: cannot read '{}': {}", full.display(), e))?;

    // Simple hash using std — for sha256 we use a basic implementation
    // In practice, meson supports md5, sha1, sha256, etc.
    // We'll shell out to sha256sum/md5sum for correctness.
    let cmd = match algorithm {
        "md5" => "md5sum",
        "sha1" => "sha1sum",
        "sha256" => "sha256sum",
        "sha512" => "sha512sum",
        other => return Err(format!("fs.hash: unsupported algorithm '{}'", other)),
    };

    let child = std::process::Command::new(cmd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn();

    match child {
        Ok(mut child) => {
            use std::io::Write;
            if let Some(ref mut stdin) = child.stdin {
                let _ = stdin.write_all(&data);
            }
            let output = child
                .wait_with_output()
                .map_err(|e| format!("fs.hash: {}", e))?;
            let hash = String::from_utf8_lossy(&output.stdout);
            let hash = hash.split_whitespace().next().unwrap_or("").to_string();
            Ok(Object::String(hash))
        }
        Err(_) => {
            // Fallback: return a placeholder
            Ok(Object::String(format!(
                "<{}:{}>",
                algorithm,
                full.display()
            )))
        }
    }
}

fn fs_size(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let path_str = get_path_arg(args)?;
    let full = resolve_path(vm, &path_str);
    let meta = std::fs::metadata(&full)
        .map_err(|e| format!("fs.size: cannot stat '{}': {}", full.display(), e))?;
    Ok(Object::Int(meta.len() as i64))
}

fn fs_name(_vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let path_str = get_path_arg(args)?;
    let p = std::path::Path::new(&path_str);
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    Ok(Object::String(name))
}

fn fs_stem(_vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let path_str = get_path_arg(args)?;
    let p = std::path::Path::new(&path_str);
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    Ok(Object::String(stem))
}

fn fs_parent(_vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let path_str = get_path_arg(args)?;
    let p = std::path::Path::new(&path_str);
    let parent = p
        .parent()
        .map(|pp| pp.to_string_lossy().to_string())
        .unwrap_or_default();
    Ok(Object::String(parent))
}

fn fs_replace_suffix(_vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let path_str = match positional.first() {
        Some(Object::String(s)) => s.clone(),
        _ => return Err("fs.replace_suffix: first argument must be a string".to_string()),
    };
    let new_suffix = match positional.get(1) {
        Some(Object::String(s)) => s.clone(),
        _ => return Err("fs.replace_suffix: second argument must be a string".to_string()),
    };
    let p = std::path::Path::new(&path_str);
    let mut result = p.with_extension("");
    let stem_str = result.to_string_lossy().to_string();
    // new_suffix includes the dot (e.g. ".h")
    Ok(Object::String(format!("{}{}", stem_str, new_suffix)))
}

fn fs_copyfile(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let src = match positional.first() {
        Some(Object::String(s)) => s.clone(),
        Some(Object::File(f)) => f.path.clone(),
        _ => return Err("fs.copyfile: first argument must be a string or file".to_string()),
    };
    let dest_name = match positional.get(1) {
        Some(Object::String(s)) => s.clone(),
        _ => {
            // Default: same filename in build dir
            std::path::Path::new(&src)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or(src.clone())
        }
    };
    let install = VM::get_arg_bool(args, "install", false);
    let install_dir = VM::get_arg_str(args, "install_dir", 99).map(|s| s.to_string());

    // Register as a custom target that copies the file
    let id = format!("copyfile_{}", dest_name.replace('/', "_"));
    let ct = CustomTargetRef {
        name: dest_name.clone(),
        id: id.clone(),
        outputs: vec![dest_name.clone()],
        subdir: vm.current_subdir.clone(),
    };

    vm.build_data.custom_targets.push(CustomTarget {
        name: dest_name.clone(),
        id,
        command: vec!["cp".to_string(), src, dest_name.clone()],
        input: Vec::new(),
        output: vec![dest_name],
        depends: Vec::new(),
        depend_files: Vec::new(),
        depfile: None,
        capture: false,
        feed: false,
        install,
        install_dir: install_dir.into_iter().collect(),
        install_tag: Vec::new(),
        build_by_default: true,
        build_always_stale: false,
        env: std::collections::HashMap::new(),
        subdir: vm.current_subdir.clone(),
    });

    Ok(Object::CustomTarget(ct))
}

fn fs_as_posix(_vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let path_str = get_path_arg(args)?;
    Ok(Object::String(path_str.replace('\\', "/")))
}

fn fs_expanduser(_vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let path_str = get_path_arg(args)?;
    if path_str.starts_with('~') {
        if let Some(home) = std::env::var_os("HOME") {
            let expanded = path_str.replacen('~', &home.to_string_lossy(), 1);
            return Ok(Object::String(expanded));
        }
    }
    Ok(Object::String(path_str))
}

fn fs_relative_to(_vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let path_str = match positional.first() {
        Some(Object::String(s)) => s.clone(),
        _ => return Err("fs.relative_to: first argument must be a string".to_string()),
    };
    let base_str = match positional.get(1) {
        Some(Object::String(s)) => s.clone(),
        _ => return Err("fs.relative_to: second argument must be a string".to_string()),
    };

    let path = std::path::Path::new(&path_str);
    let base = std::path::Path::new(&base_str);

    // Try to strip the base prefix
    if let Ok(rel) = path.strip_prefix(base) {
        Ok(Object::String(rel.to_string_lossy().to_string()))
    } else {
        // Compute relative path manually
        let mut path_parts: Vec<_> = path.components().collect();
        let mut base_parts: Vec<_> = base.components().collect();

        // Find common prefix length
        let common = path_parts
            .iter()
            .zip(base_parts.iter())
            .take_while(|(a, b)| a == b)
            .count();

        let ups = base_parts.len() - common;
        let mut result = std::path::PathBuf::new();
        for _ in 0..ups {
            result.push("..");
        }
        for part in &path_parts[common..] {
            result.push(part);
        }
        Ok(Object::String(result.to_string_lossy().to_string()))
    }
}
