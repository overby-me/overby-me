use regex::Regex;
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::fs;

use crate::ast::*;
use crate::format::{awk_replace, gensub_replace, sprintf_impl};
use crate::value::{compare_values, ControlFlow, Value};

pub struct Interpreter {
    pub globals: HashMap<String, Value>,
    pub arrays: HashMap<String, HashMap<String, Value>>,
    pub fields: Vec<Value>,
    pub functions: HashMap<String, FuncDef>,
    open_files: HashMap<String, Box<dyn Write>>,
    open_read_files: HashMap<String, Box<dyn BufRead>>,
    open_pipes: HashMap<String, Box<dyn Write>>,
    open_read_pipes: HashMap<String, Box<dyn BufRead>>,
    /// Child processes for pipes, keyed by command string
    pipe_children: HashMap<String, std::process::Child>,
    pub rng_state: u64,
    range_active: HashMap<usize, bool>,
    pub nr: i64,
    pub fnr: i64,
    pub exit_code: i32,
    /// Lines from current input stream for bare getline
    input_lines: Vec<String>,
    /// Current position in input_lines (next line to process)
    input_line_idx: usize,
}

impl Interpreter {
    pub fn new() -> Self {
        let mut globals = HashMap::new();
        globals.insert("FS".to_string(), Value::Str(" ".to_string()));
        globals.insert("RS".to_string(), Value::Str("\n".to_string()));
        globals.insert("OFS".to_string(), Value::Str(" ".to_string()));
        globals.insert("ORS".to_string(), Value::Str("\n".to_string()));
        globals.insert("NR".to_string(), Value::Num(0.0));
        globals.insert("NF".to_string(), Value::Num(0.0));
        globals.insert("FNR".to_string(), Value::Num(0.0));
        globals.insert("FILENAME".to_string(), Value::Str(String::new()));
        globals.insert("SUBSEP".to_string(), Value::Str("\x1c".to_string()));
        globals.insert("RSTART".to_string(), Value::Num(0.0));
        globals.insert("RLENGTH".to_string(), Value::Num(-1.0));
        globals.insert("OFMT".to_string(), Value::Str("%.6g".to_string()));
        globals.insert("CONVFMT".to_string(), Value::Str("%.6g".to_string()));

        Interpreter {
            globals,
            arrays: HashMap::new(),
            fields: vec![Value::Str(String::new())],
            functions: HashMap::new(),
            open_files: HashMap::new(),
            open_read_files: HashMap::new(),
            open_pipes: HashMap::new(),
            open_read_pipes: HashMap::new(),
            rng_state: 0,
            range_active: HashMap::new(),
            nr: 0,
            fnr: 0,
            exit_code: 0,
            pipe_children: HashMap::new(),
            input_lines: Vec::new(),
            input_line_idx: 0,
        }
    }

    fn set_record(&mut self, line: &str) {
        let fs = self
            .globals
            .get("FS")
            .map(|v| v.to_string_val())
            .unwrap_or(" ".to_string());
        self.fields = vec![Value::StrNum(line.to_string())];
        let parts: Vec<String> = if fs == " " {
            line.split_whitespace().map(|s| s.to_string()).collect()
        } else if fs.len() == 1 {
            line.split(fs.chars().next().unwrap())
                .map(|s| s.to_string())
                .collect()
        } else {
            match Regex::new(&fs) {
                Ok(re) => re.split(line).map(|s| s.to_string()).collect(),
                Err(_) => vec![line.to_string()],
            }
        };
        // Fields from input splitting are StrNum
        self.fields
            .extend(parts.into_iter().map(Value::StrNum));
        let nf = (self.fields.len() - 1) as f64;
        self.globals.insert("NF".to_string(), Value::Num(nf));
    }

    fn rebuild_record(&mut self) {
        let ofs = self
            .globals
            .get("OFS")
            .map(|v| v.to_string_val())
            .unwrap_or(" ".to_string());
        if self.fields.len() > 1 {
            let joined = self.fields[1..]
                .iter()
                .map(|v| v.to_string_val())
                .collect::<Vec<_>>()
                .join(&ofs);
            self.fields[0] = Value::StrNum(joined);
        }
    }

    fn get_field(&self, idx: usize) -> Value {
        if idx < self.fields.len() {
            self.fields[idx].clone()
        } else {
            Value::StrNum(String::new())
        }
    }

    fn set_field(&mut self, idx: usize, val: Value) {
        while self.fields.len() <= idx {
            self.fields.push(Value::StrNum(String::new()));
        }
        let is_str = matches!(val, Value::Str(_));
        self.fields[idx] = val;
        let nf = (self.fields.len() - 1) as f64;
        self.globals.insert("NF".to_string(), Value::Num(nf));
        if idx > 0 {
            self.rebuild_record();
        } else {
            // Re-split if $0 was assigned
            let line = self.fields[0].to_string_val();
            self.set_record(&line);
            // If $0 was assigned from a string literal, keep $0 as Str
            // (not StrNum) so boolean context uses string rules
            if is_str {
                self.fields[0] = Value::Str(line);
            }
        }
    }

    pub fn get_var(&self, name: &str) -> Value {
        match name {
            "NR" => Value::Num(self.nr as f64),
            "FNR" => Value::Num(self.fnr as f64),
            _ => self
                .globals
                .get(name)
                .cloned()
                .unwrap_or(Value::Uninitialized),
        }
    }

    pub fn set_var(&mut self, name: &str, val: Value) {
        match name {
            "NR" => self.nr = val.to_num() as i64,
            "FNR" => self.fnr = val.to_num() as i64,
            "NF" => {
                let nf = val.to_num() as usize;
                while self.fields.len() <= nf {
                    self.fields.push(Value::StrNum(String::new()));
                }
                self.fields.truncate(nf + 1);
                self.globals.insert("NF".to_string(), val);
                self.rebuild_record();
            }
            "$0" => {
                // handled elsewhere
            }
            _ => {
                self.globals.insert(name.to_string(), val);
            }
        }
    }

    fn get_array(&self, name: &str, key: &str) -> Value {
        self.arrays
            .get(name)
            .and_then(|a| a.get(key))
            .cloned()
            .unwrap_or(Value::Uninitialized)
    }

    pub fn set_array(&mut self, name: &str, key: &str, val: Value) {
        self.arrays
            .entry(name.to_string())
            .or_default()
            .insert(key.to_string(), val);
    }

