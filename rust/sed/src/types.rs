/// Regex wrapper: uses fast `regex` crate when possible, falls back to
/// `fancy_regex` for patterns with backreferences.
#[derive(Debug, Clone)]
pub enum SedRegex {
    Fast(regex::Regex),
    Fancy(fancy_regex::Regex),
}

impl SedRegex {
    pub fn new(pattern: &str) -> Result<Self, String> {
        // Try fast regex first
        match regex::Regex::new(pattern) {
            Ok(re) => Ok(SedRegex::Fast(re)),
            Err(_) => {
                // Fall back to fancy-regex (supports backreferences)
                fancy_regex::RegexBuilder::new(pattern)
                    .backtrack_limit(10_000_000)
                    .build()
                    .map(SedRegex::Fancy)
                    .map_err(|e| {
                        // Clean up fancy-regex error messages to match GNU sed format
                        let msg = format!("{e}");
                        // Strip "Parsing error at position N: " prefix
                        if let Some(rest) = msg.strip_prefix("Parsing error at position ") {
                            if let Some(colon_pos) = rest.find(": ") {
                                return rest[colon_pos + 2..].to_string();
                            }
                        }
                        msg
                    })
            }
        }
    }

    pub fn is_match(&self, text: &str) -> bool {
        match self {
            SedRegex::Fast(re) => re.is_match(text),
            SedRegex::Fancy(re) => re.is_match(text).unwrap_or(false),
        }
    }

    pub fn captures<'t>(&self, text: &'t str) -> Option<SedCaptures<'t>> {
        match self {
            SedRegex::Fast(re) => re.captures(text).map(SedCaptures::Fast),
            SedRegex::Fancy(re) => re.captures(text).ok().flatten().map(SedCaptures::Fancy),
        }
    }

    pub fn find_iter<'r, 't>(&'r self, text: &'t str) -> Vec<(usize, usize)> {
        match self {
            SedRegex::Fast(re) => re.find_iter(text).map(|m| (m.start(), m.end())).collect(),
            SedRegex::Fancy(re) => re
                .find_iter(text)
                .filter_map(|m| m.ok())
                .map(|m| (m.start(), m.end()))
                .collect(),
        }
    }

    pub fn replace_all<'a, F>(&self, text: &'a str, replacer: F) -> std::borrow::Cow<'a, str>
    where
        F: Fn(&SedCaptures) -> String,
    {
        let mut result = String::new();
        let mut last_end = 0;
        let matches = self.find_iter(text);
        for (start, end) in matches {
            result.push_str(&text[last_end..start]);
            if let Some(caps) = self.captures(&text[start..]) {
                result.push_str(&replacer(&caps));
            }
            last_end = end;
            if start == end {
                // Zero-width match — advance by one char to avoid infinite loop
                if last_end < text.len() {
                    let next = &text[last_end..];
                    let ch_len = next.chars().next().map_or(1, |c| c.len_utf8());
                    result.push_str(&text[last_end..last_end + ch_len]);
                    last_end += ch_len;
                } else {
                    break;
                }
            }
        }
        result.push_str(&text[last_end..]);
        if result == text {
            std::borrow::Cow::Borrowed(text)
        } else {
            std::borrow::Cow::Owned(result)
        }
    }
}

#[derive(Debug)]
pub enum SedCaptures<'t> {
    Fast(regex::Captures<'t>),
    Fancy(fancy_regex::Captures<'t>),
}

impl<'t> SedCaptures<'t> {
    pub fn get(&self, i: usize) -> Option<&str> {
        match self {
            SedCaptures::Fast(caps) => caps.get(i).map(|m| m.as_str()),
            SedCaptures::Fancy(caps) => caps.get(i).map(|m| m.as_str()),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Address {
    Line(usize),
    Last,                // $
    Regex(SedRegex),
    LastRegex,           // // — reuse last regex
    Step(usize, usize),  // first~step
    Relative(usize),     // +N (only as second address in range)
    Multiple(usize),     // ~N (only as second address in range)
}

#[derive(Debug, Clone)]
pub enum AddressRange {
    None,
    Single(Address),
    Range(Address, Address),
}

#[derive(Debug, Clone, Default)]
pub struct SubstFlags {
    pub global: bool,
    pub nth: Option<usize>, // replace Nth occurrence
    pub print: bool,
    pub write_file: Option<String>,
    pub case_insensitive: bool,
    pub multiline: bool,         // m/M flag
    pub execute: bool,           // e flag — execute result as shell command
    pub print_before_exec: bool, // p came before e in flags
}

#[derive(Debug, Clone)]
pub enum Command {
    Substitute {
        pattern: Option<SedRegex>, // None means reuse last regex
        replacement: String,
        flags: SubstFlags,
    },
    Delete,
    DeleteFirstLine, // D
    Print,
    PrintFirstLine, // P
    PrintEscaped(Option<usize>), // l [width]
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
pub struct SedCommand {
    pub address: AddressRange,
    pub negated: bool,
    pub command: Command,
}

#[derive(Debug)]
pub enum Flow {
    Continue,
    Restart,
    Branch(String),
    EndOfCycle,
    Quit,
    QuitNoprint,
}

/// Tracks where a script expression came from (for error messages)
#[derive(Clone)]
pub enum ScriptSource {
    Expression(usize), // -e expression #N (1-based)
    File(String),      // -f filename
}

pub struct ScriptEntry {
    pub source: ScriptSource,
    pub content: String,
}

pub struct Options {
    pub in_place: Option<String>, // -i[SUFFIX]
    pub quiet: bool,              // -n
    pub extended: bool,           // -E / -r
    pub scripts: Vec<ScriptEntry>,
    pub files: Vec<String>,
    pub posix: bool,        // --posix
    pub unbuffered: bool,   // -u
    pub null_data: bool,    // -z
    pub separate: bool,     // -s
    pub sandbox: bool,          // --sandbox
    pub follow_symlinks: bool,  // --follow-symlinks
    pub line_length: usize,     // -l N (default line width for `l` command)
}
