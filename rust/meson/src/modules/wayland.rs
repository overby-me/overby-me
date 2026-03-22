/// Wayland protocol helpers: scan_xml, find_protocol.
use crate::objects::*;
use crate::vm::*;

pub fn register(vm: &mut VM) {
    vm.method_registry.insert(
        ("module".to_string(), "wayland.scan_xml".to_string()),
        wayland_scan_xml,
    );
    vm.method_registry.insert(
        ("module".to_string(), "wayland.find_protocol".to_string()),
        wayland_find_protocol,
    );
}

fn wayland_scan_xml(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);

    // Collect XML protocol files from positional args
    let sources: Vec<String> = positional
        .iter()
        .filter_map(|o| match o {
            Object::String(s) => Some(s.clone()),
            Object::File(f) => Some(f.path.clone()),
            _ => None,
        })
        .collect();

    if sources.is_empty() {
        return Err("wayland.scan_xml: at least one XML protocol file required".to_string());
    }

    let client = VM::get_arg_bool(args, "client", true);
    let server = VM::get_arg_bool(args, "server", false);
    let public = VM::get_arg_bool(args, "public", true);

    let mut outputs = Vec::new();
    for src in &sources {
        let stem = std::path::Path::new(src)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        // wayland-scanner generates a .h and .c per protocol
        if client {
            outputs.push(format!("{}-client-protocol.h", stem));
        }
        if server {
            outputs.push(format!("{}-server-protocol.h", stem));
        }
        outputs.push(format!("{}-protocol.c", stem));
    }

    let name = "wayland_protocols".to_string();
    let id = format!("wayland_scan_{}", sources.len());

    let mut command = vec!["wayland-scanner".to_string()];
    if public {
        command.push("public-code".to_string());
    } else {
        command.push("private-code".to_string());
    }
    command.push("@INPUT@".to_string());
    command.push("@OUTPUT@".to_string());

    let ct = CustomTargetRef {
        name: name.clone(),
        id: id.clone(),
        outputs: outputs.clone(),
        subdir: vm.current_subdir.clone(),
    };

    vm.build_data.custom_targets.push(CustomTarget {
        name,
        id,
        command,
        input: sources,
        output: outputs,
        depends: Vec::new(),
        depend_files: Vec::new(),
        depfile: None,
        capture: false,
        feed: false,
        install: false,
        install_dir: Vec::new(),
        install_tag: Vec::new(),
        build_by_default: true,
        build_always_stale: false,
        env: std::collections::HashMap::new(),
        subdir: vm.current_subdir.clone(),
    });

    Ok(Object::CustomTarget(ct))
}

fn wayland_find_protocol(_vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);

    let protocol_name = match positional.first() {
        Some(Object::String(s)) => s.clone(),
        _ => return Err("wayland.find_protocol: first argument must be protocol name".to_string()),
    };

    let state = VM::get_arg_str(args, "state", 99).unwrap_or("stable");

    // Look for protocol XML in standard wayland-protocols paths
    let search_paths = [
        "/usr/share/wayland-protocols",
        "/usr/local/share/wayland-protocols",
    ];

    let subdirs = match state {
        "stable" => vec!["stable"],
        "staging" => vec!["staging"],
        "unstable" => vec!["unstable"],
        _ => vec!["stable", "staging", "unstable"],
    };

    for base in &search_paths {
        for subdir in &subdirs {
            let dir = std::path::Path::new(base).join(subdir).join(&protocol_name);
            if dir.is_dir() {
                let xml_name = format!("{}.xml", protocol_name);
                let xml_path = dir.join(&xml_name);
                if xml_path.exists() {
                    return Ok(Object::File(FileData {
                        path: xml_path.to_string_lossy().to_string(),
                        subdir: String::new(),
                        is_built: false,
                    }));
                }
                // Try with version suffix patterns
                if let Ok(entries) = std::fs::read_dir(&dir) {
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if name.ends_with(".xml") {
                            return Ok(Object::File(FileData {
                                path: entry.path().to_string_lossy().to_string(),
                                subdir: String::new(),
                                is_built: false,
                            }));
                        }
                    }
                }
            }
        }
    }

    Err(format!(
        "wayland.find_protocol: protocol '{}' not found (state: {})",
        protocol_name, state
    ))
}