    fn array_key(&self, indices: &[Value]) -> String {
        let subsep = self
            .globals
            .get("SUBSEP")
            .map(|v| v.to_string_val())
            .unwrap_or("\x1c".to_string());
        let convfmt = self
            .globals
            .get("CONVFMT")
            .map(|v| v.to_string_val())
            .unwrap_or("%.6g".to_string());
        indices
            .iter()
            .map(|v| v.to_string_with_fmt(&convfmt))
            .collect::<Vec<_>>()
            .join(&subsep)
    }

    pub fn run(&mut self, program: &Program, files: &[String]) {
        // Register functions
        for func in &program.functions {
            self.functions.insert(func.name.clone(), func.clone());
        }

        // Run BEGIN blocks
        for rule in &program.rules {
            if matches!(rule.pattern, Some(Pattern::Begin))
                && let ControlFlow::Exit(code) = self.exec_stmts(&rule.action)
            {
                self.exit_code = code;
                self.run_end_blocks(program);
                return;
            }
        }

        // Process input
        if files.is_empty() {
            self.process_stream(program, &mut io::stdin().lock(), "-");
        } else {
            for file in files {
                if file == "-" {
                    self.process_stream(program, &mut io::stdin().lock(), "-");
                } else {
                    match fs::File::open(file) {
                        Ok(f) => {
                            let mut reader = BufReader::new(f);
                            self.process_stream(program, &mut reader, file);
                        }
                        Err(e) => {
                            eprintln!("awk: can't open file {file}: {e}");
                        }
                    }
                }
                self.fnr = 0;
            }
        }

        self.run_end_blocks(program);
    }

    fn run_end_blocks(&mut self, program: &Program) {
        for rule in &program.rules {
            if matches!(rule.pattern, Some(Pattern::End))
                && let ControlFlow::Exit(code) = self.exec_stmts(&rule.action)
            {
                self.exit_code = code;
                return;
            }
        }
    }

    fn process_stream(&mut self, program: &Program, reader: &mut dyn BufRead, filename: &str) {
        self.globals
            .insert("FILENAME".to_string(), Value::Str(filename.to_string()));

        let rs = self
            .globals
            .get("RS")
            .map(|v| v.to_string_val())
            .unwrap_or("\n".to_string());

        // Read all input and split into records
        let mut all = String::new();
        reader.read_to_string(&mut all).ok();

        let records: Vec<String> = if rs == "\n" {
            all.split('\n')
                .map(|s| {
                    let s = s.strip_suffix('\r').unwrap_or(s);
                    s.to_string()
                })
                .collect()
        } else if rs.len() == 1 {
            let sep = rs.chars().next().unwrap();
            // Remove trailing newline before splitting
            if all.ends_with('\n') {
                all.pop();
            }
            all.split(sep).map(|s| s.to_string()).collect()
        } else if rs.is_empty() {
            // Paragraph mode
            let mut result = Vec::new();
            for para in all.split("\n\n") {
                let para = para.trim_matches('\n');
                if !para.is_empty() {
                    result.push(para.to_string());
                }
            }
            result
        } else {
            // Multi-char RS as regex
            match Regex::new(&rs) {
                Ok(re) => re.split(&all).map(|s| s.to_string()).collect(),
                Err(_) => all.split(&rs).map(|s| s.to_string()).collect(),
            }
        };

        // Remove trailing empty record (artifact of splitting with trailing separator)
        let records: Vec<String> = if records.last().is_some_and(|s| s.is_empty()) {
            records[..records.len() - 1].to_vec()
        } else {
            records
        };

        // Store for bare getline access
        self.input_lines = records;
        self.input_line_idx = 0;

        while self.input_line_idx < self.input_lines.len() {
            let rec = self.input_lines[self.input_line_idx].clone();
            self.input_line_idx += 1;

            self.nr += 1;
            self.fnr += 1;
            self.globals
                .insert("NR".to_string(), Value::Num(self.nr as f64));
            self.globals
                .insert("FNR".to_string(), Value::Num(self.fnr as f64));
            self.set_record(&rec);

            if let ControlFlow::Exit(code) = self.process_rules(program) {
                self.exit_code = code;
                return;
            }
        }
    }

    fn process_rules(&mut self, program: &Program) -> ControlFlow {
        for (idx, rule) in program.rules.iter().enumerate() {
            let should_run = match &rule.pattern {
                None => true,
                Some(Pattern::Begin) | Some(Pattern::End) => false,
                Some(Pattern::Expression(expr)) => {
                    let val = self.eval_expr(expr);
                    val.to_bool()
                }
                Some(Pattern::Range(start, end)) => {
                    let active = self.range_active.get(&idx).copied().unwrap_or(false);
                    if active {
                        let end_val = self.eval_expr(end);
                        if end_val.to_bool() {
                            self.range_active.insert(idx, false);
                        }
                        true
                    } else {
                        let start_val = self.eval_expr(start);
                        if start_val.to_bool() {
                            // Check end pattern on same line
                            let end_val = self.eval_expr(end);
                            if end_val.to_bool() {
                                // Range starts and ends on same line
                                self.range_active.insert(idx, false);
                            } else {
                                self.range_active.insert(idx, true);
                            }
                            true
                        } else {
                            false
                        }
                    }
                }
            };

            if should_run {
                match self.exec_stmts(&rule.action) {
                    ControlFlow::Next => return ControlFlow::None,
                    ControlFlow::Exit(code) => return ControlFlow::Exit(code),
                    _ => {}
                }
            }
        }
        ControlFlow::None
    }

    fn exec_stmts(&mut self, stmts: &[Stmt]) -> ControlFlow {
        for stmt in stmts {
            match self.exec_stmt(stmt) {
                ControlFlow::None => {}
                cf => return cf,
            }
        }
        ControlFlow::None
    }

