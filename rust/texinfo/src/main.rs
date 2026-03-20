use std::collections::HashMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process;

// ---------------------------------------------------------------------------
// Texinfo → Info converter
// ---------------------------------------------------------------------------

struct Converter {
    input_dirs: Vec<PathBuf>,
    nodes: Vec<Node>,
    current_node: Option<String>,
    text: String,
    #[allow(dead_code)]
    macros: HashMap<String, String>,
    variables: HashMap<String, String>,
    title: String,
    author: String,
    subtitle: String,
    in_menu: bool,
    in_ignore: usize,
    paragraph_indent: String,
    fill_column: usize,
    split_size: usize,
    no_split: bool,
    no_headers: bool,
    output_format: OutputFormat,
}

#[derive(Debug, Clone)]
struct Node {
    name: String,
    next: Option<String>,
    prev: Option<String>,
    up: Option<String>,
    content: String,
    is_top: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum OutputFormat {
    Info,
    Plaintext,
    Html,
}

impl Converter {
    fn new() -> Self {
        Converter {
            input_dirs: vec![PathBuf::from(".")],
            nodes: Vec::new(),
            current_node: None,
            text: String::new(),
            macros: HashMap::new(),
            variables: HashMap::new(),
            title: String::new(),
            author: String::new(),
            subtitle: String::new(),
            in_menu: false,
            in_ignore: 0,
            paragraph_indent: "  ".to_string(),
            fill_column: 72,
            split_size: 300000,
            no_split: false,
            no_headers: false,
            output_format: OutputFormat::Info,
        }
    }

    fn process_file(&mut self, path: &str) -> io::Result<()> {
        let content = self.read_file(path)?;
        self.process_text(&content);
        Ok(())
    }

    fn read_file(&self, path: &str) -> io::Result<String> {
        // Search input dirs
        for dir in &self.input_dirs {
            let full = dir.join(path);
            if full.exists() {
                return std::fs::read_to_string(&full);
            }
        }
        std::fs::read_to_string(path)
    }

    fn process_text(&mut self, input: &str) {
        let lines: Vec<&str> = input.lines().collect();
        let mut i = 0;
        while i < lines.len() {
            let line = lines[i];
            if line.starts_with('@') {
                i = self.process_command(line, &lines, i);
            } else if self.in_ignore > 0 {
                i += 1;
            } else {
                self.append_text(line);
                self.append_text("\n");
                i += 1;
            }
        }
        // Flush remaining text to current node
        self.flush_text();
    }

