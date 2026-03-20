use std::io::{self, Write};
use std::process::{self, Command};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut executable = None;
    let mut output_file = None;
    let mut name = None;
    let mut section = "1".to_string();
    let mut source = String::new();
    let mut include_file = None;
    let mut no_info = false;
    let mut no_discard_stderr = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--version" => {
                println!("help2man (rust-help2man) {}", env!("CARGO_PKG_VERSION"));
                process::exit(0);
            }
            "--help" => {
                println!("Usage: help2man [OPTION]... EXECUTABLE");
                println!("Generate a man page from --help and --version output.");
                println!("  -o, --output=FILE    output to FILE");
                println!("  -N, --no-info        suppress info reference");
                println!("  -n, --name=NAME      description for NAME paragraph");
                println!("  -s, --section=SECT   section number");
                println!("  -S, --source=TEXT    source of program");
                println!("  -i, --include=FILE   include material from FILE");
                println!("  --no-discard-stderr  include stderr in output parsing");
                process::exit(0);
            }
            "-o" | "--output" => {
                i += 1;
                if i < args.len() {
                    output_file = Some(args[i].clone());
                }
            }
            "-n" | "--name" => {
                i += 1;
                if i < args.len() {
                    name = Some(args[i].clone());
                }
            }
            "-s" | "--section" => {
                i += 1;
                if i < args.len() {
                    section = args[i].clone();
                }
            }
            "-S" | "--source" => {
                i += 1;
                if i < args.len() {
                    source = args[i].clone();
                }
            }
            "-i" | "--include" => {
                i += 1;
                if i < args.len() {
                    include_file = Some(args[i].clone());
                }
            }
            "-N" | "--no-info" => no_info = true,
            "--no-discard-stderr" => no_discard_stderr = true,
            arg if arg.starts_with("--output=") => {
                output_file = Some(arg.strip_prefix("--output=").unwrap().to_string());
            }
            arg if arg.starts_with("--name=") => {
                name = Some(arg.strip_prefix("--name=").unwrap().to_string());
            }
            arg if arg.starts_with("--section=") => {
                section = arg.strip_prefix("--section=").unwrap().to_string();
            }
            arg if arg.starts_with("--source=") => {
                source = arg.strip_prefix("--source=").unwrap().to_string();
            }
            arg if arg.starts_with("--include=") => {
                include_file = Some(arg.strip_prefix("--include=").unwrap().to_string());
            }
            arg if arg.starts_with('-') => {
                // Combined short options like -Nn
                let chars: Vec<char> = arg[1..].chars().collect();
                let mut j = 0;
                while j < chars.len() {
                    match chars[j] {
                        'N' => no_info = true,
                        'o' => {
                            let rest: String = chars[j + 1..].iter().collect();
                            if !rest.is_empty() {
                                output_file = Some(rest);
                            } else {
                                i += 1;
                                if i < args.len() {
                                    output_file = Some(args[i].clone());
                                }
                            }
                            break;
                        }
                        'n' => {
                            let rest: String = chars[j + 1..].iter().collect();
                            if !rest.is_empty() {
                                name = Some(rest);
                            } else {
                                i += 1;
                                if i < args.len() {
                                    name = Some(args[i].clone());
                                }
                            }
                            break;
                        }
                        's' => {
                            let rest: String = chars[j + 1..].iter().collect();
                            if !rest.is_empty() {
                                section = rest;
                            } else {
                                i += 1;
                                if i < args.len() {
                                    section = args[i].clone();
                                }
                            }
                            break;
                        }
                        'S' => {
                            let rest: String = chars[j + 1..].iter().collect();
                            if !rest.is_empty() {
                                source = rest;
                            } else {
                                i += 1;
                                if i < args.len() {
                                    source = args[i].clone();
                                }
                            }
                            break;
                        }
                        'i' => {
                            let rest: String = chars[j + 1..].iter().collect();
                            if !rest.is_empty() {
                                include_file = Some(rest);
                            } else {
                                i += 1;
                                if i < args.len() {
                                    include_file = Some(args[i].clone());
                                }
                            }
                            break;
                        }
                        _ => {}
                    }
                    j += 1;
                }
            }
            _ => {
                executable = Some(args[i].clone());
            }
        }
        i += 1;
    }

    let exe = match executable {
        Some(e) => e,
        None => {
            eprintln!("help2man: no executable specified");
            process::exit(1);
        }
    };

    let _ = no_discard_stderr;

    // Get --version output
    let version_output = run_command(&exe, "--version");
    let help_output = run_command(&exe, "--help");

    // Parse program name and version from --version output
    let first_line = version_output.lines().next().unwrap_or(&exe);
    let prog_name = exe
        .rsplit('/')
        .next()
        .unwrap_or(&exe)
        .to_string();
    let version_str = first_line;

    let description = name.unwrap_or_else(|| {
        // Try to extract from --help
        help_output
            .lines()
            .find(|l| !l.is_empty() && !l.starts_with("Usage"))
            .unwrap_or("manual page")
            .to_string()
    });

    // Load include file
    let include_content = include_file
        .as_ref()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .unwrap_or_default();
    let _ = include_content; // TODO: parse and insert sections

    // Generate man page
    let mut man = String::new();

    // Header
    let date = "2024-01-01";
    man.push_str(&format!(
        ".TH {} {} \"{}\" \"{}\" \"\"\n",
        prog_name.to_uppercase(),
        section,
        date,
        source,
    ));

    // NAME section
    man.push_str(".SH NAME\n");
    man.push_str(&format!(
        "{} \\- {}\n",
        prog_name,
        description.trim()
    ));

    // Parse --help into sections
    let sections = parse_help(&help_output, &prog_name);
    for (title, body) in &sections {
        man.push_str(&format!(".SH {}\n", title.to_uppercase()));
        man.push_str(body);
        if !body.ends_with('\n') {
            man.push('\n');
        }
    }

    // VERSION
    if !version_output.is_empty() {
        man.push_str(".SH VERSION\n");
        man.push_str(version_str);
        man.push('\n');
    }

    // SEE ALSO
    if !no_info {
        man.push_str(".SH \"SEE ALSO\"\n");
        man.push_str(&format!(
            "The full documentation for\n.B {prog_name}\nis maintained as a Texinfo manual.\n"
        ));
    }

    // Write output
    match output_file {
        Some(path) => {
            if let Err(e) = std::fs::write(&path, &man) {
                eprintln!("help2man: {path}: {e}");
                process::exit(1);
            }
        }
        None => {
            let stdout = io::stdout();
            let mut out = stdout.lock();
            let _ = out.write_all(man.as_bytes());
        }
    }
}