    fn exec_stmt(&mut self, stmt: &Stmt) -> ControlFlow {
        match stmt {
            Stmt::Expr(expr) => {
                self.eval_expr(expr);
            }
            Stmt::Print(args, dest) => {
                let ofs = self
                    .globals
                    .get("OFS")
                    .map(|v| v.to_string_val())
                    .unwrap_or(" ".to_string());
                let ors = self
                    .globals
                    .get("ORS")
                    .map(|v| v.to_string_val())
                    .unwrap_or("\n".to_string());
                let ofmt = self
                    .globals
                    .get("OFMT")
                    .map(|v| v.to_string_val())
                    .unwrap_or("%.6g".to_string());

                let output = if args.is_empty() {
                    self.get_field(0).to_string_val()
                } else {
                    args.iter()
                        .map(|a| self.eval_expr(a).to_string_with_fmt(&ofmt))
                        .collect::<Vec<_>>()
                        .join(&ofs)
                };

                let output = format!("{output}{ors}");
                self.write_output(&output, dest);
            }
            Stmt::Printf(args, dest) => {
                if args.is_empty() {
                    return ControlFlow::None;
                }
                let vals: Vec<Value> = args.iter().map(|a| self.eval_expr(a)).collect();
                let output = sprintf_impl(&vals);
                self.write_output(&output, dest);
            }
            Stmt::If(cond, then_branch, else_branch) => {
                let val = self.eval_expr(cond);
                if val.to_bool() {
                    return self.exec_stmt(then_branch);
                } else if let Some(else_b) = else_branch {
                    return self.exec_stmt(else_b);
                }
            }
            Stmt::While(cond, body) => loop {
                let val = self.eval_expr(cond);
                if !val.to_bool() {
                    break;
                }
                match self.exec_stmt(body) {
                    ControlFlow::Break => break,
                    ControlFlow::Continue => continue,
                    ControlFlow::None => {}
                    cf => return cf,
                }
            },
            Stmt::DoWhile(body, cond) => loop {
                match self.exec_stmt(body) {
                    ControlFlow::Break => break,
                    ControlFlow::Continue => {}
                    ControlFlow::None => {}
                    cf => return cf,
                }
                let val = self.eval_expr(cond);
                if !val.to_bool() {
                    break;
                }
            },
            Stmt::For(init, cond, update, body) => {
                if let Some(init) = init {
                    self.exec_stmt(init);
                }
                loop {
                    if let Some(cond) = cond {
                        let val = self.eval_expr(cond);
                        if !val.to_bool() {
                            break;
                        }
                    }
                    match self.exec_stmt(body) {
                        ControlFlow::Break => break,
                        ControlFlow::Continue => {}
                        ControlFlow::None => {}
                        cf => return cf,
                    }
                    if let Some(update) = update {
                        self.exec_stmt(update);
                    }
                }
            }
            Stmt::ForIn(var, array, body) => {
                let keys: Vec<String> = self
                    .arrays
                    .get(array)
                    .map(|a| a.keys().cloned().collect())
                    .unwrap_or_default();
                for key in keys {
                    self.set_var(var, Value::Str(key));
                    match self.exec_stmt(body) {
                        ControlFlow::Break => break,
                        ControlFlow::Continue => continue,
                        ControlFlow::None => {}
                        cf => return cf,
                    }
                }
            }
            Stmt::Block(stmts) => {
                return self.exec_stmts(stmts);
            }
            Stmt::Next => return ControlFlow::Next,
            Stmt::Exit(expr) => {
                let code = expr
                    .as_ref()
                    .map(|e| self.eval_expr(e).to_num() as i32)
                    .unwrap_or(0);
                return ControlFlow::Exit(code);
            }
            Stmt::Delete(name, indices) => {
                if indices.is_empty() {
                    self.arrays.remove(name);
                } else {
                    let vals: Vec<Value> = indices.iter().map(|i| self.eval_expr(i)).collect();
                    let key = self.array_key(&vals);
                    if let Some(arr) = self.arrays.get_mut(name) {
                        arr.remove(&key);
                    }
                }
            }
            Stmt::Break => return ControlFlow::Break,
            Stmt::Continue => return ControlFlow::Continue,
            Stmt::Return(expr) => {
                let val = expr
                    .as_ref()
                    .map(|e| self.eval_expr(e))
                    .unwrap_or(Value::Uninitialized);
                return ControlFlow::Return(val);
            }
            Stmt::Getline(var, file, source) => {
                self.eval_getline(
                    var.as_ref().map(|e| e.as_ref()),
                    file.as_ref().map(|e| e.as_ref()),
                    source,
                );
            }
        }
        ControlFlow::None
    }

