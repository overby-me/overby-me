/// Java module: generate_native_headers, native_headers.
use crate::objects::*;
use crate::vm::*;

pub fn register(vm: &mut VM) {
    vm.method_registry.insert(
        (
            "module".to_string(),
            "java.generate_native_headers".to_string(),
        ),
        java_generate_native_headers,
    );
    vm.method_registry.insert(
        ("module".to_string(), "java.native_headers".to_string()),
        java_native_headers,
    );
}

fn java_generate_native_headers(
    vm: &mut VM,
    _obj: &Object,
    args: &[CallArg],
) -> Result<Object, String> {
    java_native_headers_impl(vm, args)
}

fn java_native_headers(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    java_native_headers_impl(vm, args)
}

fn java_native_headers_impl(vm: &mut VM, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);

    let classes: Vec<String> = match positional.first() {
        Some(Object::Array(arr)) => arr
            .iter()
            .filter_map(|o| {
                if let Object::String(s) = o {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .collect(),
        _ => VM::get_arg_string_array(args, "classes"),
    };

    let package = VM::get_arg_str(args, "package", 99)
        .unwrap_or("")
        .to_string();

    if classes.is_empty() {
        return Err("java.native_headers: at least one class required".to_string());
    }

    let mut outputs = Vec::new();
    for class in &classes {
        let header_name = if package.is_empty() {
            format!("{}.h", class.replace('.', "_"))
        } else {
            format!(
                "{}_{}.h",
                package.replace('.', "_"),
                class.replace('.', "_")
            )
        };
        outputs.push(header_name);
    }

    let mut command = vec![
        "javac".to_string(),
        "-h".to_string(),
        "@OUTDIR@".to_string(),
    ];
    for class in &classes {
        let full_class = if package.is_empty() {
            class.clone()
        } else {
            format!("{}.{}", package, class)
        };
        command.push(full_class);
    }

    let name = "java_native_headers".to_string();
    let id = format!(
        "java_headers_{}",
        classes.first().unwrap_or(&"unknown".to_string())
    );

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
        input: Vec::new(),
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
