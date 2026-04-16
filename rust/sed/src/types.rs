use regex::Regex;

#[derive(Debug, Clone)]
pub enum Address {
    Line(usize),
    Last, // $
    Regex(Regex),
    Step(usize, usize), // first~step
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
        pattern: Option<Regex>, // None means reuse last regex
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
    pub sandbox: bool,      // --sandbox
    pub line_length: usize, // -l N (default line width for `l` command)
}
