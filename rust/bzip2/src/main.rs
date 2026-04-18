mod cli;
mod compress;
mod decompress;
mod process;

use std::process as stdprocess;

fn main() {
    let config = cli::parse_args();

    if config.files.is_empty() {
        if let Err(e) = process::process_stdin(&config) {
            if !config.quiet {
                eprintln!("bzip2: {e}");
            }
            stdprocess::exit(1);
        }
    } else {
        let mut had_error = false;
        for file in &config.files {
            if let Err(e) = process::process_file(&config, file) {
                if !config.quiet {
                    eprintln!("{e}");
                }
                had_error = true;
            }
        }
        if had_error {
            stdprocess::exit(1);
        }
    }
}
