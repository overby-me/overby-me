mod cli;
mod compress;
mod decompress;
mod ops;
mod unlzw;
mod unpack;
mod util;

use std::process;

fn main() {
    let opts = cli::parse_args();

    let exit_code = if opts.files.is_empty() || (opts.files.len() == 1 && opts.files[0] == "-") {
        ops::run_stdio(&opts)
    } else {
        ops::run_files(&opts)
    };

    process::exit(exit_code);
}
