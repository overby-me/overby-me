/// Built-in functions for the Meson interpreter.
/// Registers all global functions and type methods.
pub mod functions;
pub mod methods;

use crate::vm::VM;

pub fn register_all(vm: &mut VM) {
    functions::register(vm);
    methods::register(vm);
}
