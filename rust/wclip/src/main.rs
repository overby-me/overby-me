//! wclip — an xclip-style clipboard tool for Wayland.
//!
//! Reads from or writes to a Wayland selection by speaking the wire protocol
//! directly (no libwayland), using the `ext-data-control` or `wlr-data-control`
//! protocol. See the module docs in `proto.rs` and `wire.rs` for details.

mod args;
mod copy;
mod paste;
mod proto;
mod wire;

#[cfg(test)]
mod tests;

use std::process::ExitCode;

use args::{Args, Mode};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();

    let args = match Args::parse(&argv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("wclip: {e}");
            eprintln!("Try 'wclip -help' for more information.");
            return ExitCode::from(2);
        }
    };

    if args.show_help {
        print!("{HELP}");
        return ExitCode::SUCCESS;
    }
    if args.show_version {
        println!("wclip {VERSION}");
        return ExitCode::SUCCESS;
    }

    let result = match args.mode {
        Mode::Copy => copy::run(&args),
        Mode::Paste => paste::run(&args),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("wclip: {e}");
            ExitCode::FAILURE
        }
    }
}

const HELP: &str = "\
Usage: wclip [OPTION]... [FILE]...

An xclip-style clipboard tool for Wayland. With no mode flag it reads data
(from FILEs, or standard input) and owns a selection; with -o it prints the
current selection to standard output.

Modes:
  -i, -in            read data into a selection (default)
  -o, -out           write the selection to standard output

Options:
  -sel, -selection SEL   target selection: 'primary' (default) or 'clipboard';
                         unambiguous abbreviations such as 'c' are accepted
  -t, -target TARGET     MIME type to copy as / paste; with -o the special
                         target 'TARGETS' lists the available MIME types
  -l, -loops N           serve N paste requests before exiting (0 = unlimited)
  -r, -rmlastnl          strip a single trailing newline from the data
  -f, -filter            when copying, also echo the input to standard output
  -foreground            keep serving in the foreground (do not fork)
  -d, -display NAME      Wayland display to connect to (default $WAYLAND_DISPLAY)
  -h, -help              show this help and exit
  -version               show version information and exit

Note: like xclip, the default selection is PRIMARY (the middle-click buffer).
For the clipboard most applications paste with Ctrl+V, use '-selection clipboard'.

Examples:
  wclip -selection clipboard < file.txt     copy a file to the clipboard
  echo hi | wclip -sel c                     copy text to the clipboard
  wclip -o -sel c                            paste the clipboard to stdout
  wclip -o -sel c -t TARGETS                 list available MIME types
";
