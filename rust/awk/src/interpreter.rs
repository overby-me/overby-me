use regex::Regex;
use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use crate::ast::*;
use crate::format::{awk_replace, gensub_replace, sprintf_impl_with_convfmt};
use crate::value::{ControlFlow, Value, compare_values};

/// Port of gawk's bundled BSD random() (support/random.c). Produces the same
/// sequence gawk does for a given `srand(seed)`, so awk programs whose output
/// depends on `rand()` match bit-for-bit. TYPE_4 only (the variant gawk uses
/// via its 256-byte state buffer).
struct GawkRng {
    state: Vec<u32>, // size = DEG + 1; `state[0..DEG]` is active, extra slot is unused but keeps indexing with end_ptr simple
    fptr: usize,
    rptr: usize,
    end: usize,
    shuffle_buffer: Vec<i64>,
    shuffle_init: bool,
    shuffle_s: i64,
}

impl GawkRng {
    const DEG: usize = 63;
    const SEP: usize = 1;
    const SHUFFLE_MAX: usize = 512;
    const SHUFFLE_MASK: i64 = 511;

    fn new() -> Self {
        let mut rng = GawkRng {
            state: vec![0u32; Self::DEG + 1],
            fptr: Self::SEP,
            rptr: 0,
            end: Self::DEG,
            shuffle_buffer: vec![0i64; Self::SHUFFLE_MAX],
            shuffle_init: true,
            shuffle_s: 0xcafefeed_i64,
        };
        // gawk initializes with srand(1) on first use.
        rng.srand(1);
        rng
    }

    fn good_rand(x: i32) -> u32 {
        let mut x = if x == 0 { 123_459_876 } else { x };
        let hi = x / 127_773;
        let lo = x % 127_773;
        x = 16807_i32.wrapping_mul(lo).wrapping_sub(2836_i32.wrapping_mul(hi));
        if x < 0 {
            x = x.wrapping_add(0x7fff_ffff);
        }
        x as u32
    }

    fn random_old(&mut self) -> i64 {
        self.state[self.fptr] = self.state[self.fptr].wrapping_add(self.state[self.rptr]);
        let i = ((self.state[self.fptr] >> 1) & 0x7fff_ffff) as i64;
        self.fptr += 1;
        if self.fptr >= self.end {
            self.fptr = 0;
            self.rptr += 1;
        } else {
            self.rptr += 1;
            if self.rptr >= self.end {
                self.rptr = 0;
            }
        }
        i
    }

    fn random(&mut self) -> i64 {
        if self.shuffle_init {
            for k in 0..Self::SHUFFLE_MAX {
                self.shuffle_buffer[k] = self.random_old();
            }
            self.shuffle_s = self.random_old();
            self.shuffle_init = false;
        }
        let r = self.random_old();
        let k = (self.shuffle_s & Self::SHUFFLE_MASK) as usize;
        let result = self.shuffle_buffer[k];
        self.shuffle_buffer[k] = r;
        self.shuffle_s = result;
        result
    }

    fn srand(&mut self, seed: u64) {
        self.shuffle_init = true;
        self.state[0] = seed as u32;
        for i in 1..Self::DEG {
            self.state[i] = Self::good_rand(self.state[i - 1] as i32);
        }
        self.fptr = Self::SEP;
        self.rptr = 0;
        let lim = 10 * Self::DEG;
        for _ in 0..lim {
            let _ = self.random();
        }
    }

    /// Draws a number in `[0, 1)` the same way gawk's `rand()` does: two
    /// successive `random()` values combined to fill more mantissa bits.
    fn next_f64(&mut self) -> f64 {
        const DIVISOR: f64 = 2_147_483_648.0; // 2^31
        loop {
            let d1 = self.random() as f64;
            let d2 = self.random() as f64;
            let mut t = 0.5 + (d1 / DIVISOR + d2) / DIVISOR;
            t -= 0.5;
            if t != 1.0 {
                return t;
            }
        }
    }
}

/// Write an awk string to a byte sink. Each Unicode scalar U+0000..=U+00FF is
/// emitted as a single byte so data that entered the interpreter as raw bytes
/// (via [`bytes_to_string`]) round-trips byte-for-byte. Higher code points are
/// emitted as UTF-8 so intentionally-Unicode strings still render correctly.
fn write_awk<W: Write>(w: &mut W, s: &str) -> io::Result<()> {
    for c in s.chars() {
        let code = c as u32;
        if code < 0x100 {
            w.write_all(&[code as u8])?;
        } else {
            let mut buf = [0u8; 4];
            let encoded = c.encode_utf8(&mut buf);
            w.write_all(encoded.as_bytes())?;
        }
    }
    Ok(())
}

