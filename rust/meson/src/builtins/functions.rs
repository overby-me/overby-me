/// All built-in global functions available in meson.build.
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use crate::objects::*;
use crate::vm::*;

pub fn register(vm: &mut VM) {
    // Register function name -> Object::BuiltinFunction mapping in globals
    let funcs = [
        "project",
        "message",
        "warning",
        "error",
        "summary",
        "executable",
        "static_library",
        "shared_library",
        "library",
        "both_libraries",
        "shared_module",
        "dependency",
        "declare_dependency",
        "find_program",
        "custom_target",
        "run_target",
        "alias_target",
        "configure_file",
        "vcs_tag",
        "install_headers",
        "install_data",
        "install_subdir",
        "install_man",
        "install_emptydir",
        "install_symlink",
        "test",
        "benchmark",
        "subdir",
        "subproject",
        "environment",
        "generator",
        "run_command",
        "include_directories",
        "import",
        "files",
        "join_paths",
        "get_option",
        "configuration_data",
        "is_variable",
        "get_variable",
        "set_variable",
        "unset_variable",
        "assert",
        "range",
        "structured_sources",
        "add_project_arguments",
        "add_project_link_arguments",
        "add_global_arguments",
        "add_global_link_arguments",
        "add_languages",
        "add_test_setup",
        "disabler",
        "install_tag",
    ];
    for name in funcs {
        vm.globals
            .insert(name.to_string(), Object::BuiltinFunction(name.to_string()));
    }
    // Register meson, build_machine, host_machine, target_machine globals
    vm.globals.insert("meson".to_string(), Object::MesonObject);
    let machine = MachineInfoData::detect();
    vm.globals.insert(
        "build_machine".to_string(),
        Object::MachineInfo(machine.clone()),
    );
    vm.globals.insert(
        "host_machine".to_string(),
        Object::MachineInfo(machine.clone()),
    );
    vm.globals
        .insert("target_machine".to_string(), Object::MachineInfo(machine));

    // Register the actual function implementations
    vm.builtins.insert("project".to_string(), builtin_project);
    vm.builtins.insert("message".to_string(), builtin_message);
    vm.builtins.insert("warning".to_string(), builtin_warning);
    vm.builtins.insert("error".to_string(), builtin_error);
    vm.builtins.insert("summary".to_string(), builtin_summary);
    vm.builtins
        .insert("executable".to_string(), builtin_executable);
    vm.builtins
        .insert("static_library".to_string(), builtin_static_library);
    vm.builtins
        .insert("shared_library".to_string(), builtin_shared_library);
    vm.builtins.insert("library".to_string(), builtin_library);
    vm.builtins
        .insert("both_libraries".to_string(), builtin_both_libraries);
    vm.builtins
        .insert("shared_module".to_string(), builtin_shared_module);
    vm.builtins
        .insert("dependency".to_string(), builtin_dependency);
    vm.builtins
        .insert("declare_dependency".to_string(), builtin_declare_dependency);
    vm.builtins
        .insert("find_program".to_string(), builtin_find_program);
    vm.builtins
        .insert("custom_target".to_string(), builtin_custom_target);
    vm.builtins
        .insert("run_target".to_string(), builtin_run_target);
    vm.builtins
        .insert("alias_target".to_string(), builtin_alias_target);
    vm.builtins
        .insert("configure_file".to_string(), builtin_configure_file);
    vm.builtins.insert("vcs_tag".to_string(), builtin_vcs_tag);
    vm.builtins
        .insert("install_headers".to_string(), builtin_install_headers);
    vm.builtins
        .insert("install_data".to_string(), builtin_install_data);
    vm.builtins
        .insert("install_subdir".to_string(), builtin_install_subdir);
    vm.builtins
        .insert("install_man".to_string(), builtin_install_man);
    vm.builtins
        .insert("install_emptydir".to_string(), builtin_install_emptydir);
    vm.builtins
        .insert("install_symlink".to_string(), builtin_install_symlink);
    vm.builtins.insert("test".to_string(), builtin_test);
    vm.builtins
        .insert("benchmark".to_string(), builtin_benchmark);
    vm.builtins.insert("subdir".to_string(), builtin_subdir);
    vm.builtins
        .insert("subproject".to_string(), builtin_subproject);
    vm.builtins
        .insert("environment".to_string(), builtin_environment);
    vm.builtins
        .insert("generator".to_string(), builtin_generator);
    vm.builtins
        .insert("run_command".to_string(), builtin_run_command);
    vm.builtins.insert(
        "include_directories".to_string(),
        builtin_include_directories,
    );
    vm.builtins.insert("import".to_string(), builtin_import);
    vm.builtins.insert("files".to_string(), builtin_files);
    vm.builtins
        .insert("join_paths".to_string(), builtin_join_paths);
    vm.builtins
        .insert("get_option".to_string(), builtin_get_option);
    vm.builtins
        .insert("configuration_data".to_string(), builtin_configuration_data);
    vm.builtins
        .insert("is_variable".to_string(), builtin_is_variable);
    vm.builtins
        .insert("get_variable".to_string(), builtin_get_variable);
    vm.builtins
        .insert("set_variable".to_string(), builtin_set_variable);
    vm.builtins
        .insert("unset_variable".to_string(), builtin_unset_variable);
    vm.builtins.insert("assert".to_string(), builtin_assert);
    vm.builtins.insert("range".to_string(), builtin_range);
    vm.builtins
        .insert("structured_sources".to_string(), builtin_structured_sources);
    vm.builtins.insert(
        "add_project_arguments".to_string(),
        builtin_add_project_arguments,
    );
    vm.builtins.insert(
        "add_project_link_arguments".to_string(),
        builtin_add_project_link_arguments,
    );
    vm.builtins.insert(
        "add_global_arguments".to_string(),
        builtin_add_global_arguments,
    );
    vm.builtins.insert(
        "add_global_link_arguments".to_string(),
        builtin_add_global_link_arguments,
    );
    vm.builtins
        .insert("add_languages".to_string(), builtin_add_languages);
    vm.builtins
        .insert("add_test_setup".to_string(), builtin_add_test_setup);
    vm.builtins.insert("disabler".to_string(), builtin_disabler);
    vm.builtins
        .insert("install_tag".to_string(), builtin_install_tag);
}

