/// Internationalization module: merge_file, gettext.
use crate::objects::*;
use crate::vm::*;

pub fn register(vm: &mut VM) {
    vm.method_registry.insert(
        ("module".to_string(), "i18n.merge_file".to_string()),
        i18n_merge_file,
    );
    vm.method_registry.insert(
        ("module".to_string(), "i18n.gettext".to_string()),
        i18n_gettext,
    );
}

fn i18n_merge_file(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);

    let output = match positional.first() {
        Some(Object::String(s)) => s.clone(),
        _ => VM::get_arg_str(args, "output", 0)
            .ok_or("i18n.merge_file: 'output' is required")?
            .to_string(),
    };

    let input = match VM::get_arg_value(args, "input") {
        Some(Object::String(s)) => vec![s.clone()],
        Some(Object::File(f)) => vec![f.path.clone()],
        Some(Object::Array(arr)) => arr
            .iter()
            .filter_map(|o| match o {
                Object::String(s) => Some(s.clone()),
                Object::File(f) => Some(f.path.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };

    let po_dir = VM::get_arg_str(args, "po_dir", 99)
        .unwrap_or("po")
        .to_string();
    let _data_dirs = VM::get_arg_string_array(args, "data_dirs");
    let file_type = VM::get_arg_str(args, "type", 99)
        .unwrap_or("xml")
        .to_string();
    let install = VM::get_arg_bool(args, "install", false);
    let install_dir = VM::get_arg_str(args, "install_dir", 99).map(|s| s.to_string());
    let install_tag = VM::get_arg_str(args, "install_tag", 99).map(|s| s.to_string());

    let mut command = vec![
        "msgfmt".to_string(),
        format!("--{}", file_type),
        "-d".to_string(),
        po_dir.clone(),
        "--template".to_string(),
        "@INPUT@".to_string(),
        "-o".to_string(),
        "@OUTPUT@".to_string(),
    ];

    let id = format!("i18n_merge_{}", output.replace('/', "_"));
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
        input,
        output: vec![output],
        depends: Vec::new(),
        depend_files: Vec::new(),
        depfile: None,
        capture: false,
        feed: false,
        install,
        install_dir: install_dir.into_iter().collect(),
        install_tag: install_tag.into_iter().collect(),
        build_by_default: true,
        build_always_stale: false,
        env: std::collections::HashMap::new(),
        subdir: vm.current_subdir.clone(),
    });

    Ok(Object::CustomTarget(ct))
}

fn i18n_gettext(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);

    let package_name = match positional.first() {
        Some(Object::String(s)) => s.clone(),
        _ => return Err("i18n.gettext: first argument must be a package name".to_string()),
    };

    let args_list = VM::get_arg_string_array(args, "args");
    let _data_dirs = VM::get_arg_string_array(args, "data_dirs");
    let languages = VM::get_arg_string_array(args, "languages");
    let install = VM::get_arg_bool(args, "install", true);
    let install_dir = VM::get_arg_str(args, "install_dir", 99)
        .map(|s| s.to_string())
        .unwrap_or_else(|| "share/locale".to_string());
    let _preset = VM::get_arg_str(args, "preset", 99).map(|s| s.to_string());

    // Create pot file target
    let pot_file = format!("{}.pot", package_name);
    let pot_id = format!("i18n_pot_{}", package_name);

    let mut pot_command = vec![
        "xgettext".to_string(),
        "--package-name".to_string(),
        package_name.clone(),
    ];
    pot_command.extend(args_list);
    pot_command.push("-o".to_string());
    pot_command.push("@OUTPUT@".to_string());

    vm.build_data.custom_targets.push(CustomTarget {
        name: format!("{}-pot", package_name),
        id: pot_id.clone(),
        command: pot_command,
        input: Vec::new(),
        output: vec![pot_file.clone()],
        depends: Vec::new(),
        depend_files: Vec::new(),
        depfile: None,
        capture: false,
        feed: false,
        install: false,
        install_dir: Vec::new(),
        install_tag: Vec::new(),
        build_by_default: false,
        build_always_stale: true,
        env: std::collections::HashMap::new(),
        subdir: vm.current_subdir.clone(),
    });

    // Create update-po target
    let updatepo_id = format!("i18n_updatepo_{}", package_name);
    vm.build_data.custom_targets.push(CustomTarget {
        name: format!("{}-update-po", package_name),
        id: updatepo_id,
        command: vec![
            "msgmerge".to_string(),
            "--update".to_string(),
            "@INPUT@".to_string(),
            pot_file,
        ],
        input: Vec::new(),
        output: vec![format!("{}-update-po", package_name)],
        depends: Vec::new(),
        depend_files: Vec::new(),
        depfile: None,
        capture: false,
        feed: false,
        install: false,
        install_dir: Vec::new(),
        install_tag: Vec::new(),
        build_by_default: false,
        build_always_stale: true,
        env: std::collections::HashMap::new(),
        subdir: vm.current_subdir.clone(),
    });

    // For each language, compile .mo files
    for lang in &languages {
        let mo_file = format!("{}/{}/LC_MESSAGES/{}.mo", install_dir, lang, package_name);
        let mo_id = format!("i18n_mo_{}_{}", package_name, lang);
        vm.build_data.custom_targets.push(CustomTarget {
            name: format!("{}-{}.mo", package_name, lang),
            id: mo_id,
            command: vec![
                "msgfmt".to_string(),
                "@INPUT@".to_string(),
                "-o".to_string(),
                "@OUTPUT@".to_string(),
            ],
            input: vec![format!("{}.po", lang)],
            output: vec![format!("{}.mo", lang)],
            depends: Vec::new(),
            depend_files: Vec::new(),
            depfile: None,
            capture: false,
            feed: false,
            install,
            install_dir: vec![format!("{}/{}/LC_MESSAGES", install_dir, lang)],
            install_tag: Vec::new(),
            build_by_default: true,
            build_always_stale: false,
            env: std::collections::HashMap::new(),
            subdir: vm.current_subdir.clone(),
        });
    }

    Ok(Object::None)
}
