#![allow(
    dead_code,
    unused_variables,
    clippy::needless_return,
    clippy::collapsible_if,
    clippy::upper_case_acronyms,
    clippy::never_loop,
    clippy::single_match,
    clippy::manual_map,
    clippy::match_like_matches_macro,
    clippy::redundant_field_names,
    clippy::ptr_arg,
    clippy::len_zero,
    clippy::comparison_chain,
    clippy::manual_range_contains,
    clippy::single_char_pattern,
    clippy::unnecessary_to_owned,
    clippy::clone_on_copy,
    clippy::needless_range_loop,
    clippy::map_clone,
    clippy::useless_format,
    clippy::manual_strip,
    clippy::unnecessary_sort_by,
    clippy::collapsible_else_if,
    clippy::needless_late_init
)]

mod ast;
mod interpreter;
mod lexer;
mod parser;
mod value;

use std::env;
use std::fs;

use interpreter::Interpreter;
use lexer::Lexer;
use parser::Parser;

fn main() {
    // Run the interpreter in a thread with a larger stack than Rust's default
    // main-thread stack (typically 8 MiB, but the interpreter is recursive
    // and Perl tests like op/cond.t build deeply-nested expressions — e.g.
    // `$x ? 1 : $x ? 1 : …` 20000 levels deep, and op/list.t does `() =
    // (1,(1,(1,…)))` 100 000 levels deep). A 1 GiB thread stack is
    // generous but keeps the cost to at-most one allocation per process.
    let handle = std::thread::Builder::new()
        .stack_size(1024 * 1024 * 1024)
        .spawn(run_interpreter)
        .expect("failed to spawn interpreter thread");
    let code = handle.join().expect("interpreter thread panicked");
    std::process::exit(code);
}

