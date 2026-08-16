/// GNOME/GLib helpers: compile_resources, generate_gir, compile_schemas, etc.
use crate::objects::*;
use crate::vm::*;

pub fn register(vm: &mut VM) {
    vm.method_registry.insert(
        ("module".to_string(), "gnome.compile_resources".to_string()),
        gnome_compile_resources,
    );
    vm.method_registry.insert(
        ("module".to_string(), "gnome.generate_gir".to_string()),
        gnome_generate_gir,
    );
    vm.method_registry.insert(
        ("module".to_string(), "gnome.compile_schemas".to_string()),
        gnome_compile_schemas,
    );
    vm.method_registry.insert(
        ("module".to_string(), "gnome.gdbus_codegen".to_string()),
        gnome_gdbus_codegen,
    );
    vm.method_registry.insert(
        ("module".to_string(), "gnome.mkenums".to_string()),
        gnome_mkenums,
    );
    vm.method_registry.insert(
        ("module".to_string(), "gnome.mkenums_simple".to_string()),
        gnome_mkenums_simple,
    );
    vm.method_registry.insert(
        ("module".to_string(), "gnome.generate_vapi".to_string()),
        gnome_generate_vapi,
    );
    vm.method_registry.insert(
        ("module".to_string(), "gnome.post_install".to_string()),
        gnome_post_install,
    );
    vm.method_registry
        .insert(("module".to_string(), "gnome.yelp".to_string()), gnome_yelp);
}

fn gnome_compile_resources(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let name = match positional.first() {
        Some(Object::String(s)) => s.clone(),
        _ => return Err("gnome.compile_resources: first arg must be resource name".to_string()),
    };
    let input = match positional.get(1) {
        Some(Object::String(s)) => s.clone(),
        Some(Object::File(f)) => f.path.clone(),
        _ => return Err("gnome.compile_resources: second arg must be gresource xml".to_string()),
    };

    let c_name = VM::get_arg_str(args, "c_name", 99)
        .map(|s| s.to_string())
        .unwrap_or_else(|| name.replace('-', "_"));
    let source_dir = VM::get_arg_string_array(args, "source_dir");
    let export = VM::get_arg_bool(args, "export", false);
    let install = VM::get_arg_bool(args, "install", false);
    let _install_header = VM::get_arg_bool(args, "install_header", false);
    let install_dir = VM::get_arg_str(args, "install_dir", 99).map(|s| s.to_string());
    let extra_args = VM::get_arg_string_array(args, "extra_args");
    let dependencies = match VM::get_arg_value(args, "dependencies") {
        Some(Object::Array(arr)) => arr.clone(),
        _ => Vec::new(),
    };

    let c_file = format!("{}.c", name);
    let h_file = format!("{}.h", name);

    let mut command = vec![
        "glib-compile-resources".to_string(),
        "--generate".to_string(),
        "--target".to_string(),
        "@OUTPUT@".to_string(),
        "--sourcedir".to_string(),
    ];
    if let Some(sd) = source_dir.first() {
        command.push(sd.clone());
    } else {
        command.push(".".to_string());
    }
    if !c_name.is_empty() {
        command.push(format!("--c-name={}", c_name));
    }
    command.extend(extra_args);
    command.push("@INPUT@".to_string());

    let id = format!("gnome_compile_resources_{}", name);
    let ct = CustomTargetRef {
        name: name.clone(),
        id: id.clone(),
        outputs: vec![c_file.clone(), h_file.clone()],
        subdir: vm.current_subdir.clone(),
    };

    vm.build_data.custom_targets.push(CustomTarget {
        name: name.clone(),
        id,
        command,
        input: vec![input],
        output: vec![c_file, h_file],
        depends: dependencies,
        depend_files: Vec::new(),
        depfile: None,
        capture: false,
        feed: false,
        install,
        install_dir: install_dir.into_iter().collect(),
        install_tag: Vec::new(),
        build_by_default: true,
        build_always_stale: false,
        env: std::collections::HashMap::new(),
        subdir: vm.current_subdir.clone(),
    });

    Ok(Object::CustomTarget(ct))
}