    fn process_command(&mut self, line: &str, lines: &[&str], idx: usize) -> usize {
        let cmd = extract_command(line);
        let rest = line[cmd.len() + 1..].trim(); // +1 for @

        // Check for @end of ignore blocks
        if self.in_ignore > 0 {
            if cmd == "end" {
                let end_what = rest.split_whitespace().next().unwrap_or("");
                if is_ignore_block(end_what) {
                    self.in_ignore -= 1;
                }
            } else if is_ignore_block(&cmd) {
                self.in_ignore += 1;
            }
            return idx + 1;
        }

        match cmd.as_str() {
            // Structural
            "node" => {
                self.flush_text();
                let parts: Vec<&str> = rest.splitn(4, ',').collect();
                let name = parts.first().map(|s| s.trim().to_string()).unwrap_or_default();
                let next = parts.get(1).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
                let prev = parts.get(2).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
                let up = parts.get(3).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
                let is_top = name.eq_ignore_ascii_case("top");
                self.current_node = Some(name.clone());
                self.nodes.push(Node {
                    name,
                    next,
                    prev,
                    up,
                    content: String::new(),
                    is_top,
                });
                idx + 1
            }
            "top" => {
                self.flush_text();
                self.append_text(&format!("{rest}\n"));
                self.append_text(&"=".repeat(rest.len().max(1)));
                self.append_text("\n\n");
                idx + 1
            }
            "chapter" | "unnumbered" | "appendix" => {
                self.flush_text();
                self.append_text(&format!("\n{rest}\n"));
                self.append_text(&"*".repeat(rest.len().max(1)));
                self.append_text("\n\n");
                idx + 1
            }
            "section" | "unnumberedsec" | "appendixsec" => {
                self.flush_text();
                self.append_text(&format!("\n{rest}\n"));
                self.append_text(&"=".repeat(rest.len().max(1)));
                self.append_text("\n\n");
                idx + 1
            }
            "subsection" | "unnumberedsubsec" | "appendixsubsec" => {
                self.flush_text();
                self.append_text(&format!("\n{rest}\n"));
                self.append_text(&"-".repeat(rest.len().max(1)));
                self.append_text("\n\n");
                idx + 1
            }
            "subsubsection" | "unnumberedsubsubsec" | "appendixsubsubsec" => {
                self.flush_text();
                self.append_text(&format!("\n{rest}\n\n"));
                idx + 1
            }

            // Includes
            "include" => {
                let file = rest.trim();
                if let Ok(content) = self.read_file(file) {
                    self.process_text(&content);
                }
                idx + 1
            }

            // Meta
            "settitle" => {
                self.title = rest.to_string();
                idx + 1
            }
            "author" => {
                self.author = rest.to_string();
                idx + 1
            }
            "subtitle" => {
                self.subtitle = rest.to_string();
                idx + 1
            }
            "set" => {
                let parts: Vec<&str> = rest.splitn(2, char::is_whitespace).collect();
                let key = parts.first().unwrap_or(&"").to_string();
                let val = parts.get(1).unwrap_or(&"").trim().to_string();
                self.variables.insert(key, val);
                idx + 1
            }
            "clear" => {
                let key = rest.split_whitespace().next().unwrap_or("");
                self.variables.remove(key);
                idx + 1
            }
            "value" => {
                let key = rest.trim_matches('{').trim_matches('}');
                if let Some(val) = self.variables.get(key).cloned() {
                    self.append_text(&val);
                }
                idx + 1
            }
            "paragraphindent" => {
                if rest == "none" || rest == "0" {
                    self.paragraph_indent = String::new();
                } else if let Ok(n) = rest.parse::<usize>() {
                    self.paragraph_indent = " ".repeat(n);
                }
                idx + 1
            }

            // Formatting
            "example" | "smallexample" | "lisp" | "smalllisp" | "display"
            | "smalldisplay" | "format" | "smallformat" | "verbatim" => {
                self.append_text("\n");
                let end_tag = cmd.as_str();
                let mut j = idx + 1;
                while j < lines.len() {
                    if lines[j].trim() == format!("@end {end_tag}") {
                        break;
                    }
                    self.append_text("     ");
                    self.append_text(&expand_inline(lines[j]));
                    self.append_text("\n");
                    j += 1;
                }
                self.append_text("\n");
                j + 1 // skip @end line
            }
            "quotation" | "smallquotation" => {
                self.append_text("\n");
                let end_tag = cmd.as_str();
                let mut j = idx + 1;
                while j < lines.len() {
                    if lines[j].trim() == format!("@end {end_tag}") {
                        break;
                    }
                    self.append_text("  ");
                    self.append_text(&expand_inline(lines[j]));
                    self.append_text("\n");
                    j += 1;
                }
                self.append_text("\n");
                j + 1
            }
            "itemize" | "enumerate" => {
                self.append_text("\n");
                let end_tag = cmd.as_str();
                let mut j = idx + 1;
                let mut item_num = 0;
                while j < lines.len() {
                    let l = lines[j].trim();
                    if l == format!("@end {end_tag}") {
                        break;
                    }
                    if l.starts_with("@item") {
                        item_num += 1;
                        let marker = if end_tag == "enumerate" {
                            format!("  {}. ", item_num)
                        } else {
                            "   * ".to_string()
                        };
                        let item_text = l.strip_prefix("@item").unwrap_or("").trim();
                        self.append_text(&marker);
                        if !item_text.is_empty() {
                            self.append_text(&expand_inline(item_text));
                        }
                        self.append_text("\n");
                    } else if !l.is_empty() {
                        self.append_text("     ");
                        self.append_text(&expand_inline(l));
                        self.append_text("\n");
                    } else {
                        self.append_text("\n");
                    }
                    j += 1;
                }
                self.append_text("\n");
                j + 1
            }
            "table" | "ftable" | "vtable" => {
                self.append_text("\n");
                let end_tag = cmd.as_str();
                let mut j = idx + 1;
                while j < lines.len() {
                    let l = lines[j].trim();
                    if l == format!("@end {end_tag}") {
                        break;
                    }
                    if l.starts_with("@item") || l.starts_with("@itemx") {
                        let item_text = l
                            .strip_prefix("@itemx")
                            .or_else(|| l.strip_prefix("@item"))
                            .unwrap_or("")
                            .trim();
                        self.append_text(&expand_inline(item_text));
                        self.append_text("\n");
                    } else if !l.is_empty() {
                        self.append_text("     ");
                        self.append_text(&expand_inline(l));
                        self.append_text("\n");
                    } else {
                        self.append_text("\n");
                    }
                    j += 1;
                }
                self.append_text("\n");
                j + 1
            }
            "multitable" => {
                let mut j = idx + 1;
                while j < lines.len() {
                    if lines[j].trim() == "@end multitable" {
                        break;
                    }
                    let l = lines[j].trim();
                    if l.starts_with("@item") || l.starts_with("@tab") {
                        self.append_text(&expand_inline(l.replace("@item", "").replace("@tab", "\t").trim()));
                        self.append_text("\n");
                    }
                    j += 1;
                }
                j + 1
            }

            // Menu
            "menu" => {
                self.in_menu = true;
                self.append_text("\n* Menu:\n\n");
                idx + 1
            }
            "end" if rest.starts_with("menu") => {
                self.in_menu = false;
                self.append_text("\n");
                idx + 1
            }
            "end" => {
                // Generic @end — skip
                idx + 1
            }

            // Ignore blocks
            "ignore" | "ifhtml" | "ifxml" | "ifdocbook" | "iflatex" | "iftex"
            | "ifplaintext" | "ifnotinfo" => {
                self.in_ignore += 1;
                idx + 1
            }
            // Conditional blocks we process
            "ifinfo" | "ifnottex" | "ifnothtml" | "ifnotxml" | "ifnotdocbook"
            | "ifnotlatex" | "ifnotplaintext" => {
                // Process content (info format)
                idx + 1
            }
            "ifset" => {
                let var = rest.split_whitespace().next().unwrap_or("");
                if !self.variables.contains_key(var) {
                    self.in_ignore += 1;
                }
                idx + 1
            }
            "ifclear" => {
                let var = rest.split_whitespace().next().unwrap_or("");
                if self.variables.contains_key(var) {
                    self.in_ignore += 1;
                }
                idx + 1
            }

            // Indices
            "cindex" | "findex" | "vindex" | "pindex" | "tindex" | "kindex"
            | "defindex" | "defcodeindex" | "synindex" | "syncodeindex"
            | "printindex" => {
                idx + 1
            }

            // Title page
            "titlepage" => {
                let mut j = idx + 1;
                while j < lines.len() {
                    if lines[j].trim() == "@end titlepage" {
                        break;
                    }
                    let l = lines[j];
                    if l.starts_with("@title") {
                        self.title = l.strip_prefix("@title").unwrap_or("").trim().to_string();
                    } else if l.starts_with("@author") {
                        self.author = l.strip_prefix("@author").unwrap_or("").trim().to_string();
                    } else if l.starts_with("@subtitle") {
                        self.subtitle = l.strip_prefix("@subtitle").unwrap_or("").trim().to_string();
                    }
                    j += 1;
                }
                j + 1
            }

            // Directives we can ignore
            "dircategory" | "direntry" | "copying" | "insertcopying"
            | "contents" | "shortcontents" | "summarycontents" | "detailmenu"
            | "finalout" | "setfilename" | "documentencoding" | "documentlanguage"
            | "setchapternewpage" | "headings" | "everyheading" | "everyfoooting"
            | "oddheading" | "evenheading" | "oddfoooting" | "evenfoooting"
            | "allowcodebreakable" | "exampleindent" | "firstparagraphindent"
            | "frenchspacing" | "kbdinputstyle" | "validatemenus" | "macro"
            | "defmac" | "defun" | "deffn" | "deftp" | "defvr" | "defvar"
            | "defopt" | "defspec" | "deftypefn" | "deftypevar" | "deftypevr"
            | "deftypefun" | "rmacro" | "alias" | "definfoenclose"
            | "sp" | "page" | "vskip" | "need" | "group" | "noindent"
            | "indent" | "exdent" | "headitem" | "columnfractions"
            | "afourpaper" | "afivepaper" | "smallbook" | "pagesizes"
            | "cropmarks" | "c" | "comment" | "anchor"
            | "html" | "tex" | "docbook" | "xml" => {
                // Skip @direntry..@end direntry blocks
                if cmd == "direntry" || cmd == "copying" || cmd == "detailmenu"
                    || cmd == "macro" || cmd == "rmacro" || cmd == "html"
                    || cmd == "tex" || cmd == "docbook" || cmd == "xml"
                    || cmd == "group"
                {
                    let end_tag = cmd.as_str();
                    let mut j = idx + 1;
                    while j < lines.len() {
                        if lines[j].trim() == format!("@end {end_tag}") {
                            break;
                        }
                        j += 1;
                    }
                    return j + 1;
                }
                idx + 1
            }

            // Inline text (handled by expand_inline on the whole line)
            _ => {
                // Unknown command — just output the line as-is with inline expansion
                self.append_text(&expand_inline(line));
                self.append_text("\n");
                idx + 1
            }
        }
    }

