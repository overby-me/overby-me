/// Qt helpers module (qt4, qt5, qt6): has_tools, preprocess, compile_translations,
/// compile_resources, compile_moc, compile_ui.
use crate::objects::*;
use crate::vm::*;

pub fn register(vm: &mut VM) {
    // Register for qt4, qt5, qt6
    for prefix in &["qt4", "qt5", "qt6"] {
        vm.method_registry.insert(
            ("module".to_string(), format!("{}.has_tools", prefix)),
            qt_has_tools,
        );
        vm.method_registry.insert(
            ("module".to_string(), format!("{}.preprocess", prefix)),
            qt_preprocess,
        );
        vm.method_registry.insert(
            (
                "module".to_string(),
                format!("{}.compile_translations", prefix),
            ),
            qt_compile_translations,
        );
        vm.method_registry.insert(
            (
                "module".to_string(),
                format!("{}.compile_resources", prefix),
            ),
            qt_compile_resources,
        );
        vm.method_registry.insert(
            ("module".to_string(), format!("{}.compile_moc", prefix)),
            qt_compile_moc,
        );
        vm.method_registry.insert(
            ("module".to_string(), format!("{}.compile_ui", prefix)),
            qt_compile_ui,
        );
    }
}

fn qt_version_from_module(obj: &Object) -> &str {
    if let Object::Module(name) = obj {
        match name.as_str() {
            "qt4" => "4",
            "qt5" => "5",
            "qt6" => "6",
            _ => "5",
        }
    } else {
        "5"
    }
}

fn qt_has_tools(_vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let ver = qt_version_from_module(obj);
    let required = VM::get_arg_bool(args, "required", false);
    let _method = VM::get_arg_str(args, "method", 99).unwrap_or("auto");

    // Check if moc/uic/rcc are available
    let tools = ["moc", "uic", "rcc"];
    for tool in &tools {
        let tool_name = if ver == "4" {
            format!("{}-qt4", tool)
        } else {
            format!("{}{}", tool, if ver == "5" || ver == "6" { "" } else { "" })
        };

        let result = std::process::Command::new(&tool_name)
            .arg("--version")
            .output();

        if result.is_err() || !result.unwrap().status.success() {
            // Try without version suffix
            let result2 = std::process::Command::new(tool).arg("--version").output();
            if result2.is_err() || !result2.unwrap().status.success() {
                if required {
                    return Err(format!("Qt{} tool '{}' not found", ver, tool));
                }
                return Ok(Object::Bool(false));
            }
        }
    }

    Ok(Object::Bool(true))
}

