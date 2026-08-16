/// D language module: generate_dub_file.
use crate::objects::*;
use crate::vm::*;

pub fn register(vm: &mut VM) {
    vm.method_registry.insert(
        ("module".to_string(), "dlang.generate_dub_file".to_string()),
        dlang_generate_dub_file,
    );
}

fn dlang_generate_dub_file(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);

    let name = match positional.first() {
        Some(Object::String(s)) => s.clone(),
        _ => {
            return Err("dlang.generate_dub_file: first argument must be project name".to_string());
        }
    };

    let dir = match positional.get(1) {
        Some(Object::String(s)) => s.clone(),
        _ => vm.current_subdir.clone(),
    };

    let description = VM::get_arg_str(args, "description", 99)
        .unwrap_or("")
        .to_string();
    let authors = VM::get_arg_str(args, "authors", 99)
        .unwrap_or("")
        .to_string();
    let copyright = VM::get_arg_str(args, "copyright", 99)
        .unwrap_or("")
        .to_string();
    let license = VM::get_arg_str(args, "license", 99)
        .unwrap_or("")
        .to_string();
    let source_dir = VM::get_arg_str(args, "sourceDir", 99)
        .unwrap_or("source")
        .to_string();
    let target_type = VM::get_arg_str(args, "targetType", 99)
        .unwrap_or("library")
        .to_string();
    let target_name = VM::get_arg_str(args, "targetName", 99).map(|s| s.to_string());
    let _dependencies = VM::get_arg_value(args, "dependencies").cloned();

    // Generate dub.json content
    let mut dub = String::new();
    dub.push_str("{\n");
    dub.push_str(&format!("  \"name\": \"{}\",\n", name));
    if !description.is_empty() {
        dub.push_str(&format!("  \"description\": \"{}\",\n", description));
    }
    if !authors.is_empty() {
        dub.push_str(&format!("  \"authors\": [\"{}\"],\n", authors));
    }
    if !copyright.is_empty() {
        dub.push_str(&format!("  \"copyright\": \"{}\",\n", copyright));
    }
    if !license.is_empty() {
        dub.push_str(&format!("  \"license\": \"{}\",\n", license));
    }
    dub.push_str(&format!("  \"targetType\": \"{}\",\n", target_type));
    if let Some(ref tn) = target_name {
        dub.push_str(&format!("  \"targetName\": \"{}\",\n", tn));
    }
    dub.push_str(&format!("  \"sourcePaths\": [\"{}\"]\n", source_dir));
    dub.push_str("}\n");

    // Write dub.json to the build directory
    let output_dir = std::path::Path::new(&vm.build_root).join(&dir);
    let output_path = output_dir.join("dub.json");
    let _ = std::fs::create_dir_all(&output_dir);
    let _ = std::fs::write(&output_path, &dub);

    Ok(Object::None)
}
