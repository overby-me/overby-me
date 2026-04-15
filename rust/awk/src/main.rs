#![allow(dead_code)]

mod ast;
mod format;
mod interpreter;
mod lexer;
mod parser;
mod value;

use std::env;
use std::fs;

use interpreter::Interpreter;
use lexer::Lexer;
use parser::Parser;
use value::Value;

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut program_text = String::new();
    let mut input_files: Vec<String> = Vec::new();
    let mut var_assignments: Vec<(String, String)> = Vec::new();
    let mut field_sep: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--version" | "-V" => {
                println!("awk (rust-awk) {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "-F" => {
                i += 1;
                if i < args.len() {
                    field_sep = Some(args[i].clone());
                }
            }
            s if s.starts_with("-F") => {
                field_sep = Some(s[2..].to_string());
            }
            "-v" => {
                i += 1;
                if i < args.len()
                    && let Some(eq_pos) = args[i].find('=')
                {
                    let name = args[i][..eq_pos].to_string();
                    let val = args[i][eq_pos + 1..].to_string();
                    var_assignments.push((name, val));
                }
            }
            s if s.starts_with("-v") => {
                let rest = &s[2..];
                if let Some(eq_pos) = rest.find('=') {
                    let name = rest[..eq_pos].to_string();
                    let val = rest[eq_pos + 1..].to_string();
                    var_assignments.push((name, val));
                }
            }
            "-f" => {
                i += 1;
                if i < args.len() {
                    match fs::read_to_string(&args[i]) {
                        Ok(content) => {
                            if !program_text.is_empty() {
                                program_text.push('\n');
                            }
                            program_text.push_str(&content);
                        }
                        Err(e) => {
                            eprintln!("awk: can't open source file {}: {}", args[i], e);
                            std::process::exit(2);
                        }
                    }
                }
            }
            "--" => {
                i += 1;
                while i < args.len() {
                    input_files.push(args[i].clone());
                    i += 1;
                }
                break;
            }
            s if s.starts_with('-')
                && s.len() > 1
                && program_text.is_empty()
                && input_files.is_empty() =>
            {
                // Unknown flag, skip
                eprintln!("awk: unknown option: {s}");
            }
            _ => {
                if program_text.is_empty() && !args[i].contains('=') {
                    program_text = args[i].clone();
                } else if args[i].contains('=') && program_text.is_empty() {
                    // Could be assignment or program, treat as program if no program yet
                    program_text = args[i].clone();
                } else {
                    input_files.push(args[i].clone());
                }
            }
        }
        i += 1;
    }

    if program_text.is_empty() {
        eprintln!("usage: awk [-F fs] [-v var=value] [-f progfile] 'program' [file ...]");
        std::process::exit(1);
    }

    // Tokenize and parse
    let mut lexer = Lexer::new(&program_text);
    let tokens = lexer.tokenize();
    let mut parser = Parser::new(tokens);
    let program = parser.parse();

    // Set up interpreter
    let mut interp = Interpreter::new();

    // Apply field separator
    if let Some(fs) = field_sep {
        let fs = if fs == "\\t" || fs == "\t" {
            "\t".to_string()
        } else {
            fs
        };
        interp.set_var("FS", Value::Str(fs));
    }

    // Apply variable assignments
    for (name, val) in &var_assignments {
        interp.set_var(name, Value::Str(val.clone()));
    }

    // Set up ARGV and ARGC
    interp.set_array("ARGV", "0", Value::Str(args[0].clone()));
    for (i, file) in input_files.iter().enumerate() {
        interp.set_array("ARGV", &(i + 1).to_string(), Value::Str(file.clone()));
    }
    interp.set_var(
        "ARGC",
        Value::Num((input_files.len() + 1) as f64),
    );

    // Seed RNG
    interp.rng_state = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(12345);

    // Run
    interp.run(&program, &input_files);

    std::process::exit(interp.exit_code);
}
