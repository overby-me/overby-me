use crate::ast::{
    AndOr, AssignValue, CaseTerminator, Command, CompoundCommand, CondExpr, ParamOp, Pipeline,
    ProcessSubKind, Program, RedirFd, RedirectKind, Redirection, SimpleCommand, Word, WordPart,
};
use crate::interpreter::{Shell, capitalize_string, is_valid_identifier};
use std::collections::HashMap;

pub type BuiltinFn = fn(&mut Shell, &[String]) -> i32;

mod compgen;
mod exec;
mod flow;
mod fs;
mod io;
mod misc;
mod set;
mod test;
mod trap;
mod vars;

fn program_has_incomplete_funsub(program: &Program) -> bool {
    fn word_check(word: &Word) -> bool {
        crate::ast::word_has_incomplete_funsub(word)
    }
    fn cmd_check(cmd: &Command) -> bool {
        match cmd {
            Command::Simple(sc) => {
                sc.words.iter().any(word_check)
                    || sc.redirections.iter().any(|r| word_check(&r.target))
                    || sc.assignments.iter().any(|a| match &a.value {
                        AssignValue::Scalar(w) => word_check(w),
                        AssignValue::Array(items) => {
                            items.iter().any(|elem| word_check(&elem.value))
                        }
                        AssignValue::None => false,
                    })
            }
            Command::Compound(cc, redirs) => {
                redirs.iter().any(|r| word_check(&r.target)) || compound_check(cc)
            }
            Command::FunctionDef {
                body, redirections, ..
            } => redirections.iter().any(|r| word_check(&r.target)) || compound_check(body),
            Command::Coproc(_, inner) => cmd_check(inner),
        }
    }
    fn compound_check(cc: &CompoundCommand) -> bool {
        match cc {
            CompoundCommand::BraceGroup(cmds) | CompoundCommand::Subshell(cmds) => {
                cmds.iter().any(|cc| andor_check(&cc.list))
            }
            _ => false,
        }
    }
    fn andor_check(ao: &crate::ast::AndOrList) -> bool {
        pipeline_check(&ao.first) || ao.rest.iter().any(|(_, p)| pipeline_check(p))
    }
    fn pipeline_check(p: &Pipeline) -> bool {
        p.commands.iter().any(cmd_check)
    }
    program.iter().any(|cc| andor_check(&cc.list))
}

fn fix_scientific_notation(s: &str, uppercase: bool) -> String {
    let marker = if uppercase { 'E' } else { 'e' };
    if let Some(pos) = s.rfind(marker) {
        let (mantissa, exp_part) = s.split_at(pos);
        let exp_str = &exp_part[1..]; // skip 'e'/'E'
        let exp_val: i32 = exp_str.parse().unwrap_or(0);
        format!("{}{}{:+03}", mantissa, marker, exp_val)
    } else {
        s.to_string()
    }
}

fn list_all_signals() -> Vec<(i32, &'static str)> {
    vec![
        (1, "SIGHUP"),
        (2, "SIGINT"),
        (3, "SIGQUIT"),
        (4, "SIGILL"),
        (5, "SIGTRAP"),
        (6, "SIGABRT"),
        (7, "SIGBUS"),
        (8, "SIGFPE"),
        (9, "SIGKILL"),
        (10, "SIGUSR1"),
        (11, "SIGSEGV"),
        (12, "SIGUSR2"),
        (13, "SIGPIPE"),
        (14, "SIGALRM"),
        (15, "SIGTERM"),
        (16, "SIGSTKFLT"),
        (17, "SIGCHLD"),
        (18, "SIGCONT"),
        (19, "SIGSTOP"),
        (20, "SIGTSTP"),
        (21, "SIGTTIN"),
        (22, "SIGTTOU"),
        (23, "SIGURG"),
        (24, "SIGXCPU"),
        (25, "SIGXFSZ"),
        (26, "SIGVTALRM"),
        (27, "SIGPROF"),
        (28, "SIGWINCH"),
        (29, "SIGIO"),
        (30, "SIGPWR"),
        (31, "SIGSYS"),
        (34, "SIGRTMIN"),
        (35, "SIGRTMIN+1"),
        (36, "SIGRTMIN+2"),
        (37, "SIGRTMIN+3"),
        (38, "SIGRTMIN+4"),
        (39, "SIGRTMIN+5"),
        (40, "SIGRTMIN+6"),
        (41, "SIGRTMIN+7"),
        (42, "SIGRTMIN+8"),
        (43, "SIGRTMIN+9"),
        (44, "SIGRTMIN+10"),
        (45, "SIGRTMIN+11"),
        (46, "SIGRTMIN+12"),
        (47, "SIGRTMIN+13"),
        (48, "SIGRTMIN+14"),
        (49, "SIGRTMIN+15"),
        (50, "SIGRTMAX-14"),
        (51, "SIGRTMAX-13"),
        (52, "SIGRTMAX-12"),
        (53, "SIGRTMAX-11"),
        (54, "SIGRTMAX-10"),
        (55, "SIGRTMAX-9"),
        (56, "SIGRTMAX-8"),
        (57, "SIGRTMAX-7"),
        (58, "SIGRTMAX-6"),
        (59, "SIGRTMAX-5"),
        (60, "SIGRTMAX-4"),
        (61, "SIGRTMAX-3"),
        (62, "SIGRTMAX-2"),
        (63, "SIGRTMAX-1"),
        (64, "SIGRTMAX"),
    ]
}