fn gnome_generate_gir(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let lib = match positional.first() {
        Some(obj) => obj,
        _ => return Err("gnome.generate_gir: requires a library target".to_string()),
    };

    let lib_name = match lib {
        Object::BuildTarget(t) => t.name.clone(),
        _ => "unknown".to_string(),
    };

    let namespace = VM::get_arg_str(args, "namespace", 99)
        .unwrap_or(&lib_name)
        .to_string();
    let nsversion = VM::get_arg_str(args, "nsversion", 99)
        .unwrap_or("1.0")
        .to_string();
    let sources = VM::get_arg_string_array(args, "sources");
    let dependencies = match VM::get_arg_value(args, "dependencies") {
        Some(Object::Array(arr)) => arr.clone(),
        _ => Vec::new(),
    };
    let install = VM::get_arg_bool(args, "install", false);
    let install_dir_gir = VM::get_arg_str(args, "install_dir_gir", 99).map(|s| s.to_string());
    let _install_dir_typelib =
        VM::get_arg_str(args, "install_dir_typelib", 99).map(|s| s.to_string());

    let gir_file = format!("{}-{}.gir", namespace, nsversion);
    let typelib_file = format!("{}-{}.typelib", namespace, nsversion);

    let id = format!("gnome_gir_{}", namespace);
    let ct = CustomTargetRef {
        name: format!("{}-gir", namespace),
        id: id.clone(),
        outputs: vec![gir_file.clone(), typelib_file.clone()],
        subdir: vm.current_subdir.clone(),
    };

    vm.build_data.custom_targets.push(CustomTarget {
        name: format!("{}-gir", namespace),
        id,
        command: vec!["g-ir-scanner".to_string()],
        input: sources,
        output: vec![gir_file, typelib_file],
        depends: dependencies,
        depend_files: Vec::new(),
        depfile: None,
        capture: false,
        feed: false,
        install,
        install_dir: install_dir_gir.into_iter().collect(),
        install_tag: Vec::new(),
        build_by_default: true,
        build_always_stale: false,
        env: std::collections::HashMap::new(),
        subdir: vm.current_subdir.clone(),
    });

    Ok(Object::CustomTarget(ct))
}

fn gnome_compile_schemas(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let depend_files = VM::get_arg_string_array(args, "depend_files");
    let build_by_default = VM::get_arg_bool(args, "build_by_default", true);

    let id = "gnome_compile_schemas".to_string();
    let ct = CustomTargetRef {
        name: "gschemas.compiled".to_string(),
        id: id.clone(),
        outputs: vec!["gschemas.compiled".to_string()],
        subdir: vm.current_subdir.clone(),
    };

    vm.build_data.custom_targets.push(CustomTarget {
        name: "gschemas.compiled".to_string(),
        id,
        command: vec!["glib-compile-schemas".to_string(), ".".to_string()],
        input: Vec::new(),
        output: vec!["gschemas.compiled".to_string()],
        depends: Vec::new(),
        depend_files,
        depfile: None,
        capture: false,
        feed: false,
        install: false,
        install_dir: Vec::new(),
        install_tag: Vec::new(),
        build_by_default,
        build_always_stale: false,
        env: std::collections::HashMap::new(),
        subdir: vm.current_subdir.clone(),
    });

    Ok(Object::CustomTarget(ct))
}

