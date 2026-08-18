/// SIMD module: check method for detecting SIMD instruction set support.
use crate::objects::*;
use crate::vm::*;

pub fn register(vm: &mut VM) {
    vm.method_registry
        .insert(("module".to_string(), "simd.check".to_string()), simd_check);
}

fn simd_check(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
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

    // Get the compiler keyword arg
    let compiler = args
        .iter()
        .find(|a| a.name.as_deref() == Some("compiler"))
        .map(|a| a.value.clone());

    // SIMD instruction sets to check, in order
    let simd_sets: &[(&str, &str)] = &[
        ("mmx", "-mmmx"),
        ("sse", "-msse"),
        ("sse2", "-msse2"),
        ("sse3", "-msse3"),
        ("ssse3", "-mssse3"),
        ("sse41", "-msse4.1"),
        ("sse42", "-msse4.2"),
        ("avx", "-mavx"),
        ("avx2", "-mavx2"),
        ("neon", "-mfpu=neon"),
    ];

    let mut libs = Vec::new();
    let cfg = ConfigData::new();

    for (set_name, flag) in simd_sets {
        // Check if this SIMD set was provided as a keyword argument
        let source = VM::get_arg_str(args, set_name, usize::MAX);
        if let Some(source_file) = source {
            // Check if the compiler supports this flag
            let supported = if let Some(Object::Compiler(ref c)) = compiler {
                crate::compilers::has_argument(c, flag)
            } else {
                false
            };

            if supported {
                let define_name = format!("HAVE_{}", set_name.to_uppercase());
                cfg.values.borrow_mut().insert(
                    define_name,
                    (Object::Int(1), Some("SIMD support".to_string())),
                );

                let target_name = format!("{}_{}", name, set_name);
                let target_id = format!("{}_{}", name, set_name);
                let output = format!("lib{}.a", target_name);

                let target = BuildTarget {
                    name: target_name.clone(),
                    id: target_id,
                    target_type: TargetType::StaticLibrary,
                    sources: vec![source_file.to_string()],
                    objects: Vec::new(),
                    dependencies: Vec::new(),
                    include_dirs: Vec::new(),
                    link_with: Vec::new(),
                    link_whole: Vec::new(),
                    link_args: Vec::new(),
                    c_args: vec![flag.to_string()],
                    cpp_args: Vec::new(),
                    rust_args: Vec::new(),
                    install: false,
                    install_dir: None,
                    install_rpath: String::new(),
                    build_rpath: String::new(),
                    pic: None,
                    pie: None,
                    override_options: Vec::new(),
                    gnu_symbol_visibility: String::new(),
                    native: false,
                    extra_files: Vec::new(),
                    implicit_include_directories: true,
                    win_subsystem: String::new(),
                    name_prefix: None,
                    name_suffix: None,
                    rust_crate_type: None,
                    build_by_default: true,
                    subdir: vm.current_subdir.clone(),
                    output_name: target_name.clone(),
                };

                let target_ref = BuildTargetRef {
                    name: target_name,
                    id: target.id.clone(),
                    target_type: "static_library".to_string(),
                    subdir: vm.current_subdir.clone(),
                    outputs: vec![output],
                };

                vm.build_data.targets.push(target);
                libs.push(Object::BuildTarget(target_ref));
            }
        }
    }

    // Return [libs, config_data]
    Ok(Object::Array(vec![
        Object::Array(libs),
        Object::ConfigurationData(cfg),
    ]))
}
