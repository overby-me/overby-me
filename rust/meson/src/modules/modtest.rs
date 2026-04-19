/// Modtest module: a test-only module with print_hello().
use crate::objects::*;
use crate::vm::*;

pub fn register(vm: &mut VM) {
    vm.method_registry.insert(
        ("module".to_string(), "modtest.print_hello".to_string()),
        modtest_print_hello,
    );
}

fn modtest_print_hello(_vm: &mut VM, _obj: &Object, _args: &[CallArg]) -> Result<Object, String> {
    eprintln!("Hello from modtest module");
    Ok(Object::None)
}
