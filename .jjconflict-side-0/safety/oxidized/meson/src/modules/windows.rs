/// Windows resource compilation module.
use crate::objects::*;
use crate::vm::*;

pub fn register(vm: &mut VM) {
    vm.method_registry.insert(
        (
            "module".to_string(),
            "windows.compile_resources".to_string(),
        ),
        windows_compile_resources,
    );
}

fn windows_compile_resources(
    vm: &mut VM,
    _obj: &Object,
    args: &[CallArg],
) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);

    // Collect .rc source files from positional args
    let sources: Vec<String> = positional
        .iter()
        .filter_map(|o| match o {
            Object::String(s) => Some(s.clone()),
            Object::File(f) => Some(f.path.clone()),
            _ => None,
        })
        .collect();

    if sources.is_empty() {
        return Err("windows.compile_resources: at least one .rc file required".to_string());
    }

    let args_list = VM::get_arg_string_array(args, "args");
    let include_directories = VM::get_arg_string_array(args, "include_directories");
    let dependencies = match VM::get_arg_value(args, "dependencies") {
        Some(Object::Array(arr)) => arr.clone(),
        _ => Vec::new(),
    };

    let mut outputs = Vec::new();
    let mut _all_commands: Vec<Vec<String>> = Vec::new();

    for src in &sources {
        let stem = std::path::Path::new(src)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "resource".to_string());
        let output = format!("{}.res.o", stem);
        outputs.push(output.clone());

        let mut command = vec!["windres".to_string()];
        for arg in &args_list {
            command.push(arg.clone());
        }
        for dir in &include_directories {
            command.push(format!("-I{}", dir));
        }
        command.push(src.clone());
        command.push(output);
    }

    let name = format!(
        "windows_resources_{}",
        sources
            .first()
            .map(|s| {
                std::path::Path::new(s)
                    .file_stem()
                    .map(|st| st.to_string_lossy().to_string())
                    .unwrap_or_else(|| "rc".to_string())
            })
            .unwrap_or_else(|| "rc".to_string())
    );

    let id = format!("win_rc_{}", name);
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
            "windres".to_string(),
            "@INPUT@".to_string(),
            "@OUTPUT@".to_string(),
        ],
        input: sources,
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
