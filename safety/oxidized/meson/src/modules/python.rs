/// Python module: find_installation, extension_module.
use crate::objects::*;
use crate::vm::*;

pub fn register(vm: &mut VM) {
    // python.find_installation and python.find_python (alias)
    vm.method_registry.insert(
        ("module".to_string(), "python.find_installation".to_string()),
        python_find_installation,
    );
    vm.method_registry.insert(
        ("module".to_string(), "python.find_python".to_string()),
        python_find_installation,
    );
    // python3.find_installation and python3.find_python (alias)
    vm.method_registry.insert(
        (
            "module".to_string(),
            "python3.find_installation".to_string(),
        ),
        python_find_installation,
    );
    vm.method_registry.insert(
        ("module".to_string(), "python3.find_python".to_string()),
        python_find_installation,
    );
    // extension_module for both module names
    vm.method_registry.insert(
        ("module".to_string(), "python.extension_module".to_string()),
        python_extension_module,
    );
    vm.method_registry.insert(
        ("module".to_string(), "python3.extension_module".to_string()),
        python_extension_module,
    );
}

fn python_find_installation(
    _vm: &mut VM,
    _obj: &Object,
    args: &[CallArg],
) -> Result<Object, String> {
    let name = VM::get_arg_str(args, "name", 0).unwrap_or("python3");
    let required = VM::get_arg_bool(args, "required", true);
    let modules = VM::get_arg_string_array(args, "modules");

    // Try to find python
    let candidates = if name.is_empty() || name == "python3" {
        vec!["python3", "python"]
    } else {
        vec![name]
    };

    for candidate in &candidates {
        let output = std::process::Command::new(candidate)
            .args(["--version"])
            .output();

        if let Ok(out) = output {
            if out.status.success() {
                let version_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let version = version_str
                    .strip_prefix("Python ")
                    .unwrap_or(&version_str)
                    .to_string();

                // Check required modules if specified
                if !modules.is_empty() {
                    for module in &modules {
                        let check = std::process::Command::new(candidate)
                            .args(["-c", &format!("import {}", module)])
                            .output();
                        if let Ok(out) = check {
                            if !out.status.success() && required {
                                return Err(format!("Python module '{}' not found", module));
                            }
                        }
                    }
                }

                // Find the full path
                let which = std::process::Command::new("which").arg(candidate).output();
                let path = match which {
                    Ok(out) if out.status.success() => {
                        String::from_utf8_lossy(&out.stdout).trim().to_string()
                    }
                    _ => candidate.to_string(),
                };

                return Ok(Object::ExternalProgram(ExternalProgramData {
                    name: candidate.to_string(),
                    path,
                    found: true,
                    version: Some(version),
                }));
            }
        }
    }

    if required {
        Err(format!("Python installation '{}' not found", name))
    } else {
        Ok(Object::ExternalProgram(ExternalProgramData {
            name: name.to_string(),
            path: String::new(),
            found: false,
            version: None,
        }))
    }
}

fn python_extension_module(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);

    let name = match positional.first() {
        Some(Object::String(s)) => s.clone(),
        _ => {
            return Err(
                "python.extension_module: first argument must be a module name".to_string(),
            );
        }
    };

    // Collect sources from remaining positional args
    let sources: Vec<String> = positional
        .iter()
        .skip(1)
        .filter_map(|o| match o {
            Object::String(s) => Some(s.clone()),
            Object::File(f) => Some(f.path.clone()),
            _ => None,
        })
        .collect();

    let dependencies: Vec<Object> = match VM::get_arg_value(args, "dependencies") {
        Some(Object::Array(arr)) => arr.clone(),
        Some(dep) => vec![dep.clone()],
        None => Vec::new(),
    };

    let install = VM::get_arg_bool(args, "install", false);
    let install_dir = VM::get_arg_str(args, "install_dir", 99).map(|s| s.to_string());
    let subdir = VM::get_arg_str(args, "subdir", 99)
        .unwrap_or("")
        .to_string();

    let id = format!("python_ext_{}", name);
    let output_name = format!("{}.so", name);

    let target = BuildTarget {
        name: name.clone(),
        id: id.clone(),
        target_type: TargetType::SharedModule,
        sources,
        objects: Vec::new(),
        dependencies,
        include_dirs: Vec::new(),
        link_with: Vec::new(),
        link_whole: Vec::new(),
        link_args: Vec::new(),
        c_args: Vec::new(),
        cpp_args: Vec::new(),
        rust_args: Vec::new(),
        install,
        install_dir,
        install_rpath: String::new(),
        build_rpath: String::new(),
        pic: Some(true),
        pie: None,
        override_options: Vec::new(),
        gnu_symbol_visibility: "hidden".to_string(),
        native: false,
        extra_files: Vec::new(),
        implicit_include_directories: true,
        win_subsystem: "console".to_string(),
        name_prefix: Some(String::new()),
        name_suffix: Some("so".to_string()),
        rust_crate_type: None,
        build_by_default: true,
        subdir: if subdir.is_empty() {
            vm.current_subdir.clone()
        } else {
            subdir
        },
        output_name,
    };

    let target_ref = BuildTargetRef {
        name: name.clone(),
        id,
        target_type: "shared_module".to_string(),
        subdir: vm.current_subdir.clone(),
        outputs: vec![target.output_name.clone()],
    };

    vm.build_data.targets.push(target);

    Ok(Object::BuildTarget(target_ref))
}
