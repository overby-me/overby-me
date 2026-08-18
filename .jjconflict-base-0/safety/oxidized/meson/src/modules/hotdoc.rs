/// Hotdoc documentation module: generate_doc, has_extensions.
use crate::objects::*;
use crate::vm::*;

pub fn register(vm: &mut VM) {
    vm.method_registry.insert(
        ("module".to_string(), "hotdoc.generate_doc".to_string()),
        hotdoc_generate_doc,
    );
    vm.method_registry.insert(
        ("module".to_string(), "hotdoc.has_extensions".to_string()),
        hotdoc_has_extensions,
    );
}

fn hotdoc_generate_doc(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);

    let project_name = match positional.first() {
        Some(Object::String(s)) => s.clone(),
        _ => return Err("hotdoc.generate_doc: first argument must be project name".to_string()),
    };

    let sitemap = VM::get_arg_str(args, "sitemap", 99)
        .unwrap_or("sitemap.txt")
        .to_string();
    let index = VM::get_arg_str(args, "index", 99)
        .unwrap_or("index.md")
        .to_string();
    let project_version = VM::get_arg_str(args, "project_version", 99)
        .map(|s| s.to_string())
        .or_else(|| vm.project.as_ref().map(|p| p.version.clone()))
        .unwrap_or_else(|| "0.0.0".to_string());
    let install = VM::get_arg_bool(args, "install", false);
    let install_dir = VM::get_arg_str(args, "install_dir", 99)
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("share/doc/{}", project_name));
    let _extra_assets = VM::get_arg_string_array(args, "extra_assets");
    let extra_extensions = VM::get_arg_string_array(args, "extra_extensions");
    let _c_sources = VM::get_arg_string_array(args, "c_sources");
    let _c_smart_index = VM::get_arg_bool(args, "c_smart_index", false);
    let _languages = VM::get_arg_string_array(args, "languages");
    let _include_paths = VM::get_arg_string_array(args, "include_paths");
    let dependencies = match VM::get_arg_value(args, "dependencies") {
        Some(Object::Array(arr)) => arr.clone(),
        _ => Vec::new(),
    };
    let build_by_default = VM::get_arg_bool(args, "build_by_default", false);

    let mut command = vec![
        "hotdoc".to_string(),
        "run".to_string(),
        "--project-name".to_string(),
        project_name.clone(),
        "--project-version".to_string(),
        project_version,
        "--sitemap".to_string(),
        sitemap,
        "--index".to_string(),
        index,
        "--output".to_string(),
        "@OUTDIR@".to_string(),
    ];

    for ext in &extra_extensions {
        command.push("--extra-extension".to_string());
        command.push(ext.clone());
    }

    let output_dir = format!("{}-doc", project_name);
    let id = format!("hotdoc_{}", project_name);

    let ct = CustomTargetRef {
        name: format!("{}-doc", project_name),
        id: id.clone(),
        outputs: vec![output_dir.clone()],
        subdir: vm.current_subdir.clone(),
    };

    vm.build_data.custom_targets.push(CustomTarget {
        name: format!("{}-doc", project_name),
        id,
        command,
        input: Vec::new(),
        output: vec![output_dir],
        depends: dependencies,
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

fn hotdoc_has_extensions(_vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);

    let extensions: Vec<String> = positional
        .iter()
        .filter_map(|o| {
            if let Object::String(s) = o {
                Some(s.clone())
            } else {
                None
            }
        })
        .collect();

    // Check if hotdoc has the requested extensions
    for ext in &extensions {
        let result = std::process::Command::new("hotdoc")
            .args(["--has-extension", ext])
            .output();

        match result {
            Ok(out) if out.status.success() => continue,
            _ => return Ok(Object::Bool(false)),
        }
    }

    Ok(Object::Bool(true))
}