fn run_command(exe: &str, flag: &str) -> String {
    match Command::new(exe).arg(flag).output() {
        Ok(output) => {
            let mut result = String::from_utf8_lossy(&output.stdout).to_string();
            if result.is_empty() {
                result = String::from_utf8_lossy(&output.stderr).to_string();
            }
            result
        }
        Err(_) => String::new(),
    }
}

fn parse_help(help: &str, prog_name: &str) -> Vec<(String, String)> {
    let mut sections = Vec::new();
    let mut current_section = String::new();
    let mut current_body = String::new();

    for line in help.lines() {
        if line.starts_with("Usage:") || line.starts_with(&format!("Usage: {prog_name}")) {
            if !current_section.is_empty() {
                sections.push((current_section.clone(), current_body.clone()));
            }
            current_section = "SYNOPSIS".to_string();
            current_body = format!(
                ".B {}\n{}\n",
                prog_name,
                line.strip_prefix("Usage: ")
                    .unwrap_or(line)
                    .strip_prefix(prog_name)
                    .unwrap_or(line)
                    .trim()
            );
        } else if line.ends_with(':') && !line.starts_with(' ') && !line.starts_with('\t') {
            // Section header like "Options:" or "Mandatory arguments:"
            if !current_section.is_empty() {
                sections.push((current_section.clone(), current_body.clone()));
            }
            current_section = line.trim_end_matches(':').to_string();
            current_body = String::new();
        } else if line.starts_with("  -") || line.starts_with("      --") {
            // Option line
            let formatted = format_option_line(line);
            current_body.push_str(&formatted);
            current_body.push('\n');
        } else if !line.is_empty() {
            if current_section.is_empty() {
                current_section = "DESCRIPTION".to_string();
            }
            current_body.push_str(line);
            current_body.push('\n');
        }
    }

    if !current_section.is_empty() {
        sections.push((current_section, current_body));
    }

    sections
}

fn format_option_line(line: &str) -> String {
    let trimmed = line.trim();
    if let Some(rest) = trimmed.strip_prefix('-') {
        // Find where the description starts (after whitespace gap)
        if let Some(pos) = rest.find("  ") {
            let opt = format!("-{}", &rest[..pos].trim());
            let desc = rest[pos..].trim();
            format!(".TP\n.B {}\n{}", opt, desc)
        } else {
            format!(".TP\n.B {}", trimmed)
        }
    } else {
        trimmed.to_string()
    }
}
