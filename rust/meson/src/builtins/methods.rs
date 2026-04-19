use crate::objects::*;
use crate::vm::*;
/// Methods on built-in types (str, list, dict, bool, int, etc.)
use std::collections::HashMap;

pub fn register(vm: &mut VM) {
    // String methods
    vm.method_registry
        .insert(("str".to_string(), "contains".to_string()), str_contains);
    vm.method_registry.insert(
        ("str".to_string(), "startswith".to_string()),
        str_startswith,
    );
    vm.method_registry
        .insert(("str".to_string(), "endswith".to_string()), str_endswith);
    vm.method_registry
        .insert(("str".to_string(), "format".to_string()), str_format);
    vm.method_registry
        .insert(("str".to_string(), "join".to_string()), str_join);
    vm.method_registry
        .insert(("str".to_string(), "replace".to_string()), str_replace);
    vm.method_registry
        .insert(("str".to_string(), "split".to_string()), str_split);
    vm.method_registry.insert(
        ("str".to_string(), "splitlines".to_string()),
        str_splitlines,
    );
    vm.method_registry
        .insert(("str".to_string(), "strip".to_string()), str_strip);
    vm.method_registry
        .insert(("str".to_string(), "to_lower".to_string()), str_to_lower);
    vm.method_registry
        .insert(("str".to_string(), "to_upper".to_string()), str_to_upper);
    vm.method_registry
        .insert(("str".to_string(), "to_int".to_string()), str_to_int);
    vm.method_registry.insert(
        ("str".to_string(), "underscorify".to_string()),
        str_underscorify,
    );
    vm.method_registry.insert(
        ("str".to_string(), "version_compare".to_string()),
        str_version_compare,
    );
    vm.method_registry
        .insert(("str".to_string(), "substring".to_string()), str_substring);
    vm.method_registry
        .insert(("str".to_string(), "length".to_string()), str_length);

    // Array/list methods
    vm.method_registry
        .insert(("list".to_string(), "contains".to_string()), list_contains);
    vm.method_registry
        .insert(("list".to_string(), "length".to_string()), list_length);
    vm.method_registry
        .insert(("list".to_string(), "get".to_string()), list_get);
    vm.method_registry
        .insert(("list".to_string(), "flatten".to_string()), list_flatten);

    // Dict methods
    vm.method_registry
        .insert(("dict".to_string(), "has_key".to_string()), dict_has_key);
    vm.method_registry
        .insert(("dict".to_string(), "get".to_string()), dict_get);
    vm.method_registry
        .insert(("dict".to_string(), "keys".to_string()), dict_keys);
    vm.method_registry
        .insert(("dict".to_string(), "values".to_string()), dict_values);
    vm.method_registry
        .insert(("dict".to_string(), "length".to_string()), dict_length);
    vm.method_registry
        .insert(("dict".to_string(), "add".to_string()), dict_add);

    // Int methods
    vm.method_registry
        .insert(("int".to_string(), "is_even".to_string()), int_is_even);
    vm.method_registry
        .insert(("int".to_string(), "is_odd".to_string()), int_is_odd);
    vm.method_registry
        .insert(("int".to_string(), "to_string".to_string()), int_to_string);

    // Bool methods
    vm.method_registry.insert(
        ("bool".to_string(), "to_string".to_string()),
        bool_to_string,
    );
    vm.method_registry
        .insert(("bool".to_string(), "to_int".to_string()), bool_to_int);

    // Feature methods
    vm.method_registry.insert(
        ("feature".to_string(), "enabled".to_string()),
        feature_enabled,
    );
    vm.method_registry.insert(
        ("feature".to_string(), "disabled".to_string()),
        feature_disabled,
    );
    vm.method_registry
        .insert(("feature".to_string(), "auto".to_string()), feature_auto);
    vm.method_registry.insert(
        ("feature".to_string(), "allowed".to_string()),
        feature_allowed,
    );
    vm.method_registry.insert(
        ("feature".to_string(), "disable_auto_if".to_string()),
        feature_disable_auto_if,
    );
    vm.method_registry.insert(
        ("feature".to_string(), "enable_auto_if".to_string()),
        feature_enable_auto_if,
    );
    vm.method_registry.insert(
        ("feature".to_string(), "require".to_string()),
        feature_require,
    );
    vm.method_registry.insert(
        ("feature".to_string(), "enable_if".to_string()),
        feature_enable_if,
    );
    vm.method_registry.insert(
        ("feature".to_string(), "disable_if".to_string()),
        feature_disable_if,
    );

    // Dependency methods
    vm.method_registry
        .insert(("dep".to_string(), "found".to_string()), dep_found);
    vm.method_registry
        .insert(("dep".to_string(), "name".to_string()), dep_name);
    vm.method_registry
        .insert(("dep".to_string(), "version".to_string()), dep_version);
    vm.method_registry.insert(
        ("dep".to_string(), "get_variable".to_string()),
        dep_get_variable,
    );
    vm.method_registry.insert(
        ("dep".to_string(), "get_pkgconfig_variable".to_string()),
        dep_get_pkgconfig_variable,
    );
    vm.method_registry.insert(
        ("dep".to_string(), "get_configtool_variable".to_string()),
        dep_get_configtool_variable,
    );
    vm.method_registry
        .insert(("dep".to_string(), "type_name".to_string()), dep_type_name);
    vm.method_registry.insert(
        ("dep".to_string(), "partial_dependency".to_string()),
        dep_partial_dependency,
    );
    vm.method_registry.insert(
        ("dep".to_string(), "include_type".to_string()),
        dep_include_type,
    );
    vm.method_registry
        .insert(("dep".to_string(), "as_system".to_string()), dep_as_system);
    vm.method_registry.insert(
        ("dep".to_string(), "as_link_whole".to_string()),
        dep_as_link_whole,
    );

    // Build target methods
    vm.method_registry.insert(
        ("build_tgt".to_string(), "full_path".to_string()),
        target_full_path,
    );
    vm.method_registry
        .insert(("build_tgt".to_string(), "name".to_string()), target_name);
    vm.method_registry
        .insert(("build_tgt".to_string(), "found".to_string()), target_found);
    vm.method_registry.insert(
        ("build_tgt".to_string(), "extract_objects".to_string()),
        target_extract_objects,
    );
    vm.method_registry.insert(
        ("build_tgt".to_string(), "extract_all_objects".to_string()),
        target_extract_all_objects,
    );
    vm.method_registry.insert(
        ("build_tgt".to_string(), "private_dir_include".to_string()),
        target_private_dir_include,
    );

    // Custom target methods
    vm.method_registry.insert(
        ("custom_tgt".to_string(), "full_path".to_string()),
        custom_target_full_path,
    );
    vm.method_registry.insert(
        ("custom_tgt".to_string(), "to_list".to_string()),
        custom_target_to_list,
    );
    vm.method_registry.insert(
        ("custom_tgt".to_string(), "found".to_string()),
        custom_target_found,
    );

    // Custom target index methods
    vm.method_registry.insert(
        ("custom_idx".to_string(), "full_path".to_string()),
        custom_idx_full_path,
    );

    // External program methods
    vm.method_registry.insert(
        ("external_program".to_string(), "found".to_string()),
        program_found,
    );
    vm.method_registry.insert(
        ("external_program".to_string(), "full_path".to_string()),
        program_full_path,
    );
    vm.method_registry.insert(
        ("external_program".to_string(), "path".to_string()),
        program_full_path,
    );
    vm.method_registry.insert(
        ("external_program".to_string(), "version".to_string()),
        program_version,
    );

    // Configuration data methods
    vm.method_registry
        .insert(("cfg_data".to_string(), "set".to_string()), cfg_set);
    vm.method_registry
        .insert(("cfg_data".to_string(), "set10".to_string()), cfg_set10);
    vm.method_registry.insert(
        ("cfg_data".to_string(), "set_quoted".to_string()),
        cfg_set_quoted,
    );
    vm.method_registry
        .insert(("cfg_data".to_string(), "has".to_string()), cfg_has);
    vm.method_registry
        .insert(("cfg_data".to_string(), "get".to_string()), cfg_get);
    vm.method_registry.insert(
        ("cfg_data".to_string(), "get_unquoted".to_string()),
        cfg_get_unquoted,
    );
    vm.method_registry
        .insert(("cfg_data".to_string(), "keys".to_string()), cfg_keys);
    vm.method_registry.insert(
        ("cfg_data".to_string(), "merge_from".to_string()),
        cfg_merge_from,
    );

    // Environment methods
    vm.method_registry
        .insert(("env".to_string(), "set".to_string()), env_set);
    vm.method_registry
        .insert(("env".to_string(), "prepend".to_string()), env_prepend);
    vm.method_registry
        .insert(("env".to_string(), "append".to_string()), env_append);

    // Meson object methods
    vm.method_registry
        .insert(("meson".to_string(), "version".to_string()), meson_version);
    vm.method_registry.insert(
        ("meson".to_string(), "project_name".to_string()),
        meson_project_name,
    );
    vm.method_registry.insert(
        ("meson".to_string(), "project_version".to_string()),
        meson_project_version,
    );
    vm.method_registry.insert(
        ("meson".to_string(), "project_license".to_string()),
        meson_project_license,
    );
    vm.method_registry.insert(
        ("meson".to_string(), "project_license_files".to_string()),
        meson_project_license_files,
    );
    vm.method_registry.insert(
        ("meson".to_string(), "source_root".to_string()),
        meson_source_root,
    );
    vm.method_registry.insert(
        ("meson".to_string(), "project_source_root".to_string()),
        meson_source_root,
    );
    vm.method_registry.insert(
        ("meson".to_string(), "global_source_root".to_string()),
        meson_source_root,
    );
    vm.method_registry.insert(
        ("meson".to_string(), "build_root".to_string()),
        meson_build_root,
    );
    vm.method_registry.insert(
        ("meson".to_string(), "project_build_root".to_string()),
        meson_build_root,
    );
    vm.method_registry.insert(
        ("meson".to_string(), "global_build_root".to_string()),
        meson_build_root,
    );
    vm.method_registry.insert(
        ("meson".to_string(), "current_source_dir".to_string()),
        meson_current_source_dir,
    );
    vm.method_registry.insert(
        ("meson".to_string(), "current_build_dir".to_string()),
        meson_current_build_dir,
    );
    vm.method_registry.insert(
        ("meson".to_string(), "is_cross_build".to_string()),
        meson_is_cross_build,
    );
    vm.method_registry.insert(
        ("meson".to_string(), "is_subproject".to_string()),
        meson_is_subproject,
    );
    vm.method_registry
        .insert(("meson".to_string(), "backend".to_string()), meson_backend);
    vm.method_registry.insert(
        ("meson".to_string(), "is_unity".to_string()),
        meson_is_unity,
    );
    vm.method_registry.insert(
        ("meson".to_string(), "get_compiler".to_string()),
        meson_get_compiler,
    );
    vm.method_registry.insert(
        ("meson".to_string(), "get_cross_property".to_string()),
        meson_get_cross_property,
    );
    vm.method_registry.insert(
        ("meson".to_string(), "get_external_property".to_string()),
        meson_get_external_property,
    );
    vm.method_registry.insert(
        ("meson".to_string(), "has_external_property".to_string()),
        meson_has_external_property,
    );
    vm.method_registry.insert(
        ("meson".to_string(), "can_run_host_binaries".to_string()),
        meson_can_run_host_binaries,
    );
    vm.method_registry.insert(
        ("meson".to_string(), "add_install_script".to_string()),
        meson_noop,
    );
    vm.method_registry.insert(
        ("meson".to_string(), "add_postconf_script".to_string()),
        meson_noop,
    );
    vm.method_registry.insert(
        ("meson".to_string(), "add_dist_script".to_string()),
        meson_noop,
    );
    vm.method_registry.insert(
        (
            "meson".to_string(),
            "install_dependency_manifest".to_string(),
        ),
        meson_noop,
    );
    vm.method_registry.insert(
        ("meson".to_string(), "override_dependency".to_string()),
        meson_override_dependency,
    );
    vm.method_registry.insert(
        ("meson".to_string(), "override_find_program".to_string()),
        meson_override_find_program,
    );

    // Machine info methods
    vm.method_registry.insert(
        ("build_machine".to_string(), "system".to_string()),
        machine_system,
    );
    vm.method_registry.insert(
        ("build_machine".to_string(), "cpu_family".to_string()),
        machine_cpu_family,
    );
    vm.method_registry.insert(
        ("build_machine".to_string(), "cpu".to_string()),
        machine_cpu,
    );
    vm.method_registry.insert(
        ("build_machine".to_string(), "endian".to_string()),
        machine_endian,
    );
    vm.method_registry.insert(
        ("build_machine".to_string(), "kernel".to_string()),
        machine_kernel,
    );
    vm.method_registry.insert(
        ("build_machine".to_string(), "subsystem".to_string()),
        machine_subsystem,
    );

    // Subproject methods
    vm.method_registry.insert(
        ("subproject".to_string(), "found".to_string()),
        subproject_found,
    );
    vm.method_registry.insert(
        ("subproject".to_string(), "get_variable".to_string()),
        subproject_get_variable,
    );

    // Run result methods
    vm.method_registry.insert(
        ("runresult".to_string(), "returncode".to_string()),
        run_result_returncode,
    );
    vm.method_registry.insert(
        ("runresult".to_string(), "compiled".to_string()),
        run_result_compiled,
    );
    vm.method_registry.insert(
        ("runresult".to_string(), "stdout".to_string()),
        run_result_stdout,
    );
    vm.method_registry.insert(
        ("runresult".to_string(), "stderr".to_string()),
        run_result_stderr,
    );

    // Generator methods
    vm.method_registry.insert(
        ("generator".to_string(), "process".to_string()),
        generator_process,
    );

    // Both libraries methods
    vm.method_registry.insert(
        ("both_libs".to_string(), "get_shared_lib".to_string()),
        both_libs_shared,
    );
    vm.method_registry.insert(
        ("both_libs".to_string(), "get_static_lib".to_string()),
        both_libs_static,
    );
    vm.method_registry.insert(
        ("both_libs".to_string(), "name".to_string()),
        both_libs_name,
    );

    // File methods
    vm.method_registry.insert(
        ("file".to_string(), "full_path".to_string()),
        file_full_path,
    );

    // Compiler methods
    register_compiler_methods(vm);

    // Module methods
    vm.method_registry
        .insert(("module".to_string(), "found".to_string()), module_found);

    // Module methods are registered in modules/
    crate::modules::register_methods(vm);
}