fn qt_preprocess(vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let ver = qt_version_from_module(obj);
    let moc_headers = VM::get_arg_string_array(args, "moc_headers");
    let moc_sources = VM::get_arg_string_array(args, "moc_sources");
    let ui_files = VM::get_arg_string_array(args, "ui_files");
    let qresources = VM::get_arg_string_array(args, "qresources");
    let _moc_extra_arguments = VM::get_arg_string_array(args, "moc_extra_arguments");
    let _uic_extra_arguments = VM::get_arg_string_array(args, "uic_extra_arguments");
    let _rcc_extra_arguments = VM::get_arg_string_array(args, "rcc_extra_arguments");
    let _include_directories = VM::get_arg_string_array(args, "include_directories");
    let dependencies = match VM::get_arg_value(args, "dependencies") {
        Some(Object::Array(arr)) => arr.clone(),
        _ => Vec::new(),
    };

    let mut outputs = Vec::new();

    // Process MOC headers
    for header in &moc_headers {
        let stem = std::path::Path::new(header)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let output = format!("moc_{}.cpp", stem);
        outputs.push(output);
    }

    // Process MOC sources
    for source in &moc_sources {
        let stem = std::path::Path::new(source)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let output = format!("{}.moc", stem);
        outputs.push(output);
    }

    // Process UI files
    for ui in &ui_files {
        let stem = std::path::Path::new(ui)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let output = format!("ui_{}.h", stem);
        outputs.push(output);
    }

    // Process QRC files
    for qrc in &qresources {
        let stem = std::path::Path::new(qrc)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let output = format!("qrc_{}.cpp", stem);
        outputs.push(output);
    }

    let name = format!("qt{}_preprocess", ver);
    let id = format!(
        "qt{}_preprocess_{}",
        ver,
        outputs.first().unwrap_or(&"empty".to_string())
    );

    let ct = CustomTargetRef {
        name: name.clone(),
        id: id.clone(),
        outputs: outputs.clone(),
        subdir: vm.current_subdir.clone(),
    };

    let mut all_inputs = Vec::new();
    all_inputs.extend(moc_headers);
    all_inputs.extend(moc_sources);
    all_inputs.extend(ui_files);
    all_inputs.extend(qresources);

    vm.build_data.custom_targets.push(CustomTarget {
        name,
        id,
        command: vec!["moc".to_string()],
        input: all_inputs,
        output: outputs,
        depends: dependencies,
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

fn qt_compile_translations(vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let ver = qt_version_from_module(obj);
    let ts_files = VM::get_arg_string_array(args, "ts_files");
    let install = VM::get_arg_bool(args, "install", false);
    let install_dir = VM::get_arg_str(args, "install_dir", 99)
        .map(|s| s.to_string())
        .unwrap_or_else(|| "share/locale".to_string());
    let build_by_default = VM::get_arg_bool(args, "build_by_default", true);
    let _rcc_extra_arguments = VM::get_arg_string_array(args, "rcc_extra_arguments");

    let mut outputs = Vec::new();
    for ts in &ts_files {
        let stem = std::path::Path::new(ts)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        outputs.push(format!("{}.qm", stem));
    }

    let name = format!("qt{}_translations", ver);
    let id = format!("qt{}_translations_{}", ver, outputs.len());

    let ct = CustomTargetRef {
        name: name.clone(),
        id: id.clone(),
        outputs: outputs.clone(),
        subdir: vm.current_subdir.clone(),
    };

    vm.build_data.custom_targets.push(CustomTarget {
        name,
        id,
        command: vec![
            "lrelease".to_string(),
            "@INPUT@".to_string(),
            "-qm".to_string(),
            "@OUTPUT@".to_string(),
        ],
        input: ts_files,
        output: outputs,
        depends: Vec::new(),
        depend_files: Vec::new(),
        depfile: None,
        capture: false,
        feed: false,
        install,
        install_dir: vec![install_dir],
        install_tag: Vec::new(),
        build_by_default,
        build_always_stale: false,
        env: std::collections::HashMap::new(),
        subdir: vm.current_subdir.clone(),
    });

    Ok(Object::CustomTarget(ct))
}

fn qt_compile_resources(vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let ver = qt_version_from_module(obj);
    let positional = VM::get_positional_args(args);

    let name = match positional.first() {
        Some(Object::String(s)) => s.clone(),
        _ => VM::get_arg_str(args, "name", 0)
            .unwrap_or("qt_resources")
            .to_string(),
    };

    let sources = VM::get_arg_string_array(args, "sources");
    let extra_args = VM::get_arg_string_array(args, "extra_args");

    let output = format!("qrc_{}.cpp", name);

    let mut command = vec!["rcc".to_string()];
    command.extend(extra_args);
    command.push("-o".to_string());
    command.push("@OUTPUT@".to_string());
    command.push("@INPUT@".to_string());

    let id = format!("qt{}_rcc_{}", ver, name);
    let ct = CustomTargetRef {
        name: name.clone(),
        id: id.clone(),
        outputs: vec![output.clone()],
        subdir: vm.current_subdir.clone(),
    };

    vm.build_data.custom_targets.push(CustomTarget {
        name,
        id,
        command,
        input: sources,
        output: vec![output],
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

fn qt_compile_moc(vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let ver = qt_version_from_module(obj);
    let headers = VM::get_arg_string_array(args, "headers");
    let sources = VM::get_arg_string_array(args, "sources");
    let extra_args = VM::get_arg_string_array(args, "extra_args");
    let include_directories = VM::get_arg_string_array(args, "include_directories");
    let dependencies = match VM::get_arg_value(args, "dependencies") {
        Some(Object::Array(arr)) => arr.clone(),
        _ => Vec::new(),
    };
    let _preserve_paths = VM::get_arg_bool(args, "preserve_paths", false);

    let mut outputs = Vec::new();
    let mut inputs = Vec::new();

    for header in &headers {
        let stem = std::path::Path::new(header)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        outputs.push(format!("moc_{}.cpp", stem));
        inputs.push(header.clone());
    }

    for source in &sources {
        let stem = std::path::Path::new(source)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        outputs.push(format!("{}.moc", stem));
        inputs.push(source.clone());
    }

    let name = format!("qt{}_moc", ver);
    let id = format!("qt{}_moc_{}", ver, outputs.len());

    let mut command = vec!["moc".to_string()];
    command.extend(extra_args);
    for dir in &include_directories {
        command.push(format!("-I{}", dir));
    }
    command.push("@INPUT@".to_string());
    command.push("-o".to_string());
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
        input: inputs,
        output: outputs,
        depends: dependencies,
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

fn qt_compile_ui(vm: &mut VM, obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let ver = qt_version_from_module(obj);
    let sources = VM::get_arg_string_array(args, "sources");
    let extra_args = VM::get_arg_string_array(args, "extra_args");
    let _preserve_paths = VM::get_arg_bool(args, "preserve_paths", false);

    let mut outputs = Vec::new();
    for ui in &sources {
        let stem = std::path::Path::new(ui)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        outputs.push(format!("ui_{}.h", stem));
    }

    let name = format!("qt{}_uic", ver);
    let id = format!("qt{}_uic_{}", ver, outputs.len());

    let mut command = vec!["uic".to_string()];
    command.extend(extra_args);
    command.push("@INPUT@".to_string());
    command.push("-o".to_string());
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
