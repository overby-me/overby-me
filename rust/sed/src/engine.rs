use std::collections::HashMap;
use std::io::{self, BufRead, Write};

use crate::types::*;
use crate::util::escape_string;

pub struct Engine {
    commands: Vec<SedCommand>,
    quiet: bool,
    posix: bool,
    pattern_space: String,
    hold_space: String,
    line_number: usize,
    last_line: bool,
    last_regex: Option<SedRegex>,
    sub_happened: bool,
    output: Vec<u8>,
    append_queue: Vec<String>,
    quit: bool,
    exit_code: i32,
    suppress_default_print: bool,
    input_lines: Vec<String>,
    input_index: usize,
    pub current_filename: Option<String>,
    pub line_wrap_width: usize,
    #[allow(dead_code)]
    sandbox: bool,
    range_active: Vec<bool>,
    read_line_positions: HashMap<String, usize>, // for R command: track line offset per file
}

impl Engine {
    pub fn new(commands: Vec<SedCommand>, quiet: bool, posix: bool, sandbox: bool, line_length: usize) -> Self {
        let num_cmds = count_commands(&commands);
        Engine {
            commands,
            quiet,
            posix,
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
            line_wrap_width: line_length,
            sandbox,
            range_active: vec![false; num_cmds],
            read_line_positions: HashMap::new(),
        }
    }

    pub fn run<R: BufRead, W: Write>(&mut self, reader: R, writer: &mut W) -> io::Result<i32> {
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
            let flow = self.execute_commands_with_offset(&cmds, 0);
            match flow {
                Flow::EndOfCycle => {
                    self.suppress_default_print = true;
                }
                _ => {}
            }

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
        self.output
            .extend_from_slice(self.pattern_space.as_bytes());
        self.output.push(b'\n');
    }

    fn flush_output<W: Write>(&mut self, writer: &mut W) -> io::Result<()> {
        if !self.output.is_empty() {
            writer.write_all(&self.output)?;
            self.output.clear();
        }
        Ok(())
    }