fn builtin_project(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let name = positional
        .first()
        .and_then(|v| {
            if let Object::String(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .unwrap_or_default();

    let mut languages = Vec::new();
    for arg in positional.iter().skip(1) {
        if let Object::String(s) = arg {
            languages.push(s.clone());
        } else if let Object::Array(arr) = arg {
            for item in arr {
                if let Object::String(s) = item {
                    languages.push(s.clone());
                }
            }
        }
    }

    let version = VM::get_arg_str(args, "version", usize::MAX)
        .unwrap_or("undefined")
        .to_string();
    let meson_version = VM::get_arg_str(args, "meson_version", usize::MAX)
        .unwrap_or("")
        .to_string();
    let license = VM::get_arg_string_array(args, "license");
    let license_files = VM::get_arg_string_array(args, "license_files");
    let subproject_dir = VM::get_arg_str(args, "subproject_dir", usize::MAX)
        .unwrap_or("subprojects")
        .to_string();
    let default_options = VM::get_arg_string_array(args, "default_options");

    vm.project = Some(ProjectInfo {
        name,
        version,
        license,
        license_files,
        meson_version,
        languages: languages.clone(),
        subproject_dir,
        default_options,
    });

    // Detect compilers for requested languages
    for lang in &languages {
        crate::compilers::detect_compiler(vm, lang);
    }

    Ok(Object::None)
}

fn builtin_message(_vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let parts: Vec<String> = VM::get_positional_args(args)
        .iter()
        .map(|v| v.to_display_string())
        .collect();
    eprintln!("Message: {}", parts.join(" "));
    Ok(Object::None)
}

fn builtin_warning(_vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let parts: Vec<String> = VM::get_positional_args(args)
        .iter()
        .map(|v| v.to_display_string())
        .collect();
    eprintln!("WARNING: {}", parts.join(" "));
    Ok(Object::None)
}

fn builtin_error(_vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let parts: Vec<String> = VM::get_positional_args(args)
        .iter()
        .map(|v| v.to_display_string())
        .collect();
    Err(format!("ERROR: {}", parts.join(" ")))
}

fn builtin_summary(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let section = VM::get_arg_str(args, "section", usize::MAX)
        .unwrap_or("")
        .to_string();

    match positional.first() {
        Some(Object::Dict(entries)) => {
            let items: Vec<(String, String)> = entries
                .iter()
                .map(|(k, v)| (k.clone(), v.to_display_string()))
                .collect();
            vm.summary.push((section, items));
        }
        Some(Object::String(key)) => {
            let value = positional
                .get(1)
                .map(|v| v.to_display_string())
                .unwrap_or_default();
            vm.summary.push((section, vec![(key.clone(), value)]));
        }
        _ => {}
    }
    Ok(Object::None)
}

fn build_target_common(
    vm: &mut VM,
    args: &[CallArg],
    target_type: TargetType,
) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let name = positional
        .first()
        .and_then(|v| {
            if let Object::String(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .ok_or("Target name must be a string")?;

    let mut sources = Vec::new();
    for arg in positional.iter().skip(1) {
        collect_sources(arg, &mut sources);
    }
    // Also collect from 'sources' kwarg
    if let Some(Object::Array(arr)) = VM::get_arg_value(args, "sources") {
        for s in arr {
            collect_sources(s, &mut sources);
        }
    }

    let mut dependencies = Vec::new();
    if let Some(Object::Array(arr)) = VM::get_arg_value(args, "dependencies") {
        dependencies = arr.clone();
    } else if let Some(dep) = VM::get_arg_value(args, "dependencies") {
        if !matches!(dep, Object::None) {
            dependencies.push(dep.clone());
        }
    }

    let mut link_with = Vec::new();
    if let Some(Object::Array(arr)) = VM::get_arg_value(args, "link_with") {
        link_with = arr.clone();
    } else if let Some(obj) = VM::get_arg_value(args, "link_with") {
        if !matches!(obj, Object::None) {
            link_with.push(obj.clone());
        }
    }

    let mut link_whole = Vec::new();
    if let Some(Object::Array(arr)) = VM::get_arg_value(args, "link_whole") {
        link_whole = arr.clone();
    }

    let install = VM::get_arg_bool(args, "install", false);
    let install_dir = VM::get_arg_str(args, "install_dir", usize::MAX).map(String::from);
    let install_rpath = VM::get_arg_str(args, "install_rpath", usize::MAX)
        .unwrap_or("")
        .to_string();
    let build_rpath = VM::get_arg_str(args, "build_rpath", usize::MAX)
        .unwrap_or("")
        .to_string();
    let include_dirs_val = VM::get_arg_value(args, "include_directories");
    let mut include_dirs = Vec::new();
    if let Some(Object::Array(arr)) = include_dirs_val {
        for item in arr {
            match item {
                Object::IncludeDirs(d) => include_dirs.extend(d.dirs.clone()),
                Object::String(s) => include_dirs.push(s.clone()),
                _ => {}
            }
        }
    } else if let Some(Object::IncludeDirs(d)) = include_dirs_val {
        include_dirs = d.dirs.clone();
    }

    let c_args = VM::get_arg_string_array(args, "c_args");
    let cpp_args = VM::get_arg_string_array(args, "cpp_args");
    let rust_args = VM::get_arg_string_array(args, "rust_args");
    let link_args = VM::get_arg_string_array(args, "link_args");
    let override_options = VM::get_arg_string_array(args, "override_options");
    let gnu_symbol_visibility = VM::get_arg_str(args, "gnu_symbol_visibility", usize::MAX)
        .unwrap_or("")
        .to_string();
    let native = VM::get_arg_bool(args, "native", false);
    let build_by_default = VM::get_arg_bool(args, "build_by_default", true);
    let win_subsystem = VM::get_arg_str(args, "win_subsystem", usize::MAX)
        .unwrap_or("console")
        .to_string();
    let name_prefix = VM::get_arg_str(args, "name_prefix", usize::MAX).map(String::from);
    let name_suffix = VM::get_arg_str(args, "name_suffix", usize::MAX).map(String::from);
    let rust_crate_type = VM::get_arg_str(args, "rust_crate_type", usize::MAX).map(String::from);
    let implicit_include_directories = VM::get_arg_bool(args, "implicit_include_directories", true);

    let pic = VM::get_arg_value(args, "pic").and_then(|v| {
        if let Object::Bool(b) = v {
            Some(*b)
        } else {
            None
        }
    });
    let pie = VM::get_arg_value(args, "pie").and_then(|v| {
        if let Object::Bool(b) = v {
            Some(*b)
        } else {
            None
        }
    });

    let type_str = match target_type {
        TargetType::Executable => "executable",
        TargetType::SharedLibrary => "shared_library",
        TargetType::StaticLibrary => "static_library",
        TargetType::SharedModule => "shared_module",
        TargetType::BothLibraries => "both_libraries",
        TargetType::Jar => "jar",
    };

    let id = format!("{}@{}", name, vm.current_subdir);
    let subdir = vm.current_subdir.clone();

    // Compute output name
    let output_name = compute_output_name(
        &name,
        &target_type,
        name_prefix.as_deref(),
        name_suffix.as_deref(),
    );

    let target = BuildTarget {
        name: name.clone(),
        id: id.clone(),
        target_type,
        sources,
        objects: Vec::new(),
        dependencies,
        include_dirs,
        link_with,
        link_whole,
        link_args,
        c_args,
        cpp_args,
        rust_args,
        install,
        install_dir,
        install_rpath,
        build_rpath,
        pic,
        pie,
        override_options,
        gnu_symbol_visibility,
        native,
        extra_files: Vec::new(),
        implicit_include_directories,
        win_subsystem,
        name_prefix,
        name_suffix,
        rust_crate_type,
        build_by_default,
        subdir: subdir.clone(),
        output_name: output_name.clone(),
    };

    vm.build_data.targets.push(target);

    Ok(Object::BuildTarget(BuildTargetRef {
        name,
        id,
        target_type: type_str.to_string(),
        subdir,
        outputs: vec![output_name],
    }))
}

fn compute_output_name(
    name: &str,
    target_type: &TargetType,
    name_prefix: Option<&str>,
    name_suffix: Option<&str>,
) -> String {
    let prefix = name_prefix.unwrap_or_else(|| match target_type {
        TargetType::Executable => "",
        TargetType::SharedLibrary | TargetType::SharedModule => {
            if cfg!(target_os = "windows") {
                ""
            } else {
                "lib"
            }
        }
        TargetType::StaticLibrary => {
            if cfg!(target_os = "windows") {
                ""
            } else {
                "lib"
            }
        }
        _ => "",
    });
    let suffix = name_suffix.unwrap_or_else(|| match target_type {
        TargetType::Executable => {
            if cfg!(target_os = "windows") {
                "exe"
            } else {
                ""
            }
        }
        TargetType::SharedLibrary | TargetType::SharedModule => {
            if cfg!(target_os = "windows") {
                "dll"
            } else if cfg!(target_os = "macos") {
                "dylib"
            } else {
                "so"
            }
        }
        TargetType::StaticLibrary => "a",
        _ => "",
    });
    if suffix.is_empty() {
        format!("{}{}", prefix, name)
    } else {
        format!("{}{}.{}", prefix, name, suffix)
    }
}

fn collect_sources(obj: &Object, sources: &mut Vec<String>) {
    match obj {
        Object::String(s) => sources.push(s.clone()),
        Object::Array(arr) => {
            for item in arr {
                collect_sources(item, sources);
            }
        }
        Object::File(f) => sources.push(f.path.clone()),
        Object::GeneratedList(_) | Object::CustomTarget(_) | Object::CustomTargetIndex(_, _) => {
            // These will be handled during build graph construction
            sources.push(format!("<generated:{}>", obj.to_display_string()));
        }
        _ => {}
    }
}

fn builtin_executable(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    build_target_common(vm, args, TargetType::Executable)
}

fn builtin_static_library(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    build_target_common(vm, args, TargetType::StaticLibrary)
}

fn builtin_shared_library(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    build_target_common(vm, args, TargetType::SharedLibrary)
}

fn builtin_library(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    // Default library type depends on default_library option
    let default_lib = vm
        .options
        .get("default_library")
        .and_then(|v| {
            if let Object::String(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "shared".to_string());
    match default_lib.as_str() {
        "static" => build_target_common(vm, args, TargetType::StaticLibrary),
        "both" => builtin_both_libraries(vm, args),
        _ => build_target_common(vm, args, TargetType::SharedLibrary),
    }
}

fn builtin_both_libraries(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let shared = build_target_common(vm, args, TargetType::SharedLibrary)?;
    let static_lib = build_target_common(vm, args, TargetType::StaticLibrary)?;
    Ok(Object::BothLibraries(
        Box::new(shared),
        Box::new(static_lib),
    ))
}

fn builtin_shared_module(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    build_target_common(vm, args, TargetType::SharedModule)
}

fn builtin_dependency(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let name = positional
        .first()
        .and_then(|v| {
            if let Object::String(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .unwrap_or_default();

    let required = match VM::get_arg_value(args, "required") {
        Some(Object::Bool(b)) => *b,
        Some(Object::Feature(FeatureState::Disabled)) => false,
        Some(Object::Feature(FeatureState::Enabled)) => true,
        _ => true,
    };

    // Check if already found
    if let Some(dep) = vm.build_data.dependencies.get(&name) {
        return Ok(dep.clone());
    }

    // Try to find the dependency
    let dep = crate::dependencies::find_dependency(vm, &name, args);

    if dep.is_none() && required {
        return Err(format!("Dependency '{}' not found", name));
    }

    let obj = dep.unwrap_or_else(|| Object::Dependency(DependencyData::not_found(&name)));
    vm.build_data.dependencies.insert(name, obj.clone());
    Ok(obj)
}

fn builtin_declare_dependency(_vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let compile_args = VM::get_arg_string_array(args, "compile_args");
    let link_args = VM::get_arg_string_array(args, "link_args");
    let include_dirs_val = VM::get_arg_value(args, "include_directories");
    let mut include_dirs = Vec::new();
    if let Some(Object::Array(arr)) = include_dirs_val {
        for item in arr {
            match item {
                Object::IncludeDirs(d) => include_dirs.extend(d.dirs.clone()),
                Object::String(s) => include_dirs.push(s.clone()),
                _ => {}
            }
        }
    }

    let version = VM::get_arg_str(args, "version", usize::MAX)
        .unwrap_or("")
        .to_string();

    let mut dependencies = Vec::new();
    if let Some(Object::Array(arr)) = VM::get_arg_value(args, "dependencies") {
        dependencies = arr.clone();
    }

    let mut sources = Vec::new();
    if let Some(Object::Array(arr)) = VM::get_arg_value(args, "sources") {
        for s in arr {
            collect_sources(s, &mut sources);
        }
    }

    let mut variables = HashMap::new();
    if let Some(Object::Dict(entries)) = VM::get_arg_value(args, "variables") {
        for (k, v) in entries {
            variables.insert(k.clone(), v.to_string_value());
        }
    }

    Ok(Object::Dependency(DependencyData {
        name: String::new(),
        found: true,
        version,
        compile_args,
        link_args,
        sources,
        include_dirs,
        dependencies,
        variables,
        is_internal: true,
    }))
}

fn builtin_find_program(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let required = match VM::get_arg_value(args, "required") {
        Some(Object::Bool(b)) => *b,
        Some(Object::Feature(FeatureState::Disabled)) => false,
        _ => true,
    };
    let native = VM::get_arg_bool(args, "native", false);
    let _version = VM::get_arg_str(args, "version", usize::MAX);
    let dirs = VM::get_arg_string_array(args, "dirs");

    for arg in &positional {
        if let Object::String(name) = arg {
            // Search in specified dirs first
            for dir in &dirs {
                let path = format!("{}/{}", dir, name);
                if Path::new(&path).exists() {
                    return Ok(Object::ExternalProgram(ExternalProgramData {
                        name: name.clone(),
                        path,
                        found: true,
                        version: None,
                    }));
                }
            }
            // Search in source tree
            let src_path = if vm.current_subdir.is_empty() {
                format!("{}/{}", vm.source_root, name)
            } else {
                format!("{}/{}/{}", vm.source_root, vm.current_subdir, name)
            };
            if Path::new(&src_path).exists() {
                return Ok(Object::ExternalProgram(ExternalProgramData {
                    name: name.clone(),
                    path: src_path,
                    found: true,
                    version: None,
                }));
            }
            // Search on PATH
            if let Ok(output) = Command::new("which").arg(name).output() {
                if output.status.success() {
                    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    return Ok(Object::ExternalProgram(ExternalProgramData {
                        name: name.clone(),
                        path,
                        found: true,
                        version: None,
                    }));
                }
            }
        } else if let Object::ExternalProgram(p) = *arg {
            if p.found {
                return Ok(Object::ExternalProgram(p.clone()));
            }
        }
    }

    let name = positional
        .first()
        .map(|v| v.to_display_string())
        .unwrap_or_else(|| "unknown".to_string());

    if required && !native {
        return Err(format!("Program '{}' not found", name));
    }

    Ok(Object::ExternalProgram(ExternalProgramData {
        name,
        path: String::new(),
        found: false,
        version: None,
    }))
}

fn builtin_custom_target(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let name = positional
        .first()
        .and_then(|v| {
            if let Object::String(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .unwrap_or_default();

    let mut command = Vec::new();
    if let Some(Object::Array(arr)) = VM::get_arg_value(args, "command") {
        for item in arr {
            command.push(item.to_string_value());
        }
    }

    let mut input = Vec::new();
    if let Some(Object::Array(arr)) = VM::get_arg_value(args, "input") {
        for item in arr {
            input.push(item.to_string_value());
        }
    }

    let mut output = Vec::new();
    if let Some(Object::Array(arr)) = VM::get_arg_value(args, "output") {
        for item in arr {
            if let Object::String(s) = item {
                output.push(s.clone());
            }
        }
    }

    let capture = VM::get_arg_bool(args, "capture", false);
    let feed = VM::get_arg_bool(args, "feed", false);
    let install = VM::get_arg_bool(args, "install", false);
    let install_dir = VM::get_arg_string_array(args, "install_dir");
    let install_tag = VM::get_arg_string_array(args, "install_tag");
    let build_by_default = VM::get_arg_bool(args, "build_by_default", install);
    let build_always_stale = VM::get_arg_bool(args, "build_always_stale", false);
    let depfile = VM::get_arg_str(args, "depfile", usize::MAX).map(String::from);

    let id = format!("custom_target:{}@{}", name, vm.current_subdir);
    let subdir = vm.current_subdir.clone();

    let ct = CustomTarget {
        name: name.clone(),
        id: id.clone(),
        command,
        input,
        output: output.clone(),
        depends: Vec::new(),
        depend_files: Vec::new(),
        depfile,
        capture,
        feed,
        install,
        install_dir,
        install_tag,
        build_by_default,
        build_always_stale,
        env: HashMap::new(),
        subdir: subdir.clone(),
    };

    vm.build_data.custom_targets.push(ct);

    Ok(Object::CustomTarget(CustomTargetRef {
        name,
        id,
        outputs: output,
        subdir,
    }))
}

fn builtin_run_target(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let name = positional
        .first()
        .and_then(|v| {
            if let Object::String(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .ok_or("run_target requires a name")?;

    let mut command = Vec::new();
    if let Some(Object::Array(arr)) = VM::get_arg_value(args, "command") {
        for item in arr {
            command.push(item.to_string_value());
        }
    }

    vm.build_data.run_targets.push(RunTarget {
        name: name.clone(),
        command,
        depends: Vec::new(),
        env: HashMap::new(),
        subdir: vm.current_subdir.clone(),
    });

    Ok(Object::None)
}

fn builtin_alias_target(_vm: &mut VM, _args: &[CallArg]) -> Result<Object, String> {
    // Alias targets are a thin wrapper — just return None for now
    Ok(Object::None)
}

fn builtin_configure_file(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let input = VM::get_arg_str(args, "input", usize::MAX).map(String::from);
    let output = VM::get_arg_str(args, "output", 0)
        .ok_or("configure_file() requires 'output'")?
        .to_string();
    let configuration = VM::get_arg_value(args, "configuration").cloned();
    let format = VM::get_arg_str(args, "format", usize::MAX)
        .unwrap_or("meson")
        .to_string();
    let output_format = VM::get_arg_str(args, "output_format", usize::MAX)
        .unwrap_or("c")
        .to_string();
    let encoding = VM::get_arg_str(args, "encoding", usize::MAX)
        .unwrap_or("utf-8")
        .to_string();
    let install = VM::get_arg_bool(args, "install", false);
    let install_dir = VM::get_arg_str(args, "install_dir", usize::MAX).map(String::from);
    let install_tag = VM::get_arg_str(args, "install_tag", usize::MAX).map(String::from);
    let capture = VM::get_arg_bool(args, "capture", false);
    let depfile = VM::get_arg_str(args, "depfile", usize::MAX).map(String::from);

    let mut command = Vec::new();
    if let Some(Object::Array(arr)) = VM::get_arg_value(args, "command") {
        for item in arr {
            command.push(item.to_string_value());
        }
    }

    vm.build_data.configure_files.push(ConfigureFile {
        input,
        output: output.clone(),
        configuration,
        command,
        format,
        output_format,
        encoding,
        install,
        install_dir,
        install_tag,
        capture,
        depfile,
        subdir: vm.current_subdir.clone(),
    });

    Ok(Object::File(FileData {
        path: output,
        subdir: vm.current_subdir.clone(),
        is_built: true,
    }))
}

fn builtin_vcs_tag(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let input = VM::get_arg_str(args, "input", usize::MAX)
        .unwrap_or("")
        .to_string();
    let output = VM::get_arg_str(args, "output", usize::MAX)
        .ok_or("vcs_tag requires 'output'")?
        .to_string();
    let fallback = VM::get_arg_str(args, "fallback", usize::MAX)
        .unwrap_or("")
        .to_string();
    let replace_string = VM::get_arg_str(args, "replace_string", usize::MAX)
        .unwrap_or("@VCS_TAG@")
        .to_string();

    // Generate as a custom target that runs git describe
    let command = vec![
        "sh".to_string(),
        "-c".to_string(),
        format!(
            "git -C {} describe --dirty --always 2>/dev/null || echo '{}'",
            vm.source_root, fallback
        ),
    ];

    let id = format!("vcs_tag:{}@{}", output, vm.current_subdir);
    let subdir = vm.current_subdir.clone();

    vm.build_data.custom_targets.push(CustomTarget {
        name: format!("vcs_tag_{}", output),
        id: id.clone(),
        command,
        input: if input.is_empty() {
            vec![]
        } else {
            vec![input]
        },
        output: vec![output.clone()],
        depends: Vec::new(),
        depend_files: Vec::new(),
        depfile: None,
        capture: true,
        feed: false,
        install: false,
        install_dir: Vec::new(),
        install_tag: Vec::new(),
        build_by_default: true,
        build_always_stale: true,
        env: HashMap::new(),
        subdir: subdir.clone(),
    });

    let _ = replace_string; // Used during actual generation

    Ok(Object::CustomTarget(CustomTargetRef {
        name: format!("vcs_tag_{}", output),
        id,
        outputs: vec![output],
        subdir,
    }))
}

fn builtin_install_headers(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let mut sources = Vec::new();
    for arg in &positional {
        collect_sources(arg, &mut sources);
    }
    let install_dir = VM::get_arg_str(args, "install_dir", usize::MAX)
        .unwrap_or("include")
        .to_string();
    let subdir = VM::get_arg_str(args, "subdir", usize::MAX)
        .unwrap_or("")
        .to_string();
    let full_dir = if subdir.is_empty() {
        install_dir
    } else {
        format!("{}/{}", install_dir, subdir)
    };
    let preserve_path = VM::get_arg_bool(args, "preserve_path", false);

    vm.build_data.install_headers.push(InstallData {
        sources,
        install_dir: full_dir,
        install_mode: None,
        rename: Vec::new(),
        subdir: vm.current_subdir.clone(),
        preserve_path,
        strip_directory: false,
        exclude_files: Vec::new(),
        exclude_directories: Vec::new(),
        follow_symlinks: None,
    });
    Ok(Object::None)
}

fn builtin_install_data(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let mut sources = Vec::new();
    for arg in &positional {
        collect_sources(arg, &mut sources);
    }
    if let Some(Object::Array(arr)) = VM::get_arg_value(args, "sources") {
        for s in arr {
            collect_sources(s, &mut sources);
        }
    }
    let install_dir = VM::get_arg_str(args, "install_dir", usize::MAX)
        .unwrap_or("share")
        .to_string();
    let rename = VM::get_arg_string_array(args, "rename");
    let preserve_path = VM::get_arg_bool(args, "preserve_path", false);

    vm.build_data.install_data.push(InstallData {
        sources,
        install_dir,
        install_mode: None,
        rename,
        subdir: vm.current_subdir.clone(),
        preserve_path,
        strip_directory: false,
        exclude_files: Vec::new(),
        exclude_directories: Vec::new(),
        follow_symlinks: None,
    });
    Ok(Object::None)
}

fn builtin_install_subdir(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let dir = positional
        .first()
        .and_then(|v| {
            if let Object::String(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .ok_or("install_subdir requires directory name")?;
    let install_dir = VM::get_arg_str(args, "install_dir", usize::MAX)
        .ok_or("install_subdir requires install_dir")?
        .to_string();
    let strip_directory = VM::get_arg_bool(args, "strip_directory", false);
    let exclude_files = VM::get_arg_string_array(args, "exclude_files");
    let exclude_directories = VM::get_arg_string_array(args, "exclude_directories");

    vm.build_data.install_subdirs.push(InstallData {
        sources: vec![dir],
        install_dir,
        install_mode: None,
        rename: Vec::new(),
        subdir: vm.current_subdir.clone(),
        preserve_path: false,
        strip_directory,
        exclude_files,
        exclude_directories,
        follow_symlinks: None,
    });
    Ok(Object::None)
}

fn builtin_install_man(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let mut sources = Vec::new();
    for arg in &positional {
        collect_sources(arg, &mut sources);
    }
    let install_dir = VM::get_arg_str(args, "install_dir", usize::MAX)
        .unwrap_or("share/man")
        .to_string();

    vm.build_data.install_man.push(InstallData {
        sources,
        install_dir,
        install_mode: None,
        rename: Vec::new(),
        subdir: vm.current_subdir.clone(),
        preserve_path: false,
        strip_directory: false,
        exclude_files: Vec::new(),
        exclude_directories: Vec::new(),
        follow_symlinks: None,
    });
    Ok(Object::None)
}

fn builtin_install_emptydir(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    for arg in &positional {
        if let Object::String(s) = arg {
            vm.build_data.install_empty_dirs.push(s.clone());
        }
    }
    Ok(Object::None)
}

fn builtin_install_symlink(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let name = positional
        .first()
        .and_then(|v| {
            if let Object::String(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .ok_or("install_symlink requires a name")?;
    let pointing_to = VM::get_arg_str(args, "pointing_to", usize::MAX)
        .ok_or("install_symlink requires 'pointing_to'")?
        .to_string();
    let install_dir = VM::get_arg_str(args, "install_dir", usize::MAX)
        .ok_or("install_symlink requires 'install_dir'")?
        .to_string();

    vm.build_data.install_symlinks.push(SymlinkData {
        name,
        target: pointing_to,
        install_dir,
    });
    Ok(Object::None)
}

fn builtin_test_or_bench(
    vm: &mut VM,
    args: &[CallArg],
    is_benchmark: bool,
) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let name = positional
        .first()
        .and_then(|v| {
            if let Object::String(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .ok_or("test/benchmark requires a name")?;
    let exe = positional
        .get(1)
        .cloned()
        .ok_or("test/benchmark requires an executable")?
        .clone();

    let test_args = VM::get_arg_string_array(args, "args");
    let should_fail = VM::get_arg_bool(args, "should_fail", false);
    let timeout = VM::get_arg_int(args, "timeout", if is_benchmark { 0 } else { 30 });
    let workdir = VM::get_arg_str(args, "workdir", usize::MAX).map(String::from);
    let protocol = VM::get_arg_str(args, "protocol", usize::MAX)
        .unwrap_or("exitcode")
        .to_string();
    let priority = VM::get_arg_int(args, "priority", 0);
    let suite = VM::get_arg_string_array(args, "suite");
    let is_parallel = VM::get_arg_bool(args, "is_parallel", true);
    let verbose = VM::get_arg_bool(args, "verbose", false);

    let env = if let Some(Object::Environment(e)) = VM::get_arg_value(args, "env") {
        e.to_map()
    } else {
        HashMap::new()
    };

    let test = TestDef {
        name,
        exe,
        args: test_args,
        env,
        should_fail,
        timeout,
        workdir,
        protocol,
        priority,
        suite,
        depends: Vec::new(),
        is_parallel,
        verbose,
    };

    if is_benchmark {
        vm.build_data.benchmarks.push(test);
    } else {
        vm.build_data.tests.push(test);
    }
    Ok(Object::None)
}

fn builtin_test(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    builtin_test_or_bench(vm, args, false)
}

fn builtin_benchmark(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    builtin_test_or_bench(vm, args, true)
}

fn builtin_subdir(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let dir = positional
        .first()
        .and_then(|v| {
            if let Object::String(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .ok_or("subdir() requires directory name")?;

    // Save current state
    let old_subdir = vm.current_subdir.clone();
    let new_subdir = if old_subdir.is_empty() {
        dir.clone()
    } else {
        format!("{}/{}", old_subdir, dir)
    };

    // Read and execute the meson.build in the subdirectory
    let subdir_build = format!("{}/{}/meson.build", vm.source_root, new_subdir);
    let source = std::fs::read_to_string(&subdir_build)
        .map_err(|e| format!("Cannot read {}: {}", subdir_build, e))?;

    vm.current_subdir = new_subdir;

    // Parse and execute
    let mut lexer = crate::lexer::Lexer::new(&source);
    let tokens = lexer
        .tokenize()
        .map_err(|e| format!("In {}: {}", subdir_build, e))?;
    let mut parser = crate::parser::Parser::new(tokens);
    let program = parser
        .parse()
        .map_err(|e| format!("In {}: {}", subdir_build, e))?;
    let mut compiler = crate::compiler::Compiler::new();
    compiler
        .compile(&program)
        .map_err(|e| format!("In {}: {}", subdir_build, e))?;
    vm.execute(&compiler.chunk)
        .map_err(|e| format!("In {}: {}", subdir_build, e))?;

    // Restore
    vm.current_subdir = old_subdir;
    Ok(Object::None)
}

fn builtin_subproject(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let name = positional
        .first()
        .and_then(|v| {
            if let Object::String(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .ok_or("subproject() requires project name")?;

    let required = match VM::get_arg_value(args, "required") {
        Some(Object::Bool(b)) => *b,
        Some(Object::Feature(FeatureState::Disabled)) => false,
        _ => true,
    };

    // Check if already loaded
    if let Some(sp) = vm.build_data.subprojects.get(&name) {
        return Ok(sp.clone());
    }

    let subproject_dir = vm
        .project
        .as_ref()
        .map(|p| p.subproject_dir.clone())
        .unwrap_or_else(|| "subprojects".to_string());

    let subproject_path = format!("{}/{}/{}", vm.source_root, subproject_dir, name);
    let build_file = format!("{}/meson.build", subproject_path);

    if !Path::new(&build_file).exists() {
        // Try to download via wrap
        let wrap_file = format!("{}/{}/{}.wrap", vm.source_root, subproject_dir, name);
        if Path::new(&wrap_file).exists() {
            crate::wrap::download_wrap(&wrap_file, &subproject_path)?;
        } else if required {
            return Err(format!("Subproject '{}' not found", name));
        } else {
            let sp = Object::Subproject(SubprojectData {
                name: name.clone(),
                version: String::new(),
                found: false,
                variables: HashMap::new(),
            });
            vm.build_data.subprojects.insert(name, sp.clone());
            return Ok(sp);
        }
    }

    // Save and set up subproject state
    let old_subdir = vm.current_subdir.clone();
    let old_source = vm.source_root.clone();
    let old_project = vm.project.clone();
    let old_vars = vm.variables.clone();

    vm.source_root = subproject_path;
    vm.current_subdir = String::new();
    vm.variables = HashMap::new();

    // Re-register builtins in new scope
    crate::builtins::functions::register(vm);

    let source = std::fs::read_to_string(&build_file)
        .map_err(|e| format!("Cannot read {}: {}", build_file, e))?;
    let mut lexer = crate::lexer::Lexer::new(&source);
    let tokens = lexer.tokenize()?;
    let mut parser = crate::parser::Parser::new(tokens);
    let program = parser.parse()?;
    let mut compiler_obj = crate::compiler::Compiler::new();
    compiler_obj.compile(&program)?;
    vm.execute(&compiler_obj.chunk)?;

    let sp_project = vm.project.clone();
    let sp_vars = vm.variables.clone();

    // Restore
    vm.current_subdir = old_subdir;
    vm.source_root = old_source;
    vm.project = old_project;
    vm.variables = old_vars;

    let sp = Object::Subproject(SubprojectData {
        name: name.clone(),
        version: sp_project.map(|p| p.version).unwrap_or_default(),
        found: true,
        variables: sp_vars,
    });
    vm.build_data.subprojects.insert(name, sp.clone());
    Ok(sp)
}

fn builtin_environment(_vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let mut env = EnvData::new();
    // Can be initialized with a dict
    if let Some(Object::Dict(entries)) = VM::get_positional_args(args).first() {
        for (k, v) in entries {
            env.values
                .insert(k.clone(), EnvOp::Set(v.to_string_value()));
        }
    }
    Ok(Object::Environment(env))
}

fn builtin_generator(_vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let exe = positional
        .first()
        .ok_or("generator() requires executable")?
        .clone();
    let arguments = VM::get_arg_string_array(args, "arguments");
    let output = VM::get_arg_string_array(args, "output");
    let depfile = VM::get_arg_str(args, "depfile", usize::MAX).map(String::from);
    let capture = VM::get_arg_bool(args, "capture", false);

    Ok(Object::Generator(GeneratorData {
        exe: Box::new(exe.clone()),
        arguments,
        output,
        depfile,
        capture,
    }))
}

fn builtin_run_command(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let check = VM::get_arg_bool(args, "check", false);

    let mut cmd_parts = Vec::new();
    for arg in &positional {
        match arg {
            Object::String(s) => cmd_parts.push(s.clone()),
            Object::ExternalProgram(p) => cmd_parts.push(p.path.clone()),
            Object::File(f) => {
                let path = if f.is_built {
                    format!("{}/{}", vm.build_root, f.path)
                } else {
                    format!("{}/{}", vm.source_root, f.path)
                };
                cmd_parts.push(path);
            }
            Object::Array(arr) => {
                for item in arr {
                    cmd_parts.push(item.to_string_value());
                }
            }
            _ => cmd_parts.push(arg.to_string_value()),
        }
    }

    if cmd_parts.is_empty() {
        return Err("run_command() requires at least one argument".to_string());
    }

    let env_data = if let Some(Object::Environment(e)) = VM::get_arg_value(args, "env") {
        e.to_map()
    } else {
        HashMap::new()
    };

    let result = Command::new(&cmd_parts[0])
        .args(&cmd_parts[1..])
        .envs(&env_data)
        .current_dir(&vm.source_root)
        .output();

    match result {
        Ok(output) => {
            let returncode = output.status.code().unwrap_or(-1) as i64;
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();

            if check && returncode != 0 {
                return Err(format!(
                    "Command '{}' failed with status {}\nstdout: {}\nstderr: {}",
                    cmd_parts.join(" "),
                    returncode,
                    stdout,
                    stderr
                ));
            }

            Ok(Object::RunResult(RunResultData {
                returncode,
                stdout,
                stderr,
            }))
        }
        Err(e) => {
            if check {
                return Err(format!("Failed to execute '{}': {}", cmd_parts[0], e));
            }
            Ok(Object::RunResult(RunResultData {
                returncode: -1,
                stdout: String::new(),
                stderr: e.to_string(),
            }))
        }
    }
}

fn builtin_include_directories(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let is_system = VM::get_arg_bool(args, "is_system", false);

    let mut dirs = Vec::new();
    for arg in &positional {
        if let Object::String(s) = arg {
            let full = if vm.current_subdir.is_empty() {
                s.clone()
            } else {
                format!("{}/{}", vm.current_subdir, s)
            };
            dirs.push(full);
        }
    }

    Ok(Object::IncludeDirs(IncludeDirsData { dirs, is_system }))
}

fn builtin_import(_vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let name = positional
        .first()
        .and_then(|v| {
            if let Object::String(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .ok_or("import() requires module name")?;
    Ok(Object::Module(name))
}

fn builtin_files(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let mut files = Vec::new();
    for arg in &positional {
        if let Object::String(s) = arg {
            files.push(Object::File(FileData {
                path: s.clone(),
                subdir: vm.current_subdir.clone(),
                is_built: false,
            }));
        }
    }
    Ok(Object::Array(files))
}

fn builtin_join_paths(_vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let parts: Vec<String> = positional.iter().map(|v| v.to_string_value()).collect();
    let mut result = std::path::PathBuf::new();
    for part in &parts {
        if part.starts_with('/') {
            result = std::path::PathBuf::from(part);
        } else {
            result.push(part);
        }
    }
    Ok(Object::String(result.to_string_lossy().to_string()))
}

fn builtin_get_option(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let name = positional
        .first()
        .and_then(|v| {
            if let Object::String(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .ok_or("get_option() requires option name")?;

    if let Some(val) = vm.options.get(&name) {
        return Ok(val.clone());
    }

    // Return defaults for well-known options
    match name.as_str() {
        "prefix" => Ok(Object::String("/usr/local".to_string())),
        "bindir" => Ok(Object::String("bin".to_string())),
        "libdir" => Ok(Object::String("lib".to_string())),
        "libexecdir" => Ok(Object::String("libexec".to_string())),
        "includedir" => Ok(Object::String("include".to_string())),
        "datadir" => Ok(Object::String("share".to_string())),
        "mandir" => Ok(Object::String("share/man".to_string())),
        "infodir" => Ok(Object::String("share/info".to_string())),
        "localedir" => Ok(Object::String("share/locale".to_string())),
        "sysconfdir" => Ok(Object::String("etc".to_string())),
        "localstatedir" => Ok(Object::String("var".to_string())),
        "sharedstatedir" => Ok(Object::String("com".to_string())),
        "sbindir" => Ok(Object::String("sbin".to_string())),
        "buildtype" => Ok(Object::String("debug".to_string())),
        "debug" => Ok(Object::Bool(true)),
        "optimization" => Ok(Object::String("0".to_string())),
        "warning_level" => Ok(Object::String("1".to_string())),
        "werror" => Ok(Object::Bool(false)),
        "default_library" => Ok(Object::String("shared".to_string())),
        "b_staticpic" => Ok(Object::Bool(true)),
        "b_pie" => Ok(Object::Bool(false)),
        "b_lto" => Ok(Object::Bool(false)),
        "b_sanitize" => Ok(Object::String("none".to_string())),
        "b_coverage" => Ok(Object::Bool(false)),
        "b_pgo" => Ok(Object::String("off".to_string())),
        "b_ndebug" => Ok(Object::String("false".to_string())),
        "strip" => Ok(Object::Bool(false)),
        "unity" => Ok(Object::String("off".to_string())),
        "unity_size" => Ok(Object::Int(4)),
        "layout" => Ok(Object::String("mirror".to_string())),
        "wrap_mode" => Ok(Object::String("default".to_string())),
        "pkg_config_path" => Ok(Object::Array(Vec::new())),
        "cmake_prefix_path" => Ok(Object::Array(Vec::new())),
        "auto_features" => Ok(Object::Feature(FeatureState::Auto)),
        "backend" => Ok(Object::String("ninja".to_string())),
        "install_umask" => Ok(Object::String("0022".to_string())),
        "stdsplit" => Ok(Object::Bool(true)),
        "errorlogs" => Ok(Object::Bool(true)),
        _ => {
            // Check for feature options — default to auto
            Ok(Object::Feature(FeatureState::Auto))
        }
    }
}

fn builtin_configuration_data(_vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let mut data = ConfigData::new();
    if let Some(Object::Dict(entries)) = VM::get_positional_args(args).first() {
        for (k, v) in entries {
            data.values.insert(k.clone(), (v.clone(), None));
        }
    }
    Ok(Object::ConfigurationData(data))
}

fn builtin_is_variable(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let name = positional
        .first()
        .and_then(|v| {
            if let Object::String(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .ok_or("is_variable() requires variable name")?;
    Ok(Object::Bool(vm.variables.contains_key(&name)))
}

fn builtin_get_variable(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let name = positional
        .first()
        .and_then(|v| {
            if let Object::String(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .ok_or("get_variable() requires variable name")?;

    if let Some(val) = vm.variables.get(&name) {
        return Ok(val.clone());
    }
    if let Some(val) = vm.globals.get(&name) {
        return Ok(val.clone());
    }

    // Check for default value
    if let Some(default) = positional.get(1) {
        return Ok((*default).clone());
    }

    Err(format!("Variable '{}' not found", name))
}

fn builtin_set_variable(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let name = positional
        .first()
        .and_then(|v| {
            if let Object::String(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .ok_or("set_variable() requires variable name")?;
    let value = positional
        .get(1)
        .ok_or("set_variable() requires a value")?
        .clone();
    vm.variables.insert(name, value.clone());
    Ok(Object::None)
}

fn builtin_unset_variable(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let name = positional
        .first()
        .and_then(|v| {
            if let Object::String(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .ok_or("unset_variable() requires variable name")?;
    vm.variables.remove(&name);
    Ok(Object::None)
}

fn builtin_assert(_vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let condition = positional.first().ok_or("assert() requires condition")?;
    if !condition.is_truthy() {
        let msg = positional
            .get(1)
            .map(|v| v.to_display_string())
            .unwrap_or_else(|| "Assertion failed".to_string());
        return Err(msg);
    }
    Ok(Object::None)
}

fn builtin_range(_vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let (start, stop, step) = match positional.len() {
        1 => {
            let stop = match positional[0] {
                Object::Int(n) => *n,
                _ => return Err("range() requires integer arguments".to_string()),
            };
            (0, stop, 1)
        }
        2 => {
            let start = match positional[0] {
                Object::Int(n) => *n,
                _ => return Err("range() requires integer arguments".to_string()),
            };
            let stop = match positional[1] {
                Object::Int(n) => *n,
                _ => return Err("range() requires integer arguments".to_string()),
            };
            (start, stop, 1)
        }
        3 => {
            let start = match positional[0] {
                Object::Int(n) => *n,
                _ => return Err("range() requires integer arguments".to_string()),
            };
            let stop = match positional[1] {
                Object::Int(n) => *n,
                _ => return Err("range() requires integer arguments".to_string()),
            };
            let step = match positional[2] {
                Object::Int(n) => *n,
                _ => return Err("range() requires integer arguments".to_string()),
            };
            if step == 0 {
                return Err("range() step cannot be zero".to_string());
            }
            (start, stop, step)
        }
        _ => return Err("range() requires 1-3 arguments".to_string()),
    };
    Ok(Object::Range(start, stop, step))
}

fn builtin_structured_sources(_vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let mut groups = Vec::new();

    for arg in &positional {
        if let Object::Dict(entries) = arg {
            for (k, v) in entries {
                let mut sources = Vec::new();
                collect_sources(v, &mut sources);
                groups.push((k.clone(), sources));
            }
        } else {
            let mut sources = Vec::new();
            collect_sources(arg, &mut sources);
            groups.push((String::new(), sources));
        }
    }

    Ok(Object::StructuredSources(groups))
}

fn builtin_add_project_arguments(_vm: &mut VM, _args: &[CallArg]) -> Result<Object, String> {
    // Store for later use by backend
    Ok(Object::None)
}

fn builtin_add_project_link_arguments(_vm: &mut VM, _args: &[CallArg]) -> Result<Object, String> {
    Ok(Object::None)
}

fn builtin_add_global_arguments(_vm: &mut VM, _args: &[CallArg]) -> Result<Object, String> {
    Ok(Object::None)
}

fn builtin_add_global_link_arguments(_vm: &mut VM, _args: &[CallArg]) -> Result<Object, String> {
    Ok(Object::None)
}

fn builtin_add_languages(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let required = VM::get_arg_bool(args, "required", true);
    let _native = VM::get_arg_bool(args, "native", false);

    for arg in &positional {
        if let Object::String(lang) = arg {
            let found = crate::compilers::detect_compiler(vm, lang);
            if !found && required {
                return Err(format!("Language '{}' not found", lang));
            }
        }
    }
    Ok(Object::Bool(true))
}

fn builtin_add_test_setup(_vm: &mut VM, _args: &[CallArg]) -> Result<Object, String> {
    Ok(Object::None)
}

fn builtin_disabler(_vm: &mut VM, _args: &[CallArg]) -> Result<Object, String> {
    Ok(Object::Disabler)
}

fn builtin_install_tag(_vm: &mut VM, _args: &[CallArg]) -> Result<Object, String> {
    Ok(Object::None)
}
