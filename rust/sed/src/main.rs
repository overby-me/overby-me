use regex::Regex;
use std::io::{self, BufRead, Read, Write};
use std::process;

// ---------------------------------------------------------------------------
// Address types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Address {
    Line(usize),
    Last, // $
    Regex(Regex),
    Step(usize, usize), // first~step
}

#[derive(Debug, Clone)]
enum AddressRange {
    None,
    Single(Address),
    Range(Address, Address),
}

// ---------------------------------------------------------------------------
// Sed commands
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct SubstFlags {
    global: bool,
    nth: Option<usize>, // replace Nth occurrence
    print: bool,
    write_file: Option<String>,
    case_insensitive: bool,
    multiline: bool,     // m/M flag
    execute: bool,       // e flag — execute result as shell command
    print_before_exec: bool, // p came before e in flags
}

#[derive(Debug, Clone)]
enum Command {
    Substitute {
        pattern: Option<Regex>, // None means reuse last regex
        replacement: String,
        flags: SubstFlags,
    },
    Delete,
    DeleteFirstLine, // D
    Print,
    PrintFirstLine, // P
    PrintEscaped,   // l
    PrintLineNum,   // =
    Quit(Option<i32>),
    QuitNoprint(Option<i32>), // Q
    Append(String),
    Insert(String),
    Change(String),
    Transliterate(Vec<char>, Vec<char>), // y/src/dst/
    Next,                                // n
    NextAppend,                          // N
    HoldReplace,                         // h
    HoldAppend,                          // H
    GetReplace,                          // g (get from hold)
    GetAppend,                           // G
    Exchange,                            // x
    Branch(Option<String>),              // b [label]
    BranchIfSub(Option<String>),         // t [label]
    BranchIfNoSub(Option<String>),       // T [label]
    Label(String),                       // :label
    ReadFile(String),                    // r file
    ReadLine(String),                    // R file
    WriteFile(String),                   // w file
    WriteFirstLine(String),              // W file
    Execute(Option<String>),             // e [command]
    Filename,                            // F
    Noop,
    Block(Vec<SedCommand>),
}

#[derive(Debug, Clone)]
struct SedCommand {
    address: AddressRange,
    negated: bool,
    command: Command,
}

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

struct Options {
    in_place: Option<String>, // -i[SUFFIX]
    quiet: bool,              // -n
    extended: bool,           // -E / -r
    expressions: Vec<String>,
    files: Vec<String>,
    posix: bool,      // --posix
    unbuffered: bool, // -u
    null_data: bool,  // -z
    separate: bool,   // -s
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
    extended: bool,
    hash_n_quiet: bool, // set if #n is found as first line of first script
}