    fn append_text(&mut self, s: &str) {
        self.text.push_str(s);
    }

    fn flush_text(&mut self) {
        if self.text.is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.text);
        if let Some(ref name) = self.current_node {
            if let Some(node) = self.nodes.iter_mut().find(|n| &n.name == name) {
                node.content.push_str(&text);
            }
        }
    }

    fn generate_info(&self, filename: &str) -> String {
        let mut output = String::new();

        // File header
        let base = Path::new(filename)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        output.push_str(&format!(
            "This is {base}, produced by makeinfo version {} from {}.\n\n",
            env!("CARGO_PKG_VERSION"),
            filename
        ));

        // Tag table entries
        let offset = output.len();
        let mut tag_table = Vec::new();

        for node in &self.nodes {
            let node_start = output.len();
            tag_table.push((node.name.clone(), node_start));

            // Node header
            output.push(31 as char); // ^_ separator
            output.push('\n');
            output.push_str(&format!(
                "File: {base},  Node: {}",
                node.name
            ));
            if let Some(ref next) = node.next {
                output.push_str(&format!(",  Next: {next}"));
            }
            if let Some(ref prev) = node.prev {
                output.push_str(&format!(",  Prev: {prev}"));
            }
            if let Some(ref up) = node.up {
                output.push_str(&format!(",  Up: {up}"));
            }
            output.push_str("\n\n");
            output.push_str(&node.content);
            if !node.content.ends_with('\n') {
                output.push('\n');
            }
        }

        // Tag table
        output.push(31 as char);
        output.push('\n');
        output.push_str("Tag Table:\n");
        for (name, off) in &tag_table {
            let is_top = self.nodes.iter().any(|n| n.name == *name && n.is_top);
            if is_top {
                output.push_str(&format!("Node: {name}\x7f{off}\n"));
            } else {
                output.push_str(&format!("Node: {name}\x7f{off}\n"));
            }
        }
        output.push_str("End Tag Table\n");

        // Local variables
        output.push(31 as char);
        output.push('\n');
        output.push_str("Local Variables:\ncoding: utf-8\nEnd:\n");

        let _ = offset;
        output
    }
}

