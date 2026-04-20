//! Built-in ninja subtools (`-t <name>`).
//!
//! Each tool lives in its own submodule. Dispatch happens here.

mod compdb_targets;
mod inputs;
mod multi_inputs;
mod recompact;

use crate::cli::Options;
use crate::graph::State;

pub fn run(name: &str, state: &State, opts: &Options) -> Result<u8, String> {
    match name {
        "recompact" | "restat" => recompact::run(),
        "inputs" => inputs::run(state, &opts.tool_args),
        "multi-inputs" => multi_inputs::run(state, &opts.tool_args),
        "compdb-targets" => compdb_targets::run(state, &opts.tool_args),
        other => Err(format!("unknown tool: {other}")),
    }
}