impl<'a> Parser<'a> {
    fn new(input: &'a str, extended: bool) -> Self {
        Parser {
            input: input.as_bytes(),
            pos: 0,
            extended,
            hash_n_quiet: false,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let ch = self.input.get(self.pos).copied();
        if ch.is_some() {
            self.pos += 1;
        }
        ch
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.peek() {
            if ch == b' ' || ch == b'\t' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn parse_all(&mut self, is_first_script: bool) -> Result<Vec<SedCommand>, String> {
        let mut commands = Vec::new();
        let mut first_comment = is_first_script;
        while !self.at_end() {
            self.skip_blanks_and_newlines();
            if self.at_end() {
                break;
            }
            if self.peek() == Some(b'#') {
                self.advance(); // consume '#'
                // Check for #n at start of first script — activates quiet mode
                if first_comment && self.peek() == Some(b'n') {
                    self.hash_n_quiet = true;
                }
                first_comment = false;
                // Skip to end of line
                while let Some(ch) = self.advance() {
                    if ch == b'\n' {
                        break;
                    }
                }
                continue;
            }
            first_comment = false;
            if let Some(cmd) = self.parse_command()? {
                commands.push(cmd);
            }
        }
        Ok(commands)
    }

    fn skip_blanks_and_newlines(&mut self) {
        while let Some(ch) = self.peek() {
            if ch == b' ' || ch == b'\t' || ch == b'\n' || ch == b';' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn parse_command(&mut self) -> Result<Option<SedCommand>, String> {
        self.skip_whitespace();
        if self.at_end() {
            return Ok(None);
        }

        let address = self.parse_address_range()?;
        self.skip_whitespace();

        let negated = if self.peek() == Some(b'!') {
            self.advance();
            self.skip_whitespace();
            true
        } else {
            false
        };

        if self.at_end() {
            return Ok(Some(SedCommand {
                address,
                negated,
                command: Command::Print,
            }));
        }

        let cmd = self.parse_command_char()?;
        Ok(Some(SedCommand {
            address,
            negated,
            command: cmd,
        }))
    }

    fn parse_address_range(&mut self) -> Result<AddressRange, String> {
        let first = self.try_parse_address()?;
        match first {
            None => Ok(AddressRange::None),
            Some(addr) => {
                if self.peek() == Some(b',') {
                    self.advance();
                    let second = self.try_parse_address()?;
                    match second {
                        Some(addr2) => Ok(AddressRange::Range(addr, addr2)),
                        None => Err("expected address after ','".into()),
                    }
                } else {
                    Ok(AddressRange::Single(addr))
                }
            }
        }
    }

    fn try_parse_address(&mut self) -> Result<Option<Address>, String> {
        self.skip_whitespace();
        match self.peek() {
            Some(b'$') => {
                self.advance();
                Ok(Some(Address::Last))
            }
            Some(b'0'..=b'9') => {
                let n = self.parse_number()?;
                if self.peek() == Some(b'~') {
                    self.advance();
                    let step = self.parse_number()?;
                    Ok(Some(Address::Step(n, step)))
                } else {
                    Ok(Some(Address::Line(n)))
                }
            }
            Some(b'/') => {
                self.advance();
                let pattern = self.parse_regex_delimited(b'/')?;
                let re = self.compile_regex(&pattern)?;
                Ok(Some(Address::Regex(re)))
            }
            Some(b'\\') => {
                self.advance();
                let delim = self.advance().ok_or("expected delimiter after \\")?;
                let pattern = self.parse_regex_delimited(delim)?;
                let re = self.compile_regex(&pattern)?;
                Ok(Some(Address::Regex(re)))
            }
            _ => Ok(None),
        }
    }

    fn parse_number(&mut self) -> Result<usize, String> {
        let mut n: usize = 0;
        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() {
                n = n * 10 + (ch - b'0') as usize;
                self.advance();
            } else {
                break;
            }
        }
        Ok(n)
    }

    fn parse_regex_delimited(&mut self, delim: u8) -> Result<String, String> {
        let mut pattern = String::new();
        let mut escaped = false;
        let mut in_bracket = false;
        loop {
            let ch = self.advance().ok_or("unterminated regex")?;
            if escaped {
                if ch == delim {
                    pattern.push(ch as char);
                } else {
                    pattern.push('\\');
                    pattern.push(ch as char);
                }
                escaped = false;
            } else if ch == b'\\' {
                escaped = true;
            } else if ch == b'[' && !in_bracket {
                in_bracket = true;
                pattern.push('[');
                // Handle [^ and [] and [^] at start of character class
                if self.peek() == Some(b'^') {
                    pattern.push('^');
                    self.advance();
                }
                // ] right after [ or [^ is literal
                if self.peek() == Some(b']') {
                    pattern.push(']');
                    self.advance();
                }
            } else if ch == b']' && in_bracket {
                in_bracket = false;
                pattern.push(']');
            } else if ch == delim && !in_bracket {
                break;
            } else {
                pattern.push(ch as char);
            }
        }
        Ok(pattern)
    }

    fn compile_regex(&self, pattern: &str) -> Result<Regex, String> {
        if pattern.is_empty() {
            return Err("empty regex".into());
        }

        let pat = if self.extended {
            pattern.to_string()
        } else {
            bre_to_ere(pattern)
        };

        let pat = fix_posix_char_class(&pat);
        Regex::new(&pat).map_err(|e| format!("invalid regex: {e}"))
    }

    fn parse_command_char(&mut self) -> Result<Command, String> {
        let ch = self.advance().ok_or("expected command")?;
        match ch {
            b'{' => {
                let mut cmds = Vec::new();
                loop {
                    self.skip_blanks_and_newlines();
                    if self.peek() == Some(b'}') {
                        self.advance();
                        break;
                    }
                    if self.at_end() {
                        return Err("unterminated '{'".into());
                    }
                    if let Some(cmd) = self.parse_command()? {
                        cmds.push(cmd);
                    }
                }
                Ok(Command::Block(cmds))
            }
            b's' => self.parse_substitute(),
            b'y' => self.parse_transliterate(),
            b'd' => Ok(Command::Delete),
            b'D' => Ok(Command::DeleteFirstLine),
            b'p' => Ok(Command::Print),
            b'P' => Ok(Command::PrintFirstLine),
            b'l' => {
                // l may have optional line width: l80, l1, etc.
                if let Some(ch) = self.peek()
                    && ch.is_ascii_digit()
                {
                    let _ = self.parse_number(); // consume width (ignored for now)
                }
                Ok(Command::PrintEscaped)
            }
            b'=' => Ok(Command::PrintLineNum),
            b'q' => {
                self.skip_whitespace();
                let code = self.try_parse_exit_code();
                Ok(Command::Quit(code))
            }
            b'Q' => {
                self.skip_whitespace();
                let code = self.try_parse_exit_code();
                Ok(Command::QuitNoprint(code))
            }
            b'a' => {
                self.skip_optional_backslash_newline();
                let text = self.parse_text_arg();
                Ok(Command::Append(text))
            }
            b'i' => {
                self.skip_optional_backslash_newline();
                let text = self.parse_text_arg();
                Ok(Command::Insert(text))
            }
            b'c' => {
                self.skip_optional_backslash_newline();
                let text = self.parse_text_arg();
                Ok(Command::Change(text))
            }
            b'n' => Ok(Command::Next),
            b'N' => Ok(Command::NextAppend),
            b'h' => Ok(Command::HoldReplace),
            b'H' => Ok(Command::HoldAppend),
            b'g' => Ok(Command::GetReplace),
            b'G' => Ok(Command::GetAppend),
            b'x' => Ok(Command::Exchange),
            b':' => {
                self.skip_whitespace();
                let label = self.parse_label();
                if label.is_empty() {
                    return Err("\":\" lacks a label".into());
                }
                Ok(Command::Label(label))
            }
            b'b' => {
                self.skip_whitespace();
                let label = self.try_parse_label();
                Ok(Command::Branch(label))
            }
            b't' => {
                self.skip_whitespace();
                let label = self.try_parse_label();
                Ok(Command::BranchIfSub(label))
            }
            b'T' => {
                self.skip_whitespace();
                let label = self.try_parse_label();
                Ok(Command::BranchIfNoSub(label))
            }
            b'r' => {
                self.skip_whitespace();
                let file = self.parse_filename();
                Ok(Command::ReadFile(file))
            }
            b'R' => {
                self.skip_whitespace();
                let file = self.parse_filename();
                Ok(Command::ReadLine(file))
            }
            b'w' => {
                self.skip_whitespace();
                let file = self.parse_filename();
                Ok(Command::WriteFile(file))
            }
            b'W' => {
                self.skip_whitespace();
                let file = self.parse_filename();
                Ok(Command::WriteFirstLine(file))
            }
            b'e' => {
                // e without argument: execute pattern space as command
                // e with text: execute given command, output result
                if self.peek() == Some(b'\n')
                    || self.peek() == Some(b';')
                    || self.at_end()
                {
                    Ok(Command::Execute(None))
                } else {
                    // Anything else is the command to execute
                    self.skip_whitespace();
                    let cmd_text = self.parse_text_arg();
                    if cmd_text.is_empty() {
                        Ok(Command::Execute(None))
                    } else {
                        Ok(Command::Execute(Some(cmd_text)))
                    }
                }
            }
            b'F' => Ok(Command::Filename),
            b'v' => {
                // v VERSION — check version, we just accept anything
                self.skip_whitespace();
                // Skip to end of command
                while let Some(ch) = self.peek() {
                    if ch == b'\n' || ch == b';' {
                        break;
                    }
                    self.advance();
                }
                Ok(Command::Noop)
            }
            b'#' => {
                // Inline comment — skip to end of line
                while let Some(ch) = self.advance() {
                    if ch == b'\n' {
                        break;
                    }
                }
                Ok(Command::Noop)
            }
            b'\n' | b';' | b'}' => Ok(Command::Noop),
            _ => Err(format!("unknown command: '{}'", char::from(ch))),
        }
    }

    fn parse_substitute(&mut self) -> Result<Command, String> {
        let delim = self.advance().ok_or("expected delimiter for s command")?;
        if delim == b'\\' || delim == b'\n' {
            return Err("invalid delimiter for s command".into());
        }

        let pattern_str = self.parse_regex_delimited(delim)?;
        let replacement = self.parse_replacement(delim)?;
        let flags = self.parse_sub_flags()?;

        let mut re_pattern = if self.extended {
            pattern_str.clone()
        } else {
            bre_to_ere(&pattern_str)
        };

        if flags.case_insensitive && flags.multiline {
            re_pattern = format!("(?im){re_pattern}");
        } else if flags.case_insensitive {
            re_pattern = format!("(?i){re_pattern}");
        } else if flags.multiline {
            re_pattern = format!("(?m){re_pattern}");
        }

        let re = if pattern_str.is_empty() {
            None // Reuse last regex at runtime
        } else {
            Some(
                Regex::new(&fix_posix_char_class(&re_pattern))
                    .map_err(|e| format!("invalid regex in s command: {e}"))?,
            )
        };

        Ok(Command::Substitute {
            pattern: re,
            replacement,
            flags,
        })
    }

    fn parse_replacement(&mut self, delim: u8) -> Result<String, String> {
        let mut result = String::new();
        let mut escaped = false;
        loop {
            match self.peek() {
                None => {
                    // Unterminated — treat as if delimiter at end
                    break;
                }
                Some(b'\n') => {
                    if escaped {
                        // \ followed by newline: literal newline in replacement
                        // (GNU sed continuation)
                        self.advance();
                        result.push('\n');
                        escaped = false;
                    } else {
                        // Unterminated — treat as if delimiter at end
                        break;
                    }
                }
                Some(ch) => {
                    self.advance();
                    if escaped {
                        match ch {
                            b'n' => result.push('\n'),
                            b'a' => result.push('\x07'),
                            b'r' => result.push('\r'),
                            b't' => result.push('\t'),
                            b'f' => result.push('\x0c'),
                            b'v' => result.push('\x0b'),
                            b'b' => result.push('\x08'),
                            b'c' => {
                                // \cX = control character (X XOR 0x40)
                                // If next char is \, consume the escaped char
                                if let Some(next) = self.peek() {
                                    if next == delim {
                                        // \c at end of replacement — produce literal backslash
                                        result.push('\\');
                                    } else if next == b'\\' {
                                        // \c\\ means \c applied to escaped backslash
                                        self.advance(); // consume first \
                                        if let Some(next2) = self.peek() {
                                            if next2 == b'\\' || next2 == delim {
                                                self.advance(); // consume second \ or delim
                                            }
                                        }
                                        // The argument to \c is \  (0x5C)
                                        result.push(ctrl_char(b'\\') as char);
                                    } else {
                                        self.advance();
                                        result.push(ctrl_char(next) as char);
                                    }
                                } else {
                                    result.push('\\');
                                }
                            }
                            b'd' => {
                                // \dNNN = decimal escape
                                let mut n: u32 = 0;
                                let mut count = 0;
                                for _ in 0..3 {
                                    if let Some(d) = self.peek()
                                        && d.is_ascii_digit()
                                    {
                                        n = n * 10 + (d - b'0') as u32;
                                        self.advance();
                                        count += 1;
                                    } else {
                                        break;
                                    }
                                }
                                if count > 0 {
                                    result.push(char::from_u32(n).unwrap_or('\0'));
                                } else {
                                    // \d not followed by digits — literal 'd'
                                    result.push('d');
                                }
                            }
                            b'o' => {
                                // \oNNN = octal escape
                                let mut n: u32 = 0;
                                let mut count = 0;
                                for _ in 0..3 {
                                    if let Some(d) = self.peek()
                                        && d >= b'0'
                                        && d <= b'7'
                                    {
                                        n = n * 8 + (d - b'0') as u32;
                                        self.advance();
                                        count += 1;
                                    } else {
                                        break;
                                    }
                                }
                                if count > 0 {
                                    result.push(char::from_u32(n).unwrap_or('\0'));
                                } else {
                                    result.push('o');
                                }
                            }
                            b'x' => {
                                // \xNN = hex escape
                                let mut n: u32 = 0;
                                let mut count = 0;
                                for _ in 0..2 {
                                    if let Some(d) = self.peek()
                                        && d.is_ascii_hexdigit()
                                    {
                                        let val = if d.is_ascii_digit() {
                                            d - b'0'
                                        } else {
                                            (d | 0x20) - b'a' + 10
                                        };
                                        n = n * 16 + val as u32;
                                        self.advance();
                                        count += 1;
                                    } else {
                                        break;
                                    }
                                }
                                if count > 0 {
                                    result.push(char::from_u32(n).unwrap_or('\0'));
                                } else {
                                    result.push('x');
                                }
                            }
                            _ => {
                                if ch != delim {
                                    result.push('\\');
                                }
                                result.push(ch as char);
                            }
                        }
                        escaped = false;
                    } else if ch == b'\\' {
                        escaped = true;
                    } else if ch == delim {
                        break;
                    } else {
                        result.push(ch as char);
                    }
                }
            }
        }
        Ok(result)
    }

    fn parse_sub_flags(&mut self) -> Result<SubstFlags, String> {
        let mut flags = SubstFlags::default();
        loop {
            match self.peek() {
                Some(b'g') => {
                    self.advance();
                    flags.global = true;
                }
                Some(b'p') => {
                    self.advance();
                    flags.print = true;
                }
                Some(b'i') | Some(b'I') => {
                    self.advance();
                    flags.case_insensitive = true;
                }
                Some(b'e') => {
                    self.advance();
                    if flags.print && !flags.execute {
                        flags.print_before_exec = true;
                    }
                    flags.execute = true;
                }
                Some(b'm') | Some(b'M') => {
                    self.advance();
                    flags.multiline = true;
                }
                Some(b'w') => {
                    self.advance();
                    self.skip_whitespace();
                    flags.write_file = Some(self.parse_filename());
                    break;
                }
                Some(b'\r') => {
                    // Ignore trailing \r (Windows line endings in script files)
                    self.advance();
                }
                Some(ch) if ch.is_ascii_digit() => {
                    let n = self.parse_number()?;
                    if n > 0 {
                        flags.nth = Some(n);
                    }
                }
                _ => break,
            }
        }
        Ok(flags)
    }

    fn parse_transliterate(&mut self) -> Result<Command, String> {
        let delim = self.advance().ok_or("expected delimiter for y command")?;
        let src = self.parse_translit_chars(delim)?;
        let dst = self.parse_translit_chars(delim)?;
        if src.len() != dst.len() {
            return Err("y command: source and dest must have same length".into());
        }
        Ok(Command::Transliterate(src, dst))
    }

    fn parse_translit_chars(&mut self, delim: u8) -> Result<Vec<char>, String> {
        let mut chars = Vec::new();
        let mut escaped = false;
        loop {
            let ch = self.advance().ok_or("unterminated y command")?;
            if escaped {
                match ch {
                    b'n' | b'\n' => chars.push('\n'),
                    b'a' => chars.push('\x07'),
                    b'b' => chars.push('\x08'),
                    b'f' => chars.push('\x0c'),
                    b'r' => chars.push('\r'),
                    b't' => chars.push('\t'),
                    b'v' => chars.push('\x0b'),
                    b'c' => {
                        // \cX = control character (X XOR 0x40)
                        if let Some(next) = self.peek() {
                            if next == delim {
                                // \c at end of set — incomplete, don't consume delimiter
                            } else if next == b'\\' {
                                // \c\\ = \c applied to escaped backslash
                                self.advance();
                                if let Some(next2) = self.peek() {
                                    if next2 == b'\\' || next2 == delim {
                                        self.advance();
                                    }
                                }
                                chars.push(ctrl_char(b'\\') as char);
                            } else {
                                self.advance();
                                chars.push(ctrl_char(next) as char);
                            }
                        }
                    }
                    b'd' => {
                        // \dNNN = decimal escape
                        let mut n: u32 = 0;
                        for _ in 0..3 {
                            if let Some(d) = self.peek()
                                && d.is_ascii_digit()
                            {
                                n = n * 10 + (d - b'0') as u32;
                                self.advance();
                            } else {
                                break;
                            }
                        }
                        chars.push(char::from_u32(n).unwrap_or('\0'));
                    }
                    b'o' => {
                        // \oNNN = octal escape
                        let mut n: u32 = 0;
                        for _ in 0..3 {
                            if let Some(d) = self.peek()
                                && d >= b'0'
                                && d <= b'7'
                            {
                                n = n * 8 + (d - b'0') as u32;
                                self.advance();
                            } else {
                                break;
                            }
                        }
                        chars.push(char::from_u32(n).unwrap_or('\0'));
                    }
                    b'x' => {
                        // \xNN = hex escape
                        let mut n: u32 = 0;
                        for _ in 0..2 {
                            if let Some(d) = self.peek()
                                && d.is_ascii_hexdigit()
                            {
                                let val = if d.is_ascii_digit() {
                                    d - b'0'
                                } else {
                                    (d | 0x20) - b'a' + 10
                                };
                                n = n * 16 + val as u32;
                                self.advance();
                            } else {
                                break;
                            }
                        }
                        chars.push(char::from_u32(n).unwrap_or('\0'));
                    }
                    _ => {
                        if ch == delim {
                            chars.push(ch as char);
                        } else {
                            chars.push('\\');
                            chars.push(ch as char);
                        }
                    }
                }
                escaped = false;
            } else if ch == b'\\' {
                escaped = true;
            } else if ch == delim {
                break;
            } else {
                chars.push(ch as char);
            }
        }
        Ok(chars)
    }

    fn try_parse_exit_code(&mut self) -> Option<i32> {
        if let Some(ch) = self.peek()
            && ch.is_ascii_digit()
        {
            let n = self.parse_number().ok()?;
            return Some(n as i32);
        }
        None
    }

    fn skip_optional_backslash_newline(&mut self) {
        // GNU sed allows `a\` followed by newline, or `a ` text, or `a\` at EOF
        if self.peek() == Some(b'\\') {
            self.advance(); // consume backslash
            if self.peek() == Some(b'\n') {
                self.advance(); // consume newline
            }
            // If at EOF after backslash, that's fine — text will be empty
        } else if self.peek() == Some(b' ') || self.peek() == Some(b'\t') {
            self.skip_whitespace();
        }
    }

    fn parse_text_arg(&mut self) -> String {
        let mut text = String::new();
        let mut first = true;
        loop {
            match self.peek() {
                None => break,
                Some(b'\n') => {
                    if !first {
                        text.push('\n');
                    }
                    self.advance();
                    // Check for continuation (line ending with \)
                    // Actually, text continues until a line without trailing backslash
                    // But for single-line usage like `a\text`, we just take the rest
                    break;
                }
                Some(ch) => {
                    self.advance();
                    if ch == b'\\' && matches!(self.peek(), Some(b'n') | Some(b'\n')) {
                        self.advance();
                        text.push('\n');
                    } else {
                        text.push(ch as char);
                    }
                    first = false;
                }
            }
        }
        text
    }

    fn parse_label(&mut self) -> String {
        let mut label = String::new();
        while let Some(ch) = self.peek() {
            if ch == b'\n' || ch == b';' || ch == b'}' || ch == b' ' || ch == b'\t' {
                break;
            }
            self.advance();
            label.push(ch as char);
        }
        label
    }

    fn try_parse_label(&mut self) -> Option<String> {
        let label = self.parse_label();
        if label.is_empty() { None } else { Some(label) }
    }

    fn parse_filename(&mut self) -> String {
        let mut name = String::new();
        while let Some(ch) = self.peek() {
            if ch == b'\n' || ch == b';' {
                break;
            }
            self.advance();
            name.push(ch as char);
        }
        name.trim().to_string()
    }
}

// ---------------------------------------------------------------------------
// BRE to ERE conversion
// ---------------------------------------------------------------------------

/// Fix POSIX character class patterns that Rust's regex crate can't handle.
/// In POSIX, `[]...]` means a class containing `]` — the `]` right after `[` or `[^`
/// is a literal. Rust regex doesn't support this, so we transform it.
fn fix_posix_char_class(pattern: &str) -> String {
    let mut result = String::with_capacity(pattern.len() + 8);
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '[' {
            result.push('[');
            i += 1;
            // Check for negation
            if i < chars.len() && chars[i] == '^' {
                result.push('^');
                i += 1;
            }
            // If next char is ], it's a literal ] in POSIX.
            // We collect the rest of the class first, then append \] at the end
            // so Rust regex doesn't confuse it with the class-closing ].
            let mut has_leading_close = false;
            if i < chars.len() && chars[i] == ']' {
                has_leading_close = true;
                i += 1;
            }
            // Collect rest of character class, escaping bare [ for Rust regex
            let mut class_content = String::new();
            while i < chars.len() && chars[i] != ']' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    class_content.push(chars[i]);
                    class_content.push(chars[i + 1]);
                    i += 2;
                } else if chars[i] == '[' && !(i + 1 < chars.len() && chars[i + 1] == ':') {
                    // Bare [ that's not a POSIX class like [:alpha:]
                    class_content.push_str("\\[");
                    i += 1;
                } else {
                    class_content.push(chars[i]);
                    i += 1;
                }
            }
            result.push_str(&class_content);
            if has_leading_close {
                result.push_str("\\]");
            }
            if i < chars.len() {
                result.push(']');
                i += 1;
            }
        } else if chars[i] == '\\' && i + 1 < chars.len() {
            result.push(chars[i]);
            result.push(chars[i + 1]);
            i += 2;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

fn bre_to_ere(bre: &str) -> String {
    let mut result = String::with_capacity(bre.len());
    let chars: Vec<char> = bre.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        // Pass through character classes unchanged — inside [...], all chars are literal
        if chars[i] == '[' {
            result.push('[');
            i += 1;
            // Handle negation and ] as first char
            if i < chars.len() && chars[i] == '^' {
                result.push('^');
                i += 1;
            }
            if i < chars.len() && chars[i] == ']' {
                result.push(']');
                i += 1;
            }
            // Copy until closing ]
            while i < chars.len() && chars[i] != ']' {
                // Handle POSIX classes like [:alpha:]
                if chars[i] == '[' && i + 1 < chars.len() && chars[i + 1] == ':' {
                    result.push('[');
                    result.push(':');
                    i += 2;
                    while i < chars.len() {
                        if chars[i] == ':' && i + 1 < chars.len() && chars[i + 1] == ']' {
                            result.push(':');
                            result.push(']');
                            i += 2;
                            break;
                        }
                        result.push(chars[i]);
                        i += 1;
                    }
                } else {
                    result.push(chars[i]);
                    i += 1;
                }
            }
            if i < chars.len() {
                result.push(']');
                i += 1;
            }
            continue;
        }

        if chars[i] == '\\' && i + 1 < chars.len() {
            match chars[i + 1] {
                '(' => {
                    result.push('(');
                    i += 2;
                }
                ')' => {
                    result.push(')');
                    i += 2;
                }
                '{' => {
                    result.push('{');
                    i += 2;
                }
                '}' => {
                    result.push('}');
                    i += 2;
                }
                '|' => {
                    result.push('|');
                    i += 2;
                }
                '+' => {
                    result.push('+');
                    i += 2;
                }
                '?' => {
                    result.push('?');
                    i += 2;
                }
                'n' => {
                    result.push('\n');
                    i += 2;
                }
                't' => {
                    result.push('\t');
                    i += 2;
                }
                '1'..='9' => {
                    result.push('\\');
                    result.push(chars[i + 1]);
                    i += 2;
                }
                _ => {
                    result.push('\\');
                    result.push(chars[i + 1]);
                    i += 2;
                }
            }
        } else if chars[i] == '(' {
            result.push_str("\\(");
            i += 1;
        } else if chars[i] == ')' {
            result.push_str("\\)");
            i += 1;
        } else if chars[i] == '{' {
            result.push_str("\\{");
            i += 1;
        } else if chars[i] == '}' {
            result.push_str("\\}");
            i += 1;
        } else if chars[i] == '|' {
            result.push_str("\\|");
            i += 1;
        } else if chars[i] == '+' {
            result.push_str("\\+");
            i += 1;
        } else if chars[i] == '?' {
            result.push_str("\\?");
            i += 1;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Execution engine
// ---------------------------------------------------------------------------

struct Engine {
    commands: Vec<SedCommand>,
    quiet: bool,
    pattern_space: String,
    hold_space: String,
    line_number: usize,
    last_line: bool,
    last_regex: Option<Regex>,
    sub_happened: bool, // for t/T commands
    output: Vec<u8>,
    append_queue: Vec<String>,
    quit: bool,
    exit_code: i32,
    suppress_default_print: bool,
    input_lines: Vec<String>,
    input_index: usize,
    current_filename: Option<String>,
    range_active: Vec<bool>, // per-command range tracking
}

impl Engine {
    fn new(commands: Vec<SedCommand>, quiet: bool) -> Self {
        let num_cmds = count_commands(&commands);
        Engine {
            commands,
            quiet,
            pattern_space: String::new(),
            hold_space: String::new(),
            line_number: 0,
            last_line: false,
            last_regex: None,
            sub_happened: false,
            output: Vec::new(),
            append_queue: Vec::new(),
            quit: false,
            exit_code: 0,
            suppress_default_print: false,
            input_lines: Vec::new(),
            input_index: 0,
            current_filename: None,
            range_active: vec![false; num_cmds],
        }
    }

    fn run<R: BufRead, W: Write>(&mut self, reader: R, writer: &mut W) -> io::Result<i32> {
        self.input_lines = reader.lines().collect::<io::Result<Vec<_>>>()?;
        self.input_index = 0;
        let total = self.input_lines.len();

        while self.input_index < total {
            let line = self.input_lines[self.input_index].clone();
            self.input_index += 1;
            self.line_number = self.input_index;
            self.last_line = self.input_index == total;
            self.pattern_space = line;
            self.sub_happened = false;
            self.append_queue.clear();
            self.suppress_default_print = false;

            let cmds = self.commands.clone();
            self.execute_commands_with_offset(&cmds, 0);

            if self.quit {
                if !self.quiet && !self.suppress_default_print {
                    self.write_pattern_space();
                }
                self.flush_output(writer)?;
                return Ok(self.exit_code);
            }

            if !self.quiet && !self.suppress_default_print {
                self.write_pattern_space();
            }

            // Flush append queue
            for text in self.append_queue.clone() {
                self.output.extend_from_slice(text.as_bytes());
                if !text.ends_with('\n') {
                    self.output.push(b'\n');
                }
            }

            self.flush_output(writer)?;
        }

        Ok(self.exit_code)
    }

    fn write_pattern_space(&mut self) {
        self.output.extend_from_slice(self.pattern_space.as_bytes());
        self.output.push(b'\n');
    }

    fn flush_output<W: Write>(&mut self, writer: &mut W) -> io::Result<()> {
        if !self.output.is_empty() {
            writer.write_all(&self.output)?;
            self.output.clear();
        }
        Ok(())
    }

    fn execute_commands_with_offset(&mut self, commands: &[SedCommand], range_offset: usize) {
        let mut i = 0;
        while i < commands.len() {
            if self.quit {
                return;
            }
            let cmd = &commands[i];
            let range_idx = range_offset + i;
            let matched = self.address_matches(&cmd.address, range_idx);
            let should_run = if cmd.negated { !matched } else { matched };

            if should_run {
                match self.execute_one(&cmd.command, commands, i, range_offset) {
                    Flow::Continue => {}
                    Flow::Restart => {
                        i = 0;
                        continue;
                    }
                    Flow::Branch(label) => {
                        if let Some(target) = Self::find_label(commands, &label) {
                            i = target + 1;
                            continue;
                        }
                        // Label not found — jump to end (like GNU sed)
                        return;
                    }
                    Flow::EndOfCycle => return,
                    Flow::Quit => {
                        self.quit = true;
                        return;
                    }
                    Flow::QuitNoprint => {
                        self.quit = true;
                        // Suppress default print
                        // Set pattern space empty to prevent output
                        self.pattern_space.clear();
                        return;
                    }
                }
            }
            i += 1;
        }
    }

    fn address_matches(&mut self, addr: &AddressRange, range_idx: usize) -> bool {
        match addr {
            AddressRange::None => true,
            AddressRange::Single(a) => self.addr_matches_single(a),
            AddressRange::Range(a, b) => {
                // Ensure range_active is large enough
                if range_idx >= self.range_active.len() {
                    self.range_active.resize(range_idx + 1, false);
                }
                if self.range_active[range_idx] {
                    // We're inside a range — check if end address matches
                    if self.addr_matches_single(b) {
                        self.range_active[range_idx] = false;
                    }
                    true
                } else if self.addr_matches_single(a) {
                    // Start of range
                    // Check if end also matches on same line
                    if self.addr_matches_single(b) {
                        // Single-line range — don't enter range mode
                    } else {
                        self.range_active[range_idx] = true;
                    }
                    true
                } else {
                    false
                }
            }
        }
    }

    fn addr_matches_single(&mut self, addr: &Address) -> bool {
        match addr {
            Address::Line(n) => self.line_number == *n,
            Address::Last => self.last_line,
            Address::Regex(re) => {
                let matched = re.is_match(&self.pattern_space);
                if matched {
                    self.last_regex = Some(re.clone());
                }
                matched
            }
            Address::Step(first, step) => {
                if *step == 0 {
                    self.line_number == *first
                } else if *first == 0 {
                    self.line_number.is_multiple_of(*step)
                } else {
                    self.line_number >= *first && (self.line_number - *first).is_multiple_of(*step)
                }
            }
        }
    }

    fn find_label(commands: &[SedCommand], label: &str) -> Option<usize> {
        for (i, cmd) in commands.iter().enumerate() {
            if let Command::Label(l) = &cmd.command
                && l == label
            {
                return Some(i);
            }
            if let Command::Block(ref inner) = cmd.command {
                // Search inside blocks too
                if Self::find_label(inner, label).is_some() {
                    // Can't return inner index directly — labels in blocks
                    // are found but branching crosses block boundaries in GNU sed
                    return Some(i);
                }
            }
        }
        None
    }

    fn execute_one(
        &mut self,
        cmd: &Command,
        _all_commands: &[SedCommand],
        _cmd_idx: usize,
        range_offset: usize,
    ) -> Flow {
        match cmd {
            Command::Noop => Flow::Continue,

            Command::Substitute {
                pattern,
                replacement,
                flags,
            } => {
                let re = match pattern {
                    None => {
                        // Empty pattern — reuse last
                        match &self.last_regex {
                            Some(re) => re.clone(),
                            None => return Flow::Continue,
                        }
                    }
                    Some(re) => {
                        self.last_regex = Some(re.clone());
                        re.clone()
                    }
                };

                let result = self.do_substitute(&re, replacement, flags);
                if result {
                    self.sub_happened = true;
                    // If p before e: print, then execute
                    if flags.print && flags.print_before_exec {
                        self.write_pattern_space();
                    }
                    if flags.execute {
                        if let Ok(output) = std::process::Command::new("sh")
                            .arg("-c")
                            .arg(&self.pattern_space)
                            .output()
                        {
                            let mut text =
                                String::from_utf8_lossy(&output.stdout).into_owned();
                            if text.ends_with('\n') {
                                text.pop();
                            }
                            self.pattern_space = text;
                        }
                    }
                    // If p after e (or p without e): print after execution
                    if flags.print && !flags.print_before_exec {
                        self.write_pattern_space();
                    }
                    if let Some(ref file) = flags.write_file {
                        let _ = self.write_to_file(file);
                    }
                }
                Flow::Continue
            }

            Command::Delete => {
                self.suppress_default_print = true;
                Flow::EndOfCycle
            }

            Command::DeleteFirstLine => {
                if let Some(pos) = self.pattern_space.find('\n') {
                    self.pattern_space = self.pattern_space[pos + 1..].to_string();
                    self.suppress_default_print = true;
                    Flow::Restart
                } else {
                    // No newline — behaves like `d` (delete entire pattern space)
                    self.suppress_default_print = true;
                    Flow::EndOfCycle
                }
            }

            Command::Print => {
                self.write_pattern_space();
                Flow::Continue
            }

            Command::PrintFirstLine => {
                let line = if let Some(pos) = self.pattern_space.find('\n') {
                    &self.pattern_space[..pos]
                } else {
                    &self.pattern_space
                };
                self.output.extend_from_slice(line.as_bytes());
                self.output.push(b'\n');
                Flow::Continue
            }

            Command::PrintEscaped => {
                let escaped = escape_string(&self.pattern_space);
                self.output.extend_from_slice(escaped.as_bytes());
                self.output.push(b'$');
                self.output.push(b'\n');
                Flow::Continue
            }

            Command::PrintLineNum => {
                let s = format!("{}\n", self.line_number);
                self.output.extend_from_slice(s.as_bytes());
                Flow::Continue
            }

            Command::Quit(code) => {
                self.exit_code = code.unwrap_or(0);
                Flow::Quit
            }

            Command::QuitNoprint(code) => {
                self.exit_code = code.unwrap_or(0);
                Flow::QuitNoprint
            }

            Command::Append(text) => {
                self.append_queue.push(text.clone());
                Flow::Continue
            }

            Command::Insert(text) => {
                self.output.extend_from_slice(text.as_bytes());
                if !text.ends_with('\n') {
                    self.output.push(b'\n');
                }
                Flow::Continue
            }

            Command::Change(text) => {
                self.pattern_space = text.clone();
                // c command outputs text and starts new cycle (suppressing default print)
                self.output.extend_from_slice(text.as_bytes());
                if !text.ends_with('\n') {
                    self.output.push(b'\n');
                }
                Flow::EndOfCycle
            }

            Command::Transliterate(src, dst) => {
                let mut new = String::with_capacity(self.pattern_space.len());
                for ch in self.pattern_space.chars() {
                    if let Some(pos) = src.iter().position(|&c| c == ch) {
                        new.push(dst[pos]);
                    } else {
                        new.push(ch);
                    }
                }
                self.pattern_space = new;
                Flow::Continue
            }

            Command::Next => {
                // Print current, read next line into pattern space
                if !self.quiet {
                    self.write_pattern_space();
                }
                if self.input_index < self.input_lines.len() {
                    self.pattern_space = self.input_lines[self.input_index].clone();
                    self.input_index += 1;
                    self.line_number = self.input_index;
                    self.last_line = self.input_index == self.input_lines.len();
                    Flow::Continue
                } else {
                    Flow::Quit
                }
            }

            Command::NextAppend => {
                // Append next line to pattern space with embedded newline
                if self.input_index < self.input_lines.len() {
                    let next_line = self.input_lines[self.input_index].clone();
                    self.input_index += 1;
                    self.line_number = self.input_index;
                    self.last_line = self.input_index == self.input_lines.len();
                    self.pattern_space.push('\n');
                    self.pattern_space.push_str(&next_line);
                    Flow::Continue
                } else {
                    // No more input — default print and exit
                    if !self.quiet {
                        self.write_pattern_space();
                    }
                    Flow::Quit
                }
            }

            Command::HoldReplace => {
                self.hold_space = self.pattern_space.clone();
                Flow::Continue
            }

            Command::HoldAppend => {
                self.hold_space.push('\n');
                self.hold_space.push_str(&self.pattern_space);
                Flow::Continue
            }

            Command::GetReplace => {
                self.pattern_space = self.hold_space.clone();
                Flow::Continue
            }

            Command::GetAppend => {
                self.pattern_space.push('\n');
                self.pattern_space.push_str(&self.hold_space);
                Flow::Continue
            }

            Command::Exchange => {
                std::mem::swap(&mut self.pattern_space, &mut self.hold_space);
                Flow::Continue
            }

            Command::Label(_) => Flow::Continue,

            Command::Branch(label) => match label {
                Some(l) => Flow::Branch(l.clone()),
                None => Flow::EndOfCycle,
            },

            Command::BranchIfSub(label) => {
                if self.sub_happened {
                    self.sub_happened = false;
                    match label {
                        Some(l) => Flow::Branch(l.clone()),
                        None => Flow::EndOfCycle,
                    }
                } else {
                    Flow::Continue
                }
            }

            Command::BranchIfNoSub(label) => {
                if !self.sub_happened {
                    match label {
                        Some(l) => Flow::Branch(l.clone()),
                        None => Flow::EndOfCycle,
                    }
                } else {
                    self.sub_happened = false;
                    Flow::Continue
                }
            }

            Command::ReadFile(file) => {
                if let Ok(content) = std::fs::read_to_string(file) {
                    self.append_queue.push(content);
                }
                Flow::Continue
            }

            Command::ReadLine(file) => {
                if let Ok(content) = std::fs::read_to_string(file)
                    && let Some(line) = content.lines().next()
                {
                    self.append_queue.push(line.to_string());
                }
                Flow::Continue
            }

            Command::WriteFile(file) => {
                let _ = self.write_to_file(file);
                Flow::Continue
            }

            Command::WriteFirstLine(file) => {
                let line = if let Some(pos) = self.pattern_space.find('\n') {
                    &self.pattern_space[..pos]
                } else {
                    &self.pattern_space
                };
                let _ = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(file)
                    .and_then(|mut f| writeln!(f, "{line}"));
                Flow::Continue
            }

            Command::Execute(cmd_text) => {
                match cmd_text {
                    Some(cmd_str) => {
                        // Execute the given command and insert output before current line
                        if let Ok(output) = std::process::Command::new("sh")
                            .arg("-c")
                            .arg(cmd_str)
                            .output()
                        {
                            let text = String::from_utf8_lossy(&output.stdout).into_owned();
                            if !text.is_empty() {
                                self.output.extend_from_slice(text.as_bytes());
                                // Ensure newline termination
                                if !text.ends_with('\n') {
                                    self.output.push(b'\n');
                                }
                            }
                        }
                        Flow::Continue
                    }
                    None => {
                        // Execute pattern space as a command, replace it with output
                        if let Ok(output) = std::process::Command::new("sh")
                            .arg("-c")
                            .arg(&self.pattern_space)
                            .output()
                        {
                            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
                            if text.ends_with('\n') {
                                text.pop();
                            }
                            self.pattern_space = text;
                        }
                        Flow::Continue
                    }
                }
            }

            Command::Filename => {
                let name = self.current_filename.as_deref().unwrap_or("-");
                let s = format!("{name}\n");
                self.output.extend_from_slice(s.as_bytes());
                Flow::Continue
            }

            Command::Block(cmds) => {
                // Commands inside blocks get their own range offset
                let block_offset = range_offset + _all_commands.len();
                self.execute_commands_with_offset(cmds, block_offset);
                if self.quit {
                    Flow::Quit
                } else {
                    Flow::Continue
                }
            }
        }
    }

    fn do_substitute(&mut self, re: &Regex, replacement: &str, flags: &SubstFlags) -> bool {
        let input = self.pattern_space.clone();

        if flags.global {
            let result = re.replace_all(&input, |caps: &regex::Captures| {
                build_replacement(caps, replacement)
            });
            if result != input {
                self.pattern_space = result.into_owned();
                return true;
            }
        } else if let Some(nth) = flags.nth {
            let mut count = 0;
            let mut last_end = 0;
            let mut result = String::new();
            let mut replaced = false;

            for m in re.find_iter(&input) {
                count += 1;
                if count == nth {
                    result.push_str(&input[last_end..m.start()]);
                    if let Some(caps) = re.captures(&input[m.start()..]) {
                        result.push_str(&build_replacement(&caps, replacement));
                    }
                    last_end = m.end();
                    replaced = true;
                    break;
                }
            }

            if replaced {
                result.push_str(&input[last_end..]);
                self.pattern_space = result;
                return true;
            }
        } else {
            // Replace first occurrence
            if let Some(caps) = re.captures(&input) {
                let m = caps.get(0).unwrap();
                let mut result = String::new();
                result.push_str(&input[..m.start()]);
                result.push_str(&build_replacement(&caps, replacement));
                result.push_str(&input[m.end()..]);
                self.pattern_space = result;
                return true;
            }
        }

        false
    }

    fn write_to_file(&self, file: &str) -> io::Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(file)?;
        writeln!(f, "{}", self.pattern_space)
    }
}

/// Convert a byte to its control character equivalent.
/// For ASCII letters, uppercases first so \ca == \cA.
/// For other chars, XOR with 0x40.
fn ctrl_char(b: u8) -> u8 {
    if b.is_ascii_lowercase() {
        b.to_ascii_uppercase() ^ 0x40
    } else {
        b ^ 0x40
    }
}

fn count_commands(commands: &[SedCommand]) -> usize {
    let mut n = commands.len();
    for cmd in commands {
        if let Command::Block(ref inner) = cmd.command {
            n += count_commands(inner);
        }
    }
    n
}

fn build_replacement(caps: &regex::Captures, replacement: &str) -> String {
    let mut result = String::new();
    let chars: Vec<char> = replacement.chars().collect();
    let mut i = 0;

    // Case conversion state: 'U' = uppercase all, 'L' = lowercase all,
    // 'u' = uppercase next char, 'l' = lowercase next char, '\0' = none
    let mut case_mode: char = '\0';
    let mut next_char_mode: char = '\0'; // for \u and \l (single-char)

    let apply_case = |s: &str, result: &mut String, case_mode: &mut char, next_char_mode: &mut char| {
        for ch in s.chars() {
            let converted = if *next_char_mode == 'u' {
                *next_char_mode = '\0';
                ch.to_uppercase().collect::<String>()
            } else if *next_char_mode == 'l' {
                *next_char_mode = '\0';
                ch.to_lowercase().collect::<String>()
            } else if *case_mode == 'U' {
                ch.to_uppercase().collect::<String>()
            } else if *case_mode == 'L' {
                ch.to_lowercase().collect::<String>()
            } else {
                ch.to_string()
            };
            result.push_str(&converted);
        }
    };

    while i < chars.len() {
        if chars[i] == '&' {
            let matched = caps.get(0).map_or("", |m| m.as_str());
            apply_case(matched, &mut result, &mut case_mode, &mut next_char_mode);
            i += 1;
        } else if chars[i] == '\\' && i + 1 < chars.len() {
            match chars[i + 1] {
                '0' => {
                    // \0 = entire match (same as &)
                    let matched = caps.get(0).map_or("", |m| m.as_str());
                    apply_case(matched, &mut result, &mut case_mode, &mut next_char_mode);
                    i += 2;
                }
                '1'..='9' => {
                    let n = (chars[i + 1] as u32 - '0' as u32) as usize;
                    if let Some(m) = caps.get(n) {
                        apply_case(m.as_str(), &mut result, &mut case_mode, &mut next_char_mode);
                    }
                    i += 2;
                }
                'n' => {
                    result.push('\n');
                    i += 2;
                }
                '\\' => {
                    apply_case("\\", &mut result, &mut case_mode, &mut next_char_mode);
                    i += 2;
                }
                '&' => {
                    apply_case("&", &mut result, &mut case_mode, &mut next_char_mode);
                    i += 2;
                }
                'U' => {
                    case_mode = 'U';
                    next_char_mode = '\0';
                    i += 2;
                }
                'L' => {
                    case_mode = 'L';
                    next_char_mode = '\0';
                    i += 2;
                }
                'u' => {
                    next_char_mode = 'u';
                    i += 2;
                }
                'l' => {
                    next_char_mode = 'l';
                    i += 2;
                }
                'E' => {
                    case_mode = '\0';
                    next_char_mode = '\0';
                    i += 2;
                }
                _ => {
                    let s = chars[i + 1].to_string();
                    apply_case(&s, &mut result, &mut case_mode, &mut next_char_mode);
                    i += 2;
                }
            }
        } else {
            let s = chars[i].to_string();
            apply_case(&s, &mut result, &mut case_mode, &mut next_char_mode);
            i += 1;
        }
    }
    result
}

fn escape_string(s: &str) -> String {
    let mut result = String::new();
    for ch in s.chars() {
        match ch {
            '\\' => result.push_str("\\\\"),
            '\x07' => result.push_str("\\a"),
            '\x08' => result.push_str("\\b"),
            '\x0c' => result.push_str("\\f"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            '\x0b' => result.push_str("\\v"),
            c if c.is_ascii() && !c.is_ascii_control() => result.push(c),
            c => {
                for b in c.to_string().bytes() {
                    result.push_str(&format!("\\{:03o}", b));
                }
            }
        }
    }
    result
}

#[derive(Debug)]
enum Flow {
    Continue,
    Restart,
    Branch(String),
    EndOfCycle,
    Quit,
    QuitNoprint,
}

// ---------------------------------------------------------------------------
// Argument parsing & main
// ---------------------------------------------------------------------------

fn read_script_file(path: &str) -> Result<String, String> {
    if path == "-" {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| format!("sed: -: {e}"))?;
        Ok(buf)
    } else {
        std::fs::read_to_string(path).map_err(|e| format!("sed: {path}: {e}"))
    }
}

fn parse_options() -> Options {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut opts = Options {
        in_place: None,
        quiet: false,
        extended: false,
        expressions: Vec::new(),
        files: Vec::new(),
        posix: false,
        unbuffered: false,
        null_data: false,
        separate: false,
    };

    let mut i = 0;
    let mut saw_dashdash = false;

    while i < args.len() {
        if saw_dashdash {
            opts.files.push(args[i].clone());
            i += 1;
            continue;
        }

        match args[i].as_str() {
            "--" => {
                saw_dashdash = true;
                i += 1;
            }
            "--version" => {
                println!("sed (rust-sed) {}", env!("CARGO_PKG_VERSION"));
                process::exit(0);
            }
            "--help" => {
                println!("Usage: sed [OPTION]... {{script}} [input-file]...");
                println!("  -n, --quiet, --silent    suppress automatic printing");
                println!("  -e script                add commands");
                println!("  -f file                  add commands from file");
                println!("  -i[SUFFIX]               edit files in place");
                println!("  -E, -r, --regexp-extended use extended regexes");
                println!("  -s, --separate           treat files as separate");
                println!("  -u, --unbuffered         unbuffered I/O");
                println!("  -z, --null-data          NUL-separated lines");
                println!("  --posix                  disable extensions");
                println!("  --version                print version");
                process::exit(0);
            }
            "-n" | "--quiet" | "--silent" => {
                opts.quiet = true;
                i += 1;
            }
            "-E" | "-r" | "--regexp-extended" => {
                opts.extended = true;
                i += 1;
            }
            "-e" => {
                i += 1;
                if i < args.len() {
                    opts.expressions.push(args[i].clone());
                }
                i += 1;
            }
            "-f" => {
                i += 1;
                if i < args.len() {
                    match read_script_file(&args[i]) {
                        Ok(content) => opts.expressions.push(content),
                        Err(e) => {
                            eprintln!("{e}");
                            process::exit(2);
                        }
                    }
                }
                i += 1;
            }
            "-i" => {
                opts.in_place = Some(String::new());
                i += 1;
            }
            "-s" | "--separate" => {
                opts.separate = true;
                i += 1;
            }
            "-u" | "--unbuffered" => {
                opts.unbuffered = true;
                i += 1;
            }
            "-z" | "--null-data" => {
                opts.null_data = true;
                i += 1;
            }
            "--posix" => {
                opts.posix = true;
                i += 1;
            }
            arg if arg.starts_with("-i") => {
                opts.in_place = Some(arg[2..].to_string());
                i += 1;
            }
            arg if arg.starts_with("-e") => {
                opts.expressions.push(arg[2..].to_string());
                i += 1;
            }
            arg if arg.starts_with("-f") => {
                let file = &arg[2..];
                match read_script_file(file) {
                    Ok(content) => opts.expressions.push(content),
                    Err(e) => {
                        eprintln!("{e}");
                        process::exit(2);
                    }
                }
                i += 1;
            }
            // Combined short flags like -ne, -nE
            arg if arg.starts_with('-') && !arg.starts_with("--") && arg.len() > 1 => {
                let chars: Vec<char> = arg[1..].chars().collect();
                let mut j = 0;
                while j < chars.len() {
                    match chars[j] {
                        'n' => opts.quiet = true,
                        'E' | 'r' => opts.extended = true,
                        'u' => opts.unbuffered = true,
                        'z' => opts.null_data = true,
                        's' => opts.separate = true,
                        'e' => {
                            // Rest of arg or next arg is expression
                            let rest: String = chars[j + 1..].iter().collect();
                            if !rest.is_empty() {
                                opts.expressions.push(rest);
                            } else {
                                i += 1;
                                if i < args.len() {
                                    opts.expressions.push(args[i].clone());
                                }
                            }
                            j = chars.len();
                            continue;
                        }
                        'f' => {
                            let rest: String = chars[j + 1..].iter().collect();
                            let file = if !rest.is_empty() {
                                rest
                            } else {
                                i += 1;
                                if i < args.len() {
                                    args[i].clone()
                                } else {
                                    String::new()
                                }
                            };
                            if !file.is_empty() {
                                match read_script_file(&file) {
                                    Ok(content) => opts.expressions.push(content),
                                    Err(e) => {
                                        eprintln!("{e}");
                                        process::exit(2);
                                    }
                                }
                            }
                            j = chars.len();
                            continue;
                        }
                        'i' => {
                            let suffix: String = chars[j + 1..].iter().collect();
                            opts.in_place = Some(suffix);
                            j = chars.len();
                            continue;
                        }
                        _ => {
                            eprintln!("sed: invalid option -- '{}'", chars[j]);
                            process::exit(2);
                        }
                    }
                    j += 1;
                }
                i += 1;
            }
            arg if arg.starts_with('-') && arg.len() > 2 => {
                eprintln!("sed: unrecognized option '{arg}'");
                process::exit(2);
            }
            _ => {
                // First non-option is script if no -e was given AND not in-place mode
                if opts.expressions.is_empty() && opts.in_place.is_none() {
                    opts.expressions.push(args[i].clone());
                } else {
                    opts.files.push(args[i].clone());
                }
                i += 1;
            }
        }
    }

    if opts.expressions.is_empty() {
        eprintln!("sed: no script command has been given");
        process::exit(2);
    }

    opts
}

fn main() {
    let opts = parse_options();

    // Parse each expression separately to detect #n in the first one
    let mut commands = Vec::new();
    let mut hash_n_quiet = false;
    for (idx, expr) in opts.expressions.iter().enumerate() {
        let mut parser = Parser::new(expr, opts.extended);
        match parser.parse_all(idx == 0) {
            Ok(cmds) => {
                if idx == 0 && parser.hash_n_quiet {
                    hash_n_quiet = true;
                }
                commands.extend(cmds);
            }
            Err(e) => {
                eprintln!("sed: {e}");
                process::exit(2);
            }
        }
    }

    let quiet = opts.quiet || hash_n_quiet;

    let stdout = io::stdout();
    let mut out = stdout.lock();

    if opts.in_place.is_some() {
        // In-place editing
        let suffix = opts.in_place.as_deref().unwrap_or("");
        if opts.files.is_empty() {
            eprintln!("sed: no input files for in-place editing");
            process::exit(2);
        }
        for file in &opts.files {
            let content = match std::fs::read_to_string(file) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("sed: {file}: {e}");
                    continue;
                }
            };

            // Create backup if suffix is non-empty
            if !suffix.is_empty() {
                let backup = format!("{file}{suffix}");
                if let Err(e) = std::fs::copy(file, &backup) {
                    eprintln!("sed: cannot create backup {backup}: {e}");
                    continue;
                }
            }

            let reader = io::BufReader::new(content.as_bytes());
            let mut output = Vec::new();
            let mut engine = Engine::new(commands.clone(), quiet);
            let code = engine.run(reader, &mut output).unwrap_or_else(|e| {
                eprintln!("sed: {file}: {e}");
                1
            });

            if let Err(e) = std::fs::write(file, &output) {
                eprintln!("sed: {file}: {e}");
            }

            if code != 0 {
                process::exit(code);
            }
        }
    } else if opts.files.is_empty() || (opts.files.len() == 1 && opts.files[0] == "-") {
        // Read from stdin
        let stdin = io::stdin();
        let reader = stdin.lock();
        let mut engine = Engine::new(commands, quiet);
        let code = engine.run(reader, &mut out).unwrap_or_else(|e| {
            eprintln!("sed: {e}");
            1
        });
        process::exit(code);
    } else {
        // Process files
        let mut engine = Engine::new(commands, quiet);
        for file in &opts.files {
            let content = if file == "-" {
                let mut buf = String::new();
                io::stdin().read_to_string(&mut buf).unwrap_or_default();
                buf
            } else {
                match std::fs::read_to_string(file) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("sed: {file}: {e}");
                        continue;
                    }
                }
            };

            engine.current_filename = Some(file.clone());
            let reader = io::BufReader::new(content.as_bytes());
            if let Err(e) = engine.run(reader, &mut out) {
                eprintln!("sed: {file}: {e}");
            }
        }
    }
}