fn gnome_gdbus_codegen(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let name = match positional.first() {
        Some(Object::String(s)) => s.clone(),
        _ => return Err("gnome.gdbus_codegen: first arg must be output name".to_string()),
    };

    let sources = match positional.get(1) {
        Some(Object::String(s)) => vec![s.clone()],
        Some(Object::File(f)) => vec![f.path.clone()],
        _ => VM::get_arg_string_array(args, "sources"),
    };

    let interface_prefix = VM::get_arg_str(args, "interface_prefix", 99).map(|s| s.to_string());
    let namespace = VM::get_arg_str(args, "namespace", 99).map(|s| s.to_string());
    let object_manager = VM::get_arg_bool(args, "object_manager", false);
    let _annotations = VM::get_arg_string_array(args, "annotations");

    let c_file = format!("{}.c", name);
    let h_file = format!("{}.h", name);

    let mut command = vec!["gdbus-codegen".to_string()];
    if let Some(ref prefix) = interface_prefix {
        command.push("--interface-prefix".to_string());
        command.push(prefix.clone());
    }
    if let Some(ref ns) = namespace {
        command.push("--c-namespace".to_string());
        command.push(ns.clone());
    }
    if object_manager {
        command.push("--c-generate-object-manager".to_string());
    }
    command.push("--output-directory".to_string());
    command.push("@OUTDIR@".to_string());
    command.push("--generate-c-code".to_string());
    command.push(name.clone());
    command.push("@INPUT@".to_string());

    let id = format!("gnome_gdbus_{}", name);
    let ct = CustomTargetRef {
        name: name.clone(),
        id: id.clone(),
        outputs: vec![c_file.clone(), h_file.clone()],
        subdir: vm.current_subdir.clone(),
    };

    vm.build_data.custom_targets.push(CustomTarget {
        name: name.clone(),
        id,
        command,
        input: sources,
        output: vec![c_file, h_file],
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

fn gnome_mkenums(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let name = match positional.first() {
        Some(Object::String(s)) => s.clone(),
        _ => return Err("gnome.mkenums: first arg must be output name".to_string()),
    };

    let sources = VM::get_arg_string_array(args, "sources");
    let c_template = VM::get_arg_str(args, "c_template", 99).map(|s| s.to_string());
    let h_template = VM::get_arg_str(args, "h_template", 99).map(|s| s.to_string());
    let install_header = VM::get_arg_bool(args, "install_header", false);
    let install_dir = VM::get_arg_str(args, "install_dir", 99).map(|s| s.to_string());

    let mut command = vec!["glib-mkenums".to_string()];
    if let Some(ref ct) = c_template {
        command.push("--template".to_string());
        command.push(ct.clone());
    }
    if let Some(ref ht) = h_template {
        command.push("--template".to_string());
        command.push(ht.clone());
    }
    command.push("@INPUT@".to_string());

    let id = format!("gnome_mkenums_{}", name);
    let ct = CustomTargetRef {
        name: name.clone(),
        id: id.clone(),
        outputs: vec![name.clone()],
        subdir: vm.current_subdir.clone(),
    };

    vm.build_data.custom_targets.push(CustomTarget {
        name: name.clone(),
        id,
        command,
        input: sources,
        output: vec![name.clone()],
        depends: Vec::new(),
        depend_files: Vec::new(),
        depfile: None,
        capture: true,
        feed: false,
        install: install_header,
        install_dir: install_dir.into_iter().collect(),
        install_tag: Vec::new(),
        build_by_default: true,
        build_always_stale: false,
        env: std::collections::HashMap::new(),
        subdir: vm.current_subdir.clone(),
    });

    Ok(Object::CustomTarget(ct))
}

fn gnome_mkenums_simple(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let name = match positional.first() {
        Some(Object::String(s)) => s.clone(),
        _ => return Err("gnome.mkenums_simple: first arg must be output name".to_string()),
    };

    let sources = VM::get_arg_string_array(args, "sources");
    let install_header = VM::get_arg_bool(args, "install_header", false);
    let install_dir = VM::get_arg_str(args, "install_dir", 99).map(|s| s.to_string());
    let _identifier_prefix = VM::get_arg_str(args, "identifier_prefix", 99).map(|s| s.to_string());
    let _symbol_prefix = VM::get_arg_str(args, "symbol_prefix", 99).map(|s| s.to_string());
    let _header_prefix = VM::get_arg_str(args, "header_prefix", 99).map(|s| s.to_string());
    let _function_prefix = VM::get_arg_str(args, "function_prefix", 99).map(|s| s.to_string());
    let _body_prefix = VM::get_arg_str(args, "body_prefix", 99).map(|s| s.to_string());
    let _decorator = VM::get_arg_str(args, "decorator", 99).map(|s| s.to_string());

    let c_file = format!("{}.c", name);
    let h_file = format!("{}.h", name);

    let id = format!("gnome_mkenums_simple_{}", name);
    let ct = CustomTargetRef {
        name: name.clone(),
        id: id.clone(),
        outputs: vec![c_file.clone(), h_file.clone()],
        subdir: vm.current_subdir.clone(),
    };

    vm.build_data.custom_targets.push(CustomTarget {
        name: name.clone(),
        id,
        command: vec!["glib-mkenums".to_string()],
        input: sources,
        output: vec![c_file, h_file],
        depends: Vec::new(),
        depend_files: Vec::new(),
        depfile: None,
        capture: false,
        feed: false,
        install: install_header,
        install_dir: install_dir.into_iter().collect(),
        install_tag: Vec::new(),
        build_by_default: true,
        build_always_stale: false,
        env: std::collections::HashMap::new(),
        subdir: vm.current_subdir.clone(),
    });

    Ok(Object::CustomTarget(ct))
}

fn gnome_generate_vapi(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let name = match positional.first() {
        Some(Object::String(s)) => s.clone(),
        _ => return Err("gnome.generate_vapi: first arg must be library name".to_string()),
    };

    let sources = VM::get_arg_string_array(args, "sources");
    let packages = VM::get_arg_string_array(args, "packages");
    let _metadata_dirs = VM::get_arg_string_array(args, "metadata_dirs");
    let _gir_dirs = VM::get_arg_string_array(args, "gir_dirs");
    let _vapi_dirs = VM::get_arg_string_array(args, "vapi_dirs");
    let install = VM::get_arg_bool(args, "install", false);
    let install_dir = VM::get_arg_str(args, "install_dir", 99).map(|s| s.to_string());

    let vapi_file = format!("{}.vapi", name);
    let deps_file = format!("{}.deps", name);

    let id = format!("gnome_vapi_{}", name);
    let ct = CustomTargetRef {
        name: name.clone(),
        id: id.clone(),
        outputs: vec![vapi_file.clone()],
        subdir: vm.current_subdir.clone(),
    };

    vm.build_data.custom_targets.push(CustomTarget {
        name: name.clone(),
        id,
        command: vec!["vapigen".to_string()],
        input: sources,
        output: vec![vapi_file, deps_file],
        depends: Vec::new(),
        depend_files: Vec::new(),
        depfile: None,
        capture: false,
        feed: false,
        install,
        install_dir: install_dir.into_iter().collect(),
        install_tag: Vec::new(),
        build_by_default: true,
        build_always_stale: false,
        env: std::collections::HashMap::new(),
        subdir: vm.current_subdir.clone(),
    });

    Ok(Object::CustomTarget(ct))
}

fn gnome_post_install(_vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    // post_install registers scripts to run after meson install
    let _glib_compile_schemas = VM::get_arg_bool(args, "glib_compile_schemas", false);
    let _gtk_update_icon_cache = VM::get_arg_bool(args, "gtk_update_icon_cache", false);
    let _update_desktop_database = VM::get_arg_bool(args, "update_desktop_database", false);
    let _update_mime_database = VM::get_arg_bool(args, "update_mime_database", false);

    // In a real implementation, these would register post-install scripts.
    // For now, just acknowledge the call.
    Ok(Object::None)
}

fn gnome_yelp(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let project_id = match positional.first() {
        Some(Object::String(s)) => s.clone(),
        _ => return Err("gnome.yelp: first arg must be project id".to_string()),
    };

    let sources = VM::get_arg_string_array(args, "sources");
    let _media = VM::get_arg_string_array(args, "media");
    let _languages = VM::get_arg_string_array(args, "languages");
    let _symlink_media = VM::get_arg_bool(args, "symlink_media", true);

    let id = format!("gnome_yelp_{}", project_id);
    let ct = CustomTargetRef {
        name: format!("{}-help", project_id),
        id: id.clone(),
        outputs: vec![format!("{}-help", project_id)],
        subdir: vm.current_subdir.clone(),
    };

    vm.build_data.custom_targets.push(CustomTarget {
        name: format!("{}-help", project_id),
        id,
        command: vec!["itstool".to_string()],
        input: sources,
        output: vec![format!("{}-help", project_id)],
        depends: Vec::new(),
        depend_files: Vec::new(),
        depfile: None,
        capture: false,
        feed: false,
        install: true,
        install_dir: vec![format!("share/help/C/{}", project_id)],
        install_tag: Vec::new(),
        build_by_default: true,
        build_always_stale: false,
        env: std::collections::HashMap::new(),
        subdir: vm.current_subdir.clone(),
    });

    Ok(Object::CustomTarget(ct))
}
