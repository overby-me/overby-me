/// Rust helpers module: test, bindgen, proc_macro.
use crate::objects::*;
use crate::vm::*;

pub fn register(vm: &mut VM) {
    vm.method_registry
        .insert(("module".to_string(), "rust.test".to_string()), rust_test);
    vm.method_registry.insert(
        ("module".to_string(), "rust.bindgen".to_string()),
        rust_bindgen,
    );
    vm.method_registry.insert(
        ("module".to_string(), "rust.proc_macro".to_string()),
        rust_proc_macro,
    );
}

fn rust_test(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);

    let name = match positional.first() {
        Some(Object::String(s)) => s.clone(),
        _ => return Err("rust.test: first argument must be a test name".to_string()),
    };

    let target = match positional.get(1) {
        Some(obj @ Object::BuildTarget(_)) => obj.clone(),
        _ => return Err("rust.test: second argument must be a build target".to_string()),
    };

    let args_list = VM::get_arg_string_array(args, "args");
    let should_fail = VM::get_arg_bool(args, "should_fail", false);
    let timeout = VM::get_arg_int(args, "timeout", 30);
    let suite = VM::get_arg_string_array(args, "suite");
    let protocol = VM::get_arg_str(args, "protocol", 99)
        .unwrap_or("rust")
        .to_string();
    let is_parallel = VM::get_arg_bool(args, "is_parallel", true);
    let dependencies = match VM::get_arg_value(args, "dependencies") {
        Some(Object::Array(arr)) => arr.clone(),
        _ => Vec::new(),
    };

    let env_map = match VM::get_arg_value(args, "env") {
        Some(Object::Environment(e)) => e.to_map(),
        _ => std::collections::HashMap::new(),
    };

    vm.build_data.tests.push(TestDef {
        name,
        exe: target.clone(),
        args: args_list,
        env: env_map,
        should_fail,
        timeout,
        workdir: VM::get_arg_str(args, "workdir", 99).map(|s| s.to_string()),
        protocol,
        priority: VM::get_arg_int(args, "priority", 0),
        suite,
        depends: dependencies,
        is_parallel,
        verbose: VM::get_arg_bool(args, "verbose", false),
    });

    Ok(Object::None)
}

fn rust_bindgen(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);

    let input = match positional.first() {
        Some(Object::String(s)) => s.clone(),
        Some(Object::File(f)) => f.path.clone(),
        _ => VM::get_arg_str(args, "input", 0)
            .ok_or("rust.bindgen: 'input' is required")?
            .to_string(),
    };

    let output = match positional.get(1) {
        Some(Object::String(s)) => s.clone(),
        _ => VM::get_arg_str(args, "output", 1)
            .ok_or("rust.bindgen: 'output' is required")?
            .to_string(),
    };

    let c_args = VM::get_arg_string_array(args, "c_args");
    let args_list = VM::get_arg_string_array(args, "args");
    let include_directories = VM::get_arg_string_array(args, "include_directories");
    let dependencies = match VM::get_arg_value(args, "dependencies") {
        Some(Object::Array(arr)) => arr.clone(),
        _ => Vec::new(),
    };

    let mut command = vec![
        "bindgen".to_string(),
        "--output".to_string(),
        "@OUTPUT@".to_string(),
    ];
    for arg in &args_list {
        command.push(arg.clone());
    }
    command.push("@INPUT@".to_string());
    command.push("--".to_string());
    for arg in &c_args {
        command.push(arg.clone());
    }
    for dir in &include_directories {
        command.push(format!("-I{}", dir));
    }

    let id = format!("rust_bindgen_{}", output.replace('/', "_"));
    let ct = CustomTargetRef {
        name: output.clone(),
        id: id.clone(),
        outputs: vec![output.clone()],
        subdir: vm.current_subdir.clone(),
    };

    vm.build_data.custom_targets.push(CustomTarget {
        name: output.clone(),
        id,
        command,
        input: vec![input],
        output: vec![output],
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

fn rust_proc_macro(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);

    let name = match positional.first() {
        Some(Object::String(s)) => s.clone(),
        _ => return Err("rust.proc_macro: first argument must be a name".to_string()),
    };

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

    let rust_args = VM::get_arg_string_array(args, "rust_args");

    let id = format!("rust_proc_macro_{}", name);
    let output_name = format!("lib{}.so", name);

    let target = BuildTarget {
        name: name.clone(),
        id: id.clone(),
        target_type: TargetType::SharedLibrary,
        sources,
        objects: Vec::new(),
        dependencies,
        include_dirs: Vec::new(),
        link_with: Vec::new(),
        link_whole: Vec::new(),
        link_args: Vec::new(),
        c_args: Vec::new(),
        cpp_args: Vec::new(),
        rust_args,
        install: false,
        install_dir: None,
        install_rpath: String::new(),
        build_rpath: String::new(),
        pic: Some(true),
        pie: None,
        override_options: Vec::new(),
        gnu_symbol_visibility: String::new(),
        native: true,
        extra_files: Vec::new(),
        implicit_include_directories: true,
        win_subsystem: "console".to_string(),
        name_prefix: Some("lib".to_string()),
        name_suffix: Some("so".to_string()),
        rust_crate_type: Some("proc-macro".to_string()),
        build_by_default: true,
        subdir: vm.current_subdir.clone(),
        output_name,
    };

    let target_ref = BuildTargetRef {
        name: name.clone(),
        id,
        target_type: "shared_library".to_string(),
        subdir: vm.current_subdir.clone(),
        outputs: vec![target.output_name.clone()],
    };

    vm.build_data.targets.push(target);

    Ok(Object::BuildTarget(target_ref))
}