pub fn string_to_raw_bytes(s: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(s.len());
    for ch in s.chars() {
        let cp = ch as u32;
        if cp <= 0xFF {
            bytes.push(cp as u8);
        } else {
            let mut buf = [0u8; 4];
            let encoded = ch.encode_utf8(&mut buf);
            bytes.extend_from_slice(encoded.as_bytes());
        }
    }
    bytes
}

fn interpret_echo_escapes(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('r') => result.push('\r'),
                Some('\\') => result.push('\\'),
                Some('a') => result.push('\x07'),
                Some('b') => result.push('\x08'),
                Some('f') => result.push('\x0c'),
                Some('v') => result.push('\x0b'),
                Some('e') | Some('E') => result.push('\x1b'),
                Some('c') => break, // Stop output
                Some(first @ '0'..='7') => {
                    // \0NNN or \NNN — octal escape
                    let mut val = first as u8 - b'0';
                    let max_extra = if first == '0' { 3 } else { 2 };
                    for _ in 0..max_extra {
                        let mut peek = chars.clone();
                        match peek.next() {
                            Some(c @ '0'..='7') => {
                                val = val * 8 + (c as u8 - b'0');
                                chars = peek;
                            }
                            _ => break,
                        }
                    }
                    result.push(val as char);
                }
                Some('x') => {
                    let mut val = 0u8;
                    let mut count = 0;
                    for _ in 0..2 {
                        let mut peek = chars.clone();
                        match peek.next() {
                            Some(c) if c.is_ascii_hexdigit() => {
                                val = val * 16 + c.to_digit(16).unwrap() as u8;
                                chars = peek;
                                count += 1;
                            }
                            _ => break,
                        }
                    }
                    if count == 0 {
                        // No hex digits: output literal \x
                        result.push('\\');
                        result.push('x');
                    } else {
                        result.push(val as char);
                    }
                }
                Some('u') => {
                    let mut val = 0u32;
                    for _ in 0..4 {
                        let mut peek = chars.clone();
                        match peek.next() {
                            Some(c) if c.is_ascii_hexdigit() => {
                                val = val * 16 + c.to_digit(16).unwrap();
                                chars = peek;
                            }
                            _ => break,
                        }
                    }
                    if let Some(c) = char::from_u32(val) {
                        result.push(c);
                    }
                }
                Some('U') => {
                    let mut val = 0u32;
                    for _ in 0..8 {
                        let mut peek = chars.clone();
                        match peek.next() {
                            Some(c) if c.is_ascii_hexdigit() => {
                                val = val * 16 + c.to_digit(16).unwrap();
                                chars = peek;
                            }
                            _ => break,
                        }
                    }
                    if let Some(c) = char::from_u32(val) {
                        result.push(c);
                    }
                }
                Some(c) => {
                    result.push('\\');
                    result.push(c);
                }
                None => result.push('\\'),
            }
        } else {
            result.push(ch);
        }
    }
    result
}

fn parse_printf_int(arg: &str) -> i64 {
    if arg.starts_with("0x") || arg.starts_with("0X") {
        i64::from_str_radix(&arg[2..], 16).unwrap_or(0)
    } else if arg.starts_with("0") && arg.len() > 1 && !arg.contains(['8', '9']) {
        i64::from_str_radix(&arg[1..], 8).unwrap_or(0)
    } else if arg.starts_with('\'') || arg.starts_with('"') {
        arg.chars().nth(1).map(|c| c as i64).unwrap_or(0)
    } else {
        arg.parse().unwrap_or(0)
    }
}

fn quote_for_declare(s: &str) -> String {
    let needs_dollar_quote =
        s.bytes().any(|b| b < 0x20 || b == 0x7f || b > 0x7f) || s.contains('\'');
    if needs_dollar_quote {
        let mut out = String::from("$'");
        for b in s.bytes() {
            match b {
                b'\n' => out.push_str("\\n"),
                b'\t' => out.push_str("\\t"),
                b'\r' => out.push_str("\\r"),
                0x07 => out.push_str("\\a"),
                0x08 => out.push_str("\\b"),
                0x1b => out.push_str("\\E"),
                b'\'' => out.push_str("\\'"),
                b'\\' => out.push_str("\\\\"),
                b if b < 0x20 || b == 0x7f => {
                    // Use octal format like bash
                    out.push_str(&format!("\\{:03o}", b));
                }
                b if b > 0x7f => {
                    // Non-ASCII byte: output as octal
                    out.push_str(&format!("\\{:03o}", b));
                }
                b => out.push(b as char),
            }
        }
        out.push('\'');
        out
    } else {
        format!("\"{}\"", s)
    }
}

fn io_error_message(e: &std::io::Error) -> &'static str {
    match e.kind() {
        std::io::ErrorKind::NotFound => "No such file or directory",
        std::io::ErrorKind::PermissionDenied => "Permission denied",
        std::io::ErrorKind::AlreadyExists => "File exists",
        std::io::ErrorKind::BrokenPipe => "Broken pipe",
        std::io::ErrorKind::InvalidInput => "Invalid argument",
        _ => "Input/output error",
    }
}

