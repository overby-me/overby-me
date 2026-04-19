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
    vm.method_registry
        .insert(("module".to_string(), "fs.suffix".to_string()), fs_suffix);
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
    vm.method_registry.insert(
        ("module".to_string(), "fs.is_samepath".to_string()),
        fs_is_samepath,
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

/// Resolve a path from a CallArg, handling File objects with their own subdir context.
fn resolve_path_from_arg(
    vm: &VM,
    args: &[CallArg],
) -> Result<(std::path::PathBuf, String), String> {
    let positional = VM::get_positional_args(args);
    match positional.first() {
        Some(Object::File(f)) => {
            let base = if f.is_built {
                &vm.build_root
            } else {
                &vm.source_root
            };
            let full = if f.subdir.is_empty() {
                std::path::Path::new(base).join(&f.path)
            } else {
                std::path::Path::new(base).join(&f.subdir).join(&f.path)
            };
            Ok((full, f.path.clone()))
        }
        Some(Object::Array(arr)) => {
            if let Some(Object::File(f)) = arr.first() {
                let base = if f.is_built {
                    &vm.build_root
                } else {
                    &vm.source_root
                };
                let full = if f.subdir.is_empty() {
                    std::path::Path::new(base).join(&f.path)
                } else {
                    std::path::Path::new(base).join(&f.subdir).join(&f.path)
                };
                Ok((full, f.path.clone()))
            } else {
                let path_str = get_path_arg(args)?;
                Ok((resolve_path(vm, &path_str), path_str))
            }
        }
        _ => {
            let path_str = get_path_arg(args)?;
            Ok((resolve_path(vm, &path_str), path_str))
        }
    }
}

fn expand_path(path: &str, source_root: &str, current_subdir: &str) -> String {
    if path.starts_with('~') {
        if let Ok(home) = std::env::var("HOME") {
            return path.replacen('~', &home, 1);
        }
    }
    if path.starts_with('/') {
        return path.to_string();
    }
    if current_subdir.is_empty() {
        format!("{}/{}", source_root, path)
    } else {
        format!("{}/{}/{}", source_root, current_subdir, path)
    }
}

/// Extract a path string from various Meson object types.
/// Handles String, File, BuildTarget, CustomTarget, and CustomTargetIndex.
fn object_to_path_str(obj: &Object) -> Result<String, String> {
    match obj {
        Object::String(s) => Ok(s.clone()),
        Object::File(f) => Ok(f.path.clone()),
        Object::Array(arr) => {
            // files() returns an array; unwrap to the first File element
            if let Some(first) = arr.first() {
                object_to_path_str(first)
            } else {
                Err("fs method: empty array argument".to_string())
            }
        }
        Object::BuildTarget(t) => {
            if let Some(output) = t.outputs.first() {
                Ok(output.clone())
            } else {
                Ok(t.name.clone())
            }
        }
        Object::CustomTarget(ct) => {
            if let Some(output) = ct.outputs.first() {
                Ok(output.clone())
            } else {
                Ok(ct.name.clone())
            }
        }
        Object::CustomTargetIndex(ct, idx) => {
            if let Some(output) = ct.outputs.get(*idx) {
                Ok(output.clone())
            } else {
                Err(format!("Custom target index {} out of bounds", idx))
            }
        }
        _ => Err(format!(
            "fs method requires a string, file, or target argument, got {}",
            obj.type_name()
        )),
    }
}

fn get_path_arg(args: &[CallArg]) -> Result<String, String> {
    let positional = VM::get_positional_args(args);
    match positional.first() {
        Some(obj) => object_to_path_str(obj),
        None => Err("fs method requires at least one argument".to_string()),
    }
}

fn fs_exists(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let path_str = get_path_arg(args)?;
    let expanded = expand_path(&path_str, &vm.source_root, &vm.current_subdir);
    let full = std::path::Path::new(&expanded);
    Ok(Object::Bool(full.exists()))
}

fn fs_is_dir(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let path_str = get_path_arg(args)?;
    let expanded = expand_path(&path_str, &vm.source_root, &vm.current_subdir);
    let full = std::path::Path::new(&expanded);
    Ok(Object::Bool(full.is_dir()))
}

fn fs_is_file(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let path_str = get_path_arg(args)?;
    let expanded = expand_path(&path_str, &vm.source_root, &vm.current_subdir);
    let full = std::path::Path::new(&expanded);
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
    let (full, path_str) = resolve_path_from_arg(vm, args)?;
    let encoding = VM::get_arg_str(args, "encoding", 1).unwrap_or("utf-8");

    let text = if encoding == "utf-16" {
        // Read raw bytes and decode as UTF-16
        let data =
            std::fs::read(&full).map_err(|_| format!("File {} does not exist.", path_str))?;
        // Detect BOM and decode accordingly
        if data.len() >= 2 && data[0] == 0xFF && data[1] == 0xFE {
            // UTF-16 LE with BOM
            let u16_data: Vec<u16> = data[2..]
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            String::from_utf16(&u16_data)
                .map_err(|e| format!("fs.read: UTF-16 decode error: {}", e))?
        } else if data.len() >= 2 && data[0] == 0xFE && data[1] == 0xFF {
            // UTF-16 BE with BOM
            let u16_data: Vec<u16> = data[2..]
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            String::from_utf16(&u16_data)
                .map_err(|e| format!("fs.read: UTF-16 decode error: {}", e))?
        } else {
            // No BOM, assume LE
            let u16_data: Vec<u16> = data
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            String::from_utf16(&u16_data)
                .map_err(|e| format!("fs.read: UTF-16 decode error: {}", e))?
        }
    } else {
        std::fs::read_to_string(&full).map_err(|_| format!("File {} does not exist.", path_str))?
    };

    // Strip a single trailing newline (matching Meson behavior)
    let text = text
        .strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(&text);
    Ok(Object::String(text.to_string()))
}

fn fs_hash(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let path_str = get_path_arg(args)?;
    let full = resolve_path(vm, &path_str);
    let positional = VM::get_positional_args(args);
    let algorithm = match positional.get(1) {
        Some(Object::String(s)) => s.as_str(),
        _ => "sha256",
    };
    let data = std::fs::read(&full)
        .map_err(|e| format!("fs.hash: cannot read '{}': {}", full.display(), e))?;

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
        Err(_) => Ok(Object::String(format!(
            "<{}:{}>",
            algorithm,
            full.display()
        ))),
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
        .map(|pp| {
            let s = pp.to_string_lossy().to_string();
            if s.is_empty() { ".".to_string() } else { s }
        })
        .unwrap_or_else(|| ".".to_string());
    Ok(Object::String(parent))
}

fn fs_suffix(_vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let path_str = get_path_arg(args)?;
    let p = std::path::Path::new(&path_str);
    // Meson returns the suffix INCLUDING the dot, e.g. ".txt"
    // For no extension, return ""
    // For "foo.", return "."
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if let Some(dot_pos) = name.rfind('.') {
        Ok(Object::String(name[dot_pos..].to_string()))
    } else {
        Ok(Object::String(String::new()))
    }
}

fn fs_replace_suffix(_vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let path_str = match positional.first() {
        Some(obj) => object_to_path_str(obj)?,
        None => return Err("fs.replace_suffix: requires at least one argument".to_string()),
    };
    let new_suffix = match positional.get(1) {
        Some(Object::String(s)) => s.clone(),
        _ => return Err("fs.replace_suffix: second argument must be a string".to_string()),
    };
    let p = std::path::Path::new(&path_str);
    let result = p.with_extension("");
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

fn fs_relative_to(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let path_str = match positional.first() {
        Some(obj) => resolve_to_full_path(vm, obj)?,
        _ => return Err("fs.relative_to: first argument required".to_string()),
    };
    let base_str = match positional.get(1) {
        Some(obj) => resolve_to_full_path(vm, obj)?,
        _ => return Err("fs.relative_to: second argument required".to_string()),
    };

    let path = std::path::Path::new(&path_str);
    let base = std::path::Path::new(&base_str);

    // Try to strip the base prefix
    if let Ok(rel) = path.strip_prefix(base) {
        Ok(Object::String(rel.to_string_lossy().to_string()))
    } else {
        // Compute relative path manually
        let path_parts: Vec<_> = path.components().collect();
        let base_parts: Vec<_> = base.components().collect();

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

/// Resolve a Meson object to a full filesystem path for is_samepath / relative_to.
fn resolve_to_full_path(vm: &VM, obj: &Object) -> Result<String, String> {
    match obj {
        Object::Array(arr) => {
            // files() returns an array; unwrap to the first element
            if let Some(first) = arr.first() {
                resolve_to_full_path(vm, first)
            } else {
                Err("fs method: empty array argument".to_string())
            }
        }
        Object::String(s) => {
            let p = std::path::Path::new(s);
            if p.is_absolute() {
                Ok(s.clone())
            } else if vm.current_subdir.is_empty() {
                Ok(format!("{}/{}", vm.source_root, s))
            } else {
                Ok(format!("{}/{}/{}", vm.source_root, vm.current_subdir, s))
            }
        }
        Object::File(f) => {
            let base = if f.is_built {
                &vm.build_root
            } else {
                &vm.source_root
            };
            if f.subdir.is_empty() {
                Ok(format!("{}/{}", base, f.path))
            } else {
                Ok(format!("{}/{}/{}", base, f.subdir, f.path))
            }
        }
        Object::BuildTarget(t) => {
            let output = t.outputs.first().map(|s| s.as_str()).unwrap_or(&t.name);
            if t.subdir.is_empty() {
                Ok(format!("{}/{}", vm.build_root, output))
            } else {
                Ok(format!("{}/{}/{}", vm.build_root, t.subdir, output))
            }
        }
        Object::CustomTarget(ct) => {
            let output = ct.outputs.first().map(|s| s.as_str()).unwrap_or(&ct.name);
            if ct.subdir.is_empty() {
                Ok(format!("{}/{}", vm.build_root, output))
            } else {
                Ok(format!("{}/{}/{}", vm.build_root, ct.subdir, output))
            }
        }
        Object::CustomTargetIndex(ct, idx) => {
            let output = ct.outputs.get(*idx).map(|s| s.as_str()).unwrap_or(&ct.name);
            if ct.subdir.is_empty() {
                Ok(format!("{}/{}", vm.build_root, output))
            } else {
                Ok(format!("{}/{}/{}", vm.build_root, ct.subdir, output))
            }
        }
        _ => Err(format!(
            "fs method: unsupported argument type: {}",
            obj.type_name()
        )),
    }
}

/// Normalize a path by resolving `.` and `..` components without touching the filesystem.
fn normalize_path(p: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut parts = Vec::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                if !parts.is_empty() {
                    parts.pop();
                }
            }
            Component::CurDir => {}
            other => parts.push(other),
        }
    }
    parts.iter().collect()
}

fn fs_is_samepath(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let path1_str = match positional.first() {
        Some(obj) => resolve_to_full_path(vm, obj)?,
        _ => return Err("fs.is_samepath: first argument required".to_string()),
    };
    let path2_str = match positional.get(1) {
        Some(obj) => resolve_to_full_path(vm, obj)?,
        _ => return Err("fs.is_samepath: second argument required".to_string()),
    };

    let path1 = std::path::Path::new(&path1_str);
    let path2 = std::path::Path::new(&path2_str);

    // Try canonical comparison first (resolves symlinks + normalizes)
    let canon1 = std::fs::canonicalize(path1);
    let canon2 = std::fs::canonicalize(path2);

    match (&canon1, &canon2) {
        (Ok(c1), Ok(c2)) => Ok(Object::Bool(c1 == c2)),
        _ => {
            // Fallback: normalize paths without requiring them to exist
            let n1 = canon1.unwrap_or_else(|_| normalize_path(path1));
            let n2 = canon2.unwrap_or_else(|_| normalize_path(path2));
            Ok(Object::Bool(n1 == n2))
        }
    }
}
