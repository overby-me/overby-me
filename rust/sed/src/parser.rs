use crate::regex_util::{bre_to_ere, fix_posix_char_class};
use crate::types::*;
use crate::util::ctrl_char;

pub struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
    extended: bool,
    pub hash_n_quiet: bool,
    source: ScriptSource,
    cmd_start: usize,
    pub sandbox: bool,
    pub posix: bool,
}

impl<'a> Parser<'a> {
    pub fn new(input: &'a str, extended: bool, source: ScriptSource) -> Self {
        Parser {
            input: input.as_bytes(),
            pos: 0,
            extended,
            hash_n_quiet: false,
            source,
            cmd_start: 0,
            sandbox: false,
            posix: false,
        }
    }

    /// Format an error with explicit char position
    fn err_at(&self, char_pos: usize, msg: &str) -> String {
        match &self.source {
            ScriptSource::Expression(n) => {
                format!("-e expression #{n}, char {char_pos}: {msg}")
            }
            ScriptSource::File(name) => {
                let line = self.input[..self.cmd_start]
                    .iter()
                    .filter(|&&b| b == b'\n')
                    .count()
                    + 1;
                format!("file {name} line {line}: {msg}")
            }
        }
    }

    /// Format an error with source location info
    fn err(&self, msg: &str) -> String {
        match &self.source {
            ScriptSource::Expression(n) => {
                format!("-e expression #{n}, char {}: {msg}", self.pos - self.cmd_start)
            }
            ScriptSource::File(name) => {
                // Count line number from cmd_start position
                let line = self.input[..self.cmd_start]
                    .iter()
                    .filter(|&&b| b == b'\n')
                    .count()
                    + 1;
                format!("file {name} line {line}: {msg}")
            }
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

    pub fn parse_all(&mut self, is_first_script: bool) -> Result<Vec<SedCommand>, String> {
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
        self.cmd_start = self.pos;

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

        // Validate: some commands don't accept addresses
        if !matches!(address, AddressRange::None) {
            if matches!(cmd, Command::Label(_)) {
                return Err(self.err(": doesn't want any addresses"));
            }
        }

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
                        Some(addr2) => {
                            // Validate: 0,N (where N is a line number) is invalid
                            if matches!(addr, Address::Line(0))
                                && matches!(addr2, Address::Line(_))
                            {
                                return Err(
                                    self.err_at(self.pos + 1, "invalid usage of line address 0"),
                                );
                            }
                            Ok(AddressRange::Range(addr, addr2))
                        }
                        None => Err(self.err("expected address after ','")),
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
                if pattern.is_empty() {
                    // Empty regex — reuse last
                    Ok(Some(Address::LastRegex))
                } else {
                    let re = self.compile_regex(&pattern)?;
                    Ok(Some(Address::Regex(re)))
                }
            }
            Some(b'\\') => {
                self.advance();
                let delim = self.advance().ok_or_else(|| self.err("expected delimiter after \\"))?;
                let pattern = self.parse_regex_delimited(delim)?;
                if pattern.is_empty() {
                    Ok(Some(Address::LastRegex))
                } else {
                    let re = self.compile_regex(&pattern)?;
                    Ok(Some(Address::Regex(re)))
                }
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
        self.parse_regex_delimited_ctx(delim, "unterminated address regex")
    }

    fn parse_regex_delimited_ctx(&mut self, delim: u8, eof_msg: &str) -> Result<String, String> {
        let mut pattern = String::new();
        let mut escaped = false;
        let mut in_bracket = false;
        loop {
            let ch = self.advance().ok_or_else(|| self.err(eof_msg))?;
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

    fn compile_regex(&self, pattern: &str) -> Result<SedRegex, String> {
        if pattern.is_empty() {
            return Err(self.err("empty regex"));
        }

        let pat = if self.extended {
            pattern.to_string()
        } else {
            bre_to_ere(pattern)
        };

        let pat = fix_posix_char_class(&pat);
        SedRegex::new(&pat).map_err(|e| self.err(&format!("{e}")))
    }

    fn parse_command_char(&mut self) -> Result<Command, String> {
        let ch = self.advance().ok_or_else(|| self.err("expected command"))?;
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
                        return Err(self.err("unmatched `{'"));
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
                // l may have optional line width: l80, l1, etc. (GNU extension)
                let width = if let Some(ch) = self.peek()
                    && ch.is_ascii_digit()
                {
                    if self.posix {
                        return Err(self.err_at(
                            self.pos - self.cmd_start + 1,
                            "extra characters after command",
                        ));
                    }
                    Some(self.parse_number()?)
                } else {
                    None
                };
                Ok(Command::PrintEscaped(width))
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
                    return Err(self.err("\":\" lacks a label"));
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
            b'r' | b'R' | b'w' | b'W' => {
                if self.sandbox {
                    return Err(self.err("e/r/w commands disabled in sandbox mode"));
                }
                self.skip_whitespace();
                let file = self.parse_filename();
                if file.is_empty() {
                    return Err(
                        self.err("missing filename in r/R/w/W commands"),
                    );
                }
                match ch {
                    b'r' => Ok(Command::ReadFile(file)),
                    b'R' => Ok(Command::ReadLine(file)),
                    b'w' => Ok(Command::WriteFile(file)),
                    b'W' => Ok(Command::WriteFirstLine(file)),
                    _ => unreachable!(),
                }
            }
            b'e' => {
                if self.sandbox {
                    return Err(self.err("e/r/w commands disabled in sandbox mode"));
                }
                if self.peek() == Some(b'\n')
                    || self.peek() == Some(b';')
                    || self.at_end()
                {
                    Ok(Command::Execute(None))
                } else {
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
                self.skip_whitespace();
                while let Some(ch) = self.peek() {
                    if ch == b'\n' || ch == b';' {
                        break;
                    }
                    self.advance();
                }
                Ok(Command::Noop)
            }
            b'#' => {
                while let Some(ch) = self.advance() {
                    if ch == b'\n' {
                        break;
                    }
                }
                Ok(Command::Noop)
            }
            b'\n' | b';' | b'}' => Ok(Command::Noop),
            _ => Err(self.err(&format!("unknown command: `{}'", char::from(ch)))),
        }
    }

    fn parse_substitute(&mut self) -> Result<Command, String> {
        let delim = self.advance().ok_or_else(|| self.err("unterminated `s' command"))?;
        if delim == b'\\' || delim == b'\n' {
            return Err(self.err("delimiter character is not a single-byte character"));
        }

        let pattern_str = self.parse_regex_delimited_ctx(delim, "unterminated `s' command")?;
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
            None
        } else {
            Some(
                SedRegex::new(&fix_posix_char_class(&re_pattern))
                    .map_err(|e| self.err(&e))?,
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
                None => break,
                Some(b'\n') => {
                    if escaped {
                        self.advance();
                        result.push('\n');
                        escaped = false;
                    } else {
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
                                if let Some(next) = self.peek() {
                                    if next == delim {
                                        result.push('\\');
                                    } else if next == b'\\' {
                                        self.advance();
                                        if let Some(next2) = self.peek() {
                                            if next2 == b'\\' || next2 == delim {
                                                self.advance();
                                            }
                                        }
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
                                    result.push('d');
                                }
                            }
                            b'o' => {
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
        let mut seen_g = false;
        let mut seen_p = false;
        loop {
            match self.peek() {
                Some(b'g') => {
                    self.advance();
                    if seen_g {
                        return Err(self.err("multiple `g' options to `s' command"));
                    }
                    seen_g = true;
                    flags.global = true;
                }
                Some(b'p') => {
                    self.advance();
                    if seen_p {
                        return Err(self.err("multiple `p' options to `s' command"));
                    }
                    seen_p = true;
                    flags.print = true;
                }
                Some(b'i') | Some(b'I') => {
                    self.advance();
                    if self.posix {
                        return Err(self.err("unknown option to `s'"));
                    }
                    flags.case_insensitive = true;
                }
                Some(b'e') => {
                    self.advance();
                    if self.posix {
                        return Err(self.err("unknown option to `s'"));
                    }
                    if self.sandbox {
                        return Err(self.err("e/r/w commands disabled in sandbox mode"));
                    }
                    if flags.print && !flags.execute {
                        flags.print_before_exec = true;
                    }
                    flags.execute = true;
                }
                Some(b'm') | Some(b'M') => {
                    self.advance();
                    if self.posix {
                        return Err(self.err("unknown option to `s'"));
                    }
                    flags.multiline = true;
                }
                Some(b'w') => {
                    self.advance();
                    if self.sandbox {
                        return Err(self.err("e/r/w commands disabled in sandbox mode"));
                    }
                    self.skip_whitespace();
                    let file = self.parse_filename();
                    if file.is_empty() {
                        return Err(
                            self.err("missing filename in r/R/w/W commands"),
                        );
                    }
                    flags.write_file = Some(file);
                    break;
                }
                Some(b'\r') => {
                    self.advance();
                }
                Some(ch) if ch.is_ascii_digit() => {
                    let n = self.parse_number()?;
                    if n == 0 {
                        return Err(
                            self.err("number option to `s' command may not be zero"),
                        );
                    }
                    flags.nth = Some(n);
                }
                _ => break,
            }
        }
        Ok(flags)
    }

    fn parse_transliterate(&mut self) -> Result<Command, String> {
        let delim = self.advance().ok_or_else(|| self.err("unterminated `y' command"))?;
        let src = self.parse_translit_chars(delim)?;
        let dst = self.parse_translit_chars(delim)?;
        if src.len() != dst.len() {
            return Err(self.err("strings for `y' command are different lengths"));
        }
        Ok(Command::Transliterate(src, dst))
    }

    fn parse_translit_chars(&mut self, delim: u8) -> Result<Vec<char>, String> {
        let mut chars = Vec::new();
        let mut escaped = false;
        loop {
            let ch = self.advance().ok_or_else(|| self.err("unterminated `y' command"))?;
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
                        if let Some(next) = self.peek() {
                            if next == delim {
                                // \c at end of set — incomplete
                            } else if next == b'\\' {
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
        if self.peek() == Some(b'\\') {
            self.advance();
            if self.peek() == Some(b'\n') {
                self.advance();
            }
        } else if self.peek() == Some(b' ') || self.peek() == Some(b'\t') {
            self.skip_whitespace();
        }
    }

    fn parse_text_arg(&mut self) -> String {
        let mut text = String::new();
        loop {
            // Read one line
            let mut line = String::new();
            let mut ends_with_backslash = false;
            loop {
                match self.peek() {
                    None => break,
                    Some(b'\n') => {
                        self.advance();
                        break;
                    }
                    Some(b'\\') => {
                        self.advance();
                        match self.peek() {
                            Some(b'n') => {
                                self.advance();
                                line.push('\n');
                            }
                            Some(b'\n') => {
                                // Continuation: \ at end of line
                                self.advance();
                                ends_with_backslash = true;
                                break;
                            }
                            None => {
                                // \ at end of input — treat as continuation (empty next line)
                                ends_with_backslash = false;
                                break;
                            }
                            Some(ch) => {
                                line.push('\\');
                                line.push(ch as char);
                                self.advance();
                            }
                        }
                    }
                    Some(ch) => {
                        self.advance();
                        line.push(ch as char);
                    }
                }
            }

            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&line);

            if !ends_with_backslash {
                break;
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
