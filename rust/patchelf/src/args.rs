use std::path::PathBuf;
use std::process;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug)]
#[allow(dead_code)]
pub enum Action {
    PrintInterpreter,
    SetInterpreter(String),
    PrintRpath,
    SetRpath(String),
    ShrinkRpath,
    RemoveRpath,
    PrintSoname,
    SetSoname(String),
    PrintNeeded,
    AddNeeded(String),
    RemoveNeeded(String),
    ReplaceNeeded(String, String),
    AddRpath(String),
    AddDebugTag,
    RenameDynamicSymbols(String),
    ClearSymbolVersion(String),
}

pub struct Options {
    pub actions: Vec<Action>,
    pub output: Option<PathBuf>,
    pub page_size: Option<usize>,
    pub file: Option<PathBuf>,
    pub allowed_rpath_prefixes: Option<String>,
    pub force_rpath: bool,
}

pub fn parse_args() -> Options {
    let args: Vec<String> = std::env::args().collect();
    let mut actions = Vec::new();
    let mut output = None;
    let mut page_size = None;
    let mut file = None;
    let mut allowed_rpath_prefixes: Option<String> = None;
    let mut force_rpath = false;
    let mut i = 1;

    while i < args.len() {
        match args[i].as_str() {
            "--version" => {
                println!("patchelf (rust-patchelf) {VERSION}");
                process::exit(0);
            }
            "--help" | "-h" => {
                print_usage();
                process::exit(0);
            }
            "--print-interpreter" => actions.push(Action::PrintInterpreter),
            "--set-interpreter" => {
                i += 1;
                let path = expect_arg(&args, i, "--set-interpreter");
                actions.push(Action::SetInterpreter(path));
            }
            "--print-rpath" => actions.push(Action::PrintRpath),
            "--set-rpath" => {
                i += 1;
                let path = expect_arg(&args, i, "--set-rpath");
                actions.push(Action::SetRpath(path));
            }
            "--shrink-rpath" => actions.push(Action::ShrinkRpath),
            "--remove-rpath" => actions.push(Action::RemoveRpath),
            "--force-rpath" => {
                force_rpath = true;
            }
            "--no-default-lib" | "--clear-execstack" | "--set-execstack"
            | "--print-execstack" | "--debug" => {
                // Accepted-but-not-yet-implemented flags.
            }
            "--allowed-rpath-prefixes" => {
                i += 1;
                allowed_rpath_prefixes = Some(expect_arg(&args, i, "--allowed-rpath-prefixes"));
            }
            "--add-rpath" => {
                i += 1;
                let path = expect_arg(&args, i, "--add-rpath");
                actions.push(Action::AddRpath(path));
            }
            "--rename-dynamic-symbols" => {
                i += 1;
                let path = expect_arg(&args, i, "--rename-dynamic-symbols");
                actions.push(Action::RenameDynamicSymbols(path));
            }
            "--clear-symbol-version" => {
                i += 1;
                let sym = expect_arg(&args, i, "--clear-symbol-version");
                actions.push(Action::ClearSymbolVersion(sym));
            }
            "--add-debug-tag" => actions.push(Action::AddDebugTag),
            "--print-soname" => actions.push(Action::PrintSoname),
            "--set-soname" => {
                i += 1;
                let name = expect_arg(&args, i, "--set-soname");
                actions.push(Action::SetSoname(name));
            }
            "--print-needed" => actions.push(Action::PrintNeeded),
            "--add-needed" => {
                i += 1;
                let lib = expect_arg(&args, i, "--add-needed");
                actions.push(Action::AddNeeded(lib));
            }
            "--remove-needed" => {
                i += 1;
                let lib = expect_arg(&args, i, "--remove-needed");
                actions.push(Action::RemoveNeeded(lib));
            }
            "--replace-needed" => {
                i += 1;
                let old = expect_arg(&args, i, "--replace-needed (old)");
                i += 1;
                let new = expect_arg(&args, i, "--replace-needed (new)");
                actions.push(Action::ReplaceNeeded(old, new));
            }
            "--output" => {
                i += 1;
                output = Some(PathBuf::from(expect_arg(&args, i, "--output")));
            }
            "--page-size" => {
                i += 1;
                let s = expect_arg(&args, i, "--page-size");
                page_size = Some(s.parse::<usize>().unwrap_or_else(|_| {
                    eprintln!("patchelf: invalid page size: {s}");
                    process::exit(1);
                }));
            }
            arg if arg.starts_with('-') => {
                eprintln!("patchelf: unknown option: {arg}");
                process::exit(1);
            }
            _ => {
                file = Some(PathBuf::from(&args[i]));
            }
        }
        i += 1;
    }

    if actions.is_empty() && file.is_some() {
        eprintln!("patchelf: no operation specified");
        process::exit(1);
    }

    if file.is_none() && !actions.is_empty() {
        eprintln!("patchelf: no input file specified");
        process::exit(1);
    }

    Options {
        actions,
        output,
        page_size,
        file,
        allowed_rpath_prefixes,
        force_rpath,
    }
}

fn expect_arg(args: &[String], i: usize, flag: &str) -> String {
    if i >= args.len() {
        eprintln!("patchelf: {flag} requires an argument");
        process::exit(1);
    }
    let raw = &args[i];
    if let Some(path) = raw.strip_prefix('@') {
        match std::fs::read_to_string(path) {
            Ok(s) => s.trim_end_matches(['\n', '\r']).to_string(),
            Err(e) => {
                eprintln!("patchelf: getting info about '{path}': {e}");
                process::exit(1);
            }
        }
    } else {
        raw.clone()
    }
}

pub fn print_usage() {
    println!(
        "Usage: patchelf [OPTION]... FILE

Options:
  --print-interpreter         Print the ELF interpreter
  --set-interpreter PATH      Set the ELF interpreter
  --print-rpath               Print DT_RPATH/DT_RUNPATH
  --set-rpath PATH            Set DT_RUNPATH
  --shrink-rpath              Remove unused RPATH entries
  --remove-rpath              Remove DT_RPATH and DT_RUNPATH
  --print-soname              Print DT_SONAME
  --set-soname NAME           Set DT_SONAME
  --print-needed              Print DT_NEEDED entries
  --add-needed LIB            Add a DT_NEEDED entry
  --remove-needed LIB         Remove a DT_NEEDED entry
  --replace-needed OLD NEW    Replace a DT_NEEDED entry
  --output FILE               Write to FILE instead of modifying in-place
  --page-size SIZE            Set page alignment size
  --version                   Print version
  --help                      Print this help"
    );
}