fn shell_escape(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    // Check if the string needs quoting
    let needs_quoting = s
        .chars()
        .any(|c| !c.is_ascii_alphanumeric() && c != '_' && c != '/' && c != '.' && c != '-');
    if !needs_quoting {
        return s.to_string();
    }
    // Check if we can use simple backslash quoting (no control/non-ASCII chars)
    let has_control = s
        .chars()
        .any(|c| c.is_ascii_control() || (c as u32) >= 0x80);
    if !has_control {
        let mut result = String::new();
        for ch in s.chars() {
            if !ch.is_ascii_alphanumeric() && ch != '_' && ch != '/' && ch != '.' && ch != '-' {
                result.push('\\');
            }
            result.push(ch);
        }
        return result;
    }
    // Use $'...' quoting for strings with control characters
    let mut result = String::from("$'");
    for ch in s.chars() {
        match ch {
            '\'' => result.push_str("\\'"),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\t' => result.push_str("\\t"),
            '\r' => result.push_str("\\r"),
            '\x07' => result.push_str("\\a"),
            '\x08' => result.push_str("\\b"),
            '\x0c' => result.push_str("\\f"),
            '\x0b' => result.push_str("\\v"),
            '\x1b' => result.push_str("\\E"),
            c if c.is_ascii_graphic() || c == ' ' => result.push(c),
            c => {
                // Use octal format: for Latin-1 range (U+0080..U+00FF),
                // output as single byte; for others, use UTF-8 bytes
                let cp = c as u32;
                if cp <= 0xFF {
                    result.push_str(&format!("\\{:03o}", cp));
                } else {
                    let mut buf = [0u8; 4];
                    let encoded = c.encode_utf8(&mut buf);
                    for b in encoded.as_bytes() {
                        result.push_str(&format!("\\{:03o}", b));
                    }
                }
            }
        }
    }
    result.push('\'');
    result
}

fn format_word(word: &Word) -> String {
    let mut s = String::new();
    for part in word {
        match part {
            WordPart::Literal(t) => s.push_str(t),
            WordPart::SingleQuoted(t) => {
                // Use \char escaping for shell metacharacters (bash style)
                let all_meta = !t.is_empty()
                    && t.chars().all(|c| {
                        matches!(
                            c,
                            '$' | '`'
                                | '\\'
                                | '&'
                                | '|'
                                | ';'
                                | '<'
                                | '>'
                                | '{'
                                | '}'
                                | '%'
                                | '!'
                                | '#'
                                | '*'
                                | '?'
                                | '['
                                | ']'
                                | '~'
                        )
                    });
                if all_meta {
                    for ch in t.chars() {
                        s.push('\\');
                        s.push(ch);
                    }
                } else {
                    // Bash preserves raw bytes (including control chars) in single quotes
                    s.push('\'');
                    s.push_str(t);
                    s.push('\'');
                }
            }
            WordPart::DoubleQuoted(parts) => {
                s.push('"');
                for p in parts {
                    match p {
                        WordPart::Literal(t) => s.push_str(t),
                        WordPart::Variable(name) => {
                            s.push('$');
                            s.push_str(name);
                        }
                        WordPart::Param(expr) => {
                            s.push_str(&format_param_expr(&expr.name, &expr.op));
                        }
                        WordPart::CommandSub(cmd) => {
                            s.push_str("$(");
                            s.push_str(cmd);
                            s.push(')');
                        }
                        WordPart::BacktickSub(cmd) => {
                            s.push('`');
                            s.push_str(cmd);
                            s.push('`');
                        }
                        WordPart::ArithSub(expr) => {
                            s.push_str("$((");
                            s.push_str(expr);
                            s.push_str("))");
                        }
                        _ => s.push_str(&format_word_part(p)),
                    }
                }
                s.push('"');
            }
            _ => s.push_str(&format_word_part(part)),
        }
    }
    s
}

fn format_word_part(part: &WordPart) -> String {
    match part {
        WordPart::Literal(t) => t.clone(),
        WordPart::SingleQuoted(t) => {
            // Bash preserves raw bytes (including control chars) in single-quoted strings
            format!("'{}'", t)
        }
        WordPart::DoubleQuoted(parts) => {
            let mut s = String::from("\"");
            for p in parts {
                s.push_str(&format_word_part(p));
            }
            s.push('"');
            s
        }
        WordPart::Tilde(user) => format!("~{}", user),
        WordPart::Variable(name) => format!("${}", name),
        WordPart::Param(expr) => format_param_expr(&expr.name, &expr.op),
        WordPart::CommandSub(cmd) => {
            let trimmed = cmd.trim();
            // Normalize $(< file) — ensure space after <
            if let Some(rest) = trimmed.strip_prefix('<')
                && !rest.starts_with(' ')
                && !rest.starts_with('<')
            {
                return format!("$(< {})", rest.trim_start());
            }
            format!("$({})", trimmed)
        }
        WordPart::BacktickSub(cmd) => format!("`{}`", cmd),
        WordPart::ArithSub(expr) => format!("$(({}))", expr),
        WordPart::ProcessSub(kind, cmd) => match kind {
            ProcessSubKind::Input => format!("<({})", cmd),
            ProcessSubKind::Output => format!(">({})", cmd),
        },
        WordPart::BadSubstitution(expr) => expr.clone(),
    }
}