// ---- String methods ----

fn str_contains(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let s = obj.to_string_value();
    let substr = VM::get_positional_args(args)
        .first()
        .map(|v| v.to_string_value())
        .unwrap_or_default();
    Ok(Object::Bool(s.contains(&substr)))
}

fn str_startswith(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let s = obj.to_string_value();
    let prefix = VM::get_positional_args(args)
        .first()
        .map(|v| v.to_string_value())
        .unwrap_or_default();
    Ok(Object::Bool(s.starts_with(&prefix)))
}

fn str_endswith(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let s = obj.to_string_value();
    let suffix = VM::get_positional_args(args)
        .first()
        .map(|v| v.to_string_value())
        .unwrap_or_default();
    Ok(Object::Bool(s.ends_with(&suffix)))
}

fn str_format(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let template = obj.to_string_value();
    let positional = VM::get_positional_args(args);
    // Phase 1: Single-pass replacement of @N@ patterns.
    // @N@ (where N is digits) takes priority over @@ at each position.
    let mut result = String::new();
    let chars: Vec<char> = template.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' {
            // Try to match @N@ where N is one or more digits
            let start = i + 1;
            let mut j = start;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            if j > start && j < chars.len() && chars[j] == '@' {
                // Found @N@ pattern
                let idx: usize = chars[start..j].iter().collect::<String>().parse().unwrap();
                if idx < positional.len() {
                    result.push_str(&positional[idx].to_display_string());
                } else {
                    // Index out of range - keep original text
                    for c in &chars[i..=j] {
                        result.push(*c);
                    }
                }
                i = j + 1;
            } else {
                // Not an @N@ pattern, just output @
                result.push('@');
                i += 1;
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    // Phase 2: Replace @@ with literal @
    let result = result.replace("@@", "@");
    Ok(Object::String(result))
}

fn str_join(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let sep = obj.to_string_value();
    let positional = VM::get_positional_args(args);
    let mut parts = Vec::new();
    for arg in &positional {
        flatten_into(arg, &mut parts);
    }
    Ok(Object::String(parts.join(&sep)))
}

fn flatten_into(obj: &Object, out: &mut Vec<String>) {
    match obj {
        Object::Array(arr) => {
            for item in arr {
                flatten_into(item, out);
            }
        }
        Object::String(s) => out.push(s.clone()),
        other => out.push(other.to_display_string()),
    }
}

fn str_replace(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let s = obj.to_string_value();
    let positional = VM::get_positional_args(args);
    let old = positional
        .first()
        .map(|v| v.to_string_value())
        .unwrap_or_default();
    let new = positional
        .get(1)
        .map(|v| v.to_string_value())
        .unwrap_or_default();
    Ok(Object::String(s.replace(&old, &new)))
}

fn str_split(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let s = obj.to_string_value();
    let positional = VM::get_positional_args(args);
    let sep = positional.first().map(|v| v.to_string_value());
    let parts: Vec<Object> = if let Some(sep) = sep {
        s.split(&sep)
            .map(|p| Object::String(p.to_string()))
            .collect()
    } else {
        s.split_whitespace()
            .map(|p| Object::String(p.to_string()))
            .collect()
    };
    Ok(Object::Array(parts))
}

fn str_splitlines(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    let s = obj.to_string_value();
    let parts: Vec<Object> = s.lines().map(|l| Object::String(l.to_string())).collect();
    Ok(Object::Array(parts))
}

fn str_strip(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let s = obj.to_string_value();
    let chars = VM::get_positional_args(args)
        .first()
        .map(|v| v.to_string_value());
    let result = if let Some(chars) = chars {
        let chars: Vec<char> = chars.chars().collect();
        s.trim_matches(|c| chars.contains(&c)).to_string()
    } else {
        s.trim().to_string()
    };
    Ok(Object::String(result))
}

fn str_to_lower(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    Ok(Object::String(obj.to_string_value().to_lowercase()))
}

fn str_to_upper(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    Ok(Object::String(obj.to_string_value().to_uppercase()))
}

fn str_to_int(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    let s = obj.to_string_value();
    let s = s.trim();
    let val = if s.starts_with("0x") || s.starts_with("0X") {
        i64::from_str_radix(&s[2..], 16)
    } else if s.starts_with("0o") || s.starts_with("0O") {
        i64::from_str_radix(&s[2..], 8)
    } else if s.starts_with("0b") || s.starts_with("0B") {
        i64::from_str_radix(&s[2..], 2)
    } else {
        s.parse()
    };
    val.map(Object::Int)
        .map_err(|e| format!("Cannot convert '{}' to int: {}", s, e))
}

fn str_underscorify(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    let s = obj.to_string_value();
    let result: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    Ok(Object::String(result))
}

fn str_version_compare(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let version = obj.to_string_value();
    let constraint = VM::get_positional_args(args)
        .first()
        .map(|v| v.to_string_value())
        .unwrap_or_default();
    Ok(Object::Bool(crate::options::version_compare(
        &version,
        &constraint,
    )))
}

fn str_substring(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let s = obj.to_string_value();
    let positional = VM::get_positional_args(args);
    let start = positional
        .first()
        .and_then(|v| {
            if let Object::Int(n) = v {
                Some(*n as usize)
            } else {
                None
            }
        })
        .unwrap_or(0);
    let end = positional
        .get(1)
        .and_then(|v| {
            if let Object::Int(n) = v {
                Some(*n as usize)
            } else {
                None
            }
        })
        .unwrap_or(s.len());
    let start = start.min(s.len());
    let end = end.min(s.len());
    Ok(Object::String(s[start..end].to_string()))
}

fn str_length(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    Ok(Object::Int(obj.to_string_value().len() as i64))
}

// ---- List methods ----

fn list_contains_recursive(arr: &[Object], item: &Object) -> bool {
    for elem in arr {
        if elem == item {
            return true;
        }
        // Recursively search nested arrays for non-array items
        if let Object::Array(nested) = elem {
            if !matches!(item, Object::Array(_)) {
                if list_contains_recursive(nested, item) {
                    return true;
                }
            }
        }
    }
    false
}

fn list_contains(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Array(arr) = obj {
        let item = VM::get_positional_args(args)
            .first()
            .cloned()
            .cloned()
            .unwrap_or(Object::None);
        Ok(Object::Bool(list_contains_recursive(arr, &item)))
    } else {
        Err("list.contains() called on non-list".to_string())
    }
}

fn list_length(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::Array(arr) = obj {
        Ok(Object::Int(arr.len() as i64))
    } else {
        Err("list.length() called on non-list".to_string())
    }
}

fn list_get(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Array(arr) = obj {
        let positional = VM::get_positional_args(args);
        let idx = positional
            .first()
            .and_then(|v| {
                if let Object::Int(n) = v {
                    Some(*n)
                } else {
                    None
                }
            })
            .unwrap_or(0);
        let default = positional.get(1).cloned();
        let uidx = if idx < 0 {
            (arr.len() as i64 + idx) as usize
        } else {
            idx as usize
        };
        match arr.get(uidx) {
            Some(val) => Ok(val.clone()),
            None => default
                .cloned()
                .ok_or(format!("Index {} out of range", idx)),
        }
    } else {
        Err("list.get() called on non-list".to_string())
    }
}

// ---- Dict methods ----

fn list_flatten_recursive(arr: &[Object], out: &mut Vec<Object>) {
    for item in arr {
        if let Object::Array(nested) = item {
            list_flatten_recursive(nested, out);
        } else {
            out.push(item.clone());
        }
    }
}

fn list_flatten(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::Array(arr) = obj {
        let mut result = Vec::new();
        list_flatten_recursive(arr, &mut result);
        Ok(Object::Array(result))
    } else {
        Err("list.flatten() called on non-list".to_string())
    }
}

fn dict_has_key(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Dict(entries) = obj {
        let key = VM::get_positional_args(args)
            .first()
            .map(|v| v.to_string_value())
            .unwrap_or_default();
        Ok(Object::Bool(entries.iter().any(|(k, _)| k == &key)))
    } else {
        Err("dict.has_key() called on non-dict".to_string())
    }
}

fn dict_get(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Dict(entries) = obj {
        let positional = VM::get_positional_args(args);
        let key = positional
            .first()
            .map(|v| v.to_string_value())
            .unwrap_or_default();
        let default = positional.get(1).cloned();
        match entries.iter().find(|(k, _)| k == &key) {
            Some((_, v)) => Ok(v.clone()),
            None => default
                .cloned()
                .ok_or(format!("Key '{}' not found in dict", key)),
        }
    } else {
        Err("dict.get() called on non-dict".to_string())
    }
}

fn dict_keys(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::Dict(entries) = obj {
        Ok(Object::Array(
            entries
                .iter()
                .map(|(k, _)| Object::String(k.clone()))
                .collect(),
        ))
    } else {
        Err("dict.keys() called on non-dict".to_string())
    }
}

fn dict_values(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::Dict(entries) = obj {
        Ok(Object::Array(
            entries.iter().map(|(_, v)| v.clone()).collect(),
        ))
    } else {
        Err("dict.values() called on non-dict".to_string())
    }
}

fn dict_length(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::Dict(entries) = obj {
        Ok(Object::Int(entries.len() as i64))
    } else {
        Err("dict.length() called on non-dict".to_string())
    }
}

fn dict_add(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Dict(entries) = obj {
        let positional = VM::get_positional_args(args);
        if let Some(Object::Dict(other)) = positional.first() {
            let mut result = entries.clone();
            for (k, v) in other {
                if let Some(existing) = result.iter_mut().find(|(ek, _)| ek == k) {
                    existing.1 = v.clone();
                } else {
                    result.push((k.clone(), v.clone()));
                }
            }
            Ok(Object::Dict(result))
        } else {
            Err("dict.add() requires a dict argument".to_string())
        }
    } else {
        Err("dict.add() called on non-dict".to_string())
    }
}

// ---- Int methods ----

fn int_is_even(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::Int(n) = obj {
        Ok(Object::Bool(n % 2 == 0))
    } else {
        Err("Not an int".to_string())
    }
}

fn int_is_odd(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::Int(n) = obj {
        Ok(Object::Bool(n % 2 != 0))
    } else {
        Err("Not an int".to_string())
    }
}

fn int_to_string(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Int(n) = obj {
        let fill = VM::get_arg_int(args, "fill", 0);
        if fill > 0 {
            Ok(Object::String(format!(
                "{:0width$}",
                n,
                width = fill as usize
            )))
        } else {
            Ok(Object::String(n.to_string()))
        }
    } else {
        Err("Not an int".to_string())
    }
}

fn bool_to_string(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Bool(b) = obj {
        let positional = VM::get_positional_args(args);
        if positional.len() >= 2 {
            let true_str = positional[0].to_string_value();
            let false_str = positional[1].to_string_value();
            Ok(Object::String(if *b { true_str } else { false_str }))
        } else {
            Ok(Object::String(if *b {
                "true".to_string()
            } else {
                "false".to_string()
            }))
        }
    } else {
        Err("Not a bool".to_string())
    }
}

fn bool_to_int(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::Bool(b) = obj {
        Ok(Object::Int(if *b { 1 } else { 0 }))
    } else {
        Err("Not a bool".to_string())
    }
}

// ---- Feature methods ----

fn feature_enabled(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::Feature(f) = obj {
        Ok(Object::Bool(*f == FeatureState::Enabled))
    } else {
        Err("Not a feature".to_string())
    }
}

fn feature_disabled(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::Feature(f) = obj {
        Ok(Object::Bool(*f == FeatureState::Disabled))
    } else {
        Err("Not a feature".to_string())
    }
}

fn feature_auto(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::Feature(f) = obj {
        Ok(Object::Bool(*f == FeatureState::Auto))
    } else {
        Err("Not a feature".to_string())
    }
}

fn feature_allowed(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::Feature(f) = obj {
        Ok(Object::Bool(*f != FeatureState::Disabled))
    } else {
        Err("Not a feature".to_string())
    }
}

fn feature_disable_auto_if(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Feature(f) = obj {
        let cond = VM::get_positional_args(args)
            .first()
            .map(|v| v.is_truthy())
            .unwrap_or(false);
        if *f == FeatureState::Auto && cond {
            Ok(Object::Feature(FeatureState::Disabled))
        } else {
            Ok(obj.clone())
        }
    } else {
        Err("Not a feature".to_string())
    }
}

fn feature_enable_auto_if(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Feature(f) = obj {
        let cond = VM::get_positional_args(args)
            .first()
            .map(|v| v.is_truthy())
            .unwrap_or(false);
        if *f == FeatureState::Auto && cond {
            Ok(Object::Feature(FeatureState::Enabled))
        } else {
            Ok(obj.clone())
        }
    } else {
        Err("Not a feature".to_string())
    }
}

fn feature_require(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Feature(f) = obj {
        let cond = VM::get_positional_args(args)
            .first()
            .map(|v| v.is_truthy())
            .unwrap_or(true);
        if *f == FeatureState::Enabled && !cond {
            let msg =
                VM::get_arg_str(args, "error_message", usize::MAX).unwrap_or("requirement not met");
            return Err(format!("Feature requirement not met: {}", msg));
        }
        if !cond {
            Ok(Object::Feature(FeatureState::Disabled))
        } else {
            Ok(obj.clone())
        }
    } else {
        Err("Not a feature".to_string())
    }
}

fn feature_enable_if(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Feature(f) = obj {
        let cond = VM::get_positional_args(args)
            .first()
            .map(|v| v.is_truthy())
            .unwrap_or(false);
        if cond && *f != FeatureState::Disabled {
            Ok(Object::Feature(FeatureState::Enabled))
        } else {
            Ok(obj.clone())
        }
    } else {
        Err("Not a feature".to_string())
    }
}

fn feature_disable_if(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Feature(f) = obj {
        let cond = VM::get_positional_args(args)
            .first()
            .map(|v| v.is_truthy())
            .unwrap_or(false);
        if cond && *f != FeatureState::Enabled {
            Ok(Object::Feature(FeatureState::Disabled))
        } else {
            Ok(obj.clone())
        }
    } else {
        Err("Not a feature".to_string())
    }
}

// ---- Dependency methods ----

fn dep_found(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::Dependency(d) = obj {
        Ok(Object::Bool(d.found))
    } else {
        Err("Not a dependency".to_string())
    }
}

fn dep_name(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::Dependency(d) = obj {
        Ok(Object::String(d.name.clone()))
    } else {
        Err("Not a dependency".to_string())
    }
}

fn dep_version(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::Dependency(d) = obj {
        Ok(Object::String(d.version.clone()))
    } else {
        Err("Not a dependency".to_string())
    }
}

fn dep_get_variable(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Dependency(d) = obj {
        let positional = VM::get_positional_args(args);

        // For internal dependencies, prefer 'internal' kwarg, then positional arg
        if d.is_internal {
            let var_name = VM::get_arg_str(args, "internal", usize::MAX)
                .map(String::from)
                .or_else(|| positional.first().map(|v| v.to_string_value()));

            if let Some(name) = var_name {
                if let Some(val) = d.variables.get(&name) {
                    return Ok(Object::String(val.clone()));
                }
            }
        } else {
            // For external deps, try type-specific kwargs, falling back to stored variables
            // Try pkgconfig kwarg
            if let Some(var) = VM::get_arg_str(args, "pkgconfig", usize::MAX) {
                if let Some(val) = d.variables.get(var) {
                    return Ok(Object::String(val.clone()));
                }
                // Try running pkg-config
                if let Ok(output) = std::process::Command::new("pkg-config")
                    .args(["--variable", var, &d.name])
                    .output()
                {
                    if output.status.success() {
                        let val = String::from_utf8_lossy(&output.stdout).trim().to_string();
                        if !val.is_empty() {
                            return Ok(Object::String(val));
                        }
                    }
                }
            }
            // Try configtool kwarg
            if let Some(var) = VM::get_arg_str(args, "configtool", usize::MAX) {
                if let Some(val) = d.variables.get(var) {
                    return Ok(Object::String(val.clone()));
                }
                let config_tool = format!("{}-config", d.name);
                if let Ok(output) = std::process::Command::new(&config_tool)
                    .arg(format!("--{}", var))
                    .output()
                {
                    if output.status.success() {
                        let val = String::from_utf8_lossy(&output.stdout).trim().to_string();
                        if !val.is_empty() {
                            return Ok(Object::String(val));
                        }
                    }
                }
            }
            // Try cmake kwarg
            if let Some(var) = VM::get_arg_str(args, "cmake", usize::MAX) {
                if let Some(val) = d.variables.get(var) {
                    return Ok(Object::String(val.clone()));
                }
            }
            // Try positional arg as generic lookup
            if let Some(first) = positional.first() {
                let var = first.to_string_value();
                if let Some(val) = d.variables.get(&var) {
                    return Ok(Object::String(val.clone()));
                }
            }
        }

        // Fall back to default_value
        let default = VM::get_arg_str(args, "default_value", usize::MAX)
            .map(|s| Object::String(s.to_string()));
        default.ok_or_else(|| "Variable not found in dependency".to_string())
    } else {
        Err("Not a dependency".to_string())
    }
}

fn dep_get_pkgconfig_variable(
    _vm: &mut VM,
    obj: &Object,
    args: &[CallArg],
) -> Result<Object, String> {
    if let Object::Dependency(d) = obj {
        let positional = VM::get_positional_args(args);
        let var_name = positional.first().map(|v| v.to_string_value());

        if let Some(var) = var_name {
            // First check stored variables
            if let Some(val) = d.variables.get(&var) {
                return Ok(Object::String(val.clone()));
            }
            // Try running pkg-config
            if let Ok(output) = std::process::Command::new("pkg-config")
                .args(["--variable", &var, &d.name])
                .output()
            {
                if output.status.success() {
                    let val = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    return Ok(Object::String(val));
                }
            }
        }

        let default = VM::get_arg_str(args, "default_value", usize::MAX)
            .map(|s| Object::String(s.to_string()));
        default.ok_or_else(|| "Variable not found in dependency".to_string())
    } else {
        Err("Not a dependency".to_string())
    }
}

fn dep_get_configtool_variable(
    _vm: &mut VM,
    obj: &Object,
    args: &[CallArg],
) -> Result<Object, String> {
    if let Object::Dependency(d) = obj {
        let positional = VM::get_positional_args(args);
        let var_name = positional
            .first()
            .map(|v| v.to_string_value())
            .or_else(|| VM::get_arg_str(args, "configtool", usize::MAX).map(String::from));

        if let Some(var) = var_name {
            // First check stored variables
            if let Some(val) = d.variables.get(&var) {
                return Ok(Object::String(val.clone()));
            }
            // Try running the config tool
            let config_tool = format!("{}-config", d.name);
            if let Ok(output) = std::process::Command::new(&config_tool)
                .arg(format!("--{}", var))
                .output()
            {
                if output.status.success() {
                    let val = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    return Ok(Object::String(val));
                }
            }
        }

        let default = VM::get_arg_str(args, "default_value", usize::MAX)
            .map(|s| Object::String(s.to_string()));
        default.ok_or_else(|| "Variable not found in dependency".to_string())
    } else {
        Err("Not a dependency".to_string())
    }
}

fn dep_type_name(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::Dependency(d) = obj {
        let t = if !d.found {
            "not-found"
        } else if d.is_internal {
            "internal"
        } else {
            "pkgconfig"
        };
        Ok(Object::String(t.to_string()))
    } else {
        Err("Not a dependency".to_string())
    }
}

fn dep_partial_dependency(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Dependency(d) = obj {
        let compile_args = VM::get_arg_bool(args, "compile_args", false);
        let link_args = VM::get_arg_bool(args, "link_args", false);
        let links = VM::get_arg_bool(args, "links", false);
        let includes = VM::get_arg_bool(args, "includes", false);
        let sources = VM::get_arg_bool(args, "sources", false);

        let mut pd = DependencyData::not_found(&d.name);
        pd.found = d.found;
        pd.version = d.version.clone();
        if compile_args {
            pd.compile_args = d.compile_args.clone();
        }
        if link_args || links {
            pd.link_args = d.link_args.clone();
        }
        if includes {
            pd.include_dirs = d.include_dirs.clone();
        }
        if sources {
            pd.sources = d.sources.clone();
        }
        Ok(Object::Dependency(pd))
    } else {
        Err("Not a dependency".to_string())
    }
}

fn dep_include_type(_vm: &mut VM, _obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    Ok(Object::String("preserve".to_string()))
}

fn dep_as_system(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    Ok(obj.clone())
}

fn dep_as_link_whole(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    Ok(obj.clone())
}

// ---- Build target methods ----

fn target_full_path(vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::BuildTarget(t) = obj {
        Ok(Object::String(format!(
            "{}/{}",
            vm.build_root,
            t.outputs.first().unwrap_or(&t.name)
        )))
    } else {
        Err("Not a build target".to_string())
    }
}

fn target_name(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::BuildTarget(t) = obj {
        Ok(Object::String(t.name.clone()))
    } else {
        Err("Not a build target".to_string())
    }
}

fn target_found(_vm: &mut VM, _obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    Ok(Object::Bool(true))
}

fn target_extract_objects(
    _vm: &mut VM,
    _obj: &Object,
    _args: &[CallArg],
) -> Result<Object, String> {
    Ok(Object::Array(Vec::new()))
}

fn target_extract_all_objects(
    _vm: &mut VM,
    _obj: &Object,
    _args: &[CallArg],
) -> Result<Object, String> {
    Ok(Object::Array(Vec::new()))
}

fn target_private_dir_include(
    _vm: &mut VM,
    obj: &Object,
    _args: &[CallArg],
) -> Result<Object, String> {
    if let Object::BuildTarget(t) = obj {
        Ok(Object::IncludeDirs(IncludeDirsData {
            dirs: vec![format!("{}@private", t.id)],
            is_system: false,
        }))
    } else {
        Err("Not a build target".to_string())
    }
}

// ---- Custom target methods ----

fn custom_target_full_path(vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::CustomTarget(t) = obj {
        let output = t.outputs.first().cloned().unwrap_or_default();
        Ok(Object::String(format!("{}/{}", vm.build_root, output)))
    } else {
        Err("Not a custom target".to_string())
    }
}

fn custom_target_to_list(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::CustomTarget(t) = obj {
        let list: Vec<Object> = t
            .outputs
            .iter()
            .enumerate()
            .map(|(i, _)| Object::CustomTargetIndex(t.clone(), i))
            .collect();
        Ok(Object::Array(list))
    } else {
        Err("Not a custom target".to_string())
    }
}

fn custom_target_found(_vm: &mut VM, _obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    Ok(Object::Bool(true))
}

// ---- Custom target index methods ----

fn custom_idx_full_path(vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::CustomTargetIndex(ct_ref, idx) = obj {
        if *idx < ct_ref.outputs.len() {
            Ok(Object::String(format!(
                "{}/{}",
                vm.build_root, ct_ref.outputs[*idx]
            )))
        } else {
            Ok(Object::String(format!(
                "{}/{}_output_{}",
                vm.build_root, ct_ref.name, idx
            )))
        }
    } else {
        Err("full_path() called on non-custom-target-index".to_string())
    }
}

// ---- External program methods ----

fn program_found(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::ExternalProgram(p) = obj {
        Ok(Object::Bool(p.found))
    } else {
        Err("Not a program".to_string())
    }
}

fn program_full_path(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::ExternalProgram(p) = obj {
        Ok(Object::String(p.path.clone()))
    } else {
        Err("Not a program".to_string())
    }
}

fn program_version(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::ExternalProgram(p) = obj {
        Ok(Object::String(p.version.clone().unwrap_or_default()))
    } else {
        Err("Not a program".to_string())
    }
}

// ---- Configuration data methods ----

fn cfg_set(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let key = positional
        .first()
        .map(|v| v.to_string_value())
        .unwrap_or_default();
    let value = positional
        .get(1)
        .map(|v| (*v).clone())
        .unwrap_or(Object::None);
    let description = VM::get_arg_str(args, "description", usize::MAX).map(String::from);

    if let Object::ConfigurationData(data) = obj {
        data.values.borrow_mut().insert(key, (value, description));
        Ok(obj.clone())
    } else {
        Err("Not configuration data".to_string())
    }
}

fn cfg_set10(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let key = positional
        .first()
        .map(|v| v.to_string_value())
        .unwrap_or_default();
    let cond = positional.get(1).map(|v| v.is_truthy()).unwrap_or(false);
    let description = VM::get_arg_str(args, "description", usize::MAX).map(String::from);

    if let Object::ConfigurationData(data) = obj {
        data.values
            .borrow_mut()
            .insert(key, (Object::Int(if cond { 1 } else { 0 }), description));
        Ok(obj.clone())
    } else {
        Err("Not configuration data".to_string())
    }
}

fn cfg_set_quoted(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let key = positional
        .first()
        .map(|v| v.to_string_value())
        .unwrap_or_default();
    let value = positional
        .get(1)
        .map(|v| format!("\"{}\"", v.to_string_value()))
        .unwrap_or_default();
    let description = VM::get_arg_str(args, "description", usize::MAX).map(String::from);

    if let Object::ConfigurationData(data) = obj {
        data.values
            .borrow_mut()
            .insert(key, (Object::String(value), description));
        Ok(obj.clone())
    } else {
        Err("Not configuration data".to_string())
    }
}

fn cfg_has(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::ConfigurationData(data) = obj {
        let key = VM::get_positional_args(args)
            .first()
            .map(|v| v.to_string_value())
            .unwrap_or_default();
        Ok(Object::Bool(data.values.borrow().contains_key(&key)))
    } else {
        Err("Not configuration data".to_string())
    }
}

fn cfg_get(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::ConfigurationData(data) = obj {
        let positional = VM::get_positional_args(args);
        let key = positional
            .first()
            .map(|v| v.to_string_value())
            .unwrap_or_default();
        let default = positional.get(1);
        let values = data.values.borrow();
        match values.get(&key) {
            Some((val, _)) => Ok(val.clone()),
            None => default
                .map(|v| (*v).clone())
                .ok_or(format!("Key '{}' not found in configuration data", key)),
        }
    } else {
        Err("Not configuration data".to_string())
    }
}

fn cfg_get_unquoted(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::ConfigurationData(data) = obj {
        let key = VM::get_positional_args(args)
            .first()
            .map(|v| v.to_string_value())
            .unwrap_or_default();
        let values = data.values.borrow();
        match values.get(&key) {
            Some((Object::String(s), _)) => {
                let unquoted = s.trim_matches('"').to_string();
                Ok(Object::String(unquoted))
            }
            Some((val, _)) => Ok(val.clone()),
            None => Err(format!("Key '{}' not found", key)),
        }
    } else {
        Err("Not configuration data".to_string())
    }
}

fn cfg_keys(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::ConfigurationData(data) = obj {
        let values = data.values.borrow();
        let mut keys: Vec<String> = values.keys().cloned().collect();
        keys.sort();
        let keys: Vec<Object> = keys.into_iter().map(Object::String).collect();
        Ok(Object::Array(keys))
    } else {
        Err("Not configuration data".to_string())
    }
}

fn cfg_merge_from(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::ConfigurationData(data) = obj {
        if let Some(Object::ConfigurationData(other)) =
            VM::get_positional_args(args).first().map(|v| *v)
        {
            let other_values = other.values.borrow();
            let mut my_values = data.values.borrow_mut();
            for (k, v) in other_values.iter() {
                my_values.insert(k.clone(), v.clone());
            }
        }
        Ok(obj.clone())
    } else {
        Err("Not configuration data".to_string())
    }
}

// ---- Environment methods ----

fn env_set(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Environment(env) = obj {
        let positional = VM::get_positional_args(args);
        let key = positional
            .first()
            .map(|v| v.to_string_value())
            .unwrap_or_default();
        let value = positional
            .get(1)
            .map(|v| v.to_string_value())
            .unwrap_or_default();
        env.values.borrow_mut().push((key, EnvOp::Set(value)));
        Ok(obj.clone())
    } else {
        Err("Not an environment".to_string())
    }
}

fn env_prepend(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Environment(env) = obj {
        let positional = VM::get_positional_args(args);
        let key = positional
            .first()
            .map(|v| v.to_string_value())
            .unwrap_or_default();
        let value = positional
            .get(1)
            .map(|v| v.to_string_value())
            .unwrap_or_default();
        let sep = VM::get_arg_str(args, "separator", usize::MAX)
            .unwrap_or(":")
            .to_string();
        env.values
            .borrow_mut()
            .push((key, EnvOp::Prepend(value, sep)));
        Ok(obj.clone())
    } else {
        Err("Not an environment".to_string())
    }
}

fn env_append(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Environment(env) = obj {
        let positional = VM::get_positional_args(args);
        let key = positional
            .first()
            .map(|v| v.to_string_value())
            .unwrap_or_default();
        let value = positional
            .get(1)
            .map(|v| v.to_string_value())
            .unwrap_or_default();
        let sep = VM::get_arg_str(args, "separator", usize::MAX)
            .unwrap_or(":")
            .to_string();
        env.values
            .borrow_mut()
            .push((key, EnvOp::Append(value, sep)));
        Ok(obj.clone())
    } else {
        Err("Not an environment".to_string())
    }
}

// ---- Meson object methods ----

fn meson_version(_vm: &mut VM, _obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    Ok(Object::String("1.7.0".to_string()))
}

fn meson_project_name(vm: &mut VM, _obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    Ok(Object::String(
        vm.project
            .as_ref()
            .map(|p| p.name.clone())
            .unwrap_or_default(),
    ))
}

fn meson_project_version(vm: &mut VM, _obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    Ok(Object::String(
        vm.project
            .as_ref()
            .map(|p| p.version.clone())
            .unwrap_or_default(),
    ))
}

fn meson_project_license(vm: &mut VM, _obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    let licenses = vm
        .project
        .as_ref()
        .map(|p| p.license.clone())
        .unwrap_or_default();
    Ok(Object::Array(
        licenses.into_iter().map(Object::String).collect(),
    ))
}

fn meson_project_license_files(
    vm: &mut VM,
    _obj: &Object,
    _args: &[CallArg],
) -> Result<Object, String> {
    let files = vm
        .project
        .as_ref()
        .map(|p| p.license_files.clone())
        .unwrap_or_default();
    Ok(Object::Array(
        files.into_iter().map(Object::String).collect(),
    ))
}

fn meson_source_root(vm: &mut VM, _obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    Ok(Object::String(vm.source_root.clone()))
}

fn meson_build_root(vm: &mut VM, _obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    Ok(Object::String(vm.build_root.clone()))
}

fn meson_current_source_dir(
    vm: &mut VM,
    _obj: &Object,
    _args: &[CallArg],
) -> Result<Object, String> {
    if vm.current_subdir.is_empty() {
        Ok(Object::String(vm.source_root.clone()))
    } else {
        Ok(Object::String(format!(
            "{}/{}",
            vm.source_root, vm.current_subdir
        )))
    }
}

fn meson_current_build_dir(
    vm: &mut VM,
    _obj: &Object,
    _args: &[CallArg],
) -> Result<Object, String> {
    if vm.current_subdir.is_empty() {
        Ok(Object::String(vm.build_root.clone()))
    } else {
        Ok(Object::String(format!(
            "{}/{}",
            vm.build_root, vm.current_subdir
        )))
    }
}

fn meson_is_cross_build(_vm: &mut VM, _obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    Ok(Object::Bool(false))
}

fn meson_is_subproject(vm: &mut VM, _obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    Ok(Object::Bool(vm.is_subproject))
}

fn meson_backend(_vm: &mut VM, _obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    Ok(Object::String("ninja".to_string()))
}

fn meson_is_unity(_vm: &mut VM, _obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    Ok(Object::Bool(false))
}

fn meson_get_compiler(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let lang = VM::get_positional_args(args)
        .first()
        .map(|v| v.to_string_value())
        .unwrap_or_default();
    let key = format!("compiler_{}", lang);
    vm.variables
        .get(&key)
        .cloned()
        .or_else(|| vm.globals.get(&key).cloned())
        .ok_or(format!("No compiler for language '{}'", lang))
}

fn meson_get_cross_property(
    _vm: &mut VM,
    _obj: &Object,
    args: &[CallArg],
) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    positional
        .get(1)
        .cloned()
        .cloned()
        .ok_or("No cross property found".to_string())
}

