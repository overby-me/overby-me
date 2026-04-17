use crate::regex_util::{bre_to_ere, fix_posix_char_class, fix_posix_char_class_posix};
use crate::types::*;
use crate::util::ctrl_char;

/// Count unescaped `(` in an ERE pattern to determine number of capture groups
fn count_groups(pattern: &str) -> usize {
    let mut count = 0;
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    let mut in_bracket = false;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2; // skip escaped char
        } else if chars[i] == '[' && !in_bracket {
            in_bracket = true;
            i += 1;
        } else if chars[i] == ']' && in_bracket {
            in_bracket = false;
            i += 1;
        } else if chars[i] == '(' && !in_bracket {
            count += 1;
            i += 1;
        } else {
            i += 1;
        }
    }
    count
}

pub struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
    extended: bool,
    pub hash_n_quiet: bool,
    source: ScriptSource,
    cmd_start: usize,
    pub sandbox: bool,
    pub posix: bool,
    pub is_last_script: bool,
    block_depth: usize,
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
            is_last_script: true,
            block_depth: 0,
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

    /// Push a literal character to `target`. If `ch` is the start of a
    /// multi-byte UTF-8 sequence, consume the continuation bytes from the
    /// input so the full Unicode code point is preserved (otherwise the
    /// byte is pushed as a Latin-1 char).
    fn push_literal(&mut self, ch: u8, target: &mut String) {
        if ch < 0x80 {
            target.push(ch as char);
            return;
        }
        let n = if ch & 0xE0 == 0xC0 {
            2
        } else if ch & 0xF0 == 0xE0 {
            3
        } else if ch & 0xF8 == 0xF0 {
            4
        } else {
            1
        };
        let mut buf = [0u8; 4];
        buf[0] = ch;
        let mut len = 1;
        while len < n {
            match self.input.get(self.pos) {
                Some(&b) if b & 0xC0 == 0x80 => {
                    buf[len] = b;
                    len += 1;
                    self.pos += 1;
                }
                _ => break,
            }
        }
        match std::str::from_utf8(&buf[..len]) {
            Ok(s) => {
                if let Some(c) = s.chars().next() {
                    target.push(c);
                }
            }
            Err(_) => target.push(ch as char),
        }
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
            if self.peek() == Some(b'!') {
                self.advance();
                return Err(self.err("multiple `!'s"));
            }
            true
        } else {
            false
        };

        if self.at_end() {
            if !matches!(address, AddressRange::None) || negated {
                return Err(self.err("missing command"));
            }
            return Ok(None);
        }

        // Validate: some commands don't accept addresses — check before parsing
        if !matches!(address, AddressRange::None) {
            if self.peek() == Some(b':') {
                self.advance();
                return Err(self.err(": doesn't want any addresses"));
            }
            if self.peek() == Some(b'#') {
                self.advance();
                return Err(self.err("comments don't accept any addresses"));
            }
            if self.peek() == Some(b'}') && self.block_depth > 0 {
                return Err(self.err_at(
                    self.pos + 1,
                    "`}' doesn't want any addresses",
                ));
            }
        }

        // One-address commands with range address
        // q/Q always reject ranges; others only in --posix mode
        if matches!(address, AddressRange::Range(_, _)) {
            if let Some(ch) = self.peek() {
                let always_one_addr = b"qQ";
                let posix_one_addr = b"aicl=rR";
                if always_one_addr.contains(&ch)
                    || (self.posix && posix_one_addr.contains(&ch))
                {
                    self.advance();
                    return Err(self.err("command only uses one address"));
                }
            }
        }

        let cmd = self.parse_command_char()?;

        // Check for extra characters after zero-argument commands
        let zero_arg = matches!(
            cmd,
            Command::Delete
                | Command::DeleteFirstLine
                | Command::Print
                | Command::PrintFirstLine
                | Command::PrintLineNum
                | Command::HoldReplace
                | Command::HoldAppend
                | Command::GetReplace
                | Command::GetAppend
                | Command::Exchange
                | Command::Next
                | Command::NextAppend
                | Command::Filename
                | Command::PrintEscaped(_)
                | Command::Quit(_)
                | Command::QuitNoprint(_)
                | Command::Block(_)
                | Command::Transliterate(_, _)
        );
        if zero_arg {
            if let Some(ch) = self.peek() {
                if ch != b'\n' && ch != b';' && ch != b'}' && ch != b'#'
                    && ch != b' ' && ch != b'\t'
                {
                    return Err(self.err_at(
                        self.pos - self.cmd_start + 1,
                        "extra characters after command",
                    ));
                }
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
            Some(Address::Relative(_)) | Some(Address::Multiple(_)) => {
                return Err(self.err("invalid usage of +N or ~N as first address"));
            }
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
                            // POSIX rejects address 0
                            if self.posix && matches!(addr, Address::Line(0)) {
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
            Some(b'+') => {
                if self.posix {
                    return Err(self.err_at(
                        self.pos - self.cmd_start + 1,
                        "unexpected `,'",
                    ));
                }
                self.advance();
                let n = self.parse_number()?;
                Ok(Some(Address::Relative(n)))
            }
            Some(b'~') => {
                if self.posix {
                    return Err(self.err_at(
                        self.pos - self.cmd_start + 1,
                        "unexpected `,'",
                    ));
                }
                self.advance();
                let n = self.parse_number()?;
                Ok(Some(Address::Multiple(n)))
            }
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
                // Check for regex modifiers (I/M) after closing delimiter
                let has_modifiers = matches!(self.peek(), Some(b'I') | Some(b'M'));
                if pattern.is_empty() {
                    if has_modifiers {
                        return Err(self.err_at(
                            self.pos - self.cmd_start + 1,
                            "cannot specify modifiers on empty regexp",
                        ));
                    }
                    Ok(Some(Address::LastRegex))
                } else {
                    let mut pat = pattern.clone();
                    self.parse_addr_regex_modifiers(&mut pat);
                    let re = self.compile_regex(&pat)?;
                    Ok(Some(Address::Regex(re)))
                }
            }
            Some(b'\\') => {
                self.advance();
                let delim = self.advance().ok_or_else(|| self.err("expected delimiter after \\"))?;
                let pattern = self.parse_regex_delimited(delim)?;
                let has_modifiers = matches!(self.peek(), Some(b'I') | Some(b'M'));
                if pattern.is_empty() {
                    if has_modifiers {
                        return Err(self.err_at(
                            self.pos - self.cmd_start + 1,
                            "cannot specify modifiers on empty regexp",
                        ));
                    }
                    Ok(Some(Address::LastRegex))
                } else {
                    let mut pat = pattern.clone();
                    self.parse_addr_regex_modifiers(&mut pat);
                    let re = self.compile_regex(&pat)?;
                    Ok(Some(Address::Regex(re)))
                }
            }
            _ => Ok(None),
        }
    }

    fn parse_addr_regex_modifiers(&mut self, pattern: &mut String) {
        let mut flags = String::new();
        loop {
            match self.peek() {
                Some(b'I') => {
                    self.advance();
                    if !flags.contains('i') {
                        flags.push('i');
                    }
                }
                Some(b'M') => {
                    self.advance();
                    if !flags.contains('m') {
                        flags.push('m');
                    }
                }
                _ => break,
            }
        }
        if !flags.is_empty() {
            *pattern = format!("(?{flags}){pattern}");
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
                    self.push_literal(ch, &mut pattern);
                } else {
                    pattern.push('\\');
                    self.push_literal(ch, &mut pattern);
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
            } else if ch == b'[' && in_bracket {
                // Inside bracket: check for POSIX class like [:alpha:]
                if self.peek() == Some(b':') || self.peek() == Some(b'.') || self.peek() == Some(b'=') {
                    let kind = self.advance().unwrap();
                    pattern.push('[');
                    pattern.push(kind as char);
                    // Scan for matching :] or .] or =]
                    loop {
                        let c = self.advance().ok_or_else(|| self.err(eof_msg))?;
                        if c == kind && self.peek() == Some(b']') {
                            pattern.push(c as char);
                            pattern.push(']');
                            self.advance();
                            break;
                        }
                        self.push_literal(c, &mut pattern);
                        // Keep scanning — don't let ] close the bracket
                        // The POSIX class must be properly terminated
                    }
                } else {
                    // Plain [ inside bracket — literal
                    pattern.push('[');
                }
            } else if ch == b']' && in_bracket {
                in_bracket = false;
                pattern.push(']');
            } else if ch == delim && !in_bracket {
                break;
            } else {
                self.push_literal(ch, &mut pattern);
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

        let pat = if self.posix {
            fix_posix_char_class_posix(&pat)
        } else {
            fix_posix_char_class(&pat)
        };
        SedRegex::new(&pat).map_err(|e| self.err(&format!("{e}")))
    }

    fn parse_command_char(&mut self) -> Result<Command, String> {
        let ch = self.advance().ok_or_else(|| self.err("expected command"))?;

        // POSIX mode: reject GNU extension commands
        if self.posix
            && matches!(ch, b'e' | b'F' | b'v' | b'Q' | b'T' | b'R' | b'W')
        {
            return Err(self.err(&format!("unknown command: `{}'", ch as char)));
        }

        match ch {
            b'{' => {
                self.block_depth += 1;
                let mut cmds = Vec::new();
                loop {
                    self.skip_blanks_and_newlines();
                    if self.peek() == Some(b'}') {
                        self.advance();
                        break;
                    }
                    if self.at_end() {
                        return Err(self.err_at(0, "unmatched `{'"));
                    }
                    if let Some(cmd) = self.parse_command()? {
                        cmds.push(cmd);
                    }
                }
                self.block_depth -= 1;
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
            b'a' | b'i' | b'c' => {
                let next = self.peek();
                if next.is_none() {
                    if self.is_last_script {
                        return Err(self.err("expected \\ after `a', `c' or `i'"));
                    } else {
                        return Err(self.err("incomplete command"));
                    }
                }
                if self.posix && next != Some(b'\\') {
                    return Err(self.err_at(
                        self.pos - self.cmd_start + 1,
                        "expected \\ after `a', `c' or `i'",
                    ));
                }
                self.skip_optional_backslash_newline();
                let text = self.parse_text_arg();
                // In POSIX mode, a/c/i with no text at end of expression is incomplete
                if self.posix && text.is_empty() && self.at_end() && !self.is_last_script {
                    return Err(self.err("incomplete command"));
                }
                match ch {
                    b'a' => Ok(Command::Append(text)),
                    b'i' => Ok(Command::Insert(text)),
                    b'c' => Ok(Command::Change(text)),
                    _ => unreachable!(),
                }
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
                let mut ver = String::new();
                while let Some(ch) = self.peek() {
                    if ch == b'\n' || ch == b';' {
                        break;
                    }
                    ver.push(ch as char);
                    self.advance();
                }
                // Check version: if specified version > our version, error
                let ver = ver.trim();
                if !ver.is_empty() {
                    // Our version is 4.9 (matching GNU sed test suite)
                    let our_major = 4u32;
                    let our_minor = 9u32;
                    let parts: Vec<&str> = ver.split('.').collect();
                    let req_major = parts.first().and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
                    let req_minor = parts.get(1).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
                    if (req_major, req_minor) > (our_major, our_minor) {
                        return Err(self.err("expected newer version of sed"));
                    }
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
            b'}' => {
                if self.block_depth == 0 {
                    Err(self.err("unexpected `}'"))
                } else {
                    // Inside a block, } after an address means "close block"
                    // Put the } back so the block parser can see it
                    self.pos -= 1;
                    Ok(Command::Noop)
                }
            }
            b'\n' | b';' => Ok(Command::Noop),
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
                SedRegex::new(&if self.posix {
                    fix_posix_char_class_posix(&re_pattern)
                } else {
                    fix_posix_char_class(&re_pattern)
                })
                    .map_err(|e| self.err(&e))?,
            )
        };

        // Validate backreferences in replacement (non-POSIX mode)
        if !self.posix && !pattern_str.is_empty() {
            // Count capture groups in the ERE pattern
            let num_groups = count_groups(&re_pattern);
            // Check replacement for \1-\9 references
            let rchars: Vec<char> = replacement.chars().collect();
            let mut ri = 0;
            while ri < rchars.len() {
                if rchars[ri] == '\\' && ri + 1 < rchars.len() {
                    if let '1'..='9' = rchars[ri + 1] {
                        let n = (rchars[ri + 1] as u32 - '0' as u32) as usize;
                        if n > num_groups {
                            return Err(self.err(&format!(
                                "invalid reference \\{n} on `s' command's RHS"
                            )));
                        }
                    }
                    ri += 2;
                } else {
                    ri += 1;
                }
            }
        }

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
                    return Err(self.err("unterminated `s' command"));
                }
                Some(b'\n') => {
                    if escaped {
                        self.advance();
                        result.push('\n');
                        escaped = false;
                    } else {
                        // Unterminated at end of line — treat as end of replacement
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
                                        // \c\ — check what follows
                                        let saved = self.pos;
                                        self.advance(); // consume first \
                                        if let Some(next2) = self.peek() {
                                            if next2 == b'\\' || next2 == delim {
                                                // \c\\ or \c\<delim> — control char of \
                                                self.advance();
                                                result.push(ctrl_char(b'\\') as char);
                                            } else if b"abcdfnortxvdox".contains(&next2) {
                                                // \c\d, \c\n, etc — recursive escaping
                                                // Skip to end of replacement for correct position
                                                while let Some(c) = self.advance() {
                                                    if c == delim { break; }
                                                }
                                                return Err(self.err(
                                                    "recursive escaping after \\c not allowed",
                                                ));
                                            } else {
                                                // \c\ followed by other char
                                                result.push(ctrl_char(b'\\') as char);
                                            }
                                        } else {
                                            self.pos = saved;
                                            result.push('\\');
                                        }
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
                                    // Truncate to byte range
                                    result.push(char::from_u32(n & 0xFF).unwrap_or('\0'));
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
                                    result.push(char::from_u32(n & 0xFF).unwrap_or('\0'));
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
                                if ch != delim || ch == b'&' || ch == b'\\' {
                                    // Keep backslash for chars with special meaning
                                    // in replacement (& and \), even if they're the delimiter
                                    result.push('\\');
                                }
                                self.push_literal(ch, &mut result);
                            }
                        }
                        escaped = false;
                    } else if ch == b'\\' {
                        escaped = true;
                    } else if ch == delim {
                        break;
                    } else {
                        self.push_literal(ch, &mut result);
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
                    // \r\n (Windows line ending) is OK — skip both
                    // \r alone is treated as unknown option
                    if self.peek() != Some(b'\n') {
                        return Err(self.err("unknown option to `s'"));
                    }
                }
                Some(ch) if ch.is_ascii_digit() => {
                    if flags.nth.is_some() {
                        self.advance();
                        return Err(self.err("multiple number options to `s' command"));
                    }
                    let n = self.parse_number()?;
                    if n == 0 {
                        return Err(
                            self.err("number option to `s' command may not be zero"),
                        );
                    }
                    flags.nth = Some(n);
                }
                Some(ch) if ch.is_ascii_alphabetic() => {
                    self.advance();
                    return Err(self.err("unknown option to `s'"));
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
                        chars.push(char::from_u32(n & 0xFF).unwrap_or('\0'));
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
                        chars.push(char::from_u32(n & 0xFF).unwrap_or('\0'));
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
            if ch == b'\n' || ch == b';' || ch == b'}' || ch == b' ' || ch == b'\t' || ch == b'#' {
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