fn run_interpreter() -> i32 {
    let args: Vec<String> = env::args().collect();
    let mut program_text = String::new();
    let mut script_file = String::new();
    let mut include_dirs: Vec<String> = Vec::new();
    let mut auto_newline = false; // -l flag
    let mut warnings_flag = false; // -w flag
    let mut taint_mode_arg = false; // -T flag
    let mut record_sep_override: Option<String> = None; // -0NNN flag
    let mut loop_input = false; // -n flag (wrap script in while(<>) { … })
    let mut loop_and_print = false; // -p flag (-n plus print $_ each iter)
    let mut autosplit = false; // -a flag (split $_ into @F each iter)
    let mut autosplit_pattern: Option<String> = None; // -F pattern (sets -a too)

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--version" | "-v" => {
                println!("perl (oxidized-perl) {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "-e" | "-E" => {
                i += 1;
                if i < args.len() {
                    if !program_text.is_empty() {
                        program_text.push('\n');
                    }
                    program_text.push_str(&args[i]);
                }
            }
            s if s.starts_with("-") && s.contains('e') && !s.starts_with("-I") && s.len() > 1 => {
                // Handle combined flags like -le, -lwe, -wle, etc.
                // Everything after -e is the program text (if any), otherwise next arg
                let e_pos = s.find('e').unwrap();
                let after_e = &s[e_pos + 1..];
                if !after_e.is_empty() {
                    if !program_text.is_empty() {
                        program_text.push('\n');
                    }
                    program_text.push_str(after_e);
                } else {
                    i += 1;
                    if i < args.len() {
                        if !program_text.is_empty() {
                            program_text.push('\n');
                        }
                        program_text.push_str(&args[i]);
                    }
                }
                // Parse flags before -e
                let before_e = &s[1..e_pos];
                if before_e.contains('l') {
                    auto_newline = true;
                }
                if before_e.contains('w') || before_e.contains('W') {
                    warnings_flag = true;
                }
                if before_e.contains('T') {
                    taint_mode_arg = true;
                }
                if before_e.contains('n') {
                    loop_input = true;
                }
                if before_e.contains('p') {
                    loop_input = true;
                    loop_and_print = true;
                }
                if before_e.contains('a') {
                    // `-a` autosplit implies `-n` so `@F = split / /, $_`
                    // fires inside an input loop. perlrun: "The -a switch
                    // turns on autosplit mode when used with -n or -p"
                    // — but reference perl also turns on the input loop
                    // when -a is alone. run/switchF2.
                    autosplit = true;
                    loop_input = true;
                }
            }
            "-I" => {
                i += 1;
                if i < args.len() {
                    include_dirs.push(args[i].clone());
                }
            }
            s if s.starts_with("-I") => {
                include_dirs.push(s[2..].to_string());
            }
            "-w" | "-W" => {
                warnings_flag = true;
            }
            "-l" => {
                auto_newline = true;
            }
            "-n" => {
                loop_input = true;
            }
            "-p" => {
                loop_input = true;
                loop_and_print = true;
            }
            "-a" => {
                autosplit = true;
                loop_input = true;
            }
            s if s.starts_with("-F") && s.len() > 2 => {
                // `-FPAT` implies `-a` and `-n` (perlrun).
                autosplit = true;
                loop_input = true;
                autosplit_pattern = Some(s[2..].to_string());
            }
            "-T" => {
                taint_mode_arg = true;
            }
            // `-0NNN` — set $/ to chr(NNN) (octal by default, `x` for hex).
            // Bare `-0` is `-00` → chr(0). `-0` followed by no digits is
            // `-0` (chr 0). run/switch0.
            s if s.starts_with("-0") => {
                let digits = &s[2..];
                let code = if digits.is_empty() {
                    0u32
                } else if let Some(hex) = digits.strip_prefix("x") {
                    u32::from_str_radix(hex, 16).unwrap_or(0)
                } else {
                    u32::from_str_radix(digits, 8).unwrap_or(0)
                };
                let ch = char::from_u32(code).unwrap_or('\0');
                record_sep_override = Some(ch.to_string());
            }
            s if s.starts_with("-")
                && s.len() > 1
                && program_text.is_empty()
                && script_file.is_empty() =>
            {
                // Unknown flag, skip
            }
            _ => {
                if program_text.is_empty() && script_file.is_empty() {
                    script_file = args[i].clone();
                }
            }
        }
        i += 1;
    }

    if !script_file.is_empty() {
        if script_file == "-" {
            // Read program from stdin. Reference perl reports the file
            // label as `-` for diagnostics, which we keep here so error
            // messages match byte-for-byte under fresh_perl etc.
            use std::io::Read;
            let mut bytes = Vec::new();
            if let Err(e) = std::io::stdin().read_to_end(&mut bytes) {
                eprintln!("Can't read perl script from stdin: {}", e);
                std::process::exit(2);
            }
            program_text = String::from_utf8_lossy(&bytes).into_owned();
        } else {
            match fs::read(&script_file) {
                // Perl scripts are traditionally byte-oriented — some tests
                // ship Latin-1 content. Decode lossily so the lexer always
                // sees valid UTF-8 (invalid bytes become U+FFFD).
                Ok(bytes) => {
                    program_text = String::from_utf8_lossy(&bytes).into_owned();
                }
                Err(e) => {
                    // Reference perl prints just the OS error message
                    // text, with no `(os error N)` suffix that Rust's
                    // `io::Error` display tacks on.
                    let msg = match e.kind() {
                        std::io::ErrorKind::NotFound => "No such file or directory".to_string(),
                        std::io::ErrorKind::PermissionDenied => "Permission denied".to_string(),
                        _ => e.to_string(),
                    };
                    eprintln!("Can't open perl script \"{}\": {}", script_file, msg);
                    std::process::exit(2);
                }
            }
        }
    }

    // If no script file or -e program was given but stdin is
    // redirected (a pipe or file), reference perl reads the script
    // from stdin. Matches `perl < file.pl`. run/script test 3.
    if program_text.is_empty() && script_file.is_empty() {
        use std::io::{IsTerminal, Read};
        if !std::io::stdin().is_terminal() {
            let mut bytes = Vec::new();
            if std::io::stdin().read_to_end(&mut bytes).is_ok() {
                program_text = String::from_utf8_lossy(&bytes).into_owned();
            }
        }
    }

    if program_text.is_empty() {
        eprintln!("usage: perl [switches] [programfile] [arguments]");
        std::process::exit(1);
    }

    // Strip a leading UTF-8 BOM (U+FEFF = 0xEF 0xBB 0xBF). Reference
    // perl treats it as invisible whitespace so the script starts
    // with the first real char. io/bom.
    if let Some(rest) = program_text.strip_prefix('\u{feff}') {
        program_text = rest.to_string();
    }

    // Blank out the shebang line so it's ignored but keeps line numbers
    // accurate. Lexer treats the resulting empty line as whitespace.
    // Before blanking, check for `-T` (taint mode) on the shebang —
    // reference perl aborts when the shebang requests `-T` but the
    // command-line invocation didn't include it.
    if program_text.starts_with("#!") {
        if let Some(newline) = program_text.find('\n') {
            let shebang = &program_text[..newline];
            if has_shebang_flag(shebang, 'T') && !taint_mode_arg {
                let file_label = if script_file.is_empty() {
                    "-e"
                } else {
                    script_file.as_str()
                };
                eprintln!(
                    "\"-T\" is on the #! line, it must also be used on the command line at {file_label} line 1."
                );
                std::process::exit(255);
            }
            // `-t` (taint warnings mode) has the same shebang-vs-cmdline
            // constraint as `-T`. run/switcht.
            if has_shebang_flag(shebang, 't') && !args.iter().any(|a| a == "-t" || a == "-T") {
                let file_label = if script_file.is_empty() {
                    "-e"
                } else {
                    script_file.as_str()
                };
                eprintln!(
                    "\"-t\" is on the #! line, it must also be used on the command line at {file_label} line 1."
                );
                std::process::exit(255);
            }
            if has_shebang_flag(shebang, 'l') {
                auto_newline = true;
            }
            if has_shebang_flag(shebang, 'w') {
                warnings_flag = true;
            }
            if has_shebang_flag(shebang, 'n') {
                loop_input = true;
            }
            if has_shebang_flag(shebang, 'p') {
                loop_input = true;
                loop_and_print = true;
            }
            if has_shebang_flag(shebang, 'a') {
                autosplit = true;
            }
            // `-FPATTERN` on the shebang sets the autosplit regex.
            // `-F` in a shebang can be combined with other single-letter
            // flags: `-anFx+` = `-a -n -F x+`. Find the `F` inside any
            // `-…F…` token and take everything after as the regex.
            // `-F` implies `-an` (perlrun). Skip tokens that look like
            // `-I…` (the `I` argument-flag consumes the rest of the
            // token, so an inner `F` is part of the directory name —
            // run/switchI uses `-IFoo::Bar`).
            for token in shebang.split_whitespace() {
                if token.starts_with("-I") {
                    continue;
                }
                if let Some(rest) = token.strip_prefix('-')
                    && let Some(idx) = rest.find('F')
                {
                    let pat = &rest[idx + 1..];
                    if !pat.is_empty() {
                        autosplit = true;
                        loop_input = true;
                        autosplit_pattern = Some(pat.to_string());
                    }
                }
            }
            // `-Idir` on the shebang adds an @INC entry. Reference
            // perl unshifts each `-I` into @INC in left-to-right shebang
            // order, so the rightmost shebang `-I` ends up at @INC[0]
            // and the whole shebang block sits BEFORE the cmdline `-I`
            // dirs. run/switchI uses `#!./perl -IFoo::Bar -IBla`, which
            // expects @INC = [Bla, Foo::Bar, cmdline -I…, standard…].
            for token in shebang.split_whitespace() {
                if let Some(dir) = token.strip_prefix("-I")
                    && !dir.is_empty()
                {
                    include_dirs.insert(0, dir.to_string());
                }
            }
            // `-0NNN` on the shebang sets $/ to chr(NNN). Plain `-0`
            // (no digits) is chr(0). run/switch0.
            for token in shebang.split_whitespace() {
                if let Some(rest) = token.strip_prefix("-0") {
                    let code = if rest.is_empty() {
                        0u32
                    } else if let Some(hex) = rest.strip_prefix("x") {
                        u32::from_str_radix(hex, 16).unwrap_or(0)
                    } else {
                        u32::from_str_radix(rest, 8).unwrap_or(0)
                    };
                    let ch = char::from_u32(code).unwrap_or('\0');
                    record_sep_override = Some(ch.to_string());
                }
            }
            let rest = program_text[newline + 1..].to_string();
            program_text = format!("\n{rest}");
        }
    }

    // Tokenize
    let mut lexer = Lexer::new(&program_text);
    let tokens = lexer.tokenize();
    let token_lines = std::mem::take(&mut lexer.token_lines);
    let file_overrides = std::mem::take(&mut lexer.file_overrides);
    let data_section = std::mem::take(&mut lexer.data_section);

    if let Some(err) = lexer.error.take() {
        let file_label = if script_file.is_empty() {
            "-e"
        } else {
            script_file.as_str()
        };
        eprint!("{}", err.replace("{FILE}", file_label));
        return 255;
    }

    // Parse
    let mut parser = Parser::new_with_lines_and_files(tokens, token_lines, file_overrides);
    let mut program = parser.parse_program();

    // File-level parse errors abort compilation. We only propagate
    // errors that include `at EOF` — that's the truly-unrecoverable
    // case (e.g. `print 1+` with nothing after). Other parser-recorded
    // errors (`near "TOKEN"` style from primary's recovery path) get
    // ignored at the top level; reference perl reaches the runtime
    // for those and only fails on whatever actually breaks at exec
    // time. comp/final_line_num covers the EOF-die case.
    let pending_parse_error = parser
        .error
        .take()
        .filter(|err| err.contains("at EOF"))
        .map(|err| {
            let file_label = if script_file.is_empty() {
                "-e"
            } else {
                script_file.as_str()
            };
            err.replace("{FILE}", file_label)
        });

    // `-n` / `-p` wraps the runtime portion in `while (<>) { … }`
    // (and -p adds `continue { print or die "-p destination: $!\n"; }`).
    // BEGIN/END/Sub etc. stay at top level — they're compile-time or
    // global setup, not per-line work. run/switchn, run/switchp.
    if loop_input {
        use ast::{BinOp, Expr as E, Stmt as S, UnaryOp};
        let mut top: Vec<S> = Vec::new();
        let mut body: Vec<S> = Vec::new();
        for stmt in program {
            match &stmt {
                S::Begin(_, _)
                | S::End(_)
                | S::Check(_)
                | S::Init(_)
                | S::Sub { .. }
                | S::Package(_)
                | S::Use(_, _, _)
                | S::FileMark(_)
                | S::LineMark(_) => top.push(stmt),
                _ => body.push(stmt),
            }
        }
        // `-a` autosplit: prepend `@F = split / /, $_` (or the `-F`
        // pattern) to the loop body. run/switcha, run/switchF.
        if autosplit {
            let pat = autosplit_pattern.clone().unwrap_or_else(|| " ".to_string());
            let split_call = E::Call(
                "split".to_string(),
                vec![
                    E::RegexLit(pat, String::new()),
                    E::ScalarVar("_".to_string()),
                ],
            );
            let split_stmt = S::Expr(E::Assign(
                Box::new(E::ArrayVar("F".to_string())),
                Box::new(split_call),
            ));
            let mut new_body = vec![split_stmt];
            new_body.extend(body);
            body = new_body;
        }
        let cond = E::BinOp(
            BinOp::DefOr,
            Box::new(E::Assign(
                Box::new(E::ScalarVar("_".to_string())),
                Box::new(E::Diamond(String::new())),
            )),
            Box::new(E::BinOp(
                BinOp::And,
                Box::new(E::UnaryOp(
                    UnaryOp::Not,
                    Box::new(E::ScalarVar("_".to_string())),
                )),
                Box::new(E::IntLit(0)),
            )),
        );
        let continue_body = if loop_and_print {
            Some(vec![S::Print(None, vec![E::ScalarVar("_".to_string())])])
        } else {
            None
        };
        let while_stmt = S::While {
            cond,
            body,
            continue_body,
            label: None,
        };
        top.push(while_stmt);
        program = top;
    }

    // Execute
    let mut interp = Interpreter::new();
    if auto_newline {
        // -l flag: set $\ (output record separator) to \n
        // and auto-chomp on input
        interp.set_special_var("\\", "\n");
    }
    if let Some(rs) = record_sep_override {
        interp.set_special_var("/", &rs);
    }
    if warnings_flag {
        interp.enable_warnings();
    }
    if !script_file.is_empty() {
        interp.set_current_file(&script_file);
        interp.set_special_var("0", &script_file);
    }
    if let Some(data) = data_section {
        interp.set_data_section(data);
    }
    // Populate @INC: -I flags (prepended in order), then PERL5LIB
    // entries (also prepended; matches reference perl). Without this,
    // tests that rely on the perl install's stdlib (Carp, Exporter,
    // etc.) fail to find core modules — vanilla picks them up via its
    // compiled-in @INC, while our impl has none. PERL5LIB is the
    // standard escape hatch for adding paths without rebuilding perl.
    let mut all_dirs = include_dirs.clone();
    if let Ok(perl5lib) = env::var("PERL5LIB") {
        for p in perl5lib.split(':') {
            if !p.is_empty() {
                all_dirs.push(p.to_string());
            }
        }
    }
    interp.set_inc(&all_dirs);
    if let Some(err) = pending_parse_error {
        interp.run_with_parse_error(&program, &err);
    } else {
        interp.run(&program);
    }

    interp.exit_code
}

/// Check whether a Perl shebang line (e.g. `#!/usr/bin/perl -wT`)
/// requests a single-letter flag. Each `-flag` group's flag chars
/// run until the first `=`/`:` argument separator, or end of token —
/// so `-d:switchd_empty` declares `-d`, not also `-t` from the
/// debugger module name. run/switchd-78586.
fn has_shebang_flag(shebang: &str, flag: char) -> bool {
    for token in shebang.split_whitespace().skip(1) {
        if let Some(rest) = token.strip_prefix('-') {
            // Stop at the first arg separator (debugger spec `:NAME`,
            // module-load `=NAME`, or any digit for `-0NNN`).
            let flag_chars = rest
                .chars()
                .take_while(|c| !matches!(c, ':' | '=') && !c.is_ascii_digit() || *c == flag);
            for c in flag_chars {
                if c == flag {
                    return true;
                }
            }
        }
    }
    false
}
