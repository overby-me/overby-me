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
        "is_disabler",
        "build_target",
        "subdir_done",
        "add_project_dependencies",
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
    vm.builtins
        .insert("is_disabler".to_string(), builtin_is_disabler);
    vm.builtins
        .insert("build_target".to_string(), builtin_build_target);
    vm.builtins
        .insert("subdir_done".to_string(), builtin_subdir_done);
    vm.builtins.insert(
        "add_project_dependencies".to_string(),
        builtin_add_project_dependencies,
    );
}

/// If `cmd` is a script with a shebang (e.g. `#!/usr/bin/env python3`),
/// resolve the interpreter explicitly so the script runs even in sandboxed
/// environments where /usr/bin/env may be missing or where the script lacks
/// the executable bit.
pub fn resolve_script_interp(cmd: String, mut args: Vec<String>) -> (String, Vec<String>) {
    use std::os::unix::fs::PermissionsExt;
    // Only consider regular files we can read
    let meta = match std::fs::metadata(&cmd) {
        Ok(m) => m,
        Err(_) => return (cmd, args),
    };
    let executable = meta.permissions().mode() & 0o111 != 0;
    let content = match std::fs::read_to_string(&cmd) {
        Ok(c) => c,
        Err(_) => return (cmd, args),
    };
    let first = match content.lines().next() {
        Some(l) => l,
        None => return (cmd, args),
    };
    let rest = match first.strip_prefix("#!") {
        Some(r) => r.trim(),
        None => return (cmd, args),
    };
    let mut parts: Vec<String> = rest.split_whitespace().map(|s| s.to_string()).collect();
    if parts.is_empty() {
        return (cmd, args);
    }
    let interp = parts.remove(0);
    let uses_env = interp.ends_with("/env");
    // If the script is executable AND the interpreter exists, kernel can handle it.
    let interp_exists = std::path::Path::new(&interp).exists();
    if executable && interp_exists && !uses_env {
        return (cmd, args);
    }
    if executable && !uses_env && interp_exists {
        return (cmd, args);
    }
    // Resolve interpreter
    let real_interp = if uses_env {
        if parts.is_empty() {
            return (cmd, args);
        }
        let prog = parts.remove(0);
        // Search PATH
        std::env::var("PATH")
            .ok()
            .and_then(|p| {
                p.split(':').find_map(|d| {
                    let cand = format!("{}/{}", d, prog);
                    if std::path::Path::new(&cand).exists() {
                        Some(cand)
                    } else {
                        None
                    }
                })
            })
            .unwrap_or(prog)
    } else if interp_exists {
        interp
    } else {
        // Fallback: try basename on PATH
        let base = interp.rsplit('/').next().unwrap_or(&interp).to_string();
        std::env::var("PATH")
            .ok()
            .and_then(|p| {
                p.split(':').find_map(|d| {
                    let cand = format!("{}/{}", d, base);
                    if std::path::Path::new(&cand).exists() {
                        Some(cand)
                    } else {
                        None
                    }
                })
            })
            .unwrap_or(interp)
    };
    let mut new_args: Vec<String> = parts;
    new_args.push(cmd);
    new_args.append(&mut args);
    (real_interp, new_args)
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

    let version = {
        // First check for string value
        if let Some(s) = VM::get_arg_str(args, "version", usize::MAX) {
            s.to_string()
        } else if let Some(val) = VM::get_arg_value(args, "version") {
            // Handle files('VERSION') — version passed as a File array
            match val {
                Object::Array(arr) => {
                    if let Some(Object::File(f)) = arr.first() {
                        let file_path = if f.subdir.is_empty() {
                            format!("{}/{}", vm.source_root, f.path)
                        } else {
                            format!("{}/{}/{}", vm.source_root, f.subdir, f.path)
                        };
                        match std::fs::read_to_string(&file_path) {
                            Ok(s) => s.trim().to_string(),
                            Err(e) => {
                                return Err(format!(
                                    "Cannot read version file '{}': {}",
                                    file_path, e
                                ));
                            }
                        }
                    } else {
                        "undefined".to_string()
                    }
                }
                Object::File(f) => {
                    let file_path = if f.subdir.is_empty() {
                        format!("{}/{}", vm.source_root, f.path)
                    } else {
                        format!("{}/{}/{}", vm.source_root, f.subdir, f.path)
                    };
                    match std::fs::read_to_string(&file_path) {
                        Ok(s) => s.trim().to_string(),
                        Err(e) => {
                            return Err(format!("Cannot read version file '{}': {}", file_path, e));
                        }
                    }
                }
                _ => "undefined".to_string(),
            }
        } else {
            "undefined".to_string()
        }
    };
    let meson_version = VM::get_arg_str(args, "meson_version", usize::MAX)
        .unwrap_or("")
        .to_string();
    // Handle license as either a single string or an array of strings
    let license = {
        let mut result = VM::get_arg_string_array(args, "license");
        if result.is_empty() {
            // Check if license was passed as a single string instead of an array
            if let Some(s) = VM::get_arg_str(args, "license", usize::MAX) {
                result = vec![s.to_string()];
            }
        }
        result
    };
    let license_files = VM::get_arg_string_array(args, "license_files");
    let subproject_dir = VM::get_arg_str(args, "subproject_dir", usize::MAX)
        .unwrap_or("subprojects")
        .to_string();
    let default_options = {
        let mut result = VM::get_arg_string_array(args, "default_options");
        // Also support dict form: default_options: {'key': value, ...}
        if result.is_empty() {
            if let Some(Object::Dict(entries)) = VM::get_arg_value(args, "default_options") {
                for (k, v) in entries {
                    let val_str = match v {
                        Object::Bool(b) => b.to_string(),
                        Object::Int(n) => n.to_string(),
                        Object::String(s) => s.clone(),
                        Object::Feature(FeatureState::Enabled) => "enabled".to_string(),
                        Object::Feature(FeatureState::Disabled) => "disabled".to_string(),
                        Object::Feature(FeatureState::Auto) => "auto".to_string(),
                        other => other.to_display_string(),
                    };
                    result.push(format!("{}={}", k, val_str));
                }
            }
        }
        result
    };

    // Set top-level subproject dir if this is the main project
    if !vm.is_subproject {
        vm.top_subproject_dir = subproject_dir.clone();
    }

    vm.project = Some(ProjectInfo {
        name,
        version,
        license,
        license_files,
        meson_version,
        languages: languages.clone(),
        subproject_dir,
        default_options: default_options.clone(),
    });

    // Detect compilers for requested languages
    for lang in &languages {
        crate::compilers::detect_compiler(vm, lang);
    }

    // Load project options from meson_options.txt or meson.options FIRST.
    // These use or_insert internally, so they set defaults without overriding CLI values.
    let opts_file = format!("{}/meson.options", vm.source_root);
    let opts_file2 = format!("{}/meson_options.txt", vm.source_root);
    let opts_source = if Path::new(&opts_file).exists() {
        std::fs::read_to_string(&opts_file).ok()
    } else if Path::new(&opts_file2).exists() {
        std::fs::read_to_string(&opts_file2).ok()
    } else {
        None
    };
    if let Some(ref src) = opts_source {
        crate::options::parse_options_file(&src, &mut vm.options);
    }

    // Apply default_options AFTER loading option files.
    // Priority: CLI > caller subproject() default_options > project() default_options > option file defaults.
    // - For keys in caller_option_keys (set by caller subproject() kwargs): use or_insert to preserve.
    // - For all other keys: use insert to override option file defaults and inherited parent values.
    // Known string-typed built-in options that should not be parsed as integers
    let string_typed_builtins = [
        "buildtype",
        "optimization",
        "warning_level",
        "default_library",
        "b_sanitize",
        "b_pgo",
        "b_ndebug",
        "unity",
        "layout",
        "wrap_mode",
        "backend",
        "install_umask",
        "prefix",
        "bindir",
        "libdir",
        "libexecdir",
        "includedir",
        "datadir",
        "mandir",
        "infodir",
        "localedir",
        "sysconfdir",
        "localstatedir",
        "sharedstatedir",
        "sbindir",
        "cpp_std",
        "c_std",
        "cpp_eh",
    ];
    for opt_str in &default_options {
        if let Some(eq_pos) = opt_str.find('=') {
            let key = opt_str[..eq_pos].trim().to_string();
            let val = opt_str[eq_pos + 1..].trim();
            // Handle ':option' syntax (this project's own option)
            let key = if key.starts_with(':') {
                key[1..].to_string()
            } else if key.contains(':') {
                // Skip subproject options (key contains ':' but not at start)
                continue;
            } else {
                key
            };
            let obj = if string_typed_builtins.contains(&key.as_str()) {
                crate::objects::Object::String(val.to_string())
            } else {
                crate::options::parse_option_value(val)
            };
            if vm.caller_option_keys.contains(&key) {
                // Caller subproject() default_options take priority; don't override
                vm.options.entry(key).or_insert(obj);
            } else {
                // Override option file defaults and inherited parent values
                vm.options.insert(key, obj);
            }
        }
    }

    // Apply type coercion and deprecated option value remapping
    if let Some(ref src) = opts_source {
        let defs = crate::options::parse_options_file_defs(src);

        // Pass 0: Coerce option values based on declared types from the options file
        for def in &defs {
            if let Some(val) = vm.options.get(&def.name).cloned() {
                let coerced = match def.opt_type.as_str() {
                    "array" => match &val {
                        Object::Array(_) => val,
                        Object::String(s) => {
                            if s.is_empty() {
                                Object::Array(Vec::new())
                            } else {
                                Object::Array(
                                    s.split(',')
                                        .map(|v| Object::String(v.trim().to_string()))
                                        .collect(),
                                )
                            }
                        }
                        _ => val,
                    },
                    "feature" => match &val {
                        Object::Feature(_) => val,
                        Object::String(s) => match s.as_str() {
                            "enabled" => Object::Feature(FeatureState::Enabled),
                            "disabled" => Object::Feature(FeatureState::Disabled),
                            "auto" => Object::Feature(FeatureState::Auto),
                            _ => val,
                        },
                        Object::Bool(b) => {
                            if *b {
                                Object::Feature(FeatureState::Enabled)
                            } else {
                                Object::Feature(FeatureState::Disabled)
                            }
                        }
                        _ => val,
                    },
                    "boolean" => match &val {
                        Object::Bool(_) => val,
                        Object::String(s) => match s.as_str() {
                            "true" => Object::Bool(true),
                            "false" => Object::Bool(false),
                            _ => val,
                        },
                        _ => val,
                    },
                    _ => val,
                };
                vm.options.insert(def.name.clone(), coerced);
            }
        }

        // Pass 1: Handle Renamed options (forward values to new option names)
        for def in &defs {
            if let Some(crate::options::DeprecatedInfo::Renamed(new_name)) = &def.deprecated {
                if let Some(val) = vm.options.get(&def.name).cloned() {
                    vm.options.insert(new_name.clone(), val);
                }
            }
        }

        // Pass 2: Handle ValueMap options (remap values)
        for def in &defs {
            if let Some(crate::options::DeprecatedInfo::ValueMap(map)) = &def.deprecated {
                if let Some(val) = vm.options.get(&def.name).cloned() {
                    match &val {
                        Object::Array(arr) => {
                            // For arrays, remap individual elements
                            let new_arr: Vec<Object> = arr
                                .iter()
                                .map(|item| {
                                    if let Object::String(s) = item {
                                        if let Some(replacement) = map.get(s) {
                                            Object::String(replacement.clone())
                                        } else {
                                            item.clone()
                                        }
                                    } else {
                                        item.clone()
                                    }
                                })
                                .collect();
                            vm.options.insert(def.name.clone(), Object::Array(new_arr));
                        }
                        _ => {
                            let val_str = match &val {
                                Object::String(s) => s.clone(),
                                Object::Bool(b) => b.to_string(),
                                Object::Int(n) => n.to_string(),
                                Object::Feature(FeatureState::Enabled) => "enabled".to_string(),
                                Object::Feature(FeatureState::Disabled) => "disabled".to_string(),
                                Object::Feature(FeatureState::Auto) => "auto".to_string(),
                                other => other.to_display_string(),
                            };
                            if let Some(replacement) = map.get(&val_str) {
                                let new_val = crate::options::parse_option_value(replacement);
                                vm.options.insert(def.name.clone(), new_val);
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(Object::None)
}

fn builtin_message(_vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    for arg in &positional {
        if !arg.is_printable_type() {
            return Err(format!("message(): {}", Object::NON_PRINTABLE_ERROR));
        }
    }
    let parts: Vec<String> = positional.iter().map(|v| v.to_display_string()).collect();
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
    // Disabler propagation: if any argument is a Disabler, return Disabler
    for arg in args {
        if matches!(arg.value, Object::Disabler) {
            return Ok(Object::Disabler);
        }
        if let Object::Array(ref items) = arg.value {
            for item in items {
                if matches!(item, Object::Disabler) {
                    return Ok(Object::Disabler);
                }
            }
        }
    }

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
    match include_dirs_val {
        Some(Object::Array(arr)) => {
            for item in arr {
                match item {
                    Object::IncludeDirs(d) => include_dirs.extend(d.dirs.clone()),
                    Object::String(s) => include_dirs.push(s.clone()),
                    _ => {}
                }
            }
        }
        Some(Object::IncludeDirs(d)) => {
            include_dirs.extend(d.dirs.clone());
        }
        Some(Object::String(s)) => {
            include_dirs.push(s.clone());
        }
        _ => {}
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

    // Check for disabler in positional args
    if positional.iter().any(|a| matches!(a, Object::Disabler)) {
        return Ok(Object::Disabler);
    }

    // Parse all positional string args as dependency names to try
    let mut names: Vec<String> = Vec::new();
    for arg in &positional {
        if let Object::String(s) = arg {
            if !s.is_empty() {
                names.push(s.clone());
            }
        }
    }
    let name = names.first().cloned().unwrap_or_default();

    let required_obj = VM::get_arg_value(args, "required");
    let required = match required_obj {
        Some(Object::Bool(b)) => *b,
        Some(Object::Feature(FeatureState::Disabled)) => false,
        Some(Object::Feature(FeatureState::Enabled)) => true,
        Some(Object::Feature(FeatureState::Auto)) => false, // auto = don't hard-error
        Some(Object::Disabler) => return Ok(Object::Disabler),
        _ => true,
    };

    let is_feature_disabled = matches!(required_obj, Some(Object::Feature(FeatureState::Disabled)));

    // Parse allow_fallback kwarg
    let allow_fallback = match VM::get_arg_value(args, "allow_fallback") {
        Some(Object::Bool(b)) => Some(*b),
        _ => None,
    };

    // Threads is a virtual dependency - but respect required: disabled
    if name == "threads" && !is_feature_disabled {
        return Ok(Object::Dependency(DependencyData {
            name: "threads".to_string(),
            found: true,
            version: String::new(),
            compile_args: Vec::new(),
            link_args: vec!["-lpthread".to_string()],
            sources: Vec::new(),
            include_dirs: Vec::new(),
            dependencies: Vec::new(),
            variables: std::collections::HashMap::new(),
            is_internal: false,
            kind: String::new(),
        }));
    }

    // OpenMP is a virtual dependency provided by the compiler
    if name == "openmp" && !is_feature_disabled {
        return Ok(Object::Dependency(DependencyData {
            name: "openmp".to_string(),
            found: true,
            version: String::new(),
            compile_args: vec!["-fopenmp".to_string()],
            link_args: vec!["-fopenmp".to_string()],
            sources: Vec::new(),
            include_dirs: Vec::new(),
            dependencies: Vec::new(),
            variables: std::collections::HashMap::new(),
            is_internal: false,
            kind: String::new(),
        }));
    }

    // Build cache key from name + method to avoid cross-method cache hits
    let method = VM::get_arg_str(args, "method", usize::MAX).unwrap_or("auto");
    let cache_key = if method != "auto" {
        format!("{}:{}", name, method)
    } else {
        name.clone()
    };

    // Parse fallback kwarg
    let fallback_info = VM::get_arg_value(args, "fallback").and_then(|fallback| match fallback {
        Object::Array(arr) => {
            let sp = arr.first().map(|o| o.to_string_value()).unwrap_or_default();
            let var = arr.get(1).map(|o| o.to_string_value());
            if sp.is_empty() { None } else { Some((sp, var)) }
        }
        Object::String(s) if !s.is_empty() => Some((s.clone(), None)),
        _ => None,
    });

    let has_fallback = fallback_info.is_some() || allow_fallback == Some(true);

    // Parse version requirement for cache check
    let version_req = VM::get_arg_str(args, "version", usize::MAX).unwrap_or("");

    // Check if already found in cache - check all names
    // Skip not-found cached deps when we have a fallback or allow_fallback available
    for n in &names {
        let ck = if method != "auto" {
            format!("{}:{}", n, method)
        } else {
            n.clone()
        };
        if let Some(dep) = vm.build_data.dependencies.get(&ck) {
            match dep {
                Object::Dependency(d) if d.found => {
                    // If a version requirement is specified, check it
                    if !version_req.is_empty() && !d.version.is_empty() {
                        if !crate::options::version_compare(&d.version, version_req) {
                            // Version doesn't match - return not-found
                            if !required {
                                return Ok(Object::Dependency(DependencyData::not_found(&name)));
                            }
                            // With required=true, continue to try other methods
                            continue;
                        }
                    }
                    return Ok(dep.clone());
                }
                Object::Dependency(_) if !has_fallback && allow_fallback != Some(true) => {
                    return Ok(dep.clone());
                }
                _ => {} // Skip not-found when fallback is available
            }
        }
    }

    // If disabled via feature, return not-found without trying
    if is_feature_disabled {
        let obj = Object::Dependency(DependencyData::not_found(&name));
        vm.build_data.dependencies.insert(cache_key, obj.clone());
        return Ok(obj);
    }

    // Check if fallback subproject is already loaded
    let fallback_sp_already_loaded = fallback_info.as_ref().map_or(false, |(sp_name, _)| {
        vm.build_data.subprojects.contains_key(sp_name)
    });

    let mut dep: Option<Object> = None;

    // If fallback subproject is already loaded, use it directly (skip system search)
    // If not, try system search first
    if fallback_sp_already_loaded {
        // Subproject already loaded - check if dep was overridden first
        for n in &names {
            if let Some(cached) = vm.build_data.dependencies.get(n) {
                if matches!(cached, Object::Dependency(d) if d.found) {
                    dep = Some(cached.clone());
                    break;
                }
            }
        }
        // If not overridden, try its variable directly
        if dep.is_none() {
            let (sp_name, var_name) = fallback_info.as_ref().unwrap();
            if let Some(Object::Subproject(sp_data)) = vm.build_data.subprojects.get(sp_name) {
                if sp_data.found {
                    let dep_var_name = var_name.clone().unwrap_or_else(|| format!("{}_dep", name));
                    if let Some(dep_obj) = sp_data.variables.get(&dep_var_name) {
                        match dep_obj {
                            Object::Dependency(d) if d.found => {
                                dep = Some(dep_obj.clone());
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    } else {
        // Try system search - try each name in order
        for n in &names {
            dep = crate::dependencies::find_dependency(vm, n, args);
            let found = matches!(&dep, Some(Object::Dependency(d)) if d.found);
            if found {
                break;
            }
        }
    }

    // Check if the dep was found
    let dep_found = match &dep {
        Some(Object::Dependency(d)) => d.found,
        Some(_) => true,
        None => false,
    };

    // If not found and we have an explicit fallback that hasn't been loaded yet, try it
    if !dep_found && !fallback_sp_already_loaded {
        if let Some((sp_name, var_name)) = &fallback_info {
            // Build args for subproject call, forwarding default_options
            let mut sp_args = vec![CallArg {
                name: None,
                value: Object::String(sp_name.clone()),
            }];
            if let Some(default_opts) = VM::get_arg_value(args, "default_options") {
                sp_args.push(CallArg {
                    name: Some("default_options".to_string()),
                    value: default_opts.clone(),
                });
            }
            // required: false for subproject so we don't error on missing subprojects
            sp_args.push(CallArg {
                name: Some("required".to_string()),
                value: Object::Bool(false),
            });
            match builtin_subproject(vm, &sp_args) {
                Ok(Object::Subproject(ref sp_data)) if sp_data.found => {
                    // Check if any of our dep names got overridden
                    for n in &names {
                        if let Some(cached) = vm.build_data.dependencies.get(n) {
                            if matches!(cached, Object::Dependency(d) if d.found) {
                                dep = Some(cached.clone());
                                break;
                            }
                        }
                    }
                    // If still not found, try the variable
                    if !matches!(&dep, Some(Object::Dependency(d)) if d.found) {
                        let dep_var_name =
                            var_name.clone().unwrap_or_else(|| format!("{}_dep", name));
                        if let Some(dep_obj) = sp_data.variables.get(&dep_var_name) {
                            match dep_obj {
                                Object::Dependency(d) if d.found => {
                                    dep = Some(dep_obj.clone());
                                }
                                _ => {}
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Auto-fallback: try implicit subproject fallback if no dep found yet
    let dep_found_after_explicit = match &dep {
        Some(Object::Dependency(d)) => d.found,
        Some(_) => true,
        None => false,
    };

    if !dep_found_after_explicit && !is_feature_disabled && allow_fallback != Some(false) {
        // Helper: build subproject args with default_options and static forwarding
        let build_sp_args = |sp_name: &str| -> Vec<CallArg> {
            let mut sp_args = vec![
                CallArg {
                    name: None,
                    value: Object::String(sp_name.to_string()),
                },
                CallArg {
                    name: Some("required".to_string()),
                    value: Object::Bool(false),
                },
            ];
            // Forward default_options
            let mut default_opts_strs = VM::get_arg_string_array(args, "default_options");
            // Handle single string form
            if default_opts_strs.is_empty() {
                if let Some(Object::String(s)) = VM::get_arg_value(args, "default_options") {
                    default_opts_strs.push(s.clone());
                }
            }
            // If static: true, add default_library=static
            if VM::get_arg_bool(args, "static", false) {
                let has_default_library = default_opts_strs
                    .iter()
                    .any(|s| s.starts_with("default_library"));
                if !has_default_library {
                    default_opts_strs.push("default_library=static".to_string());
                }
            }
            if !default_opts_strs.is_empty() {
                sp_args.push(CallArg {
                    name: Some("default_options".to_string()),
                    value: Object::Array(
                        default_opts_strs.into_iter().map(Object::String).collect(),
                    ),
                });
            } else if let Some(Object::Dict(d)) = VM::get_arg_value(args, "default_options") {
                sp_args.push(CallArg {
                    name: Some("default_options".to_string()),
                    value: Object::Dict(d.clone()),
                });
            }
            sp_args
        };

        // Step 1: Try implicit fallback by subproject directory name matching a dep name
        'implicit_dir: for n in &names {
            let sp_dir = format!("{}/{}/{}", vm.top_source_root, vm.top_subproject_dir, n);
            let sp_wrap = format!(
                "{}/{}/{}.wrap",
                vm.top_source_root, vm.top_subproject_dir, n
            );
            if Path::new(&format!("{}/meson.build", sp_dir)).exists()
                || Path::new(&sp_wrap).exists()
            {
                // Check if already loaded
                if let Some(Object::Subproject(sp_data)) = vm.build_data.subprojects.get(n) {
                    if sp_data.found {
                        // Check if any dep name was overridden
                        for check_name in &names {
                            if let Some(cached) = vm.build_data.dependencies.get(check_name) {
                                if matches!(cached, Object::Dependency(d) if d.found) {
                                    dep = Some(cached.clone());
                                    break 'implicit_dir;
                                }
                            }
                        }
                        // Try default variable name
                        let var_name = format!("{}_dep", n);
                        if let Some(dep_obj) = sp_data.variables.get(&var_name) {
                            if matches!(dep_obj, Object::Dependency(d) if d.found) {
                                dep = Some(dep_obj.clone());
                                break 'implicit_dir;
                            }
                        }
                    }
                } else {
                    let sp_args = build_sp_args(n);
                    if let Ok(Object::Subproject(sp_data)) = builtin_subproject(vm, &sp_args) {
                        if sp_data.found {
                            // Check if any dep name was overridden
                            for check_name in &names {
                                if let Some(cached) = vm.build_data.dependencies.get(check_name) {
                                    if matches!(cached, Object::Dependency(d) if d.found) {
                                        dep = Some(cached.clone());
                                        break 'implicit_dir;
                                    }
                                }
                            }
                            // Try default variable name
                            let var_name = format!("{}_dep", n);
                            if let Some(dep_obj) = sp_data.variables.get(&var_name) {
                                if matches!(dep_obj, Object::Dependency(d) if d.found) {
                                    dep = Some(dep_obj.clone());
                                    break 'implicit_dir;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Step 2: Scan wrap files for [provide] sections
        if !matches!(&dep, Some(Object::Dependency(d)) if d.found) {
            let sp_dir_path = format!("{}/{}", vm.top_source_root, vm.top_subproject_dir);

            // Collect wrap files to scan (top-level and nested from loaded subprojects)
            let mut wrap_files: Vec<std::path::PathBuf> = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&sp_dir_path) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("wrap") {
                        wrap_files.push(path);
                    } else if path.is_dir() {
                        // Also scan nested subprojects for wraps (e.g. sub/subprojects/*.wrap)
                        let nested_sp_dir = format!("{}/subprojects", path.display());
                        if let Ok(nested_entries) = std::fs::read_dir(&nested_sp_dir) {
                            for ne in nested_entries.flatten() {
                                let np = ne.path();
                                if np.extension().and_then(|e| e.to_str()) == Some("wrap") {
                                    wrap_files.push(np);
                                }
                            }
                        }
                    }
                }
            }

            'wrap_scan: for wrap_path in &wrap_files {
                if let Ok(content) = std::fs::read_to_string(wrap_path) {
                    let mut in_provide = false;
                    let mut provides_dep = false;
                    let mut dep_var_name: Option<String> = None;
                    for line in content.lines() {
                        let trimmed = line.trim();
                        if trimmed == "[provide]" {
                            in_provide = true;
                        } else if trimmed.starts_with('[') {
                            in_provide = false;
                        } else if in_provide {
                            if let Some(eq) = trimmed.find('=') {
                                let key = trimmed[..eq].trim();
                                let val = trimmed[eq + 1..].trim();
                                if key == "dependency_names" {
                                    let deps: Vec<&str> =
                                        val.split(',').map(|s| s.trim()).collect();
                                    for n in &names {
                                        if deps.contains(&n.as_str()) {
                                            provides_dep = true;
                                            break;
                                        }
                                    }
                                } else {
                                    for n in &names {
                                        if key == n.as_str() {
                                            provides_dep = true;
                                            dep_var_name = Some(val.to_string());
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if provides_dep {
                        let sp_name = wrap_path.file_stem().unwrap().to_string_lossy().to_string();

                        // Check if subproject is already loaded
                        if let Some(Object::Subproject(sp_data)) =
                            vm.build_data.subprojects.get(&sp_name)
                        {
                            if sp_data.found {
                                // Check if any dep name was overridden
                                for n in &names {
                                    if let Some(cached) = vm.build_data.dependencies.get(n) {
                                        if matches!(cached, Object::Dependency(d) if d.found) {
                                            dep = Some(cached.clone());
                                            break 'wrap_scan;
                                        }
                                    }
                                }
                                // Try variable from wrap or default
                                let var_name = dep_var_name
                                    .clone()
                                    .unwrap_or_else(|| format!("{}_dep", name));
                                if let Some(dep_obj) = sp_data.variables.get(&var_name) {
                                    if matches!(dep_obj, Object::Dependency(d) if d.found) {
                                        dep = Some(dep_obj.clone());
                                        break 'wrap_scan;
                                    }
                                }
                            }
                        } else {
                            // Load subproject
                            let sp_args = build_sp_args(&sp_name);
                            if let Ok(Object::Subproject(sp_data)) =
                                builtin_subproject(vm, &sp_args)
                            {
                                if sp_data.found {
                                    // Check if any dep name was overridden
                                    for n in &names {
                                        if let Some(cached) = vm.build_data.dependencies.get(n) {
                                            if matches!(cached, Object::Dependency(d) if d.found) {
                                                dep = Some(cached.clone());
                                                break 'wrap_scan;
                                            }
                                        }
                                    }
                                    // Try variable
                                    let var_name = dep_var_name
                                        .clone()
                                        .unwrap_or_else(|| format!("{}_dep", name));
                                    if let Some(dep_obj) = sp_data.variables.get(&var_name) {
                                        if matches!(dep_obj, Object::Dependency(d) if d.found) {
                                            dep = Some(dep_obj.clone());
                                            break 'wrap_scan;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Step 3: Try nested subproject directories (sub-subprojects)
        // Also handles deep nesting via builtin_subproject's recursive search
        if !matches!(&dep, Some(Object::Dependency(d)) if d.found) {
            'nested_scan: for n in &names {
                // Just try loading the subproject - builtin_subproject will search
                // nested locations recursively if needed
                let already_loaded = vm.build_data.subprojects.contains_key(n);
                if !already_loaded {
                    let sp_args = build_sp_args(n);
                    if let Ok(Object::Subproject(sp_data)) = builtin_subproject(vm, &sp_args) {
                        if sp_data.found {
                            // Check if any dep name was overridden
                            for check_name in &names {
                                if let Some(cached) = vm.build_data.dependencies.get(check_name) {
                                    if matches!(cached, Object::Dependency(d) if d.found) {
                                        dep = Some(cached.clone());
                                        break 'nested_scan;
                                    }
                                }
                            }
                            // Try default variable name
                            let var_name = format!("{}_dep", n);
                            if let Some(dep_obj) = sp_data.variables.get(&var_name) {
                                if matches!(dep_obj, Object::Dependency(d) if d.found) {
                                    dep = Some(dep_obj.clone());
                                    break 'nested_scan;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Final check
    let dep_found_after = match &dep {
        Some(Object::Dependency(d)) => d.found,
        Some(_) => true,
        None => false,
    };

    if !dep_found_after && required {
        return Err(format!("Dependency '{}' not found", name));
    }

    let obj = dep.unwrap_or_else(|| Object::Dependency(DependencyData::not_found(&name)));

    // Only cache found deps; not-found from system search should not be cached
    // so that future calls with fallback can still try.
    // Override deps are cached directly by meson.override_dependency().
    let is_found = matches!(&obj, Object::Dependency(d) if d.found);
    if is_found {
        vm.build_data.dependencies.insert(cache_key, obj.clone());
        // For multi-name deps, cache under all names so later lookups work
        for n in &names {
            if *n != name {
                let ck = if method != "auto" {
                    format!("{}:{}", n, method)
                } else {
                    n.clone()
                };
                vm.build_data
                    .dependencies
                    .entry(ck)
                    .or_insert_with(|| obj.clone());
            }
        }
    }

    let use_disabler = VM::get_arg_bool(args, "disabler", false);
    if use_disabler && !matches!(&obj, Object::Dependency(d) if d.found) {
        return Ok(Object::Disabler);
    }

    Ok(obj)
}

fn builtin_declare_dependency(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let compile_args = VM::get_arg_string_array(args, "compile_args");
    let link_args = VM::get_arg_string_array(args, "link_args");
    let include_dirs_val = VM::get_arg_value(args, "include_directories");
    let mut include_dirs = Vec::new();
    match include_dirs_val {
        Some(Object::Array(arr)) => {
            for item in arr {
                match item {
                    Object::IncludeDirs(d) => include_dirs.extend(d.dirs.clone()),
                    Object::String(s) => include_dirs.push(s.clone()),
                    _ => {}
                }
            }
        }
        Some(Object::IncludeDirs(d)) => {
            include_dirs.extend(d.dirs.clone());
        }
        Some(Object::String(s)) => {
            include_dirs.push(s.clone());
        }
        _ => {}
    }

    let version = VM::get_arg_str(args, "version", usize::MAX)
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            // Default to project version when no version kwarg is given
            vm.project
                .as_ref()
                .map(|p| p.version.clone())
                .unwrap_or_default()
        });

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
    match VM::get_arg_value(args, "variables") {
        Some(Object::Dict(entries)) => {
            for (k, v) in entries {
                variables.insert(k.clone(), v.to_string_value());
            }
        }
        Some(Object::Array(arr)) => {
            for item in arr {
                if let Object::String(s) = item {
                    if let Some(eq_pos) = s.find('=') {
                        let key = s[..eq_pos].to_string();
                        let val = s[eq_pos + 1..].to_string();
                        variables.insert(key, val);
                    }
                }
            }
        }
        _ => {}
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
        kind: String::new(),
    }))
}

/// Helper to check program version by running it with --version
fn run_program_version(path: &str, version_arg: &str) -> Option<std::process::Output> {
    // Try running directly first
    let output = Command::new(path).arg(version_arg).output();
    match output {
        Ok(o) if o.status.success() => Some(o),
        _ if path.ends_with(".py") => {
            // If direct execution fails and it's a .py script, try with python3
            Command::new("python3")
                .arg(path)
                .arg(version_arg)
                .output()
                .ok()
                .filter(|o| o.status.success())
        }
        _ => None,
    }
}

fn extract_version_from_output(output: &std::process::Output) -> Option<String> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        for word in line.split_whitespace() {
            let clean = word.trim_matches(|c: char| !c.is_ascii_digit() && c != '.');
            if !clean.is_empty() && clean.contains('.') {
                return Some(clean.to_string());
            }
        }
    }
    None
}

fn check_program_version(path: &str, version_req: &str, version_arg: &str) -> Option<String> {
    if version_req.is_empty() {
        return Some(String::new());
    }

    let output = run_program_version(path, version_arg)?;
    let detected = extract_version_from_output(&output);

    match detected {
        Some(ver) if crate::options::version_compare(&ver, version_req) => Some(ver),
        _ => None,
    }
}

fn builtin_find_program(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let required_obj = VM::get_arg_value(args, "required");
    let is_feature_disabled = matches!(required_obj, Some(Object::Feature(FeatureState::Disabled)));
    let required = match required_obj {
        Some(Object::Bool(b)) => *b,
        Some(Object::Feature(FeatureState::Disabled)) => false,
        Some(Object::Feature(FeatureState::Enabled)) => true,
        _ => true,
    };

    // If disabled via feature, return not-found without searching
    if is_feature_disabled {
        let name = positional
            .first()
            .map(|v| v.to_string_value())
            .unwrap_or_else(|| "unknown".to_string());
        return Ok(Object::ExternalProgram(ExternalProgramData {
            name,
            path: String::new(),
            found: false,
            version: None,
        }));
    }

    let _native = VM::get_arg_bool(args, "native", false);
    let version_req = VM::get_arg_str(args, "version", usize::MAX).unwrap_or("");
    let version_arg = VM::get_arg_str(args, "version_argument", usize::MAX).unwrap_or("--version");
    let dirs = VM::get_arg_string_array(args, "dirs");

    // Flatten positional args into a list of names to try
    let mut names_to_try: Vec<String> = Vec::new();
    for arg in &positional {
        match *arg {
            Object::String(s) => names_to_try.push(s.clone()),
            Object::Array(arr) => {
                for item in arr {
                    match item {
                        Object::String(s) => names_to_try.push(s.clone()),
                        Object::File(f) => {
                            if f.is_built {
                                let full = if f.subdir.is_empty() {
                                    format!("{}/{}", vm.build_root, f.path)
                                } else {
                                    format!("{}/{}/{}", vm.build_root, f.subdir, f.path)
                                };
                                return Ok(Object::ExternalProgram(ExternalProgramData {
                                    name: f.path.clone(),
                                    path: full,
                                    found: true,
                                    version: None,
                                }));
                            }
                            names_to_try.push(f.path.clone());
                        }
                        _ => {}
                    }
                }
            }
            Object::File(f) => {
                if f.is_built {
                    let full = if f.subdir.is_empty() {
                        format!("{}/{}", vm.build_root, f.path)
                    } else {
                        format!("{}/{}/{}", vm.build_root, f.subdir, f.path)
                    };
                    return Ok(Object::ExternalProgram(ExternalProgramData {
                        name: f.path.clone(),
                        path: full,
                        found: true,
                        version: None,
                    }));
                }
                names_to_try.push(f.path.clone());
            }
            Object::ExternalProgram(p) => {
                if p.found {
                    return Ok(Object::ExternalProgram(p.clone()));
                }
                names_to_try.push(p.name.clone());
            }
            _ => names_to_try.push(arg.to_display_string()),
        }
    }

    for name in &names_to_try {
        // Check overrides first
        if let Some(prog) = vm.build_data.find_program_overrides.get(name) {
            return Ok(prog.clone());
        }

        // Collect candidate paths
        let mut candidates: Vec<String> = Vec::new();

        // Handle absolute paths directly
        if name.starts_with('/') {
            if std::path::Path::new(name).exists() {
                candidates.push(name.clone());
            }
        }

        // Search in specified dirs first
        for dir in &dirs {
            let path = format!("{}/{}", dir, name);
            if Path::new(&path).exists() {
                candidates.push(path);
            }
        }
        // Search in source tree
        let src_path = if vm.current_subdir.is_empty() {
            format!("{}/{}", vm.source_root, name)
        } else {
            format!("{}/{}/{}", vm.source_root, vm.current_subdir, name)
        };
        if Path::new(&src_path).exists() {
            candidates.push(src_path);
        }
        // Also search source root directly (not just current subdir)
        if !vm.current_subdir.is_empty() {
            let root_path = format!("{}/{}", vm.source_root, name);
            if Path::new(&root_path).exists() {
                candidates.push(root_path);
            }
        }
        // Search on PATH manually
        if let Ok(path_env) = std::env::var("PATH") {
            for dir in path_env.split(':') {
                let full_path = format!("{}/{}", dir, name);
                if Path::new(&full_path).exists() {
                    candidates.push(full_path);
                }
            }
        }

        // Try each candidate, checking version if required
        for candidate_path in candidates {
            if !version_req.is_empty() {
                if let Some(detected_ver) =
                    check_program_version(&candidate_path, version_req, version_arg)
                {
                    return Ok(Object::ExternalProgram(ExternalProgramData {
                        name: name.clone(),
                        path: candidate_path,
                        found: true,
                        version: Some(detected_ver),
                    }));
                }
                // Version didn't match, try next candidate
                continue;
            }
            // No version requirement - still try to detect version
            let detected_version = run_program_version(&candidate_path, version_arg)
                .and_then(|o| extract_version_from_output(&o));
            return Ok(Object::ExternalProgram(ExternalProgramData {
                name: name.clone(),
                path: candidate_path,
                found: true,
                version: detected_version,
            }));
        }
    }

    // Auto-load subproject from wrap files with [provide] program_names
    {
        let subproject_dir_path = format!("{}/{}", vm.top_source_root, vm.top_subproject_dir);
        if let Ok(entries) = std::fs::read_dir(&subproject_dir_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("wrap") {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        let mut in_provide = false;
                        for line in content.lines() {
                            let trimmed = line.trim();
                            if trimmed == "[provide]" {
                                in_provide = true;
                            } else if trimmed.starts_with('[') {
                                in_provide = false;
                            } else if in_provide {
                                if let Some(eq) = trimmed.find('=') {
                                    let key = trimmed[..eq].trim();
                                    let val = trimmed[eq + 1..].trim();
                                    if key == "program_names" {
                                        let programs: Vec<&str> =
                                            val.split(',').map(|s| s.trim()).collect();
                                        for try_name in &names_to_try {
                                            if programs.contains(&try_name.as_str()) {
                                                let sp_name = path
                                                    .file_stem()
                                                    .unwrap()
                                                    .to_string_lossy()
                                                    .to_string();
                                                let mut sp_args = vec![
                                                    CallArg {
                                                        name: None,
                                                        value: Object::String(sp_name),
                                                    },
                                                    CallArg {
                                                        name: Some("required".to_string()),
                                                        value: Object::Bool(false),
                                                    },
                                                ];
                                                // Forward default_options to subproject
                                                if let Some(default_opts) =
                                                    VM::get_arg_value(args, "default_options")
                                                {
                                                    sp_args.push(CallArg {
                                                        name: Some("default_options".to_string()),
                                                        value: default_opts.clone(),
                                                    });
                                                }
                                                let _ = builtin_subproject(vm, &sp_args);
                                                if let Some(prog) = vm
                                                    .build_data
                                                    .find_program_overrides
                                                    .get(try_name)
                                                {
                                                    return Ok(prog.clone());
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let name = names_to_try
        .first()
        .cloned()
        .unwrap_or_else(|| "unknown".to_string());

    if required {
        return Err(format!("Program '{}' not found", name));
    }

    // Handle disabler: true kwarg
    if VM::get_arg_bool(args, "disabler", false) {
        return Ok(Object::Disabler);
    }

    Ok(Object::ExternalProgram(ExternalProgramData {
        name,
        path: String::new(),
        found: false,
        version: None,
    }))
}

fn builtin_custom_target(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    // Disabler propagation: if any argument is a Disabler, return Disabler
    for arg in args {
        if matches!(arg.value, Object::Disabler) {
            return Ok(Object::Disabler);
        }
        if let Object::Array(ref items) = arg.value {
            for item in items {
                if matches!(item, Object::Disabler) {
                    return Ok(Object::Disabler);
                }
            }
        }
    }

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
    match VM::get_arg_value(args, "command") {
        Some(Object::Array(arr)) => {
            for item in arr {
                command.push(item.to_string_value());
            }
        }
        Some(Object::String(s)) => {
            command.push(s.clone());
        }
        Some(other) => {
            command.push(other.to_string_value());
        }
        None => {}
    }

    let mut input = Vec::new();
    match VM::get_arg_value(args, "input") {
        Some(Object::Array(arr)) => {
            for item in arr {
                input.push(item.to_string_value());
            }
        }
        Some(Object::String(s)) => {
            input.push(s.clone());
        }
        Some(other) => {
            input.push(other.to_string_value());
        }
        None => {}
    }

    let mut output = Vec::new();
    match VM::get_arg_value(args, "output") {
        Some(Object::Array(arr)) => {
            for item in arr {
                if let Object::String(s) = item {
                    output.push(s.clone());
                }
            }
        }
        Some(Object::String(s)) => {
            output.push(s.clone());
        }
        _ => {}
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
    // Resolve `input` to an absolute path. Accepts string, files() result, or
    // array thereof (using the first element).
    let input_resolved: Option<(String, bool)> = {
        let val = VM::get_arg_value(args, "input");
        let raw = match val {
            Some(Object::Array(arr)) => arr.first(),
            other => other,
        };
        match raw {
            Some(Object::String(s)) => Some((s.clone(), false)),
            Some(Object::File(f)) => {
                let p = if f.is_built {
                    if f.subdir.is_empty() {
                        format!("{}/{}", vm.build_root, f.path)
                    } else {
                        format!("{}/{}/{}", vm.build_root, f.subdir, f.path)
                    }
                } else if f.subdir.is_empty() {
                    format!("{}/{}", vm.source_root, f.path)
                } else {
                    format!("{}/{}/{}", vm.source_root, f.subdir, f.path)
                };
                Some((p, true))
            }
            _ => None,
        }
    };
    let input = input_resolved
        .as_ref()
        .map(|(p, abs)| if *abs { p.clone() } else { p.clone() });
    let input_is_absolute = input_resolved
        .as_ref()
        .map(|(_, abs)| *abs)
        .unwrap_or(false);
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
    let copy = VM::get_arg_bool(args, "copy", false);

    let mut command = Vec::new();
    if let Some(Object::Array(arr)) = VM::get_arg_value(args, "command") {
        for item in arr {
            match item {
                Object::ExternalProgram(p) => command.push(p.path.clone()),
                Object::File(f) => {
                    if f.is_built {
                        if f.subdir.is_empty() {
                            command.push(format!("{}/{}", vm.build_root, f.path));
                        } else {
                            command.push(format!("{}/{}/{}", vm.build_root, f.subdir, f.path));
                        }
                    } else {
                        if f.subdir.is_empty() {
                            command.push(format!("{}/{}", vm.source_root, f.path));
                        } else {
                            command.push(format!("{}/{}/{}", vm.source_root, f.subdir, f.path));
                        }
                    }
                }
                Object::BuildTarget(t) => {
                    command.push(format!(
                        "{}/{}",
                        vm.build_root,
                        t.outputs.first().map(|s| s.as_str()).unwrap_or(&t.name)
                    ));
                }
                Object::Array(inner) => {
                    for inner_item in inner {
                        match inner_item {
                            Object::File(f) => {
                                if f.is_built {
                                    if f.subdir.is_empty() {
                                        command.push(format!("{}/{}", vm.build_root, f.path));
                                    } else {
                                        command.push(format!(
                                            "{}/{}/{}",
                                            vm.build_root, f.subdir, f.path
                                        ));
                                    }
                                } else {
                                    if f.subdir.is_empty() {
                                        command.push(format!("{}/{}", vm.source_root, f.path));
                                    } else {
                                        command.push(format!(
                                            "{}/{}/{}",
                                            vm.source_root, f.subdir, f.path
                                        ));
                                    }
                                }
                            }
                            _ => command.push(inner_item.to_string_value()),
                        }
                    }
                }
                _ => command.push(item.to_string_value()),
            }
        }
    }

    // Actually generate the output file during setup for configuration/copy modes
    let build_subdir = if vm.current_subdir.is_empty() {
        vm.build_root.clone()
    } else {
        format!("{}/{}", vm.build_root, vm.current_subdir)
    };
    let _ = std::fs::create_dir_all(&build_subdir);
    let output_path = format!("{}/{}", build_subdir, output);

    if copy {
        // Copy mode: just copy the input file (preserving mtime)
        if let Some(ref inp) = input {
            let input_path = if input_is_absolute || std::path::Path::new(inp).is_absolute() {
                inp.clone()
            } else if vm.current_subdir.is_empty() {
                format!("{}/{}", vm.source_root, inp)
            } else {
                format!("{}/{}/{}", vm.source_root, vm.current_subdir, inp)
            };
            if std::fs::copy(&input_path, &output_path).is_ok() {
                if let Ok(meta) = std::fs::metadata(&input_path) {
                    if let Ok(mtime) = meta.modified() {
                        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&output_path) {
                            let _ = f.set_modified(mtime);
                        }
                    }
                }
            }
        }
    } else if configuration.is_some() && command.is_empty() {
        // Configuration mode: read input, substitute @VAR@ patterns, write output
        if let Some(ref inp) = input {
            let input_path = if input_is_absolute || std::path::Path::new(inp).is_absolute() {
                inp.clone()
            } else if vm.current_subdir.is_empty() {
                format!("{}/{}", vm.source_root, inp)
            } else {
                format!("{}/{}/{}", vm.source_root, vm.current_subdir, inp)
            };
            let mut content = std::fs::read_to_string(&input_path).unwrap_or_default();
            // Substitute @VAR@ patterns from configuration data
            if let Some(Object::ConfigurationData(ref cfg)) = configuration {
                let values = cfg.values.borrow();
                for (key, (val, _)) in values.iter() {
                    let pattern = format!("@{}@", key);
                    content = content.replace(&pattern, &val.to_string_value());
                }
            } else if let Some(Object::Dict(ref dict)) = configuration {
                for (key, val) in dict {
                    let pattern = format!("@{}@", key);
                    content = content.replace(&pattern, &val.to_string_value());
                }
            }
            let _ = std::fs::write(&output_path, &content);
        } else {
            // No input file, just create empty output
            let _ = std::fs::write(&output_path, "");
        }
    } else if !command.is_empty() {
        // Command mode: execute command with substitutions
        let mut resolved_command: Vec<String> = Vec::new();
        let input_path = input.as_ref().map(|inp| {
            if input_is_absolute || std::path::Path::new(inp).is_absolute() {
                inp.clone()
            } else if vm.current_subdir.is_empty() {
                format!("{}/{}", vm.source_root, inp)
            } else {
                format!("{}/{}/{}", vm.source_root, vm.current_subdir, inp)
            }
        });
        for arg in &command {
            let mut s = arg.clone();
            if let Some(ref ip) = input_path {
                s = s.replace("@INPUT@", ip);
            }
            s = s.replace("@OUTPUT@", &output_path);
            if let Some(ref inp) = input {
                let basename = inp.rsplit('/').next().unwrap_or(inp);
                s = s.replace("@BASENAME@", basename);
                let plainname = basename.rsplit('.').next_back().unwrap_or(basename);
                s = s.replace("@PLAINNAME@", plainname);
            }
            if let Some(ref dep) = depfile {
                s = s.replace("@DEPFILE@", &format!("{}/{}", build_subdir, dep));
            }
            s = s.replace("@SOURCE_ROOT@", &vm.source_root);
            s = s.replace("@BUILD_ROOT@", &vm.build_root);
            s = s.replace(
                "@CURRENT_SOURCE_DIR@",
                &if vm.current_subdir.is_empty() {
                    vm.source_root.clone()
                } else {
                    format!("{}/{}", vm.source_root, vm.current_subdir)
                },
            );
            resolved_command.push(s);
        }
        if !resolved_command.is_empty() {
            let (cmd, cmd_args) = crate::builtins::functions::resolve_script_interp(
                resolved_command[0].clone(),
                resolved_command[1..].to_vec(),
            );
            let result = std::process::Command::new(&cmd)
                .args(&cmd_args)
                .current_dir(&build_subdir)
                .env("MESON_BUILD_ROOT", &vm.build_root)
                .env("MESON_SOURCE_ROOT", &vm.source_root)
                .env("MESON_SUBDIR", &vm.current_subdir)
                .output();
            match result {
                Ok(output_result) => {
                    if capture {
                        let _ = std::fs::write(&output_path, &output_result.stdout);
                    }
                    if !output_result.status.success() {
                        if capture {
                            let stderr = String::from_utf8_lossy(&output_result.stderr);
                            return Err(format!(
                                "ERROR: Error running command:\n{}",
                                stderr.trim()
                            ));
                        }
                        // Non-capture mode: silently ignore execution failures
                        // The backend (ninja) will handle the actual execution
                    }
                }
                Err(e) => {
                    if capture {
                        return Err(format!("ERROR: Error running command:\n{}", e));
                    }
                    // Non-capture mode: silently ignore execution failures
                    // The backend (ninja) will handle the actual execution
                }
            }
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

    // Check if_found kwarg
    if let Some(if_found) = VM::get_arg_value(args, "if_found") {
        match &if_found {
            Object::Dependency(dep) if !dep.found => return Ok(Object::None),
            Object::Array(arr) => {
                for item in arr {
                    if let Object::Dependency(dep) = item {
                        if !dep.found {
                            return Ok(Object::None);
                        }
                    }
                }
            }
            _ => {}
        }
    }

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
    match vm.execute(&compiler.chunk) {
        Ok(_) => {}
        Err(e) if e == "SUBDIR_DONE" => {
            // subdir_done() was called, stop processing this subdir
        }
        Err(e) => return Err(format!("In {}: {}", subdir_build, e)),
    }

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
        Some(Object::Feature(FeatureState::Enabled)) => true,
        Some(Object::Feature(FeatureState::Auto)) => false,
        Some(Object::Feature(FeatureState::Disabled)) => {
            let sp = Object::Subproject(SubprojectData {
                name: name.clone(),
                version: String::new(),
                found: false,
                variables: HashMap::new(),
            });
            vm.build_data.subprojects.insert(name, sp.clone());
            return Ok(sp);
        }
        _ => true,
    };

    // Check if already loaded
    if let Some(sp) = vm.build_data.subprojects.get(&name) {
        return Ok(sp.clone());
    }

    // Always resolve subprojects relative to the top-level source root
    let subproject_dir = vm.top_subproject_dir.clone();
    let subproject_path = format!("{}/{}/{}", vm.top_source_root, subproject_dir, name);
    let build_file = format!("{}/meson.build", subproject_path);

    // Track the actual subproject path (may differ from default if found in nested location)
    let mut subproject_path = subproject_path;
    let mut build_file = build_file;

    if !Path::new(&build_file).exists() {
        // Try to download via wrap
        let wrap_file = format!("{}/{}/{}.wrap", vm.top_source_root, subproject_dir, name);
        if Path::new(&wrap_file).exists() {
            // First, see if the wrap declares a different directory that already
            // exists on disk (e.g. pre-extracted source) - avoids re-downloading.
            let wrap_dir_name = std::fs::read_to_string(&wrap_file).ok().and_then(|c| {
                c.lines()
                    .find(|l| l.trim().starts_with("directory"))
                    .and_then(|l| l.find('=').map(|i| l[i + 1..].trim().to_string()))
            });
            let mut already_present = false;
            if let Some(ref dir_name) = wrap_dir_name {
                let alt_path = format!("{}/{}/{}", vm.top_source_root, subproject_dir, dir_name);
                let alt_build = format!("{}/meson.build", alt_path);
                if Path::new(&alt_build).exists() {
                    subproject_path = alt_path;
                    build_file = alt_build;
                    already_present = true;
                }
            }
            if !already_present {
                crate::wrap::download_wrap(&wrap_file, &subproject_path)?;
                // Check if wrap file specifies a different directory name
                if !Path::new(&build_file).exists() {
                    if let Some(dir_name) = wrap_dir_name {
                        let alt_path =
                            format!("{}/{}/{}", vm.top_source_root, subproject_dir, dir_name);
                        let alt_build = format!("{}/meson.build", alt_path);
                        if Path::new(&alt_build).exists() {
                            subproject_path = alt_path;
                            build_file = alt_build;
                        }
                    }
                }
            }
        } else {
            // Try to find in nested subproject directories (sub-subprojects)
            // Recursively search subprojects/*/subprojects/... for the subproject
            let mut found_nested = false;
            fn find_nested_subproject(
                base: &str,
                name: &str,
                depth: u32,
            ) -> Option<(String, String)> {
                if depth > 5 {
                    return None;
                }
                if let Ok(entries) = std::fs::read_dir(base) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.is_dir() {
                            // Check for direct meson.build
                            let nested = format!("{}/subprojects/{}", p.display(), name);
                            let nested_build = format!("{}/meson.build", nested);
                            if Path::new(&nested_build).exists() {
                                return Some((nested, nested_build));
                            }
                            // Check for wrap file
                            let nested_wrap = format!("{}/subprojects/{}.wrap", p.display(), name);
                            if Path::new(&nested_wrap).exists() {
                                // Read wrap to check for directory key
                                let wrap_dir_name =
                                    std::fs::read_to_string(&nested_wrap).ok().and_then(|c| {
                                        c.lines()
                                            .find(|l| l.trim().starts_with("directory"))
                                            .and_then(|l| {
                                                l.find('=').map(|i| l[i + 1..].trim().to_string())
                                            })
                                    });
                                let sp_dir_name = wrap_dir_name.as_deref().unwrap_or(name);
                                let nested_sp =
                                    format!("{}/subprojects/{}", p.display(), sp_dir_name);
                                // Also pass the non-directory path for download
                                let download_dest = format!("{}/subprojects/{}", p.display(), name);
                                let _ = crate::wrap::download_wrap(&nested_wrap, &download_dest);
                                let nb = format!("{}/meson.build", nested_sp);
                                if Path::new(&nb).exists() {
                                    return Some((nested_sp, nb));
                                }
                            }
                            // Recurse into this directory's subprojects
                            let sub_sp = format!("{}/subprojects", p.display());
                            if Path::new(&sub_sp).is_dir() {
                                if let Some(found) =
                                    find_nested_subproject(&sub_sp, name, depth + 1)
                                {
                                    return Some(found);
                                }
                            }
                        }
                    }
                }
                None
            }
            let sp_base = format!("{}/{}", vm.top_source_root, subproject_dir);
            if let Some((nested_path, nested_build_file)) =
                find_nested_subproject(&sp_base, &name, 0)
            {
                subproject_path = nested_path;
                build_file = nested_build_file;
                found_nested = true;
            }
            if !found_nested {
                if required {
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
        }
    }

    // Save and set up subproject state
    let old_subdir = vm.current_subdir.clone();
    let old_source = vm.source_root.clone();
    let old_build = vm.build_root.clone();
    let old_project = vm.project.clone();
    let old_vars = vm.variables.clone();
    let old_is_subproject = vm.is_subproject;
    let old_options = vm.options.clone();
    let old_caller_option_keys = vm.caller_option_keys.clone();

    vm.source_root = subproject_path;
    vm.build_root = format!("{}/{}/{}", vm.top_build_root, subproject_dir, name);
    vm.current_subdir = String::new();
    vm.variables = HashMap::new();
    vm.is_subproject = true;

    // Initialize subproject options: start fresh but inherit known built-in/base options.
    // This prevents parent project-specific options from leaking into the subproject.
    {
        let mut new_options = HashMap::new();
        let builtin_options = [
            "prefix",
            "bindir",
            "libdir",
            "libexecdir",
            "includedir",
            "datadir",
            "mandir",
            "infodir",
            "localedir",
            "sysconfdir",
            "localstatedir",
            "sharedstatedir",
            "sbindir",
            "buildtype",
            "debug",
            "optimization",
            "warning_level",
            "werror",
            "default_library",
            "strip",
            "unity",
            "unity_size",
            "layout",
            "wrap_mode",
            "pkg_config_path",
            "cmake_prefix_path",
            "auto_features",
            "backend",
            "install_umask",
            "stdsplit",
            "errorlogs",
            "c_std",
            "cpp_std",
            "cpp_eh",
            "cpp_rtti",
        ];
        // Keep built-in options
        for key in &builtin_options {
            if let Some(val) = vm.options.get(*key) {
                new_options.insert(key.to_string(), val.clone());
            }
        }
        // Keep base options (b_*)
        for (k, v) in &vm.options {
            if k.starts_with("b_") {
                new_options.insert(k.clone(), v.clone());
            }
        }
        // Keep prefixed options destined for deeper subprojects
        let prefix_check = format!("{}:", name);
        for (k, v) in &vm.options {
            if k.starts_with(&prefix_check) {
                new_options.insert(k.clone(), v.clone());
            }
        }
        vm.options = new_options;
    }

    // Load subproject options
    let opts_file = format!("{}/meson_options.txt", vm.source_root);
    let opts_file2 = format!("{}/meson.options", vm.source_root);
    let opts_source = if Path::new(&opts_file2).exists() {
        std::fs::read_to_string(&opts_file2).ok()
    } else if Path::new(&opts_file).exists() {
        std::fs::read_to_string(&opts_file).ok()
    } else {
        None
    };
    if let Some(src) = opts_source {
        let defs = crate::options::parse_options_file_defs(&src);
        for def in defs {
            if def.yield_to_parent {
                // Check if parent has the same option with compatible type
                if let Some(parent_val) = old_options.get(&def.name) {
                    let types_compatible = match (&def.opt_type[..], parent_val) {
                        ("boolean", Object::Bool(_)) => true,
                        ("integer", Object::Int(_)) => true,
                        ("string", Object::String(_)) => true,
                        ("combo", Object::String(_)) => true,
                        ("array", Object::Array(_)) => true,
                        ("feature", Object::Feature(_)) => true,
                        // string and combo are interchangeable
                        ("string", Object::Feature(_)) => false,
                        ("combo", Object::Feature(_)) => false,
                        _ => false,
                    };
                    if types_compatible {
                        vm.options.entry(def.name).or_insert(parent_val.clone());
                        continue;
                    }
                    // Types don't match — fall through to use subproject's own default
                }
            }
            // Handle deprecated option renaming
            if let Some(crate::options::DeprecatedInfo::Renamed(ref new_name)) = def.deprecated {
                // This option was renamed — if the old option was set, apply to new name
                if let Some(val) = vm.options.get(&def.name).cloned() {
                    vm.options.entry(new_name.clone()).or_insert(val);
                }
            }
            // Use subproject's own default for non-yielding options
            vm.options.entry(def.name).or_insert(def.default_value);
        }
    }

    // Apply parent options for this subproject (e.g., "subproject_name:opt=val")
    let prefix = format!("{}:", name);
    let parent_opts: Vec<(String, Object)> = old_project
        .as_ref()
        .map(|p| {
            p.default_options
                .iter()
                .filter_map(|opt_str| {
                    if let Some(eq_pos) = opt_str.find('=') {
                        let key = opt_str[..eq_pos].trim();
                        if key.starts_with(&prefix) {
                            let sub_key = key[prefix.len()..].to_string();
                            let val = opt_str[eq_pos + 1..].trim();
                            Some((sub_key, crate::options::parse_option_value(val)))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    for (key, val) in parent_opts {
        vm.options.insert(key, val);
    }

    // Also extract prefixed options from the parent's vm.options
    // (for options passed via subproject() kwargs to nested subprojects)
    let prefixed_from_parent: Vec<(String, Object)> = old_options
        .iter()
        .filter_map(|(k, v)| {
            if k.starts_with(&prefix) {
                Some((k[prefix.len()..].to_string(), v.clone()))
            } else {
                None
            }
        })
        .collect();
    for (key, val) in prefixed_from_parent {
        vm.options.insert(key, val);
    }

    // Apply default_options from subproject() call kwargs
    let mut sub_default_options = VM::get_arg_string_array(args, "default_options");
    // Also handle single string form (e.g., subproject('x', default_options: 'opt=val'))
    if sub_default_options.is_empty() {
        if let Some(Object::String(s)) = VM::get_arg_value(args, "default_options") {
            sub_default_options.push(s.clone());
        }
    }
    for opt_str in &sub_default_options {
        if let Some(eq_pos) = opt_str.find('=') {
            let key = opt_str[..eq_pos].trim().to_string();
            let val = opt_str[eq_pos + 1..].trim();
            let obj = crate::options::parse_option_value(val);
            vm.options.insert(key, obj);
        }
    }
    // Also handle dict form for subproject() kwargs
    if sub_default_options.is_empty() {
        if let Some(Object::Dict(entries)) = VM::get_arg_value(args, "default_options") {
            for (k, v) in entries {
                let val_str = match v {
                    Object::Bool(b) => b.to_string(),
                    Object::Int(n) => n.to_string(),
                    Object::String(s) => s.clone(),
                    other => other.to_display_string(),
                };
                vm.options
                    .insert(k.clone(), crate::options::parse_option_value(&val_str));
            }
        }
    }

    // Build the set of option keys explicitly set by the caller (parent prefixed + subproject() kwargs).
    // These should NOT be overridden by the subproject's own project() default_options.
    {
        let mut caller_keys = std::collections::HashSet::new();
        // Keys from parent prefixed options (e.g., "sub:opt=val" in parent's default_options)
        if let Some(ref proj) = old_project {
            for opt_str in &proj.default_options {
                if let Some(eq_pos) = opt_str.find('=') {
                    let key = opt_str[..eq_pos].trim();
                    if key.starts_with(&prefix) {
                        caller_keys.insert(key[prefix.len()..].to_string());
                    }
                }
            }
        }
        // Keys from parent's vm.options that were prefixed for this subproject
        for k in old_options.keys() {
            if k.starts_with(&prefix) {
                caller_keys.insert(k[prefix.len()..].to_string());
            }
        }
        // Keys from subproject() call kwargs default_options
        for opt_str in &sub_default_options {
            if let Some(eq_pos) = opt_str.find('=') {
                let key = opt_str[..eq_pos].trim().to_string();
                caller_keys.insert(key);
            }
        }
        vm.caller_option_keys = caller_keys;
    }

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
    let exec_result = vm.execute(&compiler_obj.chunk);

    let sp_project = vm.project.clone();
    let sp_vars = vm.variables.clone();

    // Restore state regardless of success/failure
    vm.current_subdir = old_subdir;
    vm.source_root = old_source;
    vm.build_root = old_build;
    vm.project = old_project;
    vm.variables = old_vars;
    vm.is_subproject = old_is_subproject;
    vm.options = old_options;
    vm.caller_option_keys = old_caller_option_keys;

    // Re-register builtins for the parent scope
    crate::builtins::functions::register(vm);

    // Handle execution failure
    if let Err(e) = exec_result {
        if required {
            return Err(format!("Subproject '{}' failed: {}", name, e));
        }
        let sp = Object::Subproject(SubprojectData {
            name: name.clone(),
            version: String::new(),
            found: false,
            variables: HashMap::new(),
        });
        vm.build_data.subprojects.insert(name, sp.clone());
        return Ok(sp);
    }

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
    let env = EnvData::new();
    // Can be initialized with a dict
    if let Some(Object::Dict(entries)) = VM::get_positional_args(args).first() {
        let mut values = env.values.borrow_mut();
        for (k, v) in entries {
            values.push((k.clone(), EnvOp::Set(v.to_string_value())));
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
    let capture = VM::get_arg_bool(args, "capture", true);

    let mut cmd_parts = Vec::new();
    for arg in &positional {
        match arg {
            Object::String(s) => cmd_parts.push(s.clone()),
            Object::ExternalProgram(p) => cmd_parts.push(p.path.clone()),
            Object::File(f) => {
                let path = if f.is_built {
                    if f.subdir.is_empty() {
                        format!("{}/{}", vm.build_root, f.path)
                    } else {
                        format!("{}/{}/{}", vm.build_root, f.subdir, f.path)
                    }
                } else {
                    if f.subdir.is_empty() {
                        format!("{}/{}", vm.source_root, f.path)
                    } else {
                        format!("{}/{}/{}", vm.source_root, f.subdir, f.path)
                    }
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

    // Resolve command path: look in source directory if not absolute
    if !cmd_parts[0].starts_with('/') && !cmd_parts[0].contains('/') {
        let source_path = format!("{}/{}", vm.source_root, cmd_parts[0]);
        if Path::new(&source_path).exists() {
            cmd_parts[0] = source_path;
        } else if !vm.current_subdir.is_empty() {
            let subdir_path = format!("{}/{}/{}", vm.source_root, vm.current_subdir, cmd_parts[0]);
            if Path::new(&subdir_path).exists() {
                cmd_parts[0] = subdir_path;
            }
        }
    }

    // Python scripts need to be run with python3
    if cmd_parts[0].ends_with(".py") {
        cmd_parts.insert(0, "python3".to_string());
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
            let stdout = if capture {
                String::from_utf8_lossy(&output.stdout).to_string()
            } else {
                String::new()
            };
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
            // Reject absolute paths that point inside the source tree
            if s.starts_with('/') {
                let source_root_prefix = format!("{}/", vm.source_root);
                let top_source_root_prefix = format!("{}/", vm.top_source_root);
                if s == &vm.source_root
                    || s.starts_with(&source_root_prefix)
                    || s == &vm.top_source_root
                    || s.starts_with(&top_source_root_prefix)
                {
                    return Err(format!(
                        "Tried to form an absolute path to a dir in the source tree.
You should not do that but use relative paths instead, for
directories that are part of your project.

To get include path to any directory relative to the current dir do

incdir = include_directories(dirname)

After this incdir will contain both the current source dir as well as the
corresponding build dir. It can then be used in any subdirectory and
Meson will take care of all the busywork to make paths work.

Dirname can even be '.' to mark the current directory. Though you should
remember that the current source and build directories are always
put in the include directories by default so you only need to do
include_directories('.') if you intend to use the result in a
different subdirectory.

Note that this error message can also be triggered by
external dependencies being installed within your source
tree - it's not recommended to do this."
                    ));
                }
            }
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

    // Handle required: can be bool or Feature (from get_option)
    let required_val = VM::get_arg_value(args, "required");
    let (required, disabled_by_feature) = match required_val {
        Some(Object::Bool(b)) => (*b, false),
        Some(Object::Feature(FeatureState::Disabled)) => (false, true),
        Some(Object::Feature(FeatureState::Auto)) => (false, false),
        Some(Object::Feature(FeatureState::Enabled)) => (true, false),
        _ => (true, false),
    };

    // Known stabilized modules (can be imported directly without unstable- prefix)
    const STABILIZED_MODULES: &[&str] = &[
        "python",
        "python3",
        "java",
        "keyval",
        "i18n",
        "gnome",
        "pkgconfig",
        "windows",
        "cmake",
        "qt4",
        "qt5",
        "qt6",
        "sourceset",
        "fs",
        "dlang",
        "cuda",
        "rust",
        "hotdoc",
        "wayland",
        "external_project",
        "modtest",
    ];

    // Known unstable-only modules (must be imported with unstable- prefix)
    const UNSTABLE_ONLY_MODULES: &[&str] = &["icestorm", "simd"];

    // Normalize: handle unstable_ (underscore) -> unstable- (dash)
    let normalized = if name.starts_with("unstable_") {
        format!("unstable-{}", &name["unstable_".len()..])
    } else {
        name.clone()
    };

    // Resolve the canonical module name
    let canonical = if let Some(base) = normalized.strip_prefix("unstable-") {
        // unstable- prefix: check if the base module is known (either stabilized or unstable-only)
        if STABILIZED_MODULES.contains(&base) || UNSTABLE_ONLY_MODULES.contains(&base) {
            Some(base.to_string())
        } else {
            None
        }
    } else if STABILIZED_MODULES.contains(&normalized.as_str()) {
        // Direct import of a stabilized module
        Some(normalized.clone())
    } else {
        // Not a known module (could be unstable-only without prefix -> invalid)
        None
    };

    let use_disabler = VM::get_arg_bool(args, "disabler", false);

    match canonical {
        Some(resolved) => {
            if disabled_by_feature {
                // Feature is disabled, so return not-found module or disabler
                if use_disabler {
                    Ok(Object::Disabler)
                } else {
                    Ok(Object::Module(String::new()))
                }
            } else {
                Ok(Object::Module(resolved))
            }
        }
        None => {
            if required {
                Err(format!("Module '{}' not found", name))
            } else if use_disabler {
                Ok(Object::Disabler)
            } else {
                // Return a not-found module (empty name signals not-found)
                Ok(Object::Module(String::new()))
            }
        }
    }
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

    // Collect the string parts: support both join_paths('a', 'b') and join_paths(['a', 'b'])
    let parts: Vec<String> = if positional.len() == 1 {
        match &positional[0] {
            Object::Array(arr) => arr.iter().map(|v| v.to_string_value()).collect(),
            other => return Ok(Object::String(other.to_string_value())),
        }
    } else {
        positional.iter().map(|v| v.to_string_value()).collect()
    };

    if parts.is_empty() {
        return Err("join_paths requires at least one argument".to_string());
    }

    // Implement os.path.join semantics:
    // - Absolute components discard everything before them
    // - Otherwise join with '/'
    // - Empty last component adds trailing '/'
    let mut result = String::new();
    for (i, part) in parts.iter().enumerate() {
        if part.starts_with('/') {
            result = part.clone();
        } else if i == 0 {
            result = part.clone();
        } else {
            if !result.ends_with('/') {
                result.push('/');
            }
            result.push_str(part);
        }
    }

    Ok(Object::String(result))
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

    // Validate option name: must not start with '.' and must have at most one '.'
    if name.starts_with('.') || name.matches('.').count() > 1 {
        return Err(format!("Invalid option name '{}'", name));
    }

    if let Some(val) = vm.options.get(&name) {
        // Apply auto_features override for feature options with Auto value
        if let Object::Feature(FeatureState::Auto) = val {
            if let Some(auto_feat) = vm.options.get("auto_features") {
                match auto_feat {
                    Object::Feature(FeatureState::Disabled) => {
                        return Ok(Object::Feature(FeatureState::Disabled));
                    }
                    Object::Feature(FeatureState::Enabled) => {
                        return Ok(Object::Feature(FeatureState::Enabled));
                    }
                    _ => {}
                }
            }
        }
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
        "b_pch" => Ok(Object::Bool(true)),
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
        "c_args" | "cpp_args" | "c_link_args" | "cpp_link_args" | "rust_args" | "objc_args"
        | "objcpp_args" => Ok(Object::Array(Vec::new())),
        "c_std" | "cpp_std" => Ok(Object::String("none".to_string())),
        "cpp_eh" => Ok(Object::String("default".to_string())),
        "cpp_rtti" => Ok(Object::Bool(true)),
        _ => {
            // For unknown options in subprojects, check if auto_features applies
            if let Some(auto_feat) = vm.options.get("auto_features") {
                Ok(auto_feat.clone())
            } else {
                Ok(Object::Feature(FeatureState::Auto))
            }
        }
    }
}

fn builtin_configuration_data(_vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let data = ConfigData::new();
    if let Some(Object::Dict(entries)) = VM::get_positional_args(args).first() {
        let mut values = data.values.borrow_mut();
        for (k, v) in entries {
            values.insert(k.clone(), (v.clone(), None));
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
    // If the name argument is a disabler, propagate it
    if matches!(positional.first(), Some(Object::Disabler)) {
        return Ok(Object::Disabler);
    }

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
            .unwrap_or_else(|| format!("Assert failed: {}", condition.to_display_string()));
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
    let required_obj = VM::get_arg_value(args, "required");
    let is_feature_disabled = matches!(required_obj, Some(Object::Feature(FeatureState::Disabled)));
    let required = match required_obj {
        Some(Object::Bool(b)) => *b,
        Some(Object::Feature(FeatureState::Disabled)) => false,
        Some(Object::Feature(FeatureState::Enabled)) => true,
        _ => true,
    };
    let _native = VM::get_arg_bool(args, "native", false);

    // If disabled via feature, skip detection entirely
    if is_feature_disabled {
        return Ok(Object::Bool(false));
    }

    let mut all_found = true;
    for arg in &positional {
        if let Object::String(lang) = arg {
            let found = crate::compilers::detect_compiler(vm, lang);
            if !found {
                if required {
                    return Err(format!("No compiler for language '{}'", lang));
                }
                all_found = false;
            }
        }
    }
    Ok(Object::Bool(all_found))
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

fn builtin_is_disabler(_vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    if args.is_empty() {
        return Err("is_disabler() requires exactly 1 argument".to_string());
    }
    Ok(Object::Bool(matches!(args[0].value, Object::Disabler)))
}

fn builtin_build_target(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    // build_target() is like executable() but with target_type kwarg
    // Default to executable if not specified
    let target_type_str = VM::get_arg_str(args, "target_type", usize::MAX).unwrap_or("executable");
    // Delegate to the appropriate implementation based on target_type kwarg
    match target_type_str {
        "executable" => builtin_executable(vm, args),
        "shared_library" => builtin_shared_library(vm, args),
        "static_library" => builtin_static_library(vm, args),
        "shared_module" => builtin_shared_module(vm, args),
        "both_libraries" => builtin_both_libraries(vm, args),
        "library" => builtin_library(vm, args),
        "jar" => builtin_executable(vm, args), // Treat jar as executable for now
        other => Err(format!("Unknown target type: '{}'", other)),
    }
}

fn builtin_subdir_done(_vm: &mut VM, _args: &[CallArg]) -> Result<Object, String> {
    // subdir_done() causes the rest of the current meson.build to be skipped
    // We signal this with a special error that the subdir handler catches
    Err("SUBDIR_DONE".to_string())
}

fn builtin_add_project_dependencies(_vm: &mut VM, _args: &[CallArg]) -> Result<Object, String> {
    // add_project_dependencies() adds dependencies to all targets in the project
    // For now, accept the arguments silently
    Ok(Object::None)
}