/// Decode raw input bytes to a Rust string using Latin-1 mapping (byte `b` →
/// char `U+00xx`). This is the inverse of [`write_awk`] and ensures that
/// non-UTF-8 sources and input can flow through the interpreter without loss.
pub fn bytes_to_string(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

struct PipeRecordState {
    records: Vec<String>,
    terminators: Vec<String>,
    pos: usize,
}

pub struct Interpreter {
    pub globals: HashMap<String, Value>,
    pub arrays: HashMap<String, HashMap<String, Value>>,
    pub fields: Vec<Value>,
    pub functions: HashMap<String, FuncDef>,
    open_files: HashMap<String, Box<dyn Write>>,
    open_read_files: HashMap<String, Box<dyn BufRead>>,
    open_pipes: HashMap<String, Box<dyn Write>>,
    open_read_pipes: HashMap<String, Box<dyn BufRead>>,
    /// Pipe output that's been fully read and split by RS. Keyed by the
    /// command string. Used for `cmd | getline` when RS isn't "\n", since
    /// line-at-a-time reading can't honor arbitrary record separators.
    pipe_records: HashMap<String, PipeRecordState>,
    /// Child processes for pipes, keyed by command string
    pipe_children: HashMap<String, std::process::Child>,
    pub rng_state: u64,
    /// gawk-compatible RNG (port of `support/random.c`). Used so rand/srand
    /// produce the same sequence as gawk for a given seed. Lazily
    /// initialized on first use.
    rng: GawkRng,
    range_active: HashMap<usize, bool>,
    pub nr: i64,
    pub fnr: i64,
    pub exit_code: i32,
    /// Lines from current input stream for bare getline
    input_lines: Vec<String>,
    /// Record terminators matched while splitting input. One entry per record
    /// in `input_lines` (may be empty if no RS match followed the last
    /// record). Used to populate the `RT` variable.
    input_terminators: Vec<String>,
    /// Current position in input_lines (next line to process)
    input_line_idx: usize,
    /// Currently active function parameter names (for error messages)
    current_params: Vec<String>,
    /// Parameters that have been used as scalars in current function call
    scalar_params: std::collections::HashSet<String>,
    /// Parameters that have been used as arrays in current function call
    array_params: std::collections::HashSet<String>,
    /// Map from parameter name to origin variable name (for error provenance)
    param_origins: HashMap<String, String>,
    /// Globals that were passed to functions and used as scalars (for post-call type checking)
    global_scalar_via_func: std::collections::HashSet<String>,
    /// Array reference aliases for the current function frame. Maps a local
    /// array parameter name to the canonical (caller-side) array name that it
    /// shares storage with. Enables awk's reference semantics for arrays
    /// passed as function arguments — multiple params sharing the same caller
    /// array all see each other's writes.
    array_aliases: HashMap<String, String>,
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
        globals.insert("RT".to_string(), Value::Str(String::new()));

        Interpreter {
            globals,
            arrays: HashMap::new(),
            fields: vec![Value::Str(String::new())],
            functions: HashMap::new(),
            open_files: HashMap::new(),
            open_read_files: HashMap::new(),
            open_pipes: HashMap::new(),
            open_read_pipes: HashMap::new(),
            pipe_records: HashMap::new(),
            rng_state: 0,
            rng: GawkRng::new(),
            range_active: HashMap::new(),
            nr: 0,
            fnr: 0,
            exit_code: 0,
            pipe_children: HashMap::new(),
            input_lines: Vec::new(),
            input_terminators: Vec::new(),
            input_line_idx: 0,
            current_params: Vec::new(),
            scalar_params: std::collections::HashSet::new(),
            array_params: std::collections::HashSet::new(),
            param_origins: HashMap::new(),
            global_scalar_via_func: std::collections::HashSet::new(),
            array_aliases: HashMap::new(),
        }
    }

    /// Resolve an array name through the current frame's alias map. Returns
    /// the canonical name to use for actual storage access.
    fn resolve_array_name(&self, name: &str) -> String {
        self.array_aliases
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_string())
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
        self.fields.extend(parts.into_iter().map(Value::StrNum));
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
                let nf_val = val.to_num();
                if nf_val < 0.0 {
                    eprintln!("awk: fatal: NF set to negative value");
                    std::process::exit(2);
                }
                let nf = nf_val as usize;
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
        let resolved = self.resolve_array_name(name);
        self.arrays
            .get(&resolved)
            .and_then(|a| a.get(key))
            .cloned()
            .unwrap_or(Value::Uninitialized)
    }

    pub fn set_array(&mut self, name: &str, key: &str, val: Value) {
        let resolved = self.resolve_array_name(name);
        self.arrays
            .entry(resolved)
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

    fn check_divzero(expr: &Expr) {
        match expr {
            Expr::Binop(left, BinOp::Div, right) | Expr::Binop(left, BinOp::Mod, right) => {
                if matches!(right.as_ref(), Expr::Number(n) if *n == 0.0) {
                    eprintln!("awk: error: division by zero attempted");
                    std::process::exit(1);
                }
                Self::check_divzero(left);
                Self::check_divzero(right);
            }
            Expr::Binop(left, _, right) => {
                Self::check_divzero(left);
                Self::check_divzero(right);
            }
            Expr::Unop(_, operand) => Self::check_divzero(operand),
            Expr::Assign(lhs, rhs) | Expr::OpAssign(lhs, _, rhs) => {
                Self::check_divzero(lhs);
                Self::check_divzero(rhs);
            }
            Expr::FuncCall(_, args) => {
                for a in args {
                    Self::check_divzero(a);
                }
            }
            Expr::Ternary(c, t, f) => {
                Self::check_divzero(c);
                Self::check_divzero(t);
                Self::check_divzero(f);
            }
            Expr::Match(l, r) | Expr::NotMatch(l, r) => {
                Self::check_divzero(l);
                Self::check_divzero(r);
            }
            Expr::FieldRef(e)
            | Expr::PostIncrement(e)
            | Expr::PostDecrement(e)
            | Expr::In(e, _) => Self::check_divzero(e),
            Expr::ArrayRef(_, indices) => {
                for idx in indices {
                    Self::check_divzero(idx);
                }
            }
            Expr::Getline(var, file, _) => {
                if let Some(v) = var {
                    Self::check_divzero(v);
                }
                if let Some(f) = file {
                    Self::check_divzero(f);
                }
            }
            _ => {}
        }
    }

    fn check_divzero_stmts(stmts: &[Stmt]) {
        for stmt in stmts {
            Self::check_divzero_stmt(stmt);
        }
    }

    fn check_divzero_stmt(stmt: &Stmt) {
        match stmt {
            Stmt::Expr(e) | Stmt::Exit(Some(e)) | Stmt::Return(Some(e)) => {
                Self::check_divzero(e);
            }
            Stmt::Print(args, _) | Stmt::Printf(args, _) => {
                for a in args {
                    Self::check_divzero(a);
                }
            }
            Stmt::If(cond, then_body, else_body) => {
                Self::check_divzero(cond);
                Self::check_divzero_stmt(then_body.as_ref());
                if let Some(eb) = else_body {
                    Self::check_divzero_stmt(eb.as_ref());
                }
            }
            Stmt::While(cond, body) | Stmt::DoWhile(body, cond) => {
                Self::check_divzero(cond);
                Self::check_divzero_stmt(body.as_ref());
            }
            Stmt::For(init, cond, update, body) => {
                if let Some(i) = init {
                    Self::check_divzero_stmt(i);
                }
                if let Some(c) = cond {
                    Self::check_divzero(c);
                }
                if let Some(u) = update {
                    Self::check_divzero_stmt(u);
                }
                Self::check_divzero_stmt(body.as_ref());
            }
            Stmt::ForIn(_, _, body) => Self::check_divzero_stmt(body.as_ref()),
            Stmt::Block(stmts) => Self::check_divzero_stmts(stmts),
            Stmt::Getline(var, file, _) => {
                if let Some(v) = var {
                    Self::check_divzero(v);
                }
                if let Some(f) = file {
                    Self::check_divzero(f);
                }
            }
            _ => {}
        }
    }

    fn check_func_as_var(
        stmts: &[Stmt],
        func_names: &std::collections::HashSet<String>,
        had_error: &mut bool,
    ) {
        for stmt in stmts {
            Self::check_func_as_var_stmt(stmt, func_names, had_error);
        }
    }

    fn check_func_as_var_stmt(
        stmt: &Stmt,
        func_names: &std::collections::HashSet<String>,
        had_error: &mut bool,
    ) {
        match stmt {
            Stmt::Expr(e) | Stmt::Exit(Some(e)) | Stmt::Return(Some(e)) => {
                Self::check_func_as_var_expr(e, func_names, had_error);
            }
            Stmt::Print(args, _) | Stmt::Printf(args, _) => {
                for a in args {
                    Self::check_func_as_var_expr(a, func_names, had_error);
                }
            }
            Stmt::If(cond, then_b, else_b) => {
                Self::check_func_as_var_expr(cond, func_names, had_error);
                Self::check_func_as_var_stmt(then_b, func_names, had_error);
                if let Some(eb) = else_b {
                    Self::check_func_as_var_stmt(eb, func_names, had_error);
                }
            }
            Stmt::While(cond, body) | Stmt::DoWhile(body, cond) => {
                Self::check_func_as_var_expr(cond, func_names, had_error);
                Self::check_func_as_var_stmt(body, func_names, had_error);
            }
            Stmt::For(init, cond, update, body) => {
                if let Some(i) = init {
                    Self::check_func_as_var_stmt(i, func_names, had_error);
                }
                if let Some(c) = cond {
                    Self::check_func_as_var_expr(c, func_names, had_error);
                }
                if let Some(u) = update {
                    Self::check_func_as_var_stmt(u, func_names, had_error);
                }
                Self::check_func_as_var_stmt(body, func_names, had_error);
            }
            Stmt::Block(stmts) => {
                Self::check_func_as_var(stmts, func_names, had_error);
            }
            Stmt::Delete(_, _) => {}
            _ => {}
        }
    }

    fn check_func_as_var_expr(
        expr: &Expr,
        func_names: &std::collections::HashSet<String>,
        had_error: &mut bool,
    ) {
        match expr {
            // Check gsub/sub with function name as third arg
            Expr::FuncCall(name, args) if (name == "gsub" || name == "sub") && args.len() >= 3 => {
                if let Expr::Var(tname) = &args[2]
                    && func_names.contains(tname)
                {
                    eprintln!(
                        "awk: error: function `{tname}' called with space between name and `(',"
                    );
                    eprintln!("or used as a variable or an array");
                    *had_error = true;
                }
                for a in args {
                    Self::check_func_as_var_expr(a, func_names, had_error);
                }
            }
            Expr::FuncCall(_, args) => {
                for a in args {
                    Self::check_func_as_var_expr(a, func_names, had_error);
                }
            }
            Expr::Binop(l, _, r) | Expr::Assign(l, r) | Expr::OpAssign(l, _, r) => {
                Self::check_func_as_var_expr(l, func_names, had_error);
                Self::check_func_as_var_expr(r, func_names, had_error);
            }
            Expr::Unop(_, e)
            | Expr::FieldRef(e)
            | Expr::PostIncrement(e)
            | Expr::PostDecrement(e) => {
                Self::check_func_as_var_expr(e, func_names, had_error);
            }
            Expr::Ternary(c, t, f) => {
                Self::check_func_as_var_expr(c, func_names, had_error);
                Self::check_func_as_var_expr(t, func_names, had_error);
                Self::check_func_as_var_expr(f, func_names, had_error);
            }
            _ => {}
        }
    }

    pub fn run(&mut self, program: &Program, files: &[String]) {
        // Check for constant division by zero (like gawk does at compile time)
        for rule in &program.rules {
            Self::check_divzero_stmts(&rule.action);
        }
        for func in &program.functions {
            Self::check_divzero_stmts(&func.body);
        }

        // Register functions
        for func in &program.functions {
            self.functions.insert(func.name.clone(), func.clone());
        }

        // Check for function-name-as-variable errors (pre-execution, like gawk)
        let func_names: std::collections::HashSet<String> =
            program.functions.iter().map(|f| f.name.clone()).collect();
        let mut had_func_var_error = false;
        for func in &program.functions {
            Self::check_func_as_var(&func.body, &func_names, &mut had_func_var_error);
        }
        if had_func_var_error {
            std::process::exit(1);
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

        // Check if there are any non-BEGIN/END rules that need input
        let has_input_rules = program
            .rules
            .iter()
            .any(|r| !matches!(r.pattern, Some(Pattern::Begin) | Some(Pattern::End)));
        let has_end_rules = program
            .rules
            .iter()
            .any(|r| matches!(r.pattern, Some(Pattern::End)));

        // Process input — also needed for END rules (to set NR, $0, etc.)
        if files.is_empty() && (has_input_rules || has_end_rules) {
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

        // Close all open pipes and files to flush output
        self.open_pipes.clear();
        self.open_files.clear();
        self.open_read_pipes.clear();
        self.open_read_files.clear();
        // Wait for all child processes
        for (_, mut child) in self.pipe_children.drain() {
            child.wait().ok();
        }
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

    /// Split an input buffer into records and matching terminators according
    /// to the current value of the awk RS variable. Mirrors gawk semantics:
    /// `"\n"` splits on LF (stripping CR), single-char RS splits on that
    /// character, empty RS is paragraph mode (splits on blank lines, strips
    /// leading newlines), longer RS is treated as a regex.
    fn split_by_rs(all: &str, rs: &str) -> (Vec<String>, Vec<String>) {
        let mut pairs: Vec<(String, String)> = Vec::new();
        let mut all_buf = all.to_string();
        if rs == "\n" {
            let mut start = 0usize;
            let bytes = all_buf.as_bytes();
            for (i, &b) in bytes.iter().enumerate() {
                if b == b'\n' {
                    let mut rec_end = i;
                    if rec_end > start && bytes[rec_end - 1] == b'\r' {
                        rec_end -= 1;
                    }
                    pairs.push((all_buf[start..rec_end].to_string(), "\n".to_string()));
                    start = i + 1;
                }
            }
            if start < all_buf.len() {
                pairs.push((all_buf[start..].to_string(), String::new()));
            }
        } else if rs.len() == 1 {
            let sep = rs.chars().next().unwrap();
            if all_buf.ends_with('\n') {
                all_buf.pop();
            }
            let mut start = 0usize;
            for (i, c) in all_buf.char_indices() {
                if c == sep {
                    pairs.push((all_buf[start..i].to_string(), c.to_string()));
                    start = i + c.len_utf8();
                }
            }
            if start < all_buf.len() {
                pairs.push((all_buf[start..].to_string(), String::new()));
            }
        } else if rs.is_empty() {
            let blank = Regex::new(r"\n\n+").unwrap();
            let leading_nl = all_buf.bytes().take_while(|&b| b == b'\n').count();
            let mut last_end = leading_nl;
            for m in blank.find_iter(&all_buf[leading_nl..]) {
                let abs_start = leading_nl + m.start();
                let abs_end = leading_nl + m.end();
                let para = &all_buf[last_end..abs_start];
                if !para.is_empty() {
                    pairs.push((para.to_string(), m.as_str().to_string()));
                }
                last_end = abs_end;
            }
            if last_end < all_buf.len() {
                let rest = &all_buf[last_end..];
                let trimmed_end = rest.trim_end_matches('\n');
                if !trimmed_end.is_empty() {
                    let rt = rest[trimmed_end.len()..].to_string();
                    pairs.push((trimmed_end.to_string(), rt));
                }
            }
        } else {
            match Regex::new(rs) {
                Ok(re) => {
                    let mut last_end = 0usize;
                    let mut any_non_empty = false;
                    for m in re.find_iter(&all_buf) {
                        if m.end() == m.start() {
                            continue;
                        }
                        any_non_empty = true;
                        pairs.push((
                            all_buf[last_end..m.start()].to_string(),
                            m.as_str().to_string(),
                        ));
                        last_end = m.end();
                    }
                    if any_non_empty {
                        if last_end < all_buf.len() {
                            pairs.push((all_buf[last_end..].to_string(), String::new()));
                        }
                    } else {
                        pairs.push((all_buf.clone(), String::new()));
                    }
                }
                Err(_) => {
                    let mut start = 0usize;
                    while let Some(pos) = all_buf[start..].find(rs) {
                        let abs = start + pos;
                        pairs.push((all_buf[start..abs].to_string(), rs.to_string()));
                        start = abs + rs.len();
                    }
                    pairs.push((all_buf[start..].to_string(), String::new()));
                }
            }
        }
        pairs.into_iter().unzip()
    }

    fn process_stream(&mut self, program: &Program, reader: &mut dyn BufRead, filename: &str) {
        self.globals
            .insert("FILENAME".to_string(), Value::Str(filename.to_string()));

        let rs = self
            .globals
            .get("RS")
            .map(|v| v.to_string_val())
            .unwrap_or("\n".to_string());

        // Read all input as bytes and map to a Latin-1-style Rust string so
        // every byte round-trips exactly (paired with `write_awk` on output).
        // This matches gawk's byte-level handling in the C locale, which the
        // test sandbox uses.
        let mut raw: Vec<u8> = Vec::new();
        use std::io::Read;
        reader.read_to_end(&mut raw).ok();
        let all = bytes_to_string(&raw);

        let (records, terminators) = Self::split_by_rs(&all, &rs);

        // Store for bare getline access
        self.input_lines = records;
        self.input_terminators = terminators;
        self.input_line_idx = 0;

        while self.input_line_idx < self.input_lines.len() {
            let rec = self.input_lines[self.input_line_idx].clone();
            let rt = self
                .input_terminators
                .get(self.input_line_idx)
                .cloned()
                .unwrap_or_default();
            self.input_line_idx += 1;

            self.nr += 1;
            self.fnr += 1;
            self.globals
                .insert("NR".to_string(), Value::Num(self.nr as f64));
            self.globals
                .insert("FNR".to_string(), Value::Num(self.fnr as f64));
            self.globals.insert("RT".to_string(), Value::Str(rt));
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
                let output = sprintf_impl_with_convfmt(
                    &vals,
                    &self
                        .globals
                        .get("CONVFMT")
                        .map(|v| v.to_string_val())
                        .unwrap_or("%.6g".to_string()),
                );
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
                // Track parameter used as array
                if self.current_params.contains(&array.to_string()) {
                    self.array_params.insert(array.to_string());
                }
                // The loop variable is used as scalar — check array→scalar conflict
                if self.current_params.contains(&var.to_string()) {
                    let var_resolved = self.resolve_array_name(var);
                    if self.arrays.contains_key(&var_resolved) || self.array_params.contains(var) {
                        let prov = if let Some(origin) = self.param_origins.get(var) {
                            format!("`{var} (from {origin})'")
                        } else {
                            format!("`{var}'")
                        };
                        eprintln!("awk: fatal: attempt to use array {prov} in a scalar context");
                        std::process::exit(2);
                    }
                    self.scalar_params.insert(var.to_string());
                }
                let array_resolved = self.resolve_array_name(array);
                // Check if the variable is a scalar (not an array)
                if !self.arrays.contains_key(&array_resolved)
                    && self.globals.contains_key(array)
                    && !matches!(self.globals.get(array), Some(Value::Uninitialized))
                {
                    eprintln!("awk: fatal: attempt to use scalar `{array}' as an array");
                    std::process::exit(2);
                }
                let mut keys: Vec<String> = self
                    .arrays
                    .get(&array_resolved)
                    .map(|a| a.keys().cloned().collect())
                    .unwrap_or_default();
                // Sort keys for deterministic for-in ordering
                keys.sort();
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
                // Track parameter used as array
                if self.current_params.contains(&name.to_string()) {
                    self.array_params.insert(name.to_string());
                }
                if self.functions.contains_key(name) {
                    eprintln!(
                        "awk: error: function `{name}' called with space between name and `(',"
                    );
                    eprintln!("or used as a variable or an array");
                    std::process::exit(1);
                }
                let resolved = self.resolve_array_name(name);
                if indices.is_empty() {
                    self.arrays.remove(&resolved);
                } else {
                    let vals: Vec<Value> = indices.iter().map(|i| self.eval_expr(i)).collect();
                    let key = self.array_key(&vals);
                    if let Some(arr) = self.arrays.get_mut(&resolved) {
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
                let mut out = io::stdout().lock();
                write_awk(&mut out, output).ok();
                out.flush().ok();
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
                    write_awk(f, output).ok();
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
                    write_awk(f, output).ok();
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
                    write_awk(p, output).ok();
                    p.flush().ok();
                }
            }
        }
    }

    /// Evaluate an expression's side effects without using the result
    /// (e.g., evaluate array index with ++c even when getline fails)
    fn eval_side_effects(&mut self, expr: &Expr) {
        match expr {
            Expr::ArrayRef(_, indices) => {
                for idx in indices {
                    self.eval_expr(idx);
                }
            }
            _ => {
                self.eval_expr(expr);
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
                } else if !self.input_lines.is_empty() {
                    // Had pre-read input that's now exhausted — return EOF
                    if let Some(v) = var {
                        self.eval_side_effects(v);
                    }
                    Value::Num(0.0)
                } else {
                    // No pre-read input — try stdin directly (for interactive/pipe)
                    let mut line = String::new();
                    match io::stdin().lock().read_line(&mut line) {
                        Ok(0) => {
                            // Evaluate target side effects even on EOF
                            if let Some(v) = var {
                                self.eval_side_effects(v);
                            }
                            Value::Num(0.0)
                        }
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
                        Ok(0) => {
                            // Evaluate target side effects even on EOF
                            if let Some(v) = var {
                                self.eval_side_effects(v);
                            }
                            Value::Num(0.0)
                        }
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
                    let rs = self
                        .globals
                        .get("RS")
                        .map(|v| v.to_string_val())
                        .unwrap_or_else(|| "\n".to_string());

                    // For non-default RS (multi-char regex, paragraph mode, or
                    // any separator other than "\n"), line-at-a-time reading
                    // can't honor the separator. Read the full pipe output on
                    // first access, split by current RS, and serve subsequent
                    // getlines from the cached records.
                    if rs != "\n" {
                        if !self.pipe_records.contains_key(&cmd) {
                            let mut output = String::new();
                            match Command::new("sh")
                                .arg("-c")
                                .arg(&cmd)
                                .stdout(Stdio::piped())
                                .spawn()
                            {
                                Ok(mut child) => {
                                    if let Some(mut stdout) = child.stdout.take() {
                                        use std::io::Read;
                                        stdout.read_to_string(&mut output).ok();
                                    }
                                    self.pipe_children.insert(cmd.clone(), child);
                                }
                                Err(_) => return Value::Num(-1.0),
                            }
                            let (records, terminators) = Self::split_by_rs(&output, &rs);
                            self.pipe_records.insert(
                                cmd.clone(),
                                PipeRecordState {
                                    records,
                                    terminators,
                                    pos: 0,
                                },
                            );
                        }
                        let state = self.pipe_records.get_mut(&cmd).unwrap();
                        if state.pos >= state.records.len() {
                            if let Some(v) = var {
                                self.eval_side_effects(v);
                            }
                            return Value::Num(0.0);
                        }
                        let rec = state.records[state.pos].clone();
                        let rt = state
                            .terminators
                            .get(state.pos)
                            .cloned()
                            .unwrap_or_default();
                        state.pos += 1;
                        self.globals.insert("RT".to_string(), Value::Str(rt));
                        if let Some(var_expr) = var {
                            self.assign_to(var_expr, Value::StrNum(rec));
                        } else {
                            self.set_record(&rec);
                            self.nr += 1;
                            self.globals
                                .insert("NR".to_string(), Value::Num(self.nr as f64));
                        }
                        return Value::Num(1.0);
                    }

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
                        Ok(0) => {
                            // Evaluate target side effects even on EOF
                            if let Some(v) = var {
                                self.eval_side_effects(v);
                            }
                            Value::Num(0.0)
                        }
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
    /// True if the pattern ends in an unescaped backslash — the specific
    /// condition gawk reports as "Trailing backslash" at regex-compile time.
    fn has_trailing_backslash(s: &str) -> bool {
        let mut count = 0usize;
        for c in s.chars().rev() {
            if c == '\\' {
                count += 1;
            } else {
                break;
            }
        }
        count % 2 == 1
    }

    /// Emit a gawk-style runtime regex error with the current FILENAME/FNR
    /// context. Used when the regex source is dynamic (not a compile-time
    /// literal) so the wording matches gawk's per-record error reporting.
    fn runtime_regex_error(&self, cause: &str, pattern: &str) -> ! {
        let fname = self
            .globals
            .get("FILENAME")
            .map(|v| v.to_string_val())
            .unwrap_or_default();
        let fname_display = if fname.is_empty() {
            "-".to_string()
        } else {
            fname
        };
        let msg = format!(
            "awk: (FILENAME={} FNR={}) fatal: invalid regexp: {}: /{}/\n",
            fname_display, self.fnr, cause, pattern,
        );
        let mut err = io::stderr().lock();
        write_awk(&mut err, &msg).ok();
        err.flush().ok();
        std::process::exit(2);
    }

    fn compile_regex(pattern: &str) -> Option<Regex> {
        // Expand `\xHH` hex escapes to the literal character up front. gawk
        // substitutes them before bracket-expression parsing, so e.g.
        // `[^[]\x5b` becomes `[^[][` — an unbalanced class that gawk rejects.
        // Doing the same here preserves that error path.
        let expanded = Self::expand_hex_escapes(pattern);
        let expanded = Self::expand_collating_elements(&expanded);
        let fixed = Self::fix_awk_regex_warn(&expanded);
        if let Ok(re) = Regex::new(&fixed) {
            return Some(re);
        }
        // Second attempt: escape problematic chars in character classes
        let fixed2 = Self::fix_char_classes(&fixed);
        match Regex::new(&fixed2) {
            Ok(re) => Some(re),
            Err(_) => {
                eprintln!("awk: error: Invalid regular expression: /{pattern}/");
                std::process::exit(2);
            }
        }
    }

    /// Rewrite POSIX single-character collating elements `[.c.]` as the bare
    /// character `c`. Rust's regex crate rejects the collating syntax, but
    /// most real-world patterns that reach us only use single-char elements,
    /// where the substitution is equivalent. Multi-character collations fall
    /// through unchanged and will surface as a regex error later.
    fn expand_collating_elements(pattern: &str) -> String {
        let chars: Vec<char> = pattern.chars().collect();
        let mut result = String::new();
        let mut i = 0;
        while i < chars.len() {
            if i + 4 < chars.len()
                && chars[i] == '['
                && chars[i + 1] == '.'
                && chars[i + 3] == '.'
                && chars[i + 4] == ']'
            {
                result.push(chars[i + 2]);
                i += 5;
            } else {
                result.push(chars[i]);
                i += 1;
            }
        }
        result
    }

    /// Replace `\xHH` (1-2 hex digits) with the literal character. Matches
    /// gawk's pre-parsing behavior, where hex escapes are expanded before
    /// bracket-class parsing so any resulting special characters affect the
    /// structural interpretation of the pattern.
    fn expand_hex_escapes(pattern: &str) -> String {
        let chars: Vec<char> = pattern.chars().collect();
        let mut result = String::new();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == 'x' {
                let mut j = i + 2;
                let mut val = 0u32;
                let mut count = 0;
                while j < chars.len() && count < 2 && chars[j].is_ascii_hexdigit() {
                    val = val * 16 + chars[j].to_digit(16).unwrap();
                    j += 1;
                    count += 1;
                }
                if count > 0 {
                    if let Some(c) = char::from_u32(val) {
                        result.push(c);
                    } else {
                        result.push_str(&format!("\\x{{{val:X}}}"));
                    }
                    i = j;
                    continue;
                }
            }
            result.push(chars[i]);
            i += 1;
        }
        result
    }

    /// Fix problematic character class patterns (e.g., [---] → [\-])
    fn fix_char_classes(pattern: &str) -> String {
        let chars: Vec<char> = pattern.chars().collect();
        let mut result = String::new();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '[' {
                result.push('[');
                i += 1;
                // Handle negation
                if i < chars.len() && chars[i] == '^' {
                    result.push('^');
                    i += 1;
                }
                // Handle ] as first char
                if i < chars.len() && chars[i] == ']' {
                    result.push(']');
                    i += 1;
                }
                // Process character class contents
                while i < chars.len() && chars[i] != ']' {
                    if chars[i] == '-' {
                        // Check if dash can be a range operator
                        let is_start = result.ends_with('[') || result.ends_with('^');
                        let is_end = i + 1 < chars.len() && chars[i + 1] == ']';
                        let is_consecutive = i + 1 < chars.len() && chars[i + 1] == '-';
                        if is_start || is_end || is_consecutive {
                            result.push_str("\\-");
                        } else {
                            result.push('-');
                        }
                    } else if chars[i] == '\\' && i + 1 < chars.len() {
                        result.push(chars[i]);
                        i += 1;
                        result.push(chars[i]);
                    } else {
                        result.push(chars[i]);
                    }
                    i += 1;
                }
                if i < chars.len() {
                    result.push(']');
                    i += 1;
                }
            } else if chars[i] == '\\' && i + 1 < chars.len() {
                result.push(chars[i]);
                i += 1;
                result.push(chars[i]);
                i += 1;
            } else {
                result.push(chars[i]);
                i += 1;
            }
        }
        result
    }

    fn fix_awk_regex_warn(pattern: &str) -> String {
        use std::sync::Mutex;
        static WARNED: Mutex<Option<std::collections::HashSet<String>>> = Mutex::new(None);
        let mut warned = WARNED.lock().unwrap();
        let warned = warned.get_or_insert_with(std::collections::HashSet::new);

        let chars: Vec<char> = pattern.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '\\' && i + 1 < chars.len() {
                let next = chars[i + 1];
                if !"dDwWsSbBtbnrfax01234567.^$*+?()[]{}|\\/\"".contains(next) {
                    let key = format!("\\{next}");
                    if warned.insert(key) {
                        if next == '8' || next == '9' {
                            eprintln!(
                                "awk: warning: regexp escape sequence `\\{next}' treated as plain `{next}'"
                            );
                        } else if next == 'u' || next == 'U' {
                            eprintln!("awk: warning: no hex digits in `\\{next}' escape sequence");
                        } else {
                            eprintln!(
                                "awk: warning: regexp escape sequence `\\{next}' is not a known regexp operator"
                            );
                        }
                    }
                }
                i += 2;
            } else if chars[i] == '[' {
                i += 1;
                if i < chars.len() && chars[i] == '^' {
                    i += 1;
                }
                if i < chars.len() && chars[i] == ']' {
                    i += 1;
                }
                while i < chars.len() && chars[i] != ']' {
                    if chars[i] == '\\' {
                        i += 1;
                    }
                    i += 1;
                }
                if i < chars.len() {
                    i += 1;
                }
            } else {
                i += 1;
            }
        }
        Self::fix_awk_regex(pattern)
    }

    fn fix_awk_regex(pattern: &str) -> String {
        let chars: Vec<char> = pattern.chars().collect();
        let mut result = String::new();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '\\' && i + 1 < chars.len() {
                let next = chars[i + 1];
                // Octal escape: \d, \dd, \ddd where each d is 0..=7. Rust's
                // regex crate doesn't parse these; emit \x{HEX} instead so the
                // same byte/char is matched.
                if next.is_ascii_digit() && next <= '7' {
                    let mut j = i + 1;
                    let mut octal = 0u32;
                    while j < chars.len()
                        && j - i <= 3
                        && chars[j].is_ascii_digit()
                        && chars[j] <= '7'
                    {
                        octal = octal * 8 + (chars[j] as u32 - '0' as u32);
                        j += 1;
                    }
                    result.push_str(&format!("\\x{{{octal:X}}}"));
                    i = j;
                    continue;
                }
                // Known regex escapes — pass through
                if "dDwWsSbBtbnrfax.^$*+?()[]{}|\\/".contains(next) {
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
                if i < chars.len() && chars[i] == '^' {
                    result.push(chars[i]);
                    i += 1;
                }
                // Record class_start now (position right after `[` or `[^`)
                // so the at-start check used by dash handling reflects logical
                // class content, not the escaped form of a leading `]`.
                let class_start = result.len();
                // `]` as the first character in a bracket class is literal in
                // POSIX/awk (the class isn't empty, it matches `]`). Rust's
                // regex crate reads `[]` as an empty class and errors, so
                // escape the `]` explicitly.
                if i < chars.len() && chars[i] == ']' {
                    result.push('\\');
                    result.push(']');
                    i += 1;
                }
                while i < chars.len() && chars[i] != ']' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        let next = chars[i + 1];
                        // Octal escape inside a character class: convert to
                        // \x{HEX} so the Rust regex crate accepts the class.
                        if next.is_ascii_digit() && next <= '7' {
                            let mut j = i + 1;
                            let mut octal = 0u32;
                            while j < chars.len()
                                && j - i <= 3
                                && chars[j].is_ascii_digit()
                                && chars[j] <= '7'
                            {
                                octal = octal * 8 + (chars[j] as u32 - '0' as u32);
                                j += 1;
                            }
                            result.push_str(&format!("\\x{{{octal:X}}}"));
                            i = j;
                            continue;
                        }
                        result.push(chars[i]);
                        i += 1;
                        result.push(chars[i]);
                        i += 1;
                    } else if chars[i] == '-' {
                        // Check context: escape dashes that would create ambiguous ranges
                        let at_start = result.len() == class_start;
                        let at_end = i + 1 < chars.len() && chars[i + 1] == ']';
                        let after_dash = result.ends_with('-') || result.ends_with("\\-");
                        let before_dash = i + 1 < chars.len() && chars[i + 1] == '-';
                        // `[-X...]` or `[--X...]`: gawk treats a leading `-`
                        // followed by another `-` and a non-`]` as the start
                        // of a range whose low endpoint is `-` itself. Emit
                        // the hex form so Rust's regex crate sees a plain
                        // character (not an ambiguous/escaped `-`) and can
                        // interpret the following `-X` as a range.
                        if at_start && before_dash && i + 2 < chars.len() && chars[i + 2] != ']' {
                            result.push_str("\\x{2D}");
                            i += 1;
                        } else if at_start || at_end || after_dash || before_dash {
                            result.push('\\');
                            result.push('-');
                            i += 1;
                        } else {
                            result.push(chars[i]);
                            i += 1;
                        }
                    } else if chars[i] == '[' {
                        // `[` inside a bracket class: literal in awk/POSIX.
                        // Rust's regex crate requires it escaped — except when
                        // it introduces a POSIX named class like `[:upper:]`,
                        // in which case pass the whole class through verbatim.
                        if chars.get(i + 1) == Some(&':') {
                            // POSIX class: copy until `:]`
                            result.push('[');
                            i += 1;
                            while i < chars.len() {
                                result.push(chars[i]);
                                if chars[i] == ']'
                                    && result.len() >= 2
                                    && result.as_bytes()[result.len() - 2] == b':'
                                {
                                    i += 1;
                                    break;
                                }
                                i += 1;
                            }
                        } else {
                            result.push('\\');
                            result.push('[');
                            i += 1;
                        }
                    } else {
                        result.push(chars[i]);
                        i += 1;
                    }
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
            Expr::Var(name) => {
                if self.functions.contains_key(name)
                    && !self.current_params.contains(&name.to_string())
                {
                    eprintln!(
                        "awk: error: function `{name}' called with space between name and `(',"
                    );
                    eprintln!("or used as a variable or an array");
                    std::process::exit(1);
                }
                // Track parameter used as scalar (via assignment)
                if self.current_params.contains(&name.to_string()) {
                    self.scalar_params.insert(name.to_string());
                }
                // Check array→scalar conflict for parameters
                if self.current_params.contains(&name.to_string())
                    && self.array_params.contains(name)
                {
                    let prov = if let Some(origin) = self.param_origins.get(name) {
                        format!("`{name} (from {origin})'")
                    } else {
                        format!("`{name}'")
                    };
                    eprintln!("awk: fatal: attempt to use array {prov} in a scalar context");
                    std::process::exit(2);
                }
                // Check if a parameter with this global as origin was used as array
                if !self.current_params.contains(&name.to_string()) {
                    for ap in &self.array_params {
                        if let Some(origin) = self.param_origins.get(ap) {
                            let origin_root = origin.split(", from ").last().unwrap_or(origin);
                            if origin_root == name {
                                eprintln!(
                                    "awk: fatal: attempt to use array `{name}' in a scalar context"
                                );
                                std::process::exit(2);
                            }
                        }
                    }
                }
                self.set_var(name, val);
            }
            Expr::FieldRef(idx_expr) => {
                let idx = self.eval_expr(idx_expr).to_num() as usize;
                self.set_field(idx, val);
            }
            Expr::ArrayRef(name, indices) => {
                // Track parameter used as array and check cross-param conflicts
                if self.current_params.contains(&name.to_string()) {
                    // Check if a sibling param (same origin) was used as scalar
                    if let Some(my_origin) = self.param_origins.get(name).cloned() {
                        for sp in &self.scalar_params {
                            if let Some(sp_origin) = self.param_origins.get(sp)
                                && (*sp_origin == my_origin
                                    || my_origin.contains(sp_origin.as_str()))
                            {
                                eprintln!(
                                    "awk: fatal: attempt to use scalar parameter `{name}' as an array"
                                );
                                std::process::exit(2);
                            }
                        }
                        // Also check if the origin variable is now a scalar global
                        let origin_root = my_origin.split(", from ").last().unwrap_or(&my_origin);
                        if self.globals.contains_key(origin_root)
                            && !self.arrays.contains_key(origin_root)
                            && !matches!(
                                self.globals.get(origin_root),
                                Some(Value::Uninitialized) | None
                            )
                        {
                            eprintln!(
                                "awk: fatal: attempt to use scalar parameter `{name}' as an array"
                            );
                            std::process::exit(2);
                        }
                    }
                    self.array_params.insert(name.to_string());
                }
                // Check scalar-as-array conflict
                let name_resolved = self.resolve_array_name(name);
                if !self.arrays.contains_key(&name_resolved) && self.scalar_params.contains(name) {
                    let kind = if self.current_params.contains(&name.to_string()) {
                        "scalar parameter"
                    } else {
                        "scalar"
                    };
                    eprintln!("awk: fatal: attempt to use {kind} `{name}' as an array");
                    std::process::exit(2);
                }
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
            Expr::Var(name) => {
                // Check function-as-variable
                if self.functions.contains_key(name)
                    && !self.globals.contains_key(name)
                    && !self.current_params.contains(&name.to_string())
                {
                    eprintln!(
                        "awk: error: function `{name}' called with space between name and `(',"
                    );
                    eprintln!("or used as a variable or an array");
                    std::process::exit(1);
                }
                // Track parameter used as scalar and check for array→scalar conflict
                if self.current_params.contains(&name.to_string()) {
                    if self.array_params.contains(name) {
                        let prov = if let Some(origin) = self.param_origins.get(name) {
                            format!("`{name} (from {origin})'")
                        } else {
                            format!("`{name}'")
                        };
                        eprintln!("awk: fatal: attempt to use array {prov} in a scalar context");
                        std::process::exit(2);
                    }
                    self.scalar_params.insert(name.to_string());
                } else {
                    // If this global is the origin of a param, mark params with that origin as scalar
                    for (param, origin) in &self.param_origins {
                        let origin_root = origin.split(", from ").last().unwrap_or(origin);
                        if origin_root == name {
                            self.scalar_params.insert(param.clone());
                        }
                    }
                }
                self.get_var(name)
            }
            Expr::FieldRef(idx_expr) => {
                let idx = self.eval_expr(idx_expr).to_num() as usize;
                self.get_field(idx)
            }
            Expr::ArrayRef(name, indices) => {
                let name_resolved = self.resolve_array_name(name);
                // Check function-as-array
                if self.functions.contains_key(name) && !self.arrays.contains_key(&name_resolved) {
                    eprintln!(
                        "awk: error: function `{name}' called with space between name and `(',"
                    );
                    eprintln!("or used as a variable or an array");
                    std::process::exit(1);
                }
                // Check scalar-as-array (including params tracked as scalar)
                if !self.arrays.contains_key(&name_resolved) {
                    let is_scalar_param = self.scalar_params.contains(name);
                    let is_scalar_global = (self.globals.contains_key(name)
                        && !matches!(self.globals.get(name), Some(Value::Uninitialized)))
                        || self.global_scalar_via_func.contains(name);
                    if is_scalar_param || is_scalar_global {
                        let kind = if self.current_params.contains(&name.to_string()) {
                            "scalar parameter"
                        } else {
                            "scalar"
                        };
                        eprintln!("awk: fatal: attempt to use {kind} `{name}' as an array");
                        std::process::exit(2);
                    }
                }
                // Track parameter used as array and check origin conflicts
                if self.current_params.contains(&name.to_string()) {
                    if let Some(my_origin) = self.param_origins.get(name).cloned() {
                        // Check if origin variable is now a scalar global
                        let origin_root = my_origin.split(", from ").last().unwrap_or(&my_origin);
                        if self.globals.contains_key(origin_root)
                            && !self.arrays.contains_key(origin_root)
                            && !matches!(
                                self.globals.get(origin_root),
                                Some(Value::Uninitialized) | None
                            )
                        {
                            eprintln!(
                                "awk: fatal: attempt to use scalar parameter `{name}' as an array"
                            );
                            std::process::exit(2);
                        }
                    }
                    self.array_params.insert(name.to_string());
                }
                let vals: Vec<Value> = indices.iter().map(|i| self.eval_expr(i)).collect();
                let key = self.array_key(&vals);
                // Auto-vivify: reading an array element creates it
                let val = self.get_array(name, &key);
                if !self
                    .arrays
                    .get(&name_resolved)
                    .is_some_and(|a| a.contains_key(&key))
                {
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
                            // Division by zero returns inf (IEEE behavior)
                            Value::Num(lv.to_num() / rv.to_num())
                        }
                        BinOp::Mod => {
                            let d = rv.to_num();
                            if d == 0.0 {
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
                // For FieldRef, cache the index to avoid double evaluation
                if let Expr::FieldRef(idx_expr) = operand.as_ref() {
                    let idx = self.eval_expr(idx_expr).to_num() as usize;
                    let v = self.get_field(idx).to_num();
                    self.set_field(idx, Value::Num(v + 1.0));
                    Value::Num(v)
                } else {
                    let v = self.eval_expr(operand).to_num();
                    self.assign_to(operand, Value::Num(v + 1.0));
                    Value::Num(v)
                }
            }
            Expr::PostDecrement(operand) => {
                if let Expr::FieldRef(idx_expr) = operand.as_ref() {
                    let idx = self.eval_expr(idx_expr).to_num() as usize;
                    let v = self.get_field(idx).to_num();
                    self.set_field(idx, Value::Num(v - 1.0));
                    Value::Num(v)
                } else {
                    let v = self.eval_expr(operand).to_num();
                    self.assign_to(operand, Value::Num(v - 1.0));
                    Value::Num(v)
                }
            }
            Expr::Assign(lhs, rhs) => {
                let val = self.eval_expr(rhs);
                self.assign_to(lhs, val.clone());
                val
            }
            Expr::OpAssign(lhs, op, rhs) => {
                // For array refs with side effects (a[b++] += 1),
                // evaluate the index once and cache the resolved target.
                // For simple vars (b += b += 1), evaluate rhs first since
                // it may modify the same variable.
                let (lv, rv, resolved_target) = match lhs.as_ref() {
                    Expr::ArrayRef(name, indices) => {
                        let vals: Vec<Value> = indices.iter().map(|i| self.eval_expr(i)).collect();
                        let key = self.array_key(&vals);
                        let cur = self.get_array(name, &key).to_num();
                        let rv = self.eval_expr(rhs).to_num();
                        (cur, rv, Some((name.clone(), key)))
                    }
                    _ => {
                        // Evaluate rhs first (may have side effects on lhs var)
                        let rv = self.eval_expr(rhs).to_num();
                        let lv = self.eval_expr(lhs).to_num();
                        (lv, rv, None)
                    }
                };
                let result = match op {
                    BinOp::Add => lv + rv,
                    BinOp::Sub => lv - rv,
                    BinOp::Mul => lv * rv,
                    BinOp::Div => lv / rv,
                    BinOp::Mod => {
                        if rv == 0.0 {
                            0.0
                        } else {
                            lv % rv
                        }
                    }
                    BinOp::Pow => lv.powf(rv),
                    _ => lv,
                };
                let val = Value::Num(result);
                if let Some((name, key)) = resolved_target {
                    self.set_array(&name, &key, val.clone());
                } else {
                    self.assign_to(lhs, val.clone());
                }
                val
            }
            Expr::Match(left, right) => {
                let s = self.eval_expr(left).to_string_val();
                let (pattern, is_literal) =
                    if let Some(r) = self.extract_regex_pattern(right) {
                        (r, true)
                    } else {
                        (self.eval_expr(right).to_string_val(), false)
                    };
                if !is_literal && Self::has_trailing_backslash(&pattern) {
                    self.runtime_regex_error("Trailing backslash", &pattern);
                }
                match Self::compile_regex(&pattern) {
                    Some(re) => Value::Num(if re.is_match(&s) { 1.0 } else { 0.0 }),
                    None => Value::Num(0.0),
                }
            }
            Expr::NotMatch(left, right) => {
                let s = self.eval_expr(left).to_string_val();
                let (pattern, is_literal) =
                    if let Some(r) = self.extract_regex_pattern(right) {
                        (r, true)
                    } else {
                        (self.eval_expr(right).to_string_val(), false)
                    };
                if !is_literal && Self::has_trailing_backslash(&pattern) {
                    self.runtime_regex_error("Trailing backslash", &pattern);
                }
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
                // Track parameter used as array
                if self.current_params.contains(&array.to_string()) {
                    if self.scalar_params.contains(array) {
                        eprintln!(
                            "awk: fatal: attempt to use scalar parameter `{array}' as an array"
                        );
                        std::process::exit(2);
                    }
                    self.array_params.insert(array.to_string());
                }
                let array_resolved = self.resolve_array_name(array);
                // Check scalar-as-array
                if !self.arrays.contains_key(&array_resolved)
                    && self.globals.contains_key(array)
                    && !matches!(self.globals.get(array), Some(Value::Uninitialized))
                {
                    eprintln!("awk: fatal: attempt to use scalar `{array}' as an array");
                    std::process::exit(2);
                }
                let key = self.eval_expr(expr).to_string_val();
                let exists = self
                    .arrays
                    .get(&array_resolved)
                    .is_some_and(|a| a.contains_key(&key));
                Value::Num(if exists { 1.0 } else { 0.0 })
            }
            Expr::MultiIn(indices, array) => {
                let vals: Vec<Value> = indices.iter().map(|i| self.eval_expr(i)).collect();
                let key = self.array_key(&vals);
                let array_resolved = self.resolve_array_name(array);
                let exists = self
                    .arrays
                    .get(&array_resolved)
                    .is_some_and(|a| a.contains_key(&key));
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
                Value::Str(sprintf_impl_with_convfmt(
                    &vals,
                    &self
                        .globals
                        .get("CONVFMT")
                        .map(|v| v.to_string_val())
                        .unwrap_or("%.6g".to_string()),
                ))
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
                if let Some(Expr::Var(arr_name)) = args.first() {
                    let resolved = self.resolve_array_name(arr_name);
                    if self.arrays.contains_key(&resolved) {
                        return Value::Num(self.arrays[&resolved].len() as f64);
                    }
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

                // Clear the array (resolve alias so split() on a passed-by-ref
                // array param clears the caller's storage, not a stale slot)
                let arr_resolved = self.resolve_array_name(&arr_name);
                self.arrays.remove(&arr_resolved);

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
                    // Check if target is a function name used as variable
                    if let Expr::Var(tname) = &args[2]
                        && self.functions.contains_key(tname)
                        && !self.current_params.contains(&tname.to_string())
                    {
                        eprintln!(
                            "awk: error: function `{tname}' called with space between name and `(',"
                        );
                        eprintln!("or used as a variable or an array");
                        std::process::exit(1);
                    }
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
                        // Always assign for vars (marks type), skip for field refs when no match
                        // to avoid unnecessary $0 rebuild
                        if count > 0 || !matches!(target_expr, Expr::FieldRef(_)) {
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
                // Get optional array name for capture groups (3rd arg)
                let arr_name = if args.len() >= 3 {
                    match &args[2] {
                        Expr::Var(n) => Some(n.clone()),
                        _ => None,
                    }
                } else {
                    None
                };
                match Self::compile_regex(&pattern) {
                    Some(re) => {
                        if let Some(caps) = re.captures(&s) {
                            let m = caps.get(0).unwrap();
                            let start = s[..m.start()].chars().count() + 1;
                            let length = m.as_str().chars().count();
                            self.set_var("RSTART", Value::Num(start as f64));
                            self.set_var("RLENGTH", Value::Num(length as f64));
                            // Populate capture array if provided
                            if let Some(ref arr) = arr_name {
                                let arr_resolved = self.resolve_array_name(arr);
                                self.arrays.remove(&arr_resolved);
                                // arr[0] = entire match
                                self.set_array(arr, "0", Value::Str(m.as_str().to_string()));
                                // arr[1..n] = capture groups
                                for i in 1..caps.len() {
                                    if let Some(g) = caps.get(i) {
                                        self.set_array(
                                            arr,
                                            &i.to_string(),
                                            Value::Str(g.as_str().to_string()),
                                        );
                                    }
                                }
                            }
                            Value::Num(start as f64)
                        } else {
                            self.set_var("RSTART", Value::Num(0.0));
                            self.set_var("RLENGTH", Value::Num(-1.0));
                            if let Some(ref arr) = arr_name {
                                let arr_resolved = self.resolve_array_name(arr);
                                self.arrays.remove(&arr_resolved);
                            }
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
                Value::Str(sprintf_impl_with_convfmt(
                    &vals,
                    &self
                        .globals
                        .get("CONVFMT")
                        .map(|v| v.to_string_val())
                        .unwrap_or("%.6g".to_string()),
                ))
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
            "rand" => Value::Num(self.rng.next_f64()),
            "srand" => {
                let old = self.rng_state;
                let new_seed = if args.is_empty() {
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos() as u64)
                        .unwrap_or(0)
                } else {
                    self.eval_expr(&args[0]).to_num() as u64
                };
                self.rng_state = new_seed;
                self.rng.srand(new_seed);
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
                if let Some(mut child) = self.pipe_children.remove(&name)
                    && let Ok(status) = child.wait()
                {
                    let code = status.code().unwrap_or(-1);
                    return Value::Num(code as f64);
                }
                if !found {
                    self.set_var(
                        "ERRNO",
                        Value::Str("close of redirection that was never opened".to_string()),
                    );
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
                    let resolved = self.resolve_array_name(name);
                    if self.arrays.contains_key(&resolved) {
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

                let arr_resolved = self.resolve_array_name(&arr_name);
                let arr = match self.arrays.get(&arr_resolved) {
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
                let dest_resolved = self.resolve_array_name(&dest_name);
                self.arrays.insert(dest_resolved, new_arr);
                Value::Num(count as f64)
            }
            _ => {
                // User-defined function
                let func = self.functions.get(name).cloned();
                if let Some(func) = func {
                    let mut saved_vars: Vec<(String, Option<Value>)> = Vec::new();
                    let mut saved_arrays: Vec<(String, Option<HashMap<String, Value>>)> =
                        Vec::new();

                    // Collect argument info. Resolve each Var arg through the
                    // current frame's alias map so array args arriving as
                    // already-aliased params are detected and their canonical
                    // storage name is recorded.
                    let mut arg_vals: Vec<Value> = Vec::new();
                    let mut arg_var_names: Vec<Option<String>> = Vec::new();
                    let mut arg_canonical: Vec<Option<String>> = Vec::new();
                    let mut arg_was_array: Vec<bool> = Vec::new();
                    for arg in args {
                        if let Expr::Var(var_name) = arg {
                            let canonical = self.resolve_array_name(var_name);
                            let is_array = self.arrays.contains_key(&canonical);
                            // Uninitialized variables also pass by reference:
                            // if the callee uses the param as an array, the
                            // caller's variable must be promoted to an array,
                            // which requires shared storage (awk semantics).
                            let is_definite_scalar = self
                                .globals
                                .get(var_name)
                                .is_some_and(|v| !matches!(v, Value::Uninitialized));
                            let passes_by_ref = is_array || !is_definite_scalar;
                            if passes_by_ref {
                                arg_vals.push(Value::Uninitialized);
                                arg_canonical.push(Some(canonical));
                            } else {
                                arg_vals.push(self.get_var(var_name));
                                arg_canonical.push(None);
                            }
                            arg_var_names.push(Some(var_name.clone()));
                            arg_was_array.push(passes_by_ref);
                        } else {
                            arg_vals.push(self.eval_expr(arg));
                            arg_var_names.push(None);
                            arg_canonical.push(None);
                            arg_was_array.push(false);
                        }
                    }

                    // Swap in a fresh alias map for this frame. Multiple params
                    // sharing the same caller-side array all alias to the same
                    // canonical name, so writes via any param are seen by all.
                    let saved_aliases = std::mem::take(&mut self.array_aliases);
                    let mut new_aliases: HashMap<String, String> = HashMap::new();

                    // First pass: decide aliases, so the scalar-slot handling
                    // in the second pass can tell when a param name is a
                    // canonical alias target that must not be shadowed in
                    // `self.arrays`.
                    for (i, param) in func.params.iter().enumerate() {
                        if arg_was_array.get(i).copied().unwrap_or(false)
                            && let Some(canonical) = arg_canonical.get(i).and_then(|c| c.as_ref())
                            && param != canonical
                        {
                            new_aliases.insert(param.clone(), canonical.clone());
                        }
                    }
                    let alias_targets: std::collections::HashSet<String> =
                        new_aliases.values().cloned().collect();

                    for (i, param) in func.params.iter().enumerate() {
                        // Always save the scalar slot — the param shadows any
                        // global with the same name.
                        saved_vars.push((param.clone(), self.globals.remove(param)));

                        if arg_was_array.get(i).copied().unwrap_or(false) {
                            // Array by reference: alias already recorded (or
                            // skipped as a self-alias). No scratch storage to
                            // allocate — the canonical slot is the caller's.
                            continue;
                        }

                        // Scalar param: save the array slot too so any local
                        // array vivification (`param[k] = v`) is cleared on
                        // return instead of leaking into the caller's scope.
                        // Skip this when the param's name coincides with a
                        // canonical alias target (a sibling array param aliases
                        // to it), since removing that storage would orphan the
                        // alias.
                        if !alias_targets.contains(param) {
                            saved_arrays.push((param.clone(), self.arrays.remove(param)));
                        }
                        if i < arg_vals.len() {
                            self.set_var(param, arg_vals[i].clone());
                        } else {
                            self.set_var(param, Value::Uninitialized);
                        }
                    }
                    self.array_aliases = new_aliases;

                    let saved_params =
                        std::mem::replace(&mut self.current_params, func.params.clone());
                    let saved_scalar_params = std::mem::take(&mut self.scalar_params);
                    let saved_array_params = std::mem::take(&mut self.array_params);
                    let saved_param_origins = std::mem::take(&mut self.param_origins);
                    // Build param origin map
                    for (i, param) in func.params.iter().enumerate() {
                        if let Some(Some(orig_name)) = arg_var_names.get(i) {
                            // Chain: if orig was itself a param with origin, build chain
                            if let Some(prev_origin) = saved_param_origins.get(orig_name) {
                                self.param_origins.insert(
                                    param.clone(),
                                    format!("{orig_name}, from {prev_origin}"),
                                );
                            } else {
                                self.param_origins.insert(param.clone(), orig_name.clone());
                            }
                        }
                    }
                    // Pre-mark params that received actual arrays. Uninit
                    // vars pass by reference too but haven't committed to
                    // being arrays — only mark when the caller's storage
                    // already exists, so scalar usage of the param inside
                    // the function isn't spuriously flagged as a conflict.
                    for (i, param) in func.params.iter().enumerate() {
                        if let Some(Some(canonical)) = arg_canonical.get(i)
                            && self.arrays.contains_key(canonical)
                        {
                            self.array_params.insert(param.clone());
                        }
                    }
                    let result = match self.exec_stmts(&func.body) {
                        ControlFlow::Return(val) => val,
                        _ => Value::Uninitialized,
                    };
                    // Propagate scalar usage to origin globals
                    for sp in &self.scalar_params {
                        if let Some(origin) = self.param_origins.get(sp) {
                            let origin_root =
                                origin.split(", from ").last().unwrap_or(origin).to_string();
                            self.global_scalar_via_func.insert(origin_root);
                        }
                    }

                    self.current_params = saved_params;
                    self.scalar_params = saved_scalar_params;
                    self.array_params = saved_array_params;
                    self.param_origins = saved_param_origins;
                    self.array_aliases = saved_aliases;

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
                    eprintln!("awk: error: attempt to use non-function `{name}' in function call");
                    std::process::exit(1);
                }
            }
        }
    }
}