fn meson_get_external_property(
    _vm: &mut VM,
    _obj: &Object,
    args: &[CallArg],
) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    positional
        .get(1)
        .cloned()
        .cloned()
        .ok_or("No external property found".to_string())
}

fn meson_has_external_property(
    _vm: &mut VM,
    _obj: &Object,
    _args: &[CallArg],
) -> Result<Object, String> {
    Ok(Object::Bool(false))
}

fn meson_can_run_host_binaries(
    _vm: &mut VM,
    _obj: &Object,
    _args: &[CallArg],
) -> Result<Object, String> {
    Ok(Object::Bool(true))
}

fn meson_noop(_vm: &mut VM, _obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    Ok(Object::None)
}

fn meson_override_dependency(
    vm: &mut VM,
    _obj: &Object,
    args: &[CallArg],
) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let name = positional
        .first()
        .map(|v| v.to_string_value())
        .unwrap_or_default();
    let dep = positional.get(1).cloned().cloned().unwrap_or(Object::None);
    vm.build_data.dependencies.insert(name, dep);
    Ok(Object::None)
}

fn meson_override_find_program(
    vm: &mut VM,
    _obj: &Object,
    args: &[CallArg],
) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let name = positional
        .first()
        .map(|v| v.to_string_value())
        .unwrap_or_default();
    let prog = positional.get(1).cloned().cloned().unwrap_or(Object::None);
    vm.globals
        .insert(format!("__program_override_{}", name), prog);
    Ok(Object::None)
}

