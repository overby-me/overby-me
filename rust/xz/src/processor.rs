//! Per-file driver: figure out output paths, open files, and call into
//! the codec helpers.

use std::fs::{self, File};
use std::io;
use std::path::Path;

use crate::codec::{compress_stream, decompress_or_passthrough, decompress_stream_opts};
use crate::options::{Format, Mode, Options};
use crate::suffix::{compressed_suffixes, output_path_compress, output_path_decompress};

pub fn process_file(path: &str, opts: &Options) -> io::Result<()> {
    let input_path = Path::new(path);

    if !input_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{path}: No such file or directory"),
        ));
    }

    match opts.mode {
        Mode::Compress => {
            let has_suffix = compressed_suffixes().iter().any(|s| path.ends_with(s));
            if has_suffix && !opts.force && opts.suffix.is_none() {
                if !opts.quiet {
                    eprintln!("xz: {path}: already has a compressed suffix, skipping");
                }
                return Ok(());
            }

            if opts.stdout {
                let input = File::open(input_path)?;
                compress_stream(
                    input,
                    io::stdout().lock(),
                    opts.level,
                    opts.format,
                    opts.filter.as_ref(),
                )?;
                return Ok(());
            }

            let output_path = match output_path_compress(input_path, opts.format, opts.suffix.as_deref()) {
                Some(p) => p,
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!(
                            "{path}: no suffix to use for output (use --suffix= with raw format)"
                        ),
                    ));
                }
            };

            if output_path.exists() && !opts.force {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!(
                        "{}: already exists; use -f to overwrite",
                        output_path.display()
                    ),
                ));
            }
            let input = File::open(input_path)?;
            let output = File::create(&output_path)?;
            compress_stream(input, output, opts.level, opts.format, opts.filter.as_ref())?;

            if opts.verbose {
                eprintln!("xz: {path} -> {}", output_path.display());
            }

            if !opts.keep {
                fs::remove_file(input_path)?;
            }
        }
        Mode::Decompress => {
            // Raw mode requires an explicit --suffix= for both stdin
            // and file modes when writing back to a file. When writing
            // to stdout we still need it for *file* inputs because xz
            // can't infer the output name from a raw stream.
            if opts.format == Format::Raw && opts.suffix.is_none() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{path}: raw format requires --suffix="),
                ));
            }

            // When writing to stdout we don't need a known suffix --
            // xz happily decodes anything it can recognise. With -f
            // we additionally fall back to copying unrecognised input
            // verbatim (matches xz's `-dfc` behaviour).
            if opts.stdout {
                let input = File::open(input_path)?;
                if opts.force {
                    decompress_or_passthrough(
                        input,
                        io::stdout().lock(),
                        opts.no_warn,
                        opts.format,
                        opts.filter.as_ref(),
                    )?;
                } else {
                    decompress_stream_opts(
                        input,
                        io::stdout().lock(),
                        opts.no_warn,
                        opts.format,
                        opts.filter.as_ref(),
                    )?;
                }
                return Ok(());
            }

            let output_path = match output_path_decompress(input_path, opts.suffix.as_deref()) {
                Some(p) => p,
                None => {
                    if !opts.quiet {
                        eprintln!("xz: {path}: unknown suffix -- ignored");
                    }
                    return Ok(());
                }
            };

            if output_path.exists() && !opts.force {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!(
                        "{}: already exists; use -f to overwrite",
                        output_path.display()
                    ),
                ));
            }
            let input = File::open(input_path)?;
            let output = File::create(&output_path)?;
            decompress_stream_opts(
                input,
                output,
                opts.no_warn,
                opts.format,
                opts.filter.as_ref(),
            )?;

            if opts.verbose {
                eprintln!("xz: {} -> {}", path, output_path.display());
            }

            if !opts.keep {
                fs::remove_file(input_path)?;
            }
        }
        Mode::Test => {
            let input = File::open(input_path)?;
            decompress_stream_opts(
                input,
                io::sink(),
                opts.no_warn,
                opts.format,
                opts.filter.as_ref(),
            )?;
            if opts.verbose {
                eprintln!("xz: {path}: OK");
            }
        }
        Mode::List => {
            // -l file dispatch happens in main.rs; reaching here would
            // be a programming error.
            unreachable!("Mode::List handled in main()");
        }
    }

    Ok(())
}
