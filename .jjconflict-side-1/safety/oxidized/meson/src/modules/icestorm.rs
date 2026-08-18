/// FPGA Icestorm module: project.
use crate::objects::*;
use crate::vm::*;

pub fn register(vm: &mut VM) {
    vm.method_registry.insert(
        ("module".to_string(), "icestorm.project".to_string()),
        icestorm_project,
    );
}

fn icestorm_project(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);

    let project_name = match positional.first() {
        Some(Object::String(s)) => s.clone(),
        _ => return Err("icestorm.project: first argument must be project name".to_string()),
    };

    let sources = VM::get_arg_string_array(args, "sources");
    let constraint_file = VM::get_arg_str(args, "constraint_file", 99)
        .ok_or("icestorm.project: 'constraint_file' is required")?
        .to_string();
    let device = VM::get_arg_str(args, "device", 99)
        .unwrap_or("hx8k")
        .to_string();
    let package = VM::get_arg_str(args, "package", 99)
        .unwrap_or("ct256")
        .to_string();

    // Step 1: yosys synthesis -> .blif
    let blif_file = format!("{}.blif", project_name);
    let blif_id = format!("icestorm_{}_synth", project_name);

    let mut yosys_cmd = vec![
        "yosys".to_string(),
        "-p".to_string(),
        format!("synth_ice40 -blif @OUTPUT@"),
    ];
    for src in &sources {
        yosys_cmd.push(src.clone());
    }

    vm.build_data.custom_targets.push(CustomTarget {
        name: format!("{}-synth", project_name),
        id: blif_id,
        command: yosys_cmd,
        input: sources.clone(),
        output: vec![blif_file.clone()],
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

    // Step 2: arachne-pnr -> .asc
    let asc_file = format!("{}.asc", project_name);
    let asc_id = format!("icestorm_{}_pnr", project_name);

    vm.build_data.custom_targets.push(CustomTarget {
        name: format!("{}-pnr", project_name),
        id: asc_id,
        command: vec![
            "arachne-pnr".to_string(),
            "-d".to_string(),
            device.clone(),
            "-P".to_string(),
            package.clone(),
            "-p".to_string(),
            constraint_file.clone(),
            "-o".to_string(),
            "@OUTPUT@".to_string(),
            "@INPUT@".to_string(),
        ],
        input: vec![blif_file],
        output: vec![asc_file.clone()],
        depends: Vec::new(),
        depend_files: vec![constraint_file],
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

    // Step 3: icepack -> .bin
    let bin_file = format!("{}.bin", project_name);
    let bin_id = format!("icestorm_{}_pack", project_name);

    let ct = CustomTargetRef {
        name: project_name.clone(),
        id: bin_id.clone(),
        outputs: vec![bin_file.clone()],
        subdir: vm.current_subdir.clone(),
    };

    vm.build_data.custom_targets.push(CustomTarget {
        name: project_name.clone(),
        id: bin_id,
        command: vec![
            "icepack".to_string(),
            "@INPUT@".to_string(),
            "@OUTPUT@".to_string(),
        ],
        input: vec![asc_file],
        output: vec![bin_file.clone()],
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

    // Also add upload run target
    vm.build_data.run_targets.push(RunTarget {
        name: format!("{}-upload", project_name),
        command: vec!["iceprog".to_string(), bin_file.clone()],
        depends: vec![Object::CustomTarget(ct.clone())],
        env: std::collections::HashMap::new(),
        subdir: vm.current_subdir.clone(),
    });

    Ok(Object::CustomTarget(ct))
}