// ---- Machine info methods ----

fn machine_system(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::MachineInfo(m) = obj {
        Ok(Object::String(m.system.clone()))
    } else {
        Err("Not machine info".to_string())
    }
}

fn machine_cpu_family(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::MachineInfo(m) = obj {
        Ok(Object::String(m.cpu_family.clone()))
    } else {
        Err("Not machine info".to_string())
    }
}

fn machine_cpu(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::MachineInfo(m) = obj {
        Ok(Object::String(m.cpu.clone()))
    } else {
        Err("Not machine info".to_string())
    }
}

fn machine_endian(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::MachineInfo(m) = obj {
        Ok(Object::String(m.endian.clone()))
    } else {
        Err("Not machine info".to_string())
    }
}

fn machine_kernel(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::MachineInfo(m) = obj {
        Ok(Object::String(m.kernel.clone()))
    } else {
        Err("Not machine info".to_string())
    }
}

fn machine_subsystem(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::MachineInfo(m) = obj {
        Ok(Object::String(m.subsystem.clone()))
    } else {
        Err("Not machine info".to_string())
    }
}

// ---- Subproject methods ----

fn subproject_found(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::Subproject(sp) = obj {
        Ok(Object::Bool(sp.found))
    } else {
        Err("Not a subproject".to_string())
    }
}

