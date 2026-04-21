//! Entry point for the `xz` CLI. The bulk of the code lives in the
//! `rust_xz` library crate; this binary just wires the modules
//! together.

use std::io;
use std::process;

use rust_xz::cli::parse_args;
use rust_xz::codec::{compress_stream, decompress_or_passthrough, decompress_stream_opts};
use rust_xz::list::list_files;
use rust_xz::options::{Mode, Options};
use rust_xz::processor::process_file;

fn run_stdio(opts: &Options) -> io::Result<()> {
    let stdin = io::stdin().lock();
    let stdout = io::stdout().lock();
    match opts.mode {
        Mode::Compress => {
            compress_stream(stdin, stdout, opts.level, opts.format, opts.filter.as_ref())
        }
        Mode::Decompress => {
            // Passthrough on unrecognised input is only enabled with
            // both -f and -c (matches xz semantics). Without stdout,
            // an error is the right answer.
            if opts.force && opts.stdout {
                decompress_or_passthrough(
                    stdin,
                    stdout,
                    opts.no_warn,
                    opts.format,
                    opts.filter.as_ref(),
                )
            } else {
                decompress_stream_opts(
                    stdin,
                    stdout,
                    opts.no_warn,
                    opts.format,
                    opts.filter.as_ref(),
                )
            }
        }
        Mode::Test => decompress_stream_opts(
            stdin,
            io::sink(),
            opts.no_warn,
            opts.format,
            opts.filter.as_ref(),
        ),
        Mode::List => {
            // -l reading from stdin is not supported (matches upstream).
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "--list does not support reading from standard input",
            ))
        }
    }
}

fn main() {
    let opts = parse_args();

    if opts.mode == Mode::List {
        match list_files(&opts.files, &mut io::stdout().lock(), opts.verbose) {
            Ok(true) => {}
            Ok(false) => process::exit(1),
            Err(e) => {
                if !opts.quiet {
                    eprintln!("xz: {e}");
                }
                process::exit(1);
            }
        }
        return;
    }

    if opts.files.is_empty() {
        if let Err(e) = run_stdio(&opts) {
            if !opts.quiet {
                eprintln!("xz: {e}");
            }
            process::exit(1);
        }
    } else {
        let mut errors = false;
        for file in &opts.files {
            if file == "-" {
                if let Err(e) = run_stdio(&opts) {
                    if !opts.quiet {
                        eprintln!("xz: stdin: {e}");
                    }
                    errors = true;
                }
            } else if let Err(e) = process_file(file, &opts) {
                if !opts.quiet {
                    eprintln!("xz: {e}");
                }
                errors = true;
            }
        }
        if errors {
            process::exit(1);
        }
    }
}