fn format_param_expr(name: &str, op: &ParamOp) -> String {
    match op {
        ParamOp::None => format!("${{{}}}", name),
        ParamOp::Length => format!("${{#{}}}", name),
        ParamOp::Indirect => format!("${{!{}}}", name),
        ParamOp::NamePrefix(ch) => format!("${{!{}{}}}", name, ch),
        ParamOp::ArrayIndices(ch) => format!("${{!{}[{}]}}", name, ch),
        ParamOp::Default(colon, w) => {
            let op_str = if *colon { ":-" } else { "-" };
            format!("${{{}{}{}}}", name, op_str, format_word(w))
        }
        ParamOp::Assign(colon, w) => {
            let op_str = if *colon { ":=" } else { "=" };
            format!("${{{}{}{}}}", name, op_str, format_word(w))
        }
        ParamOp::Error(colon, w) => {
            let op_str = if *colon { ":?" } else { "?" };
            format!("${{{}{}{}}}", name, op_str, format_word(w))
        }
        ParamOp::Alt(colon, w) => {
            let op_str = if *colon { ":+" } else { "+" };
            format!("${{{}{}{}}}", name, op_str, format_word(w))
        }
        ParamOp::TrimSmallLeft(w) => format!("${{{}#{}}}", name, format_word(w)),
        ParamOp::TrimLargeLeft(w) => format!("${{{}##{}}}", name, format_word(w)),
        ParamOp::TrimSmallRight(w) => format!("${{{}%{}}}", name, format_word(w)),
        ParamOp::TrimLargeRight(w) => format!("${{{}%%{}}}", name, format_word(w)),
        ParamOp::Replace(pat, rep) => {
            format!("${{{}/{}/{}}}", name, format_word(pat), format_word(rep))
        }
        ParamOp::ReplaceAll(pat, rep) => {
            format!("${{{}//{}/{}}}", name, format_word(pat), format_word(rep))
        }
        ParamOp::ReplacePrefix(pat, rep) => {
            format!("${{{}/#/{}/{}}}", name, format_word(pat), format_word(rep))
        }
        ParamOp::ReplaceSuffix(pat, rep) => {
            format!("${{{}/%/{}/{}}}", name, format_word(pat), format_word(rep))
        }
        ParamOp::Substring(offset, len) => {
            if let Some(l) = len {
                format!("${{{}:{}:{}}}", name, offset, l)
            } else {
                format!("${{{}:{}}}", name, offset)
            }
        }
        ParamOp::UpperFirst(w) => format!("${{{}^{}}}", name, format_word(w)),
        ParamOp::UpperAll(w) => format!("${{{}^^{}}}", name, format_word(w)),
        ParamOp::LowerFirst(w) => format!("${{{},{}}}", name, format_word(w)),
        ParamOp::LowerAll(w) => format!("${{{},, {}}}", name, format_word(w)),
        ParamOp::ToggleFirst(w) => format!("${{{}~{}}}", name, format_word(w)),
        ParamOp::ToggleAll(w) => format!("${{{}~~{}}}", name, format_word(w)),
        ParamOp::Transform(ch) => format!("${{{}@{}}}", name, ch),
    }
}


fn format_redirection(redir: &Redirection) -> String {
    let mut s = String::new();
    // For dup redirects with no explicit fd, print the default
    if redir.fd.is_none() {
        match redir.kind {
            RedirectKind::DupOutput => s.push('1'),
            RedirectKind::DupInput => s.push('0'),
            _ => {}
        }
    }
    if let Some(ref fd) = redir.fd {
        match fd {
            RedirFd::Number(n) => {
                // Only print fd number when it differs from the default
                match redir.kind {
                    RedirectKind::DupInput | RedirectKind::DupOutput => {
                        // Always print fd for dup redirects
                        s.push_str(&n.to_string());
                    }
                    RedirectKind::Input
                    | RedirectKind::ReadWrite
                    | RedirectKind::HereDoc(_, _)
                    | RedirectKind::HereString
                    | RedirectKind::ProcessSubIn => {
                        if *n != 0 {
                            s.push_str(&n.to_string());
                        }
                    }
                    _ => {
                        if *n != 1 {
                            s.push_str(&n.to_string());
                        }
                    }
                }
            }
            RedirFd::Var(name) => {
                s.push('{');
                s.push_str(name);
                s.push('}');
            }
        }
    }
    match redir.kind {
        RedirectKind::Input => s.push_str("< "),
        RedirectKind::Output => s.push_str("> "),
        RedirectKind::Append => s.push_str(">> "),
        RedirectKind::Clobber => s.push_str(">| "),
        RedirectKind::DupInput => s.push_str("<&"),
        RedirectKind::DupOutput => s.push_str(">&"),
        RedirectKind::ReadWrite => s.push_str("<> "),
        RedirectKind::HereDoc(strip, ref delim) => {
            if strip {
                s.push_str("<<-");
            } else {
                s.push_str("<<");
            }
            s.push_str(delim);
            s.push('\n');
            s.push_str(&format_word(&redir.target));
            s.push('\n');
            s.push_str(delim);
            return s;
        }
        RedirectKind::HereString => s.push_str("<<< "),
        RedirectKind::OutputAll => s.push_str("&> "),
        RedirectKind::AppendAll => s.push_str("&>> "),
        RedirectKind::ProcessSubIn => s.push_str("< "),
        RedirectKind::ProcessSubOut => s.push_str("> "),
    }
    s.push_str(&format_word(&redir.target));
    s
}

fn format_simple_command(cmd: &SimpleCommand) -> String {
    let mut parts = Vec::new();
    for a in &cmd.assignments {
        let op = if a.append { "+=" } else { "=" };
        match &a.value {
            AssignValue::None => parts.push(a.name.clone()),
            AssignValue::Scalar(w) => parts.push(format!("{}{}{}", a.name, op, format_word(w))),
            AssignValue::Array(elements) => {
                let elems: Vec<String> = elements
                    .iter()
                    .map(|e| {
                        if let Some(ref idx) = e.index {
                            format!("[{}]={}", format_word(idx), format_word(&e.value))
                        } else {
                            format_word(&e.value)
                        }
                    })
                    .collect();
                parts.push(format!("{}{}({})", a.name, op, elems.join(" ")));
            }
        }
    }
    for w in &cmd.words {
        parts.push(format_word(w));
    }
    // Put non-heredoc redirects and heredoc headers on the command line,
    // then append heredoc bodies after
    let mut heredoc_bodies = Vec::new();
    for r in &cmd.redirections {
        let formatted = format_redirection(r);
        if matches!(r.kind, RedirectKind::HereDoc(..)) {
            // Split heredoc: <<DELIM on command line, body\nDELIM after
            if let Some(first_nl) = formatted.find('\n') {
                parts.push(formatted[..first_nl].to_string());
                heredoc_bodies.push(formatted[first_nl..].to_string());
            } else {
                parts.push(formatted);
            }
        } else {
            parts.push(formatted);
        }
    }
    let mut result = parts.join(" ");
    for body in heredoc_bodies {
        result.push_str(&body);
    }
    result
}