    fn execute_commands_with_offset(
        &mut self,
        commands: &[SedCommand],
        range_offset: usize,
    ) -> Flow {
        let mut i = 0;
        while i < commands.len() {
            if self.quit {
                return Flow::Quit;
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
                    Flow::Branch(ref label) => {
                        if let Some(target) = Self::find_label(commands, label) {
                            i = target + 1;
                            continue;
                        }
                        // Label not found at this level — propagate up
                        return Flow::Branch(label.clone());
                    }
                    flow @ Flow::EndOfCycle => return flow,
                    Flow::Quit => {
                        self.quit = true;
                        return Flow::Quit;
                    }
                    Flow::QuitNoprint => {
                        self.quit = true;
                        self.suppress_default_print = true;
                        return Flow::QuitNoprint;
                    }
                }
            }
            i += 1;
        }
        Flow::Continue
    }

    fn address_matches(&mut self, addr: &AddressRange, range_idx: usize) -> bool {
        match addr {
            AddressRange::None => true,
            AddressRange::Single(a) => self.addr_matches_single(a),
            AddressRange::Range(a, b) => {
                if range_idx >= self.range_active.len() {
                    self.range_active.resize(range_idx + 1, false);
                }
                // Handle addr 0 as range start: range is active from the very start
                let is_addr_0_start = matches!(a, Address::Line(0));
                if is_addr_0_start && self.line_number == 1 && !self.range_active[range_idx] {
                    // 0,/regex/ — start active, check end on line 1
                    if self.addr_matches_single(b) {
                        // End matches on line 1 — single-line range
                    } else {
                        self.range_active[range_idx] = true;
                    }
                    return true;
                }
                if self.range_active[range_idx] {
                    if self.addr_matches_single(b) {
                        self.range_active[range_idx] = false;
                    }
                    true
                } else if self.addr_matches_single(a) {
                    if self.addr_matches_single(b) {
                        // Single-line range
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
            Address::Line(0) => self.line_number == 1, // addr 0 matches line 1 as single addr
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
                    self.line_number >= *first
                        && (self.line_number - *first).is_multiple_of(*step)
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
                if Self::find_label(inner, label).is_some() {
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
                    None => match &self.last_regex {
                        Some(re) => re.clone(),
                        None => {
                            eprintln!(
                                "sed: -e expression #1, char 0: no previous regular expression"
                            );
                            self.exit_code = 1;
                            return Flow::Quit;
                        }
                    },
                    Some(re) => {
                        self.last_regex = Some(re.clone());
                        re.clone()
                    }
                };

                let result = self.do_substitute(&re, replacement, flags);
                if result {
                    self.sub_happened = true;
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

            Command::PrintEscaped(width) => {
                let escaped = escape_string(&self.pattern_space);
                let line_width = width.unwrap_or(self.line_wrap_width);
                let full = format!("{escaped}$");
                if line_width == 0 {
                    // No wrapping
                    self.output.extend_from_slice(full.as_bytes());
                } else {
                    let bytes = full.as_bytes();
                    let mut pos = 0;
                    while pos < bytes.len() {
                        let remaining = bytes.len() - pos;
                        if remaining <= line_width {
                            self.output.extend_from_slice(&bytes[pos..]);
                            break;
                        } else {
                            // Continuation: (line_width - 1) data bytes + '\'
                            self.output
                                .extend_from_slice(&bytes[pos..pos + line_width - 1]);
                            self.output.push(b'\\');
                            self.output.push(b'\n');
                            pos += line_width - 1;
                        }
                    }
                }
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
                if self.input_index < self.input_lines.len() {
                    let next_line = self.input_lines[self.input_index].clone();
                    self.input_index += 1;
                    self.line_number = self.input_index;
                    self.last_line = self.input_index == self.input_lines.len();
                    self.pattern_space.push('\n');
                    self.pattern_space.push_str(&next_line);
                    Flow::Continue
                } else {
                    // No more input
                    if self.posix || self.quiet {
                        // POSIX/quiet mode: exit without printing
                        self.suppress_default_print = true;
                    }
                    // GNU extension: default print happens via normal cycle end
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
                if let Ok(content) = std::fs::read_to_string(file) {
                    let pos = self.read_line_positions.entry(file.clone()).or_insert(0);
                    if let Some(line) = content.lines().nth(*pos) {
                        self.append_queue.push(line.to_string());
                        *pos += 1;
                    }
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
                    if let Ok(output) = std::process::Command::new("sh")
                        .arg("-c")
                        .arg(cmd_str)
                        .output()
                    {
                        let text = String::from_utf8_lossy(&output.stdout).into_owned();
                        if !text.is_empty() {
                            self.output.extend_from_slice(text.as_bytes());
                            if !text.ends_with('\n') {
                                self.output.push(b'\n');
                            }
                        }
                    }
                    Flow::Continue
                }
                None => {
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
                let block_offset = range_offset + _all_commands.len();
                let flow = self.execute_commands_with_offset(cmds, block_offset);
                match flow {
                    Flow::Continue => Flow::Continue,
                    other => other, // Propagate Branch, EndOfCycle, Quit, etc.
                }
            }
        }
    }

    fn do_substitute(&mut self, re: &SedRegex, replacement: &str, flags: &SubstFlags) -> bool {
        let input = self.pattern_space.clone();

        if flags.global {
            let result = re.replace_all(&input, |caps: &SedCaptures| {
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

            for (start, end) in re.find_iter(&input) {
                count += 1;
                if count == nth {
                    result.push_str(&input[last_end..start]);
                    if let Some(caps) = re.captures(&input[start..]) {
                        result.push_str(&build_replacement(&caps, replacement));
                    }
                    last_end = end;
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
            // First match — use find_iter for position, captures for groups
            let matches = re.find_iter(&input);
            if let Some((start, end)) = matches.into_iter().next() {
                if let Some(caps) = re.captures(&input[start..]) {
                    let mut result = String::new();
                    result.push_str(&input[..start]);
                    result.push_str(&build_replacement(&caps, replacement));
                    result.push_str(&input[end..]);
                    self.pattern_space = result;
                    return true;
                }
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

fn count_commands(commands: &[SedCommand]) -> usize {
    let mut n = commands.len();
    for cmd in commands {
        if let Command::Block(ref inner) = cmd.command {
            n += count_commands(inner);
        }
    }
    n
}

fn build_replacement(caps: &SedCaptures, replacement: &str) -> String {
    let mut result = String::new();
    let chars: Vec<char> = replacement.chars().collect();
    let mut i = 0;

    let mut case_mode: char = '\0';
    let mut next_char_mode: char = '\0';

    let apply_case =
        |s: &str, result: &mut String, case_mode: &mut char, next_char_mode: &mut char| {
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
            let matched = caps.get(0).unwrap_or("");
            apply_case(matched, &mut result, &mut case_mode, &mut next_char_mode);
            i += 1;
        } else if chars[i] == '\\' && i + 1 < chars.len() {
            match chars[i + 1] {
                '0' => {
                    let matched = caps.get(0).unwrap_or("");
                    apply_case(matched, &mut result, &mut case_mode, &mut next_char_mode);
                    i += 2;
                }
                '1'..='9' => {
                    let n = (chars[i + 1] as u32 - '0' as u32) as usize;
                    if let Some(m) = caps.get(n) {
                        apply_case(
                            m,
                            &mut result,
                            &mut case_mode,
                            &mut next_char_mode,
                        );
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