fn subproject_get_variable(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Subproject(sp) = obj {
        let positional = VM::get_positional_args(args);
        let name = positional
            .first()
            .map(|v| v.to_string_value())
            .unwrap_or_default();
        let default = positional.get(1);
        match sp.variables.get(&name) {
            Some(val) => Ok(val.clone()),
            None => default.cloned().cloned().ok_or(format!(
                "Variable '{}' not found in subproject '{}'",
                name, sp.name
            )),
        }
    } else {
        Err("Not a subproject".to_string())
    }
}

// ---- Run result methods ----

fn run_result_returncode(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::RunResult(r) = obj {
        Ok(Object::Int(r.returncode))
    } else {
        Err("Not a run result".to_string())
    }
}

fn run_result_compiled(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::RunResult(_r) = obj {
        Ok(Object::Bool(true))
    } else {
        Err("Not a run result".to_string())
    }
}

fn run_result_stdout(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::RunResult(r) = obj {
        Ok(Object::String(r.stdout.clone()))
    } else {
        Err("Not a run result".to_string())
    }
}

fn run_result_stderr(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::RunResult(r) = obj {
        Ok(Object::String(r.stderr.clone()))
    } else {
        Err("Not a run result".to_string())
    }
}

// ---- Generator methods ----

fn generator_process(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Generator(g) = obj {
        let positional = VM::get_positional_args(args);
        let mut sources = Vec::new();
        for arg in &positional {
            match arg {
                Object::String(s) => sources.push(s.clone()),
                Object::File(f) => sources.push(f.path.clone()),
                Object::Array(arr) => {
                    for item in arr {
                        sources.push(item.to_string_value());
                    }
                }
                _ => {}
            }
        }
        let extra_args = VM::get_arg_string_array(args, "extra_args");
        let preserve_path_from =
            VM::get_arg_str(args, "preserve_path_from", usize::MAX).map(String::from);

        Ok(Object::GeneratedList(GeneratedListData {
            generator: g.clone(),
            sources,
            extra_args,
            preserve_path_from,
        }))
    } else {
        Err("Not a generator".to_string())
    }
}