fn format_pipeline_indent(pipeline: &Pipeline, indent: usize) -> String {
    let mut s = String::new();
    if pipeline.negated {
        s.push_str("! ");
    }
    if pipeline.timed {
        if pipeline.time_posix {
            s.push_str("time -p ");
        } else {
            s.push_str("time ");
        }
    }
    for (i, cmd) in pipeline.commands.iter().enumerate() {
        if i > 0 {
            // Check if the pipe connection has |& (pipe stderr)
            if i - 1 < pipeline.pipe_stderr.len() && pipeline.pipe_stderr[i - 1] {
                s.push_str(" 2>&1 | ");
            } else {
                s.push_str(" | ");
            }
        }
        s.push_str(&format_command_indent(cmd, indent));
    }
    s
}

fn format_command_indent(cmd: &Command, indent: usize) -> String {
    match cmd {
        Command::Simple(sc) => format_simple_command(sc),
        Command::Compound(cc, redirections) => {
            let mut s = format_compound_command_indent(cc, indent);
            // Put non-heredoc redirects on the command line first,
            // then append heredoc bodies
            let mut heredoc_parts = Vec::new();
            for r in redirections {
                if matches!(r.kind, RedirectKind::HereDoc(..)) {
                    heredoc_parts.push(format_redirection(r));
                } else {
                    s.push(' ');
                    s.push_str(&format_redirection(r));
                }
            }
            for h in heredoc_parts {
                // Heredoc format: <<DELIM\nbody\nDELIM
                // Split at first \n to put <<DELIM on command line
                if let Some(first_nl) = h.find('\n') {
                    s.push(' ');
                    s.push_str(&h[..first_nl]);
                    s.push_str(&h[first_nl..]);
                } else {
                    s.push(' ');
                    s.push_str(&h);
                }
            }
            s
        }
        Command::FunctionDef {
            name,
            body,
            has_function_keyword,
            redirections,
            ..
        } => {
            // Bash always prints 'function' for nested definitions
            let prefix = if *has_function_keyword || indent > 0 {
                "function "
            } else {
                ""
            };
            // Bash always wraps function bodies in { ... } even if originally ( ... )
            let body_str = format_func_body_with_redirs(body, indent, redirections);
            format!("{}{} () \n{}", prefix, name, body_str)
        }
        Command::Coproc(name, inner) => {
            let inner_str = format_command_indent(inner, indent);
            // Only show explicit COPROC name for compound commands (subshells/braces)
            let is_compound = matches!(inner.as_ref(), Command::Compound(..));
            match name.as_deref() {
                None => format!("coproc {}", inner_str),
                Some("COPROC") if !is_compound => format!("coproc {}", inner_str),
                Some(n) => format!("coproc {} {}", n, inner_str),
            }
        }
    }
}

fn format_program(program: &Program, indent: usize) -> String {
    format_program_impl(program, indent, indent > 1)
}

fn format_program_impl(program: &Program, indent: usize, semi_last: bool) -> String {
    let prefix = "    ".repeat(indent);
    let mut lines = Vec::new();
    let mut pending_bg: Option<String> = None;
    for (idx, cc) in program.iter().enumerate() {
        let mut line = String::new();
        // If previous command was background, combine on same line
        if let Some(bg_line) = pending_bg.take() {
            line.push_str(&bg_line);
            line.push(' ');
        } else {
            line.push_str(&prefix);
        }
        line.push_str(&format_pipeline_indent(&cc.list.first, indent));
        for (op, pipeline) in &cc.list.rest {
            match op {
                AndOr::And => line.push_str(" && "),
                AndOr::Or => line.push_str(" || "),
            }
            line.push_str(&format_pipeline_indent(pipeline, indent));
        }
        if cc.background {
            line.push_str(" &");
            // Save this line to combine with next command
            pending_bg = Some(line);
            continue;
        }
        // Add semicolons after commands (bash style):
        {
            let is_last = idx == program.len() - 1;
            let add_semi = if is_last { semi_last } else { true };
            if add_semi {
                let trimmed = line.trim_end();
                let is_keyword = trimmed.ends_with('{')
                    || trimmed.ends_with("then")
                    || trimmed.ends_with("do")
                    || trimmed.ends_with("else");
                // Don't add ; if the command ends with a heredoc body
                // (the last line is the heredoc delimiter, e.g., "EOF")
                let ends_with_heredoc = if let Some(last_line) = line.rsplit('\n').next() {
                    let llt = last_line.trim();
                    !llt.is_empty()
                        && !llt.contains(' ')
                        && !llt.contains('\t')
                        && !llt.ends_with(';')
                        && !llt.ends_with('}')
                        && !llt.ends_with(')')
                        && line.contains("<<")
                } else {
                    false
                };
                if !is_keyword
                    && !trimmed.ends_with('&')
                    && !trimmed.is_empty()
                    && !ends_with_heredoc
                {
                    line.push(';');
                }
                // Add blank line after heredoc body (bash puts \n after delimiter)
                if ends_with_heredoc {
                    line.push('\n');
                }
            }
        }
        lines.push(line);
    }
    // If last command was background, push it
    if let Some(bg_line) = pending_bg {
        lines.push(bg_line);
    }
    lines.join("\n")
}

