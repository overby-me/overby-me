//! Command-line parsing, modelled on `xclip(1)`.
//!
//! Options accept either a separate value (`-selection clipboard`) or an
//! attached one (`-selection=clipboard`), and both single- and double-dash
//! spellings are honoured (`-out`, `--out`). A lone `-` is treated as standard
//! input, like most Unix tools.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Read data in and own a selection (`-i`, the default).
    Copy,
    /// Print the current selection (`-o`).
    Paste,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    Primary,
    Clipboard,
}

pub struct Args {
    pub mode: Mode,
    pub selection: Selection,
    pub target: Option<String>,
    /// Paste requests to serve before exiting; 0 means unlimited.
    pub loops: u32,
    /// Strip a single trailing newline from the data.
    pub rmlastnl: bool,
    /// When copying, also echo the input to stdout.
    pub filter: bool,
    /// Stay in the foreground instead of forking a serving daemon.
    pub foreground: bool,
    pub display: Option<String>,
    /// Positional arguments — input files when copying.
    pub files: Vec<String>,
    pub show_help: bool,
    pub show_version: bool,
}

impl Args {
    pub fn parse(argv: &[String]) -> Result<Args, String> {
        let mut args = Args {
            // xclip defaults: copy into the PRIMARY selection.
            mode: Mode::Copy,
            selection: Selection::Primary,
            target: None,
            loops: 0,
            rmlastnl: false,
            filter: false,
            foreground: false,
            display: None,
            files: Vec::new(),
            show_help: false,
            show_version: false,
        };

        let mut i = 0;
        while i < argv.len() {
            let arg = &argv[i];

            if arg == "-" || !arg.starts_with('-') {
                args.files.push(arg.clone());
                i += 1;
                continue;
            }

            let bare = arg.trim_start_matches('-');
            let (name, inline) = match bare.split_once('=') {
                Some((k, v)) => (k, Some(v.to_string())),
                None => (bare, None),
            };

            // Fetch this option's value from `=value` or the next argument.
            let mut value = || -> Result<String, String> {
                if let Some(v) = inline.clone() {
                    return Ok(v);
                }
                i += 1;
                argv.get(i)
                    .cloned()
                    .ok_or_else(|| format!("option '-{name}' requires an argument"))
            };

            match name {
                "i" | "in" => args.mode = Mode::Copy,
                "o" | "out" => args.mode = Mode::Paste,
                "sel" | "selection" => args.selection = parse_selection(&value()?)?,
                "t" | "tar" | "target" => args.target = Some(value()?),
                "l" | "loops" => {
                    let v = value()?;
                    args.loops = v.parse().map_err(|_| format!("invalid loop count '{v}'"))?;
                }
                "r" | "rmlastnl" => args.rmlastnl = true,
                "f" | "filter" => args.filter = true,
                "fg" | "foreground" | "quiet" => args.foreground = true,
                "d" | "display" => args.display = Some(value()?),
                "h" | "help" => args.show_help = true,
                "version" => args.show_version = true,
                // Accepted for xclip compatibility but not meaningful here.
                "noutf8" | "verbose" | "silent" => {}
                _ => return Err(format!("invalid option '{arg}'")),
            }
            i += 1;
        }

        Ok(args)
    }
}

/// Resolve a selection name, accepting unambiguous abbreviations like xclip.
fn parse_selection(value: &str) -> Result<Selection, String> {
    let v = value.to_ascii_lowercase();
    if v.is_empty() {
        return Err("empty selection name".to_string());
    }
    if "primary".starts_with(&v) {
        Ok(Selection::Primary)
    } else if "clipboard".starts_with(&v) || v == "b" {
        Ok(Selection::Clipboard)
    } else if "secondary".starts_with(&v) {
        Err("the secondary selection does not exist on Wayland".to_string())
    } else {
        Err(format!("unknown selection '{value}'"))
    }
}
