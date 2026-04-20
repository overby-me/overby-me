use crate::objects::*;
use crate::vm::*;
/// Dependency detection: pkg-config, cmake, config-tool, system library.
use std::collections::HashMap;
use std::process::Command;

/// Try to find a dependency using various methods.
pub fn find_dependency(vm: &mut VM, name: &str, args: &[CallArg]) -> Option<Object> {
    let method = VM::get_arg_str(args, "method", usize::MAX).unwrap_or("auto");
    let version_req = VM::get_arg_str(args, "version", usize::MAX);
    let modules = VM::get_arg_string_array(args, "modules");
    let components = VM::get_arg_string_array(args, "components");

    // Check for internal/declared dependencies first
    if let Some(dep) = vm.build_data.dependencies.get(name) {
        return Some(dep.clone());
    }

    // Try subproject fallback
    if let Some(fallback) = VM::get_arg_value(args, "fallback") {
        if let Some(dep) = try_fallback(vm, name, fallback) {
            return Some(dep);
        }
    }

    match method {
        "pkg-config" => find_pkgconfig(name, version_req, &modules),
        "cmake" => find_cmake(name, version_req, &components),
        "config-tool" => find_config_tool(name, version_req),
        "system" => find_system_library(name),
        "auto" | _ => {
            // Try methods in order
            find_pkgconfig(name, version_req, &modules)
                .or_else(|| find_cmake(name, version_req, &components))
                .or_else(|| find_config_tool(name, version_req))
                .or_else(|| find_system_library(name))
        }
    }
}

fn find_pkgconfig(name: &str, version_req: Option<&str>, modules: &[String]) -> Option<Object> {
    let pkg_name = if modules.is_empty() {
        name.to_string()
    } else {
        modules[0].clone()
    };

    let output = Command::new("pkg-config")
        .args(["--modversion", &pkg_name])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Check version requirement
    if let Some(req) = version_req {
        if !crate::options::version_compare(&version, req) {
            return None;
        }
    }

    // Get compile flags
    let mut compile_args = Vec::new();
    let mut include_dirs = Vec::new();
    if let Ok(output) = Command::new("pkg-config")
        .args(["--cflags", &pkg_name])
        .output()
    {
        if output.status.success() {
            let flags = String::from_utf8_lossy(&output.stdout);
            for flag in flags.split_whitespace() {
                if flag.starts_with("-I") {
                    include_dirs.push(flag[2..].to_string());
                } else {
                    compile_args.push(flag.to_string());
                }
            }
        }
    }

    // Get link flags
    let mut link_args = Vec::new();
    if let Ok(output) = Command::new("pkg-config")
        .args(["--libs", &pkg_name])
        .output()
    {
        if output.status.success() {
            let flags = String::from_utf8_lossy(&output.stdout);
            for flag in flags.split_whitespace() {
                link_args.push(flag.to_string());
            }
        }
    }

    // Get variables
    let mut variables = HashMap::new();
    if let Ok(output) = Command::new("pkg-config")
        .args(["--print-variables", &pkg_name])
        .output()
    {
        if output.status.success() {
            let vars_text = String::from_utf8_lossy(&output.stdout);
            for var in vars_text.lines() {
                let var = var.trim();
                if !var.is_empty() {
                    if let Ok(val_output) = Command::new("pkg-config")
                        .args(["--variable", var, &pkg_name])
                        .output()
                    {
                        if val_output.status.success() {
                            let val = String::from_utf8_lossy(&val_output.stdout)
                                .trim()
                                .to_string();
                            variables.insert(var.to_string(), val);
                        }
                    }
                }
            }
        }
    }

    Some(Object::Dependency(DependencyData {
        name: name.to_string(),
        found: true,
        version,
        compile_args,
        link_args,
        sources: Vec::new(),
        include_dirs,
        dependencies: Vec::new(),
        variables,
        is_internal: false,
        kind: String::new(),
    }))
}