// ---- Both libraries methods ----

fn both_libs_shared(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::BothLibraries(shared, _) = obj {
        Ok(*shared.clone())
    } else {
        Err("Not both_libraries".to_string())
    }
}

fn both_libs_static(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::BothLibraries(_, static_lib) = obj {
        Ok(*static_lib.clone())
    } else {
        Err("Not both_libraries".to_string())
    }
}

fn both_libs_name(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::BothLibraries(shared, _) = obj {
        if let Object::BuildTarget(t) = shared.as_ref() {
            Ok(Object::String(t.name.clone()))
        } else {
            Ok(Object::String(String::new()))
        }
    } else {
        Err("Not both_libraries".to_string())
    }
}

// ---- File methods ----

fn file_full_path(vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::File(f) = obj {
        if f.is_built {
            Ok(Object::String(format!("{}/{}", vm.build_root, f.path)))
        } else if f.subdir.is_empty() {
            Ok(Object::String(format!("{}/{}", vm.source_root, f.path)))
        } else {
            Ok(Object::String(format!(
                "{}/{}/{}",
                vm.source_root, f.subdir, f.path
            )))
        }
    } else {
        Err("Not a file".to_string())
    }
}

fn module_found(_vm: &mut VM, _obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    Ok(Object::Bool(true))
}
// ---- Compiler methods (registered separately) ----

/// Resolve include_directories kwarg to -I flags
fn resolve_include_dirs(vm: &VM, args: &[CallArg]) -> Vec<String> {
    let mut result = Vec::new();
    let inc_value = VM::get_arg_value(args, "include_directories");

    let process_dirs = |data: &IncludeDirsData, result: &mut Vec<String>| {
        for dir in &data.dirs {
            // Add both source and build directory variants
            let source_dir = if vm.current_subdir.is_empty() {
                format!("{}/{}", vm.source_root, dir)
            } else {
                format!("{}/{}/{}", vm.source_root, vm.current_subdir, dir)
            };
            let build_dir = if vm.current_subdir.is_empty() {
                format!("{}/{}", vm.build_root, dir)
            } else {
                format!("{}/{}/{}", vm.build_root, vm.current_subdir, dir)
            };
            result.push(format!("-I{}", source_dir));
            result.push(format!("-I{}", build_dir));
        }
    };

    match inc_value {
        Some(Object::Array(arr)) => {
            for item in arr {
                if let Object::IncludeDirs(data) = item {
                    process_dirs(data, &mut result);
                }
            }
        }
        Some(Object::IncludeDirs(data)) => {
            process_dirs(data, &mut result);
        }
        _ => {}
    }
    result
}

/// Flatten positional arguments into a list of strings.
/// Handles both individual string args and array args containing strings.
fn flatten_positional_strings(args: &[CallArg]) -> Vec<String> {
    let mut result = Vec::new();
    for obj in VM::get_positional_args(args) {
        match obj {
            Object::Array(arr) => {
                for item in arr {
                    result.push(item.to_string_value());
                }
            }
            other => {
                result.push(other.to_string_value());
            }
        }
    }
    result
}

