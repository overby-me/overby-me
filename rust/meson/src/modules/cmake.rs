/// CMake integration module: write_basic_package_version_file,
/// configure_package_config_file, subproject.
use crate::objects::*;
use crate::vm::*;

pub fn register(vm: &mut VM) {
    vm.method_registry.insert(
        (
            "module".to_string(),
            "cmake.write_basic_package_version_file".to_string(),
        ),
        cmake_write_basic_package_version_file,
    );
    vm.method_registry.insert(
        (
            "module".to_string(),
            "cmake.configure_package_config_file".to_string(),
        ),
        cmake_configure_package_config_file,
    );
    vm.method_registry.insert(
        ("module".to_string(), "cmake.subproject".to_string()),
        cmake_subproject,
    );
}

fn cmake_write_basic_package_version_file(
    vm: &mut VM,
    _obj: &Object,
    args: &[CallArg],
) -> Result<Object, String> {
    let name = VM::get_arg_str(args, "name", 0)
        .ok_or("cmake.write_basic_package_version_file: 'name' is required")?
        .to_string();
    let version = VM::get_arg_str(args, "version", 99)
        .map(|s| s.to_string())
        .or_else(|| vm.project.as_ref().map(|p| p.version.clone()))
        .unwrap_or_else(|| "0.0.0".to_string());
    let compatibility = VM::get_arg_str(args, "compatibility", 99)
        .unwrap_or("AnyNewerVersion")
        .to_string();
    let install_dir = VM::get_arg_str(args, "install_dir", 99)
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("lib/cmake/{}", name));
    let _arch_independent = VM::get_arg_bool(args, "arch_independent", false);

    let filename = format!("{}ConfigVersion.cmake", name);
    let build_path = std::path::Path::new(&vm.build_root)
        .join(&vm.current_subdir)
        .join(&filename);

    // Generate a basic CMake package version file
    let content = format!(
        r#"set(PACKAGE_VERSION "{version}")

if(PACKAGE_VERSION VERSION_LESS PACKAGE_FIND_VERSION)
  set(PACKAGE_VERSION_COMPATIBLE FALSE)
else()
  set(PACKAGE_VERSION_COMPATIBLE TRUE)
  if(PACKAGE_FIND_VERSION STREQUAL PACKAGE_VERSION)
    set(PACKAGE_VERSION_EXACT TRUE)
  endif()
endif()
"#,
        version = version
    );

    let _ = std::fs::create_dir_all(build_path.parent().unwrap_or(std::path::Path::new(".")));
    let _ = std::fs::write(&build_path, &content);

    vm.build_data.install_data.push(InstallData {
        sources: vec![build_path.to_string_lossy().to_string()],
        install_dir: install_dir.clone(),
        install_mode: None,
        rename: Vec::new(),
        subdir: vm.current_subdir.clone(),
        preserve_path: false,
        strip_directory: false,
        exclude_files: Vec::new(),
        exclude_directories: Vec::new(),
        follow_symlinks: None,
    });

    Ok(Object::None)
}

fn cmake_configure_package_config_file(
    vm: &mut VM,
    _obj: &Object,
    args: &[CallArg],
) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);

    let name = VM::get_arg_str(args, "name", 99)
        .ok_or("cmake.configure_package_config_file: 'name' is required")?
        .to_string();
    let input = match positional.first() {
        Some(Object::String(s)) => s.clone(),
        Some(Object::File(f)) => f.path.clone(),
        _ => VM::get_arg_str(args, "input", 0)
            .ok_or("cmake.configure_package_config_file: 'input' is required")?
            .to_string(),
    };
    let install_dir = VM::get_arg_str(args, "install_dir", 99)
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("lib/cmake/{}", name));
    let _configuration = VM::get_arg_value(args, "configuration").cloned();

    let filename = format!("{}Config.cmake", name);
    let build_path = std::path::Path::new(&vm.build_root)
        .join(&vm.current_subdir)
        .join(&filename);

    // Read the input template and do basic substitution
    let input_path = if std::path::Path::new(&input).is_absolute() {
        std::path::PathBuf::from(&input)
    } else {
        std::path::Path::new(&vm.source_root)
            .join(&vm.current_subdir)
            .join(&input)
    };

    let template = std::fs::read_to_string(&input_path)
        .unwrap_or_else(|_| format!("# CMake config for {}\n", name));

    // Basic @PACKAGE_INIT@ substitution
    let content = template.replace("@PACKAGE_INIT@", &format!(
        "get_filename_component(PACKAGE_PREFIX_DIR \"${{CMAKE_CURRENT_LIST_DIR}}/../../..\" ABSOLUTE)\n\
         macro(set_and_check _var _file)\n\
           set(${{_var}} \"${{_file}}\")\n\
           if(NOT EXISTS \"${{_file}}\")\n\
             message(FATAL_ERROR \"File or directory ${{_file}} referenced by variable ${{_var}} does not exist !\")\n\
           endif()\n\
         endmacro()\n"
    ));

    let _ = std::fs::create_dir_all(build_path.parent().unwrap_or(std::path::Path::new(".")));
    let _ = std::fs::write(&build_path, &content);

    vm.build_data.install_data.push(InstallData {
        sources: vec![build_path.to_string_lossy().to_string()],
        install_dir,
        install_mode: None,
        rename: Vec::new(),
        subdir: vm.current_subdir.clone(),
        preserve_path: false,
        strip_directory: false,
        exclude_files: Vec::new(),
        exclude_directories: Vec::new(),
        follow_symlinks: None,
    });

    Ok(Object::None)
}

fn cmake_subproject(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);
    let name = match positional.first() {
        Some(Object::String(s)) => s.clone(),
        _ => return Err("cmake.subproject: first arg must be subproject name".to_string()),
    };

    let required = VM::get_arg_bool(args, "required", true);
    let _options = VM::get_arg_value(args, "options").cloned();
    let _cmake_options = VM::get_arg_string_array(args, "cmake_options");

    // Check if subproject directory exists
    let subproject_dir = vm
        .project
        .as_ref()
        .map(|p| p.subproject_dir.clone())
        .unwrap_or_else(|| "subprojects".to_string());

    let subproject_path = std::path::Path::new(&vm.source_root)
        .join(&subproject_dir)
        .join(&name);

    let cmake_lists = subproject_path.join("CMakeLists.txt");

    if cmake_lists.exists() {
        Ok(Object::Subproject(SubprojectData {
            name: name.clone(),
            version: String::new(),
            found: true,
            variables: std::collections::HashMap::new(),
        }))
    } else if required {
        Err(format!(
            "cmake.subproject: CMake project '{}' not found at '{}'",
            name,
            subproject_path.display()
        ))
    } else {
        Ok(Object::Subproject(SubprojectData {
            name,
            version: String::new(),
            found: false,
            variables: std::collections::HashMap::new(),
        }))
    }
}