fn format_cond_expr(expr: &CondExpr) -> String {
    match expr {
        CondExpr::Unary(op, word) => format!("{} {}", op, format_word(word)),
        CondExpr::Binary(left, op, right) => {
            format!("{} {} {}", format_word(left), op, format_word(right))
        }
        CondExpr::Not(inner) => format!("! {}", format_cond_expr(inner)),
        CondExpr::And(left, right) => {
            format!("{} && {}", format_cond_expr(left), format_cond_expr(right))
        }
        CondExpr::Or(left, right) => {
            format!("{} || {}", format_cond_expr(left), format_cond_expr(right))
        }
        CondExpr::Word(word) => format_word(word),
    }
}

/// Format a function body — bash always wraps in { ... } even for subshell bodies
pub fn format_func_body_with_redirs(
    body: &CompoundCommand,
    indent: usize,
    redirections: &[Redirection],
) -> String {
    let redir_str = if redirections.is_empty() {
        String::new()
    } else {
        let parts: Vec<String> = redirections.iter().map(format_redirection).collect();
        format!(" {}", parts.join(" "))
    };
    let iprefix = "    ".repeat(indent);
    match body {
        CompoundCommand::BraceGroup(_) => {
            // Add iprefix to opening brace (format_compound_command_indent
            // no longer adds it to allow inline BraceGroup to avoid double-indent)
            let s = format_compound_command_indent(body, indent);
            format!("{}{}{}", iprefix, s, redir_str)
        }
        // Non-brace body (e.g., subshell): wrap in { ... }, redirections go inside
        other => {
            let inner_prefix = "    ".repeat(indent + 1);
            let inner = format_compound_command_indent(other, 0);
            format!(
                "{}{{ \n{}{}{}\n{}}}",
                iprefix, inner_prefix, inner, redir_str, iprefix
            )
        }
    }
}

