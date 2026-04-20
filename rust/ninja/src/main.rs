//! ninja entry point. CLI dispatch only — all real work lives in submodules.

mod build;
mod cli;
mod graph;
mod manifest;
mod status;
mod tools;

use std::process::ExitCode;

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().collect();
    let opts = match cli::parse(&raw) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("ninja: {e}");
            return ExitCode::from(1);
        }
    };

    if opts.show_version {
        println!("1.13.1");
        return ExitCode::SUCCESS;
    }

    // -C must take effect before we read the manifest, exactly like the
    // reference binary, which prints the chdir banner to stdout.
    if let Some(dir) = &opts.chdir {
        if let Err(e) = std::env::set_current_dir(dir) {
            eprintln!("ninja: chdir to '{dir}' - {e}");
            return ExitCode::from(1);
        }
        println!("ninja: Entering directory `{dir}'");
    }

    match run(&opts) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("ninja: {e}");
            ExitCode::from(1)
        }
    }
}

fn run(opts: &cli::Options) -> Result<u8, String> {
    let manifest_src = std::fs::read_to_string(&opts.manifest_file)
        .map_err(|e| format!("loading '{}': {}", opts.manifest_file, e))?;
    let state =
        manifest::parse(&manifest_src).map_err(|e| format!("{}: {}", opts.manifest_file, e))?;

    if let Some(tool) = &opts.tool {
        return tools::run(tool, &state, opts);
    }

    build::run(&state, opts)
}
