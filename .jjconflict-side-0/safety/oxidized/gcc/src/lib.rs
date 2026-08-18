#![recursion_limit = "512"]
#![allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::needless_range_loop,
    clippy::manual_range_contains,
    clippy::collapsible_if,
    clippy::collapsible_else_if,
    clippy::single_match,
    clippy::match_single_binding,
    clippy::redundant_field_names,
    clippy::needless_return,
    clippy::len_zero,
    clippy::if_same_then_else,
    clippy::derivable_impls,
    clippy::manual_is_multiple_of,
    clippy::unnecessary_cast,
    clippy::enum_variant_names,
    clippy::only_used_in_recursion,
    clippy::redundant_closure,
    clippy::redundant_pattern_matching,
    clippy::map_entry,
    clippy::question_mark,
    clippy::single_char_add_str,
    clippy::for_kv_map,
    clippy::useless_format,
    clippy::get_first,
    clippy::unwrap_or_default,
    clippy::manual_map,
    clippy::ptr_arg,
    clippy::clone_on_copy,
    clippy::wrong_self_convention,
    clippy::needless_borrow,
    clippy::op_ref,
    clippy::search_is_some,
    clippy::identity_op,
    clippy::bool_comparison,
    unused_variables,
    dead_code
)]

pub mod backend;
pub(crate) mod common;
pub mod driver;
pub(crate) mod frontend;
pub(crate) mod ir;
pub(crate) mod passes;

/// Shared entry point for all compiler binaries. Spawns the real work on a
/// thread with a large stack so deeply recursive C files don't overflow.
pub fn compiler_main() {
    const STACK_SIZE: usize = 64 * 1024 * 1024; // 64 MB
    let builder = std::thread::Builder::new().stack_size(STACK_SIZE);
    let handler = builder
        .spawn(|| {
            let args: Vec<String> = std::env::args().collect();
            let mut driver = driver::Driver::new();
            if driver.parse_cli_args(&args)? {
                return Ok(());
            }
            if !driver.has_input_files() {
                return Err("no input files".to_string());
            }
            driver.run()
        })
        .expect("failed to spawn main thread");

    match handler.join() {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            eprintln!("gcc: error: {}", e);
            std::process::exit(1);
        }
        Err(e) => {
            if let Some(s) = e.downcast_ref::<&str>() {
                eprintln!("gcc: internal error: {}", s);
            } else if let Some(s) = e.downcast_ref::<String>() {
                eprintln!("gcc: internal error: {}", s);
            } else {
                eprintln!("gcc: internal error (thread panicked)");
            }
            std::process::exit(1);
        }
    }
}
