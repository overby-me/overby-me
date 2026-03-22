/// Source set module: conditional source sets.
use crate::objects::*;
use crate::vm::*;

pub fn register(vm: &mut VM) {
    vm.method_registry.insert(
        ("module".to_string(), "sourceset.source_set".to_string()),
        sourceset_source_set,
    );
}

fn sourceset_source_set(_vm: &mut VM, _obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    // Returns a new source set object represented as an empty dict with a marker.
    // In a full implementation this would be a dedicated object type.
    // We represent it as a Dict with special keys for sources/dependencies.
    Ok(Object::Dict(vec![
        (
            "_type".to_string(),
            Object::String("source_set".to_string()),
        ),
        ("sources".to_string(), Object::Array(Vec::new())),
        ("dependencies".to_string(), Object::Array(Vec::new())),
        ("rules".to_string(), Object::Array(Vec::new())),
    ]))
}
