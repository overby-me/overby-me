//! `opt`, with the part of upstream's interface this tier can honour.
//!
//! Reads textual IR, runs the passes it is asked for, and writes textual IR.
//! The pass set is `verify` and `no-op-module` today; anything else is an
//! error naming the pass rather than a silent no-op, because a caller that
//! asked for `instcombine` and got its input back unchanged has been lied to.
//!
//! Bitcode output is refused for the same reason: `opt` without `-S` writes
//! bitcode, and writing something else under that name would be worse than
//! saying no.

use std::io::{Read as _, Write as _};
use std::process::ExitCode;

const USAGE: &str = "\
OVERVIEW: LLVM-rs optimizer and analysis printer

USAGE: opt [options] <input .ll file>

OPTIONS:
  -S                   Write LLVM assembly rather than bitcode. Required:
                       bitcode output is not implemented yet.
  -o <file>            Write output to <file> rather than to standard output.
  -passes=<list>       Comma-separated passes to run. Implemented: verify,
                       no-op-module, no-op-function.
  --verify-each        Verify after every pass.
  --help               Print this message.
  --version            Print version information.

Reads standard input when <input> is '-'.
";

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match run(&arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("opt: {message}");
            ExitCode::FAILURE
        }
    }
}

struct Options {
    input: Option<String>,
    output: Option<String>,
    passes: Vec<String>,
    textual: bool,
    verify_each: bool,
}

fn run(arguments: &[String]) -> Result<(), String> {
    let mut options = Options {
        input: None,
        output: None,
        passes: Vec::new(),
        textual: false,
        verify_each: false,
    };

    let mut index = 0;
    while index < arguments.len() {
        let argument = arguments[index].as_str();
        match argument {
            "--help" | "-help" | "-h" => {
                print!("{USAGE}");
                return Ok(());
            }
            "--version" | "-version" => {
                println!("LLVM-rs {}", env!("CARGO_PKG_VERSION"));
                println!("  Optimized build: {}", !cfg!(debug_assertions));
                return Ok(());
            }
            "-S" => options.textual = true,
            "--verify-each" | "-verify-each" => options.verify_each = true,
            "-o" => {
                index += 1;
                let target = arguments
                    .get(index)
                    .ok_or_else(|| "-o needs a file name".to_string())?;
                options.output = Some(target.clone());
            }
            _ => {
                if let Some(list) = argument
                    .strip_prefix("-passes=")
                    .or_else(|| argument.strip_prefix("--passes="))
                {
                    options.passes.extend(
                        list.split(',')
                            .filter(|pass| !pass.is_empty())
                            .map(ToString::to_string),
                    );
                } else if let Some(target) = argument.strip_prefix("-o=") {
                    options.output = Some(target.to_string());
                } else if argument.starts_with('-') && argument != "-" {
                    return Err(format!("unknown option '{argument}'"));
                } else if options.input.is_some() {
                    return Err("more than one input file".to_string());
                } else {
                    options.input = Some(argument.to_string());
                }
            }
        }
        index += 1;
    }

    let input = options.input.unwrap_or_else(|| "-".to_string());
    if !options.textual {
        return Err("bitcode output is not implemented; pass -S to write textual IR".to_string());
    }

    let text = read_input(&input)?;
    let module = llvm_ir_parse::parse_module(&text).map_err(|error| format!("{input}: {error}"))?;

    let mut ran_verify = false;
    for pass in &options.passes {
        match pass.as_str() {
            "verify" => {
                report_verification(&module, &input)?;
                ran_verify = true;
            }
            "no-op-module" | "no-op-function" => {}
            other => {
                return Err(format!(
                    "pass '{other}' is not implemented; \
                     this tier runs verify and the no-op passes only"
                ));
            }
        }
    }
    if options.verify_each && !ran_verify {
        report_verification(&module, &input)?;
    }

    let printed = llvm_ir_print::print_module(&module);
    write_output(options.output.as_deref(), &printed)
}

fn report_verification(module: &llvm_ir::Module, input: &str) -> Result<(), String> {
    let errors = llvm_ir::verify_module(module);
    if errors.is_empty() {
        return Ok(());
    }
    let mut report = format!("{input}: the module is not well formed\n");
    for error in &errors {
        report.push_str(&format!("  {error}\n"));
    }
    Err(report.trim_end().to_string())
}

fn read_input(input: &str) -> Result<String, String> {
    if input == "-" {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .map_err(|error| format!("standard input: {error}"))?;
        return Ok(text);
    }
    std::fs::read_to_string(input).map_err(|error| format!("{input}: {error}"))
}

fn write_output(output: Option<&str>, text: &str) -> Result<(), String> {
    // `-o -` is standard output, the same way `-` is standard input.
    match output.filter(|path| *path != "-") {
        None => {
            let stdout = std::io::stdout();
            let mut handle = stdout.lock();
            handle
                .write_all(text.as_bytes())
                .map_err(|error| format!("standard output: {error}"))
        }
        Some(path) => std::fs::write(path, text).map_err(|error| format!("{path}: {error}")),
    }
}
