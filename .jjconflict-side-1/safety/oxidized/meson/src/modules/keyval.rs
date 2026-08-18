/// Key-value file parsing module: load.
use crate::objects::*;
use crate::vm::*;

pub fn register(vm: &mut VM) {
    vm.method_registry.insert(
        ("module".to_string(), "keyval.load".to_string()),
        keyval_load,
    );
}

fn keyval_load(vm: &mut VM, _obj: &Object, args: &[CallArg]) -> Result<Object, String> {
    let positional = VM::get_positional_args(args);

    let path_str = match positional.first() {
        Some(Object::String(s)) => s.clone(),
        Some(Object::File(f)) => f.path.clone(),
        _ => return Err("keyval.load: first argument must be a file path".to_string()),
    };

    let full_path = if std::path::Path::new(&path_str).is_absolute() {
        std::path::PathBuf::from(&path_str)
    } else {
        std::path::Path::new(&vm.source_root)
            .join(&vm.current_subdir)
            .join(&path_str)
    };

    let content = std::fs::read_to_string(&full_path)
        .map_err(|e| format!("keyval.load: cannot read '{}': {}", full_path.display(), e))?;

    let mut entries = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        // Skip empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(eq_pos) = trimmed.find('=') {
            let key = trimmed[..eq_pos].trim().to_string();
            let value = trimmed[eq_pos + 1..].trim().to_string();
            entries.push((key, Object::String(value)));
        }
    }

    Ok(Object::Dict(entries))
}