fn register_compiler_methods(vm: &mut VM) {
    vm.method_registry.insert(
        ("compiler".to_string(), "get_id".to_string()),
        compiler_get_id,
    );
    vm.method_registry.insert(
        ("compiler".to_string(), "get_linker_id".to_string()),
        compiler_get_linker_id,
    );
    vm.method_registry.insert(
        ("compiler".to_string(), "version".to_string()),
        compiler_version,
    );
    vm.method_registry.insert(
        ("compiler".to_string(), "cmd_array".to_string()),
        compiler_cmd_array,
    );
    vm.method_registry.insert(
        ("compiler".to_string(), "has_header".to_string()),
        compiler_has_header,
    );
    vm.method_registry.insert(
        ("compiler".to_string(), "has_header_symbol".to_string()),
        compiler_has_header_symbol,
    );
    vm.method_registry.insert(
        ("compiler".to_string(), "check_header".to_string()),
        compiler_check_header,
    );
    vm.method_registry.insert(
        ("compiler".to_string(), "has_function".to_string()),
        compiler_has_function,
    );
    vm.method_registry.insert(
        ("compiler".to_string(), "has_member".to_string()),
        compiler_has_member,
    );
    vm.method_registry.insert(
        ("compiler".to_string(), "has_members".to_string()),
        compiler_has_members,
    );
    vm.method_registry.insert(
        ("compiler".to_string(), "has_type".to_string()),
        compiler_has_type,
    );
    vm.method_registry.insert(
        ("compiler".to_string(), "sizeof".to_string()),
        compiler_sizeof,
    );
    vm.method_registry.insert(
        ("compiler".to_string(), "alignment".to_string()),
        compiler_alignment,
    );
    vm.method_registry.insert(
        ("compiler".to_string(), "compiles".to_string()),
        compiler_compiles,
    );
    vm.method_registry.insert(
        ("compiler".to_string(), "links".to_string()),
        compiler_links,
    );
    vm.method_registry
        .insert(("compiler".to_string(), "runs".to_string()), compiler_runs);
    vm.method_registry.insert(
        ("compiler".to_string(), "has_argument".to_string()),
        compiler_has_argument,
    );
    vm.method_registry.insert(
        ("compiler".to_string(), "has_multi_arguments".to_string()),
        compiler_has_multi_arguments,
    );
    vm.method_registry.insert(
        ("compiler".to_string(), "has_link_argument".to_string()),
        compiler_has_link_argument,
    );
    vm.method_registry.insert(
        (
            "compiler".to_string(),
            "has_multi_link_arguments".to_string(),
        ),
        compiler_has_multi_link_arguments,
    );
    vm.method_registry.insert(
        (
            "compiler".to_string(),
            "first_supported_argument".to_string(),
        ),
        compiler_first_supported_argument,
    );
    vm.method_registry.insert(
        (
            "compiler".to_string(),
            "first_supported_link_argument".to_string(),
        ),
        compiler_first_supported_link_argument,
    );
    vm.method_registry.insert(
        (
            "compiler".to_string(),
            "get_supported_arguments".to_string(),
        ),
        compiler_get_supported_arguments,
    );
    vm.method_registry.insert(
        (
            "compiler".to_string(),
            "get_supported_link_arguments".to_string(),
        ),
        compiler_get_supported_link_arguments,
    );
    vm.method_registry.insert(
        ("compiler".to_string(), "get_define".to_string()),
        compiler_get_define,
    );
    vm.method_registry.insert(
        ("compiler".to_string(), "find_library".to_string()),
        compiler_find_library,
    );
    vm.method_registry.insert(
        ("compiler".to_string(), "has_function_attribute".to_string()),
        compiler_has_function_attribute,
    );
    vm.method_registry.insert(
        ("compiler".to_string(), "get_argument_syntax".to_string()),
        compiler_get_argument_syntax,
    );
    vm.method_registry.insert(
        ("compiler".to_string(), "compute_int".to_string()),
        compiler_compute_int,
    );
    vm.method_registry.insert(
        ("compiler".to_string(), "preprocess".to_string()),
        compiler_preprocess,
    );
    vm.method_registry.insert(
        (
            "compiler".to_string(),
            "symbols_have_underscore_prefix".to_string(),
        ),
        compiler_symbols_have_underscore_prefix,
    );
    vm.method_registry.insert(
        ("compiler".to_string(), "has_define".to_string()),
        compiler_has_define,
    );
    vm.method_registry
        .insert(("compiler".to_string(), "run".to_string()), compiler_run);
}

fn compiler_get_id(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::Compiler(c) = obj {
        Ok(Object::String(c.id.clone()))
    } else {
        Err("Not a compiler".to_string())
    }
}

fn compiler_get_linker_id(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::Compiler(c) = obj {
        Ok(Object::String(c.linker_id.clone()))
    } else {
        Err("Not a compiler".to_string())
    }
}

fn compiler_version(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::Compiler(c) = obj {
        Ok(Object::String(c.version.clone()))
    } else {
        Err("Not a compiler".to_string())
    }
}

fn compiler_cmd_array(_vm: &mut VM, obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    if let Object::Compiler(c) = obj {
        Ok(Object::Array(
            c.cmd.iter().map(|s| Object::String(s.clone())).collect(),
        ))
    } else {
        Err("Not a compiler".to_string())
    }
}

fn compiler_has_header(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Compiler(c) = obj {
        let header = VM::get_positional_args(args)
            .first()
            .map(|v| v.to_string_value())
            .unwrap_or_default();
        Ok(Object::Bool(crate::compilers::check_header(c, &header)))
    } else {
        Err("Not a compiler".to_string())
    }
}

fn compiler_has_header_symbol(
    _vm: &mut VM,
    obj: &Object,
    args: &[CallArg],
) -> Result<Object, String> {
    if let Object::Compiler(c) = obj {
        let positional = VM::get_positional_args(args);
        let header = positional
            .first()
            .map(|v| v.to_string_value())
            .unwrap_or_default();
        let symbol = positional
            .get(1)
            .map(|v| v.to_string_value())
            .unwrap_or_default();
        Ok(Object::Bool(crate::compilers::check_header_symbol(
            c, &header, &symbol, args,
        )))
    } else {
        Err("Not a compiler".to_string())
    }
}

fn compiler_check_header(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    compiler_has_header(_vm, obj, args)
}

fn compiler_has_function(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Compiler(c) = obj {
        let func = VM::get_positional_args(args)
            .first()
            .map(|v| v.to_string_value())
            .unwrap_or_default();
        Ok(Object::Bool(crate::compilers::check_function(
            c, &func, args,
        )))
    } else {
        Err("Not a compiler".to_string())
    }
}

fn compiler_has_member(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Compiler(c) = obj {
        let positional = VM::get_positional_args(args);
        let typename = positional
            .first()
            .map(|v| v.to_string_value())
            .unwrap_or_default();
        let member = positional
            .get(1)
            .map(|v| v.to_string_value())
            .unwrap_or_default();
        Ok(Object::Bool(crate::compilers::check_member(
            c, &typename, &member, args,
        )))
    } else {
        Err("Not a compiler".to_string())
    }
}

fn compiler_has_members(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Compiler(c) = obj {
        let positional = VM::get_positional_args(args);
        let typename = positional
            .first()
            .map(|v| v.to_string_value())
            .unwrap_or_default();
        // All remaining positional args are member names
        for member_obj in positional.iter().skip(1) {
            let member = member_obj.to_string_value();
            if !crate::compilers::check_member(c, &typename, &member, args) {
                return Ok(Object::Bool(false));
            }
        }
        Ok(Object::Bool(true))
    } else {
        Err("Not a compiler".to_string())
    }
}

fn compiler_has_type(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Compiler(c) = obj {
        let typename = VM::get_positional_args(args)
            .first()
            .map(|v| v.to_string_value())
            .unwrap_or_default();
        Ok(Object::Bool(crate::compilers::check_type(
            c, &typename, args,
        )))
    } else {
        Err("Not a compiler".to_string())
    }
}

fn compiler_sizeof(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Compiler(c) = obj {
        let typename = VM::get_positional_args(args)
            .first()
            .map(|v| v.to_string_value())
            .unwrap_or_default();
        Ok(Object::Int(crate::compilers::get_sizeof(
            c, &typename, args,
        )))
    } else {
        Err("Not a compiler".to_string())
    }
}

fn compiler_alignment(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Compiler(c) = obj {
        let typename = VM::get_positional_args(args)
            .first()
            .map(|v| v.to_string_value())
            .unwrap_or_default();
        Ok(Object::Int(crate::compilers::get_alignment(c, &typename)))
    } else {
        Err("Not a compiler".to_string())
    }
}

fn compiler_compiles(vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Compiler(c) = obj {
        let positional = VM::get_positional_args(args);
        let first = positional.first().unwrap_or(&&Object::None);
        // Handle files() returning Array of File objects
        let resolved_first = match first {
            Object::Array(arr) => arr.first().unwrap_or(&Object::None),
            other => other,
        };
        let code = match resolved_first {
            Object::File(f) => {
                // Read code from file - resolve relative to source dir
                let path = if std::path::Path::new(&f.path).is_absolute() {
                    f.path.clone()
                } else if vm.current_subdir.is_empty() {
                    format!("{}/{}", vm.source_root, f.path)
                } else {
                    format!("{}/{}/{}", vm.source_root, vm.current_subdir, f.path)
                };
                std::fs::read_to_string(&path).unwrap_or_default()
            }
            other => other.to_string_value(),
        };
        // Resolve include_directories to -I flags and add as extra args
        let inc_args = resolve_include_dirs(vm, args);
        let mut augmented_args: Vec<CallArg> = args.to_vec();
        if !inc_args.is_empty() {
            // Merge include dir flags into the "args" kwarg
            let mut extra = crate::compilers::extra_args_from_callargs(args);
            extra.extend(inc_args);
            // Remove existing "args" kwarg and add merged one
            augmented_args.retain(|a| a.name.as_deref() != Some("args"));
            augmented_args.push(CallArg {
                name: Some("args".to_string()),
                value: Object::Array(extra.into_iter().map(Object::String).collect()),
            });
        }
        Ok(Object::Bool(crate::compilers::try_compile(
            c,
            &code,
            &augmented_args,
        )))
    } else {
        Err("Not a compiler".to_string())
    }
}