fn format_compound_command_indent(cmd: &CompoundCommand, indent: usize) -> String {
    let iprefix = "    ".repeat(indent);
    match cmd {
        CompoundCommand::BraceGroup(program) => {
            if program.is_empty() {
                format!("{{ \n{}}}", iprefix)
            } else {
                format!(
                    "{{ \n{}\n{}}}",
                    format_program_impl(program, indent + 1, false),
                    iprefix
                )
            }
        }
        CompoundCommand::Subshell(program) => {
            let body = format_program(program, 0);
            let trimmed = body.trim();
            // Single simple command (possibly with heredoc) → ( cmd ... )
            let is_single_simple_cmd = program.len() == 1
                && program[0].list.rest.is_empty()
                && !program[0].background
                && program[0].list.first.commands.len() == 1
                && matches!(
                    &program[0].list.first.commands[0],
                    crate::ast::Command::Simple(_)
                );
            if is_single_simple_cmd {
                // Format as ( cmd\nheredoc\n ) for heredocs or ( cmd ) for simple
                let cmd_str = trimmed.trim_end_matches(';');
                if cmd_str.contains('\n') {
                    format!("( {}\n )", cmd_str)
                } else {
                    format!("( {} )", cmd_str)
                }
            } else if !trimmed.contains('\n') {
                format!("( {} )", trimmed.trim_end_matches(';'))
            } else {
                // Check if body is a single compound command with a brace group
                // and redirections on the command — format as ( { ... } ) redirects
                let single_compound = if program.len() == 1
                    && program[0].list.rest.is_empty()
                    && !program[0].background
                    && program[0].list.first.commands.len() == 1
                {
                    let cmd = &program[0].list.first.commands[0];
                    if let crate::ast::Command::Compound(
                        CompoundCommand::BraceGroup(inner),
                        redirs,
                    ) = cmd
                    {
                        Some((inner, redirs))
                    } else {
                        None
                    }
                } else {
                    None
                };

                if let Some((inner, redirs)) = single_compound {
                    let inner_body = format_program_impl(inner, indent + 1, false);
                    let redir_str: String = redirs
                        .iter()
                        .map(|r| format!(" {}", format_redirection(r)))
                        .collect();
                    format!("( {{ \n{}\n{}}} ){redir_str}", inner_body, iprefix,)
                } else {
                    format!("( \n{}\n )", format_program(program, indent + 1))
                }
            }
        }
        CompoundCommand::If(clause) => {
            let mut s = String::from("if ");
            let cond = format_program(&clause.condition, 0);
            let cond = cond.trim().trim_end_matches(';');
            s.push_str(cond);
            s.push_str("; then\n");
            s.push_str(&format_program(&clause.then_body, indent + 1));
            // Bash expands elif to nested else { if ... fi }
            let mut remaining_elifs = clause.elif_parts.iter().peekable();
            let else_body_ref = clause.else_body.as_ref();
            if remaining_elifs.peek().is_some() {
                // Build nested else/if structure
                let mut else_content = String::new();
                let mut nest_level = 0;
                for (elif_cond, elif_body) in remaining_elifs {
                    let inner_prefix = "    ".repeat(indent + 1 + nest_level);
                    let c = format_program(elif_cond, 0);
                    let c = c.trim().trim_end_matches(';');
                    else_content.push_str(&format!(
                        "\n{}else\n{}if {}; then\n{}",
                        "    ".repeat(indent + nest_level),
                        inner_prefix,
                        c,
                        format_program(elif_body, indent + 2 + nest_level)
                    ));
                    nest_level += 1;
                }
                if let Some(eb) = else_body_ref {
                    else_content.push_str(&format!(
                        "\n{}else\n{}",
                        "    ".repeat(indent + nest_level),
                        format_program(eb, indent + 1 + nest_level)
                    ));
                }
                // Close all nested fi's (all get ; since they're inside the else)
                for i in (0..nest_level).rev() {
                    else_content.push_str(&format!("\n{}fi;", "    ".repeat(indent + 1 + i)));
                }
                s.push_str(&else_content);
            } else if let Some(else_body) = else_body_ref {
                s.push_str(&format!("\n{iprefix}else\n"));
                s.push_str(&format_program(else_body, indent + 1));
            }
            s.push_str(&format!("\n{iprefix}fi"));
            s
        }
        CompoundCommand::For(clause) => {
            let mut s = format!("for {} in", clause.var);
            if let Some(ref words) = clause.words {
                for w in words {
                    s.push(' ');
                    s.push_str(&format_word(w));
                }
            }
            s.push_str(&format!(";\n{iprefix}do\n"));
            s.push_str(&format_program(&clause.body, indent + 1));
            s.push_str(&format!("\n{iprefix}done"));
            s
        }
        CompoundCommand::ArithFor(clause) => {
            let init = if clause.init.trim().is_empty() {
                "1".to_string()
            } else {
                clause.init.trim().to_string()
            };
            let cond = if clause.cond.trim().is_empty() {
                "1".to_string()
            } else {
                clause.cond.trim().to_string()
            };
            // Step: keep trailing whitespace from original, empty → "1"
            let step_part = if clause.step.trim().is_empty() {
                "1".to_string()
            } else {
                // Trim start but keep trailing whitespace
                clause.step.trim_start().to_string()
            };
            let mut s = format!("for (({init}; {cond}; {step_part}))\n{iprefix}do\n");
            s.push_str(&format_program(&clause.body, indent + 1));
            s.push_str(&format!("\n{iprefix}done"));
            s
        }
        CompoundCommand::While(clause) => {
            let cond = format_program(&clause.condition, 0);
            let cond = cond.trim().trim_end_matches(';');
            let mut s = format!("while {}; do\n", cond);
            s.push_str(&format_program(&clause.body, indent + 1));
            s.push_str(&format!("\n{iprefix}done"));
            s
        }
        CompoundCommand::Until(clause) => {
            let cond = format_program(&clause.condition, 0);
            let cond = cond.trim().trim_end_matches(';');
            let mut s = format!("until {}; do\n", cond);
            s.push_str(&format_program(&clause.body, indent + 1));
            s.push_str(&format!("\n{iprefix}done"));
            s
        }
        CompoundCommand::Case(clause) => {
            let pat_prefix = "    ".repeat(indent + 1);
            let mut s = format!("case {} in \n", format_word(&clause.word));
            for item in &clause.items {
                let patterns: Vec<String> = item.patterns.iter().map(format_word).collect();
                s.push_str(&format!("{pat_prefix}{})\n", patterns.join(" | ")));
                let body = format_program(&item.body, indent + 2);
                let body = body.trim_end_matches(';');
                s.push_str(body);
                s.push('\n');
                let term = match item.terminator {
                    CaseTerminator::Break => ";;",
                    CaseTerminator::FallThrough => ";&",
                    CaseTerminator::TestNext => ";;&",
                };
                s.push_str(&format!("{pat_prefix}{term}\n"));
            }
            s.push_str(&format!("{iprefix}esac"));
            s
        }
        CompoundCommand::Conditional(expr) => {
            format!("[[ {} ]]", format_cond_expr(expr))
        }
        CompoundCommand::Arithmetic(expr) => {
            format!("(( {} ))", expr.trim())
        }
    }
}

pub use vars::parse_assoc_literal;

pub fn parse_array_literal(s: &str) -> Vec<String> {
    let trimmed = s.trim();
    let inner = if trimmed.starts_with('(') && trimmed.ends_with(')') {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };

    if inner.trim().is_empty() {
        return Vec::new();
    }

    // Check for \x1F separator (from parser's inline array handling)
    if inner.contains('\x1F') {
        return inner.split('\x1F').map(|s| s.to_string()).collect();
    }

    // Simple word splitting, respecting quotes
    let mut elements = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escape_next = false;

    for ch in inner.chars() {
        if escape_next {
            current.push(ch);
            escape_next = false;
            continue;
        }
        if ch == '\\' && !in_single_quote {
            escape_next = true;
            continue;
        }
        if ch == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            continue;
        }
        if ch == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            continue;
        }
        if ch.is_whitespace() && !in_single_quote && !in_double_quote {
            if !current.is_empty() {
                elements.push(current.clone());
                current.clear();
            }
            continue;
        }
        current.push(ch);
    }
    if !current.is_empty() {
        elements.push(current);
    }
    elements
}