fn extract_command(line: &str) -> String {
    let s = &line[1..]; // skip @
    let mut cmd = String::new();
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            cmd.push(ch);
        } else {
            break;
        }
    }
    cmd
}

fn is_ignore_block(name: &str) -> bool {
    matches!(
        name,
        "ignore" | "ifhtml" | "ifxml" | "ifdocbook" | "iflatex" | "iftex"
            | "ifplaintext" | "ifnotinfo" | "ifset" | "ifclear"
    )
}

fn expand_inline(text: &str) -> String {
    let mut result = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' {
            i += 1;
            if i >= chars.len() {
                break;
            }
            // Handle @{ @} @@ @.
            match chars[i] {
                '{' => {
                    result.push('{');
                    i += 1;
                }
                '}' => {
                    result.push('}');
                    i += 1;
                }
                '@' => {
                    result.push('@');
                    i += 1;
                }
                '.' | ':' | '!' | '?' | '*' | '/' | '-' => {
                    // Special formatting — mostly ignore
                    i += 1;
                }
                '\n' => {
                    result.push('\n');
                    i += 1;
                }
                _ => {
                    // Command like @code{...}, @var{...}, etc.
                    let mut cmd = String::new();
                    while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '-') {
                        cmd.push(chars[i]);
                        i += 1;
                    }
                    if i < chars.len() && chars[i] == '{' {
                        i += 1; // skip {
                        let mut depth = 1;
                        let mut content = String::new();
                        while i < chars.len() && depth > 0 {
                            if chars[i] == '{' {
                                depth += 1;
                                content.push('{');
                            } else if chars[i] == '}' {
                                depth -= 1;
                                if depth > 0 {
                                    content.push('}');
                                }
                            } else {
                                content.push(chars[i]);
                            }
                            i += 1;
                        }
                        let expanded = expand_inline(&content);
                        match cmd.as_str() {
                            "code" | "samp" | "kbd" | "env" | "command" | "option"
                            | "file" | "dfn" | "cite" | "abbr" | "acronym" => {
                                result.push('`');
                                result.push_str(&expanded);
                                result.push('\'');
                            }
                            "var" | "emph" | "i" | "slanted" => {
                                result.push_str(&expanded);
                            }
                            "b" | "strong" => {
                                result.push_str(&expanded);
                            }
                            "sc" => {
                                result.push_str(&expanded.to_uppercase());
                            }
                            "t" | "r" | "w" | "math" | "dmn" => {
                                result.push_str(&expanded);
                            }
                            "key" => {
                                result.push('<');
                                result.push_str(&expanded);
                                result.push('>');
                            }
                            "url" | "uref" => {
                                result.push_str(&expanded);
                            }
                            "email" => {
                                result.push('<');
                                result.push_str(&expanded);
                                result.push('>');
                            }
                            "xref" | "pxref" | "ref" | "inforef" => {
                                let parts: Vec<&str> = expanded.splitn(2, ',').collect();
                                let node = parts[0].trim();
                                result.push_str(&format!("*Note {node}::"));
                            }
                            "value" => {
                                result.push_str(&expanded);
                            }
                            "dots" | "enddots" => {
                                result.push_str("...");
                            }
                            "bullet" => {
                                result.push_str("*");
                            }
                            "copyright" => {
                                result.push_str("(C)");
                            }
                            "registeredsymbol" => {
                                result.push_str("(R)");
                            }
                            "result" => {
                                result.push_str("=>");
                            }
                            "expansion" => {
                                result.push_str("==>");
                            }
                            "print" => {
                                result.push_str("-|");
                            }
                            "error" => {
                                result.push_str("error-->");
                            }
                            "point" => {
                                result.push_str("-!-");
                            }
                            "equiv" => {
                                result.push_str("==");
                            }
                            "tie" => {
                                result.push(' ');
                            }
                            _ => {
                                // Unknown command — just output content
                                result.push_str(&expanded);
                            }
                        }
                    } else {
                        // Command without braces
                        match cmd.as_str() {
                            "c" | "comment" => {
                                // Rest of line is comment — skip
                                return result;
                            }
                            "dots" => result.push_str("..."),
                            "enddots" => result.push_str("..."),
                            "bullet" => result.push('*'),
                            "copyright" => result.push_str("(C)"),
                            "minus" => result.push('-'),
                            "comma" => result.push(','),
                            "tab" => result.push('\t'),
                            "noindent" | "indent" | "sp" | "page"
                            | "need" | "vskip" | "group" | "finalout"
                            | "exdent" => {}
                            _ => {
                                // Unknown — pass through
                                result.push('@');
                                result.push_str(&cmd);
                            }
                        }
                    }
                }
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut input_files = Vec::new();
    let mut output_file = None;
    let mut converter = Converter::new();
    let mut no_validate = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--version" => {
                println!("makeinfo (rust-texinfo) {}", env!("CARGO_PKG_VERSION"));
                process::exit(0);
            }
            "--help" => {
                println!("Usage: makeinfo [OPTION]... TEXINFO-FILE...");
                println!("Translate Texinfo source to Info format.");
                println!("  -o, --output=FILE    output to FILE");
                println!("  -I DIR               append DIR to @include search path");
                println!("  --no-split           don't split output");
                println!("  --no-headers         suppress node headers");
                println!("  --no-validate        suppress node cross-reference validation");
                println!("  --plaintext          output plain text");
                println!("  --html               output HTML");
                process::exit(0);
            }
            "-o" | "--output" => {
                i += 1;
                if i < args.len() {
                    output_file = Some(args[i].clone());
                }
            }
            "-I" => {
                i += 1;
                if i < args.len() {
                    converter.input_dirs.push(PathBuf::from(&args[i]));
                }
            }
            "--no-split" => converter.no_split = true,
            "--no-headers" => converter.no_headers = true,
            "--no-validate" => no_validate = true,
            "--plaintext" => converter.output_format = OutputFormat::Plaintext,
            "--html" => converter.output_format = OutputFormat::Html,
            "--xml" | "--docbook" => {
                // Not implemented — ignore silently
            }
            "--fill-column" => {
                i += 1;
                if i < args.len() {
                    converter.fill_column = args[i].parse().unwrap_or(72);
                }
            }
            "--paragraph-indent" => {
                i += 1;
                if i < args.len() {
                    if args[i] == "none" || args[i] == "0" {
                        converter.paragraph_indent = String::new();
                    } else if let Ok(n) = args[i].parse::<usize>() {
                        converter.paragraph_indent = " ".repeat(n);
                    }
                }
            }
            "--split-size" => {
                i += 1;
                if i < args.len() {
                    converter.split_size = args[i].parse().unwrap_or(300000);
                }
            }
            arg if arg.starts_with("--output=") => {
                output_file = Some(arg.strip_prefix("--output=").unwrap().to_string());
            }
            arg if arg.starts_with("-I") && arg.len() > 2 => {
                converter.input_dirs.push(PathBuf::from(&arg[2..]));
            }
            arg if arg.starts_with("--fill-column=") => {
                converter.fill_column = arg
                    .strip_prefix("--fill-column=")
                    .unwrap()
                    .parse()
                    .unwrap_or(72);
            }
            arg if arg.starts_with('-') => {
                // Unknown option — skip
            }
            _ => {
                input_files.push(args[i].clone());
            }
        }
        i += 1;
    }

    let _ = no_validate;

    if input_files.is_empty() {
        eprintln!("makeinfo: no input files");
        process::exit(1);
    }

    for file in &input_files {
        // Add the input file's directory to search path
        if let Some(dir) = Path::new(file).parent() {
            if dir != Path::new("") {
                converter.input_dirs.push(dir.to_path_buf());
            }
        }

        if let Err(e) = converter.process_file(file) {
            eprintln!("makeinfo: {file}: {e}");
            process::exit(1);
        }
    }

    converter.flush_text();

    let input_name = input_files.first().unwrap();
    let info_output = converter.generate_info(input_name);

    let out_path = output_file.unwrap_or_else(|| {
        let p = Path::new(input_name);
        let stem = p.file_stem().unwrap_or_default().to_string_lossy();
        format!("{stem}.info")
    });

    if out_path == "-" {
        let stdout = io::stdout();
        let mut out = stdout.lock();
        let _ = out.write_all(info_output.as_bytes());
    } else {
        if let Some(dir) = Path::new(&out_path).parent() {
            if !dir.exists() && dir != Path::new("") {
                let _ = std::fs::create_dir_all(dir);
            }
        }
        if let Err(e) = std::fs::write(&out_path, &info_output) {
            eprintln!("makeinfo: {out_path}: {e}");
            process::exit(1);
        }
    }
}