fn find_cmake(name: &str, version_req: Option<&str>, components: &[String]) -> Option<Object> {
    // Try cmake --find-package
    let mut args = vec![
        "--find-package".to_string(),
        format!("-DNAME={}", name),
        "-DCOMPILER_ID=GNU".to_string(),
        "-DLANGUAGE=C".to_string(),
        "-DMODE=EXIST".to_string(),
    ];

    if !components.is_empty() {
        args.push(format!("-DCOMPONENTS={}", components.join(";")));
    }

    let output = Command::new("cmake").args(&args).output().ok()?;
    if !output.status.success() {
        return None;
    }

    // Get compile flags
    let compile_output = Command::new("cmake")
        .args([
            "--find-package",
            &format!("-DNAME={}", name),
            "-DCOMPILER_ID=GNU",
            "-DLANGUAGE=C",
            "-DMODE=COMPILE",
        ])
        .output()
        .ok()?;

    let compile_args: Vec<String> = if compile_output.status.success() {
        String::from_utf8_lossy(&compile_output.stdout)
            .split_whitespace()
            .map(String::from)
            .collect()
    } else {
        Vec::new()
    };

    // Get link flags
    let link_output = Command::new("cmake")
        .args([
            "--find-package",
            &format!("-DNAME={}", name),
            "-DCOMPILER_ID=GNU",
            "-DLANGUAGE=C",
            "-DMODE=LINK",
        ])
        .output()
        .ok()?;

    let link_args: Vec<String> = if link_output.status.success() {
        String::from_utf8_lossy(&link_output.stdout)
            .split_whitespace()
            .map(String::from)
            .collect()
    } else {
        Vec::new()
    };

    Some(Object::Dependency(DependencyData {
        name: name.to_string(),
        found: true,
        version: String::new(),
        compile_args,
        link_args,
        sources: Vec::new(),
        include_dirs: Vec::new(),
        dependencies: Vec::new(),
        variables: HashMap::new(),
        is_internal: false,
        kind: String::new(),
    }))
}

fn find_config_tool(name: &str, version_req: Option<&str>) -> Option<Object> {
    let config_tool = format!("{}-config", name);
    let output = Command::new(&config_tool).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }

    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if let Some(req) = version_req {
        if !crate::options::version_compare(&version, req) {
            return None;
        }
    }

    let compile_args: Vec<String> = Command::new(&config_tool)
        .arg("--cflags")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();

    let link_args: Vec<String> = Command::new(&config_tool)
        .arg("--libs")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();

    Some(Object::Dependency(DependencyData {
        name: name.to_string(),
        found: true,
        version,
        compile_args,
        link_args,
        sources: Vec::new(),
        include_dirs: Vec::new(),
        dependencies: Vec::new(),
        variables: HashMap::new(),
        is_internal: false,
        kind: String::new(),
    }))
}

fn find_system_library(name: &str) -> Option<Object> {
    // Try to link with -l<name>
    let code = "int main(void) { return 0; }";
    let output = Command::new("cc")
        .args(["-x", "c", "-", "-o", "/dev/null", &format!("-l{}", name)])
        .stdin(std::process::Stdio::piped())
        .output();

    match output {
        Ok(o) if o.status.success() => Some(Object::Dependency(DependencyData {
            name: name.to_string(),
            found: true,
            version: String::new(),
            compile_args: Vec::new(),
            link_args: vec![format!("-l{}", name)],
            sources: Vec::new(),
            include_dirs: Vec::new(),
            dependencies: Vec::new(),
            variables: HashMap::new(),
            is_internal: false,
            kind: String::new(),
        })),
        _ => None,
    }
}

fn try_fallback(_vm: &mut VM, _name: &str, fallback: &Object) -> Option<Object> {
    match fallback {
        Object::Array(_arr) => {
            // Subproject fallback handling — deferred to subproject loading
            None
        }
        Object::String(_sp_name) => {
            // Auto-provided dependency from subproject
            None
        }
        _ => None,
    }
}