/// Quote a value for `set` output, matching bash's format.
/// Values that need quoting are wrapped in $'...' with proper escaping.
fn quote_value_for_set(value: &str) -> String {
    // Check if the value needs quoting
    let needs_quoting = value.is_empty()
        || value.starts_with('~')
        || value.starts_with('#')
        || value
            .chars()
            .any(|c| " \t\n\\\"'`$!&|;()<>{}[]?*".contains(c));

    if !needs_quoting {
        return value.to_string();
    }

    // Use single-quote style with \' for embedded single quotes
    // Bash uses a mix: simple values get \-escaping, complex ones get $'...' or '...'
    let mut out = String::new();
    let mut needs_dollar = false;

    for ch in value.chars() {
        match ch {
            '\n' | '\t' | '\r' | '\x07' | '\x08' | '\x0b' | '\x0c' | '\x1b' => {
                needs_dollar = true;
            }
            _ => {}
        }
    }

    if needs_dollar {
        out.push_str("$'");
        for ch in value.chars() {
            match ch {
                '\'' => out.push_str("\\'"),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\t' => out.push_str("\\t"),
                '\r' => out.push_str("\\r"),
                '\x07' => out.push_str("\\a"),
                '\x08' => out.push_str("\\b"),
                '\x0b' => out.push_str("\\v"),
                '\x0c' => out.push_str("\\f"),
                '\x1b' => out.push_str("\\E"),
                c if c.is_control() => {
                    out.push_str(&format!("\\x{:02x}", c as u32));
                }
                c => out.push(c),
            }
        }
        out.push('\'');
    } else if value.contains('\'') {
        // Value contains single quotes — use backslash escaping
        for ch in value.chars() {
            if ch == '\'' {
                out.push('\\');
            }
            out.push(ch);
        }
    } else {
        // Wrap in single quotes
        out.push('\'');
        out.push_str(value);
        out.push('\'');
    }

    out
}

fn shell_quote(s: &str) -> String {
    if s.contains('\'') {
        format!("$'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
    } else {
        format!("'{}'", s)
    }
}

pub fn find_command_path(name: &str) -> Option<String> {
    find_in_path_opt(name)
}

pub fn find_executable(name: &str) -> String {
    if name.contains('/') {
        return name.to_string();
    }
    find_in_path(name)
}

fn find_in_path(name: &str) -> String {
    find_in_path_opt(name).unwrap_or_else(|| name.to_string())
}

fn find_in_path_opt(name: &str) -> Option<String> {
    if name.contains('/') {
        if std::path::Path::new(name).exists() {
            return Some(name.to_string());
        }
        return None;
    }

    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            let full = format!("{}/{}", dir, name);
            if std::path::Path::new(&full).exists() {
                return Some(full);
            }
        }
    }
    None
}

pub fn builtins() -> HashMap<&'static str, BuiltinFn> {
    let mut map: HashMap<&'static str, BuiltinFn> = HashMap::new();
    // io
    map.insert("echo", io::builtin_echo);
    map.insert("printf", io::builtin_printf);
    map.insert("read", io::builtin_read);
    map.insert("mapfile", io::builtin_mapfile);
    map.insert("readarray", io::builtin_mapfile);
    // fs
    map.insert("cd", fs::builtin_cd);
    map.insert("pwd", fs::builtin_pwd);
    map.insert("dirs", fs::builtin_dirs);
    map.insert("pushd", fs::builtin_pushd);
    map.insert("popd", fs::builtin_popd);
    // vars
    map.insert("export", vars::builtin_export);
    map.insert("unset", vars::builtin_unset);
    map.insert("readonly", vars::builtin_readonly);
    map.insert("local", vars::builtin_local);
    map.insert("declare", vars::builtin_declare);
    map.insert("typeset", vars::builtin_declare);
    map.insert("let", vars::builtin_let);
    // flow
    map.insert("break", flow::builtin_break);
    map.insert("continue", flow::builtin_continue);
    map.insert("exit", flow::builtin_exit);
    map.insert("return", flow::builtin_return);
    map.insert("shift", flow::builtin_shift);
    map.insert("logout", flow::builtin_logout);
    // exec
    map.insert("eval", exec::builtin_eval);
    map.insert("exec", exec::builtin_exec);
    map.insert("source", exec::builtin_source);
    map.insert(".", exec::builtin_source);
    map.insert("help", exec::builtin_help);
    map.insert("type", exec::builtin_type);
    map.insert("builtin", exec::builtin_builtin);
    map.insert("command", exec::builtin_command);
    map.insert("which", exec::builtin_which);
    map.insert("hash", exec::builtin_hash);
    // trap
    map.insert("trap", trap::builtin_trap);
    map.insert("wait", trap::builtin_wait);
    map.insert("kill", trap::builtin_kill);
    map.insert("enable", trap::builtin_enable);
    map.insert("suspend", trap::builtin_suspend);
    map.insert("times", trap::builtin_times);
    map.insert("ulimit", trap::builtin_ulimit);
    // set
    map.insert("set", set::builtin_set);
    map.insert("shopt", set::builtin_shopt);
    // test
    map.insert("test", test::builtin_test);
    map.insert("[", test::builtin_test_bracket);
    // compgen
    map.insert("complete", compgen::builtin_complete);
    map.insert("compgen", compgen::builtin_compgen);
    // misc
    map.insert("true", misc::builtin_true);
    map.insert("false", misc::builtin_false);
    map.insert(":", misc::builtin_true);
    map.insert("getopts", misc::builtin_getopts);
    map.insert("umask", misc::builtin_umask);
    map.insert("caller", misc::builtin_caller);
    map.insert("alias", misc::builtin_alias);
    map.insert("unalias", misc::builtin_unalias);
    map.insert("jobs", misc::builtin_jobs);
    map.insert("disown", misc::builtin_disown);
    map.insert("fg", misc::builtin_fg);
    map.insert("bg", misc::builtin_bg);
    map
}