    fn write_output(&mut self, output: &str, dest: &Option<OutputDest>) {
        match dest {
            None => {
                print!("{output}");
                io::stdout().flush().ok();
            }
            Some(OutputDest::File(expr)) => {
                let filename = self.eval_expr(expr).to_string_val();
                if !self.open_files.contains_key(&filename) {
                    match fs::File::create(&filename) {
                        Ok(f) => {
                            self.open_files.insert(filename.clone(), Box::new(f));
                        }
                        Err(e) => {
                            eprintln!("awk: can't redirect to {filename}: {e}");
                            return;
                        }
                    }
                }
                if let Some(f) = self.open_files.get_mut(&filename) {
                    f.write_all(output.as_bytes()).ok();
                    f.flush().ok();
                }
            }
            Some(OutputDest::Append(expr)) => {
                let filename = self.eval_expr(expr).to_string_val();
                if !self.open_files.contains_key(&filename) {
                    match fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&filename)
                    {
                        Ok(f) => {
                            self.open_files.insert(filename.clone(), Box::new(f));
                        }
                        Err(e) => {
                            eprintln!("awk: can't redirect to {filename}: {e}");
                            return;
                        }
                    }
                }
                if let Some(f) = self.open_files.get_mut(&filename) {
                    f.write_all(output.as_bytes()).ok();
                    f.flush().ok();
                }
            }
            Some(OutputDest::Pipe(expr)) => {
                let cmd = self.eval_expr(expr).to_string_val();
                if !self.open_pipes.contains_key(&cmd) {
                    match Command::new("sh")
                        .arg("-c")
                        .arg(&cmd)
                        .stdin(Stdio::piped())
                        .spawn()
                    {
                        Ok(mut child) => {
                            if let Some(stdin) = child.stdin.take() {
                                self.open_pipes.insert(cmd.clone(), Box::new(stdin));
                                self.pipe_children.insert(cmd.clone(), child);
                            }
                        }
                        Err(e) => {
                            eprintln!("awk: can't open pipe to {cmd}: {e}");
                            return;
                        }
                    }
                }
                if let Some(p) = self.open_pipes.get_mut(&cmd) {
                    p.write_all(output.as_bytes()).ok();
                    p.flush().ok();
                }
            }
        }
    }

    fn eval_getline(
        &mut self,
        var: Option<&Expr>,
        file: Option<&Expr>,
        source: &GetlineSource,
    ) -> Value {
        match source {
            GetlineSource::Stdin => {
                // Bare getline reads from current input stream
                if self.input_line_idx < self.input_lines.len() {
                    let line = self.input_lines[self.input_line_idx].clone();
                    self.input_line_idx += 1;
                    self.nr += 1;
                    self.fnr += 1;
                    self.globals
                        .insert("NR".to_string(), Value::Num(self.nr as f64));
                    self.globals
                        .insert("FNR".to_string(), Value::Num(self.fnr as f64));
                    if let Some(var_expr) = var {
                        self.assign_to(var_expr, Value::StrNum(line));
                    } else {
                        self.set_record(&line);
                    }
                    Value::Num(1.0)
                } else {
                    // No more input — try stdin directly (for interactive/pipe)
                    let mut line = String::new();
                    match io::stdin().lock().read_line(&mut line) {
                        Ok(0) => Value::Num(0.0),
                        Ok(_) => {
                            if line.ends_with('\n') {
                                line.pop();
                            }
                            if line.ends_with('\r') {
                                line.pop();
                            }
                            self.nr += 1;
                            self.globals
                                .insert("NR".to_string(), Value::Num(self.nr as f64));
                            if let Some(var_expr) = var {
                                self.assign_to(var_expr, Value::StrNum(line));
                            } else {
                                self.set_record(&line);
                            }
                            Value::Num(1.0)
                        }
                        Err(_) => Value::Num(-1.0),
                    }
                }
            }
            GetlineSource::File => {
                if let Some(file_expr) = file {
                    let filename = self.eval_expr(file_expr).to_string_val();
                    if !self.open_read_files.contains_key(&filename) {
                        match fs::File::open(&filename) {
                            Ok(f) => {
                                self.open_read_files
                                    .insert(filename.clone(), Box::new(BufReader::new(f)));
                            }
                            Err(_) => return Value::Num(-1.0),
                        }
                    }
                    let mut line = String::new();
                    let result = if let Some(reader) = self.open_read_files.get_mut(&filename) {
                        reader.read_line(&mut line)
                    } else {
                        return Value::Num(-1.0);
                    };
                    match result {
                        Ok(0) => Value::Num(0.0),
                        Ok(_) => {
                            if line.ends_with('\n') {
                                line.pop();
                            }
                            if line.ends_with('\r') {
                                line.pop();
                            }
                            if let Some(var_expr) = var {
                                self.assign_to(var_expr, Value::StrNum(line.clone()));
                            } else {
                                self.set_record(&line);
                            }
                            Value::Num(1.0)
                        }
                        Err(_) => Value::Num(-1.0),
                    }
                } else {
                    Value::Num(-1.0)
                }
            }
            GetlineSource::Pipe => {
                if let Some(cmd_expr) = file {
                    let cmd = self.eval_expr(cmd_expr).to_string_val();
                    if !self.open_read_pipes.contains_key(&cmd) {
                        match Command::new("sh")
                            .arg("-c")
                            .arg(&cmd)
                            .stdout(Stdio::piped())
                            .spawn()
                        {
                            Ok(mut child) => {
                                if let Some(stdout) = child.stdout.take() {
                                    self.open_read_pipes
                                        .insert(cmd.clone(), Box::new(BufReader::new(stdout)));
                                    self.pipe_children.insert(cmd.clone(), child);
                                }
                            }
                            Err(_) => return Value::Num(-1.0),
                        }
                    }
                    let mut line = String::new();
                    let result = if let Some(reader) = self.open_read_pipes.get_mut(&cmd) {
                        reader.read_line(&mut line)
                    } else {
                        return Value::Num(-1.0);
                    };
                    match result {
                        Ok(0) => Value::Num(0.0),
                        Ok(_) => {
                            if line.ends_with('\n') {
                                line.pop();
                            }
                            if line.ends_with('\r') {
                                line.pop();
                            }
                            if let Some(var_expr) = var {
                                self.assign_to(var_expr, Value::StrNum(line.clone()));
                            } else {
                                self.set_record(&line);
                                self.nr += 1;
                                self.globals
                                    .insert("NR".to_string(), Value::Num(self.nr as f64));
                            }
                            Value::Num(1.0)
                        }
                        Err(_) => Value::Num(-1.0),
                    }
                } else {
                    Value::Num(-1.0)
                }
            }
        }
    }

    /// Compile a regex, handling awk-specific patterns that Rust's regex crate
    /// might reject (e.g., leading +, *, ? which awk treats as literals).
    fn compile_regex(pattern: &str) -> Option<Regex> {
        // Pre-process: in awk, quantifiers (+, *, ?) after anchors (^) or at
        // start of alternatives (|, () are literals. Fix before compiling.
        let fixed = Self::fix_awk_regex(pattern);
        Regex::new(&fixed).ok()
    }

    fn fix_awk_regex(pattern: &str) -> String {
        let chars: Vec<char> = pattern.chars().collect();
        let mut result = String::new();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '\\' && i + 1 < chars.len() {
                let next = chars[i + 1];
                // Known regex escapes — pass through
                if "dDwWsStbnrfaevx01234567.^$*+?()[]{}|\\/"
                    .contains(next)
                {
                    result.push(chars[i]);
                    i += 1;
                    result.push(chars[i]);
                    i += 1;
                } else {
                    // Unknown escape (like \8, \@, etc.) — treat as literal char
                    i += 1; // skip backslash
                    result.push(chars[i]);
                    i += 1;
                }
            } else if chars[i] == '[' {
                // Character class — pass through until closing ]
                result.push(chars[i]);
                i += 1;
                // Handle ] as first char in class
                if i < chars.len() && chars[i] == '^' {
                    result.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() && chars[i] == ']' {
                    result.push(chars[i]);
                    i += 1;
                }
                while i < chars.len() && chars[i] != ']' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        result.push(chars[i]);
                        i += 1;
                    }
                    result.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() {
                    result.push(chars[i]); // ]
                    i += 1;
                }
            } else if (chars[i] == '+' || chars[i] == '*' || chars[i] == '?')
                && (i == 0 || matches!(chars[i - 1], '^' | '|' | '('))
            {
                // Quantifier in invalid position — treat as literal
                result.push('\\');
                result.push(chars[i]);
                i += 1;
            } else {
                result.push(chars[i]);
                i += 1;
            }
        }
        result
    }

    /// Extract a regex pattern from an expression.
    /// Handles both Expr::Regex(r) and Expr::Match($0, Regex(r)) which is
    /// how the parser wraps bare /regex/ literals.
    fn extract_regex_pattern(&mut self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::Regex(r) => Some(r.clone()),
            Expr::Match(_, right) => {
                if let Expr::Regex(r) = right.as_ref() {
                    Some(r.clone())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn assign_to(&mut self, expr: &Expr, val: Value) {
        match expr {
            Expr::Var(name) => self.set_var(name, val),
            Expr::FieldRef(idx_expr) => {
                let idx = self.eval_expr(idx_expr).to_num() as usize;
                self.set_field(idx, val);
            }
            Expr::ArrayRef(name, indices) => {
                let vals: Vec<Value> = indices.iter().map(|i| self.eval_expr(i)).collect();
                let key = self.array_key(&vals);
                self.set_array(name, &key, val);
            }
            _ => {}
        }
    }

    fn eval_expr(&mut self, expr: &Expr) -> Value {
        match expr {
            Expr::Number(n) => Value::Num(*n),
            Expr::StringLit(s) => Value::Str(s.clone()),
            Expr::Regex(_) => {
                // Bare regex - shouldn't appear standalone normally
                Value::Str(String::new())
            }
            Expr::Var(name) => self.get_var(name),
            Expr::FieldRef(idx_expr) => {
                let idx = self.eval_expr(idx_expr).to_num() as usize;
                self.get_field(idx)
            }
            Expr::ArrayRef(name, indices) => {
                let vals: Vec<Value> = indices.iter().map(|i| self.eval_expr(i)).collect();
                let key = self.array_key(&vals);
                // Auto-vivify: reading an array element creates it
                let val = self.get_array(name, &key);
                if !self.arrays.get(name).is_some_and(|a| a.contains_key(&key)) {
                    self.set_array(name, &key, Value::Uninitialized);
                }
                val
            }
            Expr::Binop(left, op, right) => match op {
                BinOp::And => {
                    let lv = self.eval_expr(left);
                    if !lv.to_bool() {
                        return Value::Num(0.0);
                    }
                    let rv = self.eval_expr(right);
                    Value::Num(if rv.to_bool() { 1.0 } else { 0.0 })
                }
                BinOp::Or => {
                    let lv = self.eval_expr(left);
                    if lv.to_bool() {
                        return Value::Num(1.0);
                    }
                    let rv = self.eval_expr(right);
                    Value::Num(if rv.to_bool() { 1.0 } else { 0.0 })
                }
                _ => {
                    let lv = self.eval_expr(left);
                    let rv = self.eval_expr(right);
                    match op {
                        BinOp::Add => Value::Num(lv.to_num() + rv.to_num()),
                        BinOp::Sub => Value::Num(lv.to_num() - rv.to_num()),
                        BinOp::Mul => Value::Num(lv.to_num() * rv.to_num()),
                        BinOp::Div => {
                            let d = rv.to_num();
                            if d == 0.0 {
                                eprintln!("awk: division by zero");
                                Value::Num(0.0)
                            } else {
                                Value::Num(lv.to_num() / d)
                            }
                        }
                        BinOp::Mod => {
                            let d = rv.to_num();
                            if d == 0.0 {
                                eprintln!("awk: division by zero");
                                Value::Num(0.0)
                            } else {
                                Value::Num(lv.to_num() % d)
                            }
                        }
                        BinOp::Pow => Value::Num(lv.to_num().powf(rv.to_num())),
                        BinOp::Eq => {
                            let ord = compare_values(&lv, &rv);
                            Value::Num(if ord == std::cmp::Ordering::Equal {
                                1.0
                            } else {
                                0.0
                            })
                        }
                        BinOp::Ne => {
                            let ord = compare_values(&lv, &rv);
                            Value::Num(if ord != std::cmp::Ordering::Equal {
                                1.0
                            } else {
                                0.0
                            })
                        }
                        BinOp::Lt => {
                            let ord = compare_values(&lv, &rv);
                            Value::Num(if ord == std::cmp::Ordering::Less {
                                1.0
                            } else {
                                0.0
                            })
                        }
                        BinOp::Gt => {
                            let ord = compare_values(&lv, &rv);
                            Value::Num(if ord == std::cmp::Ordering::Greater {
                                1.0
                            } else {
                                0.0
                            })
                        }
                        BinOp::Le => {
                            let ord = compare_values(&lv, &rv);
                            Value::Num(if ord != std::cmp::Ordering::Greater {
                                1.0
                            } else {
                                0.0
                            })
                        }
                        BinOp::Ge => {
                            let ord = compare_values(&lv, &rv);
                            Value::Num(if ord != std::cmp::Ordering::Less {
                                1.0
                            } else {
                                0.0
                            })
                        }
                        BinOp::And | BinOp::Or => unreachable!(),
                    }
                }
            },
            Expr::Unop(op, operand) => match op {
                UnOp::Neg => {
                    let v = self.eval_expr(operand);
                    Value::Num(-v.to_num())
                }
                UnOp::Not => {
                    let v = self.eval_expr(operand);
                    Value::Num(if v.to_bool() { 0.0 } else { 1.0 })
                }
                UnOp::PreIncrement => {
                    let v = self.eval_expr(operand).to_num() + 1.0;
                    let new_val = Value::Num(v);
                    self.assign_to(operand, new_val.clone());
                    new_val
                }
                UnOp::PreDecrement => {
                    let v = self.eval_expr(operand).to_num() - 1.0;
                    let new_val = Value::Num(v);
                    self.assign_to(operand, new_val.clone());
                    new_val
                }
            },
            Expr::PostIncrement(operand) => {
                let v = self.eval_expr(operand).to_num();
                self.assign_to(operand, Value::Num(v + 1.0));
                Value::Num(v)
            }
            Expr::PostDecrement(operand) => {
                let v = self.eval_expr(operand).to_num();
                self.assign_to(operand, Value::Num(v - 1.0));
                Value::Num(v)
            }
            Expr::Assign(lhs, rhs) => {
                let val = self.eval_expr(rhs);
                self.assign_to(lhs, val.clone());
                val
            }
            Expr::OpAssign(lhs, op, rhs) => {
                let lv = self.eval_expr(lhs).to_num();
                let rv = self.eval_expr(rhs).to_num();
                let result = match op {
                    BinOp::Add => lv + rv,
                    BinOp::Sub => lv - rv,
                    BinOp::Mul => lv * rv,
                    BinOp::Div => {
                        if rv == 0.0 {
                            eprintln!("awk: division by zero");
                            0.0
                        } else {
                            lv / rv
                        }
                    }
                    BinOp::Mod => {
                        if rv == 0.0 {
                            eprintln!("awk: division by zero");
                            0.0
                        } else {
                            lv % rv
                        }
                    }
                    BinOp::Pow => lv.powf(rv),
                    _ => lv,
                };
                let val = Value::Num(result);
                self.assign_to(lhs, val.clone());
                val
            }
            Expr::Match(left, right) => {
                let s = self.eval_expr(left).to_string_val();
                let pattern = if let Some(r) = self.extract_regex_pattern(right) {
                    r
                } else {
                    self.eval_expr(right).to_string_val()
                };
                match Self::compile_regex(&pattern) {
                    Some(re) => Value::Num(if re.is_match(&s) { 1.0 } else { 0.0 }),
                    None => Value::Num(0.0),
                }
            }
            Expr::NotMatch(left, right) => {
                let s = self.eval_expr(left).to_string_val();
                let pattern = if let Some(r) = self.extract_regex_pattern(right) {
                    r
                } else {
                    self.eval_expr(right).to_string_val()
                };
                match Self::compile_regex(&pattern) {
                    Some(re) => Value::Num(if re.is_match(&s) { 0.0 } else { 1.0 }),
                    None => Value::Num(1.0),
                }
            }
            Expr::Ternary(cond, then_expr, else_expr) => {
                let val = self.eval_expr(cond);
                if val.to_bool() {
                    self.eval_expr(then_expr)
                } else {
                    self.eval_expr(else_expr)
                }
            }
            Expr::Concat(left, right) => {
                let convfmt = self
                    .globals
                    .get("CONVFMT")
                    .map(|v| v.to_string_val())
                    .unwrap_or("%.6g".to_string());
                let lv = self.eval_expr(left);
                let rv = self.eval_expr(right);
                let ls = lv.to_string_with_fmt(&convfmt);
                let rs = rv.to_string_with_fmt(&convfmt);
                Value::Str(format!("{ls}{rs}"))
            }
            Expr::In(expr, array) => {
                let key = self.eval_expr(expr).to_string_val();
                let exists = self.arrays.get(array).is_some_and(|a| a.contains_key(&key));
                Value::Num(if exists { 1.0 } else { 0.0 })
            }
            Expr::MultiIn(indices, array) => {
                let vals: Vec<Value> = indices.iter().map(|i| self.eval_expr(i)).collect();
                let key = self.array_key(&vals);
                let exists = self.arrays.get(array).is_some_and(|a| a.contains_key(&key));
                Value::Num(if exists { 1.0 } else { 0.0 })
            }
            Expr::FuncCall(name, args) => self.call_function(name, args),
            Expr::Getline(var, file, source) => self.eval_getline(
                var.as_ref().map(|e| e.as_ref()),
                file.as_ref().map(|e| e.as_ref()),
                source,
            ),
            Expr::Sprintf(args) => {
                let vals: Vec<Value> = args.iter().map(|a| self.eval_expr(a)).collect();
                Value::Str(sprintf_impl(&vals))
            }
            Expr::Pipe(cmd, getline_expr) => {
                // cmd | getline [var]
                let cmd_str = self.eval_expr(cmd).to_string_val();
                let var = match getline_expr.as_ref() {
                    Expr::Getline(v, _, _) => v.as_ref().map(|e| e.as_ref()),
                    _ => None,
                };
                let cmd_expr = Expr::StringLit(cmd_str);
                self.eval_getline(var, Some(&cmd_expr), &GetlineSource::Pipe)
            }
        }
    }

    fn call_function(&mut self, name: &str, args: &[Expr]) -> Value {
        // Built-in functions
        match name {
            "length" => {
                if args.is_empty() {
                    return Value::Num(self.get_field(0).to_string_val().len() as f64);
                }
                // Check if argument is an array name
                if let Some(Expr::Var(arr_name)) = args.first()
                    && self.arrays.contains_key(arr_name)
                {
                    return Value::Num(self.arrays[arr_name].len() as f64);
                }
                let v = self.eval_expr(&args[0]);
                Value::Num(v.to_string_val().len() as f64)
            }
            "substr" => {
                if args.len() < 2 {
                    return Value::Str(String::new());
                }
                let s = self.eval_expr(&args[0]).to_string_val();
                let start = (self.eval_expr(&args[1]).to_num() as i64).max(1) as usize;
                let chars: Vec<char> = s.chars().collect();
                if start > chars.len() {
                    return Value::Str(String::new());
                }
                let start_idx = start - 1;
                if args.len() >= 3 {
                    let len = self.eval_expr(&args[2]).to_num().max(0.0) as usize;
                    let end = (start_idx + len).min(chars.len());
                    Value::Str(chars[start_idx..end].iter().collect())
                } else {
                    Value::Str(chars[start_idx..].iter().collect())
                }
            }
            "index" => {
                if args.len() < 2 {
                    return Value::Num(0.0);
                }
                let s = self.eval_expr(&args[0]).to_string_val();
                let t = self.eval_expr(&args[1]).to_string_val();
                match s.find(&t) {
                    Some(pos) => {
                        // Convert byte offset to char position
                        let char_pos = s[..pos].chars().count() + 1;
                        Value::Num(char_pos as f64)
                    }
                    None => Value::Num(0.0),
                }
            }
            "split" => {
                if args.len() < 2 {
                    return Value::Num(0.0);
                }
                let s = self.eval_expr(&args[0]).to_string_val();
                let arr_name = match &args[1] {
                    Expr::Var(n) => n.clone(),
                    Expr::ArrayRef(n, _) => n.clone(),
                    _ => return Value::Num(0.0),
                };
                // Check if third arg is a regex literal
                let (fs, is_regex) = if args.len() >= 3 {
                    if let Some(r) = self.extract_regex_pattern(&args[2]) {
                        (r, true)
                    } else {
                        (self.eval_expr(&args[2]).to_string_val(), false)
                    }
                } else {
                    (
                        self.globals
                            .get("FS")
                            .map(|v| v.to_string_val())
                            .unwrap_or(" ".to_string()),
                        false,
                    )
                };

                // Clear the array
                self.arrays.remove(&arr_name);

                let parts: Vec<String> = if !is_regex && fs == " " {
                    s.split_whitespace().map(|p| p.to_string()).collect()
                } else if !is_regex && fs.len() == 1 {
                    s.split(fs.chars().next().unwrap())
                        .map(|p| p.to_string())
                        .collect()
                } else {
                    let fixed_fs = Self::fix_awk_regex(&fs);
                    match Regex::new(&fixed_fs) {
                        Ok(re) => {
                            let mut parts: Vec<String> =
                                re.split(&s).map(|p| p.to_string()).collect();
                            // Remove leading/trailing empty strings from anchor matches
                            if parts.first().is_some_and(|p| p.is_empty()) && parts.len() > 1 {
                                parts.remove(0);
                            }
                            if parts.last().is_some_and(|p| p.is_empty()) && parts.len() > 1 {
                                parts.pop();
                            }
                            parts
                        }
                        Err(_) => s.split(&fs).map(|p| p.to_string()).collect(),
                    }
                };

                let count = parts.len();
                for (i, part) in parts.into_iter().enumerate() {
                    self.set_array(&arr_name, &(i + 1).to_string(), Value::StrNum(part));
                }
                Value::Num(count as f64)
            }
            "sub" | "gsub" => {
                if args.len() < 2 {
                    return Value::Num(0.0);
                }
                let pattern = if let Some(r) = self.extract_regex_pattern(&args[0]) {
                    r
                } else {
                    // String patterns are used as regex in sub/gsub (not escaped)
                    self.eval_expr(&args[0]).to_string_val()
                };
                let replacement = self.eval_expr(&args[1]).to_string_val();

                // Target: either specified or $0
                let target_expr = if args.len() >= 3 {
                    args[2].clone()
                } else {
                    Expr::FieldRef(Box::new(Expr::Number(0.0)))
                };

                let target_val = self.eval_expr(&target_expr).to_string_val();
                let is_global = name == "gsub";

                match Self::compile_regex(&pattern) {
                    Some(re) => {
                        let mut count = 0;
                        let result = if is_global {
                            let r = re.replace_all(&target_val, |caps: &regex::Captures| {
                                count += 1;
                                awk_replace(&replacement, &caps[0])
                            });
                            r.to_string()
                        } else {
                            let r = re.replace(&target_val, |caps: &regex::Captures| {
                                count += 1;
                                awk_replace(&replacement, &caps[0])
                            });
                            r.to_string()
                        };
                        // Only assign back if replacements were made
                        if count > 0 {
                            self.assign_to(&target_expr, Value::Str(result));
                        }
                        Value::Num(count as f64)
                    }
                    None => Value::Num(0.0),
                }
            }
            "gensub" => {
                if args.len() < 3 {
                    return Value::Str(String::new());
                }
                let pattern = if let Some(r) = self.extract_regex_pattern(&args[0]) {
                    r
                } else {
                    self.eval_expr(&args[0]).to_string_val()
                };
                let replacement = self.eval_expr(&args[1]).to_string_val();
                let how = self.eval_expr(&args[2]).to_string_val();

                let target = if args.len() >= 4 {
                    self.eval_expr(&args[3]).to_string_val()
                } else {
                    self.get_field(0).to_string_val()
                };

                let is_global = how == "g" || how == "G";

                match Self::compile_regex(&pattern) {
                    Some(re) => {
                        let result = if is_global {
                            re.replace_all(&target, |caps: &regex::Captures| {
                                gensub_replace(&replacement, caps)
                            })
                            .to_string()
                        } else {
                            let n = how.parse::<usize>().unwrap_or(1);
                            let mut count = 0;
                            re.replace_all(&target, |caps: &regex::Captures| {
                                count += 1;
                                if count == n {
                                    gensub_replace(&replacement, caps)
                                } else {
                                    caps[0].to_string()
                                }
                            })
                            .to_string()
                        };
                        Value::Str(result)
                    }
                    None => Value::Str(target),
                }
            }
            "match" => {
                if args.len() < 2 {
                    return Value::Num(0.0);
                }
                let s = self.eval_expr(&args[0]).to_string_val();
                let pattern = if let Some(r) = self.extract_regex_pattern(&args[1]) {
                    r
                } else {
                    self.eval_expr(&args[1]).to_string_val()
                };
                match Self::compile_regex(&pattern) {
                    Some(re) => {
                        if let Some(m) = re.find(&s) {
                            let start = s[..m.start()].chars().count() + 1;
                            let length = m.as_str().chars().count();
                            self.set_var("RSTART", Value::Num(start as f64));
                            self.set_var("RLENGTH", Value::Num(length as f64));
                            Value::Num(start as f64)
                        } else {
                            self.set_var("RSTART", Value::Num(0.0));
                            self.set_var("RLENGTH", Value::Num(-1.0));
                            Value::Num(0.0)
                        }
                    }
                    None => {
                        self.set_var("RSTART", Value::Num(0.0));
                        self.set_var("RLENGTH", Value::Num(-1.0));
                        Value::Num(0.0)
                    }
                }
            }
            "sprintf" => {
                let vals: Vec<Value> = args.iter().map(|a| self.eval_expr(a)).collect();
                Value::Str(sprintf_impl(&vals))
            }
            "tolower" => {
                if args.is_empty() {
                    return Value::Str(String::new());
                }
                let s = self.eval_expr(&args[0]).to_string_val();
                Value::Str(s.to_lowercase())
            }
            "toupper" => {
                if args.is_empty() {
                    return Value::Str(String::new());
                }
                let s = self.eval_expr(&args[0]).to_string_val();
                Value::Str(s.to_uppercase())
            }
            // Math functions
            "sin" => {
                let n = if args.is_empty() {
                    0.0
                } else {
                    self.eval_expr(&args[0]).to_num()
                };
                Value::Num(n.sin())
            }
            "cos" => {
                let n = if args.is_empty() {
                    0.0
                } else {
                    self.eval_expr(&args[0]).to_num()
                };
                Value::Num(n.cos())
            }
            "atan2" => {
                if args.len() < 2 {
                    return Value::Num(0.0);
                }
                let y = self.eval_expr(&args[0]).to_num();
                let x = self.eval_expr(&args[1]).to_num();
                Value::Num(y.atan2(x))
            }
            "exp" => {
                let n = if args.is_empty() {
                    0.0
                } else {
                    self.eval_expr(&args[0]).to_num()
                };
                Value::Num(n.exp())
            }
            "log" => {
                let n = if args.is_empty() {
                    0.0
                } else {
                    self.eval_expr(&args[0]).to_num()
                };
                Value::Num(n.ln())
            }
            "sqrt" => {
                let n = if args.is_empty() {
                    0.0
                } else {
                    self.eval_expr(&args[0]).to_num()
                };
                Value::Num(n.sqrt())
            }
            "int" => {
                let n = if args.is_empty() {
                    0.0
                } else {
                    self.eval_expr(&args[0]).to_num()
                };
                Value::Num(n.trunc())
            }
            "rand" => {
                // Simple LCG random
                self.rng_state = self
                    .rng_state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let val = (self.rng_state >> 33) as f64 / (1u64 << 31) as f64;
                Value::Num(val)
            }
            "srand" => {
                let old = self.rng_state;
                if args.is_empty() {
                    self.rng_state = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos() as u64)
                        .unwrap_or(0);
                } else {
                    self.rng_state = self.eval_expr(&args[0]).to_num() as u64;
                }
                Value::Num(old as f64)
            }
            "system" => {
                if args.is_empty() {
                    return Value::Num(0.0);
                }
                let cmd = self.eval_expr(&args[0]).to_string_val();
                // Flush stdout before running system command
                io::stdout().flush().ok();
                match Command::new("sh").arg("-c").arg(&cmd).status() {
                    Ok(status) => Value::Num(status.code().unwrap_or(-1) as f64),
                    Err(_) => Value::Num(-1.0),
                }
            }
            "close" => {
                if args.is_empty() {
                    return Value::Num(-1.0);
                }
                let name = self.eval_expr(&args[0]).to_string_val();
                let mut found = false;
                if self.open_files.remove(&name).is_some() {
                    found = true;
                }
                // Drop pipe handles first so child can finish
                if self.open_pipes.remove(&name).is_some() {
                    found = true;
                }
                if self.open_read_pipes.remove(&name).is_some() {
                    found = true;
                }
                if self.open_read_files.remove(&name).is_some() {
                    found = true;
                }
                // Wait for child process and get exit status
                if let Some(mut child) = self.pipe_children.remove(&name) {
                    if let Ok(status) = child.wait() {
                        let code = status.code().unwrap_or(-1);
                        return Value::Num(code as f64);
                    }
                }
                Value::Num(if found { 0.0 } else { -1.0 })
            }
            "mktime" | "systime" => {
                // systime returns current epoch
                if name == "systime" {
                    let epoch = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    return Value::Num(epoch as f64);
                }
                Value::Num(0.0)
            }
            "strftime" => {
                // Minimal strftime
                Value::Str(String::new())
            }
            "typeof" => {
                if args.is_empty() {
                    return Value::Str("uninitialized".to_string());
                }
                // Check if arg is an array name
                if let Expr::Var(name) = &args[0] {
                    if self.arrays.contains_key(name) {
                        return Value::Str("array".to_string());
                    }
                }
                let val = self.eval_expr(&args[0]);
                let t = match &val {
                    Value::Num(_) => "number",
                    Value::Str(_) => "string",
                    Value::StrNum(_) => "strnum",
                    Value::Uninitialized => "uninitialized",
                };
                Value::Str(t.to_string())
            }
            "asort" | "asorti" => {
                if args.is_empty() {
                    return Value::Num(0.0);
                }
                let arr_name = match &args[0] {
                    Expr::Var(n) => n.clone(),
                    _ => return Value::Num(0.0),
                };
                let dest_name = if args.len() >= 2 {
                    match &args[1] {
                        Expr::Var(n) => n.clone(),
                        _ => arr_name.clone(),
                    }
                } else {
                    arr_name.clone()
                };

                let arr = match self.arrays.get(&arr_name) {
                    Some(a) => a.clone(),
                    None => return Value::Num(0.0),
                };

                let mut items: Vec<(String, Value)> = arr.into_iter().collect();
                if name == "asort" {
                    items.sort_by(|a, b| {
                        let sa = a.1.to_string_val();
                        let sb = b.1.to_string_val();
                        sa.cmp(&sb)
                    });
                } else {
                    // asorti: sort by indices
                    items.sort_by(|a, b| a.0.cmp(&b.0));
                }

                let count = items.len();
                let mut new_arr = HashMap::new();
                for (i, (key, val)) in items.into_iter().enumerate() {
                    if name == "asort" {
                        new_arr.insert((i + 1).to_string(), val);
                    } else {
                        new_arr.insert((i + 1).to_string(), Value::Str(key));
                    }
                }
                self.arrays.insert(dest_name, new_arr);
                Value::Num(count as f64)
            }
            _ => {
                // User-defined function
                let func = self.functions.get(name).cloned();
                if let Some(func) = func {
                    // Save and set up local scope
                    let mut saved_vars: Vec<(String, Option<Value>)> = Vec::new();
                    let mut saved_arrays: Vec<(String, Option<HashMap<String, Value>>)> =
                        Vec::new();

                    // Collect argument info: (value, variable_name for array pass-by-ref)
                    let mut arg_vals: Vec<Value> = Vec::new();
                    let mut arg_var_names: Vec<Option<String>> = Vec::new();
                    for arg in args {
                        if let Expr::Var(var_name) = arg {
                            if self.arrays.contains_key(var_name) {
                                // Pass existing array by reference
                                arg_vals.push(Value::Uninitialized);
                            } else {
                                arg_vals.push(self.eval_expr(arg));
                            }
                            arg_var_names.push(Some(var_name.clone()));
                        } else {
                            arg_vals.push(self.eval_expr(arg));
                            arg_var_names.push(None);
                        }
                    }

                    for (i, param) in func.params.iter().enumerate() {
                        // Save old value
                        saved_vars.push((param.clone(), self.globals.remove(param)));
                        saved_arrays.push((param.clone(), self.arrays.remove(param)));

                        if i < arg_vals.len() {
                            // Check if this arg is a variable with an array
                            if let Some(Some(var_name)) = arg_var_names.get(i) {
                                if let Some(arr) = self.arrays.get(var_name).cloned() {
                                    // Pass array by reference
                                    self.arrays.insert(param.clone(), arr);
                                    continue;
                                }
                            }
                            self.set_var(param, arg_vals[i].clone());
                        } else {
                            // Extra params are local variables, initialized to 0/""
                            self.set_var(param, Value::Uninitialized);
                        }
                    }

                    let result = match self.exec_stmts(&func.body) {
                        ControlFlow::Return(val) => val,
                        _ => Value::Uninitialized,
                    };

                    // Copy back arrays to original variable names (pass-by-reference)
                    for (i, _arg) in args.iter().enumerate() {
                        if let Some(Some(orig_name)) = arg_var_names.get(i) {
                            if let Some(param) = func.params.get(i) {
                                if let Some(arr) = self.arrays.get(param).cloned() {
                                    self.arrays.insert(orig_name.clone(), arr);
                                } else {
                                    // Function may have deleted the array
                                    self.arrays.remove(orig_name);
                                }
                            }
                        }
                    }

                    // Restore scope
                    for (name, old_val) in saved_vars {
                        self.globals.remove(&name);
                        if let Some(val) = old_val {
                            self.globals.insert(name, val);
                        }
                    }
                    for (name, old_arr) in saved_arrays {
                        self.arrays.remove(&name);
                        if let Some(arr) = old_arr {
                            self.arrays.insert(name, arr);
                        }
                    }

                    result
                } else {
                    eprintln!("awk: unknown function {name}");
                    Value::Uninitialized
                }
            }
        }
    }
}