fn compiler_links(vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Compiler(c) = obj {
        let positional = VM::get_positional_args(args);
        let first = positional.first().unwrap_or(&&Object::None);
        // Handle files() returning Array of File objects
        let resolved_first = match first {
            Object::Array(arr) => arr.first().unwrap_or(&Object::None),
            other => other,
        };
        let code = match resolved_first {
            Object::File(f) => {
                let path = if std::path::Path::new(&f.path).is_absolute() {
                    f.path.clone()
                } else if vm.current_subdir.is_empty() {
                    format!("{}/{}", vm.source_root, f.path)
                } else {
                    format!("{}/{}/{}", vm.source_root, vm.current_subdir, f.path)
                };
                std::fs::read_to_string(&path).unwrap_or_default()
            }
            other => other.to_string_value(),
        };
        Ok(Object::Bool(crate::compilers::try_link(c, &code, args)))
    } else {
        Err("Not a compiler".to_string())
    }
}

fn compiler_runs(vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Compiler(c) = obj {
        let positional = VM::get_positional_args(args);
        let first = positional.first().unwrap_or(&&Object::None);
        // Handle files() returning Array of File objects
        let resolved_first = match first {
            Object::Array(arr) => arr.first().unwrap_or(&Object::None),
            other => other,
        };
        let code = match resolved_first {
            Object::File(f) => {
                let path = if std::path::Path::new(&f.path).is_absolute() {
                    f.path.clone()
                } else if vm.current_subdir.is_empty() {
                    format!("{}/{}", vm.source_root, f.path)
                } else {
                    format!("{}/{}/{}", vm.source_root, vm.current_subdir, f.path)
                };
                std::fs::read_to_string(&path).unwrap_or_default()
            }
            other => other.to_string_value(),
        };
        let result = crate::compilers::try_run(c, &code, args);
        Ok(Object::RunResult(result))
    } else {
        Err("Not a compiler".to_string())
    }
}

fn compiler_has_argument(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Compiler(c) = obj {
        let arg = VM::get_positional_args(args)
            .first()
            .map(|v| v.to_string_value())
            .unwrap_or_default();
        Ok(Object::Bool(crate::compilers::has_argument(c, &arg)))
    } else {
        Err("Not a compiler".to_string())
    }
}

fn compiler_has_multi_arguments(
    _vm: &mut VM,
    obj: &Object,
    args: &[CallArg],
) -> Result<Object, String> {
    if let Object::Compiler(c) = obj {
        let test_args = flatten_positional_strings(args);
        Ok(Object::Bool(crate::compilers::has_multi_arguments(
            c, &test_args,
        )))
    } else {
        Err("Not a compiler".to_string())
    }
}

fn compiler_has_link_argument(
    _vm: &mut VM,
    obj: &Object,
    args: &[CallArg],
) -> Result<Object, String> {
    if let Object::Compiler(c) = obj {
        let arg = VM::get_positional_args(args)
            .first()
            .map(|v| v.to_string_value())
            .unwrap_or_default();
        Ok(Object::Bool(crate::compilers::has_link_argument(c, &arg)))
    } else {
        Err("Not a compiler".to_string())
    }
}

fn compiler_has_multi_link_arguments(
    _vm: &mut VM,
    obj: &Object,
    args: &[CallArg],
) -> Result<Object, String> {
    if let Object::Compiler(c) = obj {
        let test_args = flatten_positional_strings(args);
        Ok(Object::Bool(crate::compilers::has_multi_link_arguments(
            c, &test_args,
        )))
    } else {
        Err("Not a compiler".to_string())
    }
}

fn compiler_first_supported_argument(
    _vm: &mut VM,
    obj: &Object,
    args: &[CallArg],
) -> Result<Object, String> {
    if let Object::Compiler(c) = obj {
        let test_args = flatten_positional_strings(args);
        for arg in &test_args {
            if crate::compilers::has_argument(c, arg) {
                return Ok(Object::Array(vec![Object::String(arg.clone())]));
            }
        }
        Ok(Object::Array(Vec::new()))
    } else {
        Err("Not a compiler".to_string())
    }
}

fn compiler_first_supported_link_argument(
    _vm: &mut VM,
    obj: &Object,
    args: &[CallArg],
) -> Result<Object, String> {
    if let Object::Compiler(c) = obj {
        let test_args = flatten_positional_strings(args);
        for arg in &test_args {
            if crate::compilers::has_link_argument(c, arg) {
                return Ok(Object::Array(vec![Object::String(arg.clone())]));
            }
        }
        Ok(Object::Array(Vec::new()))
    } else {
        Err("Not a compiler".to_string())
    }
}

fn compiler_get_supported_arguments(
    _vm: &mut VM,
    obj: &Object,
    args: &[CallArg],
) -> Result<Object, String> {
    if let Object::Compiler(c) = obj {
        let test_args = flatten_positional_strings(args);
        let supported: Vec<Object> = test_args
            .into_iter()
            .filter(|a| crate::compilers::has_argument(c, a))
            .map(Object::String)
            .collect();
        Ok(Object::Array(supported))
    } else {
        Err("Not a compiler".to_string())
    }
}

fn compiler_get_supported_link_arguments(
    _vm: &mut VM,
    obj: &Object,
    args: &[CallArg],
) -> Result<Object, String> {
    if let Object::Compiler(c) = obj {
        let test_args = flatten_positional_strings(args);
        let supported: Vec<Object> = test_args
            .into_iter()
            .filter(|a| crate::compilers::has_link_argument(c, a))
            .map(Object::String)
            .collect();
        Ok(Object::Array(supported))
    } else {
        Err("Not a compiler".to_string())
    }
}

fn compiler_get_define(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Compiler(c) = obj {
        let name = VM::get_positional_args(args)
            .first()
            .map(|v| v.to_string_value())
            .unwrap_or_default();
        Ok(Object::String(crate::compilers::get_define(c, &name, args)))
    } else {
        Err("Not a compiler".to_string())
    }
}

fn compiler_find_library(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Compiler(c) = obj {
        let name = VM::get_positional_args(args)
            .first()
            .map(|v| v.to_string_value())
            .unwrap_or_default();
        let required = VM::get_arg_bool(args, "required", true);
        let dirs = VM::get_arg_string_array(args, "dirs");
        let result = crate::compilers::find_library(c, &name, &dirs);
        if result.is_none() && required {
            return Err(format!("Library '{}' not found", name));
        }
        Ok(result.unwrap_or(Object::Dependency(DependencyData::not_found(&name))))
    } else {
        Err("Not a compiler".to_string())
    }
}

fn compiler_has_function_attribute(
    _vm: &mut VM,
    obj: &Object,
    args: &[CallArg],
) -> Result<Object, String> {
    if let Object::Compiler(c) = obj {
        let attr = VM::get_positional_args(args)
            .first()
            .map(|v| v.to_string_value())
            .unwrap_or_default();
        Ok(Object::Bool(crate::compilers::has_function_attribute(
            c, &attr,
        )))
    } else {
        Err("Not a compiler".to_string())
    }
}

fn compiler_get_argument_syntax(
    _vm: &mut VM,
    obj: &Object,
    _args: &[CallArg],
) -> Result<Object, String> {
    if let Object::Compiler(c) = obj {
        let syntax = if c.id.contains("msvc") || c.id.contains("cl") {
            "msvc"
        } else {
            "gcc"
        };
        Ok(Object::String(syntax.to_string()))
    } else {
        Err("Not a compiler".to_string())
    }
}

fn compiler_compute_int(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Compiler(c) = obj {
        let expr = VM::get_positional_args(args)
            .first()
            .map(|v| v.to_string_value())
            .unwrap_or_default();
        Ok(Object::Int(crate::compilers::compute_int(c, &expr, args)))
    } else {
        Err("Not a compiler".to_string())
    }
}

fn compiler_preprocess(_vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let mut sources = Vec::new();
    for arg in VM::get_positional_args(args) {
        if let Object::String(s) = arg {
            sources.push(Object::File(FileData {
                path: s.clone(),
                subdir: String::new(),
                is_built: true,
            }));
        }
    }
    Ok(Object::Array(sources))
}

fn compiler_symbols_have_underscore_prefix(
    _vm: &mut VM,
    _obj: &Object,
    _args: &[CallArg],
) -> Result<Object, String> {
    // On Linux, symbols don't have underscore prefix
    Ok(Object::Bool(false))
}

fn compiler_has_define(_vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let define = positional
        .first()
        .map(|v| v.to_string_value())
        .unwrap_or_default();
    // Common compiler defines that are typically always set
    let result = matches!(
        define.as_str(),
        "__GNUC__"
            | "__STDC__"
            | "__linux__"
            | "__unix__"
            | "__x86_64__"
            | "__amd64__"
            | "__LP64__"
            | "__SIZEOF_INT__"
            | "__SIZEOF_POINTER__"
    );
    Ok(Object::Bool(result))
}

fn compiler_run(vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    if let Object::Compiler(c) = obj {
        let positional = VM::get_positional_args(args);
        let first = positional.first().unwrap_or(&&Object::None);
        // Handle files() returning Array of File objects
        let resolved_first = match first {
            Object::Array(arr) => arr.first().unwrap_or(&Object::None),
            other => other,
        };
        let code = match resolved_first {
            Object::File(f) => {
                let path = if std::path::Path::new(&f.path).is_absolute() {
                    f.path.clone()
                } else if vm.current_subdir.is_empty() {
                    format!("{}/{}", vm.source_root, f.path)
                } else {
                    format!("{}/{}/{}", vm.source_root, vm.current_subdir, f.path)
                };
                std::fs::read_to_string(&path).unwrap_or_default()
            }
            other => other.to_string_value(),
        };
        let result = crate::compilers::try_run(c, &code, args);
        Ok(Object::RunResult(result))
    } else {
        Err("Not a compiler".to_string())
    }
}
