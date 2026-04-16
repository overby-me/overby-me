use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};

use crate::ast::*;
use crate::value::{Value, format_number};

/// Control flow signal
enum Flow {
    None,
    Last(Option<String>),
    Next(Option<String>),
    Return(Value),
    Die(String),
    Exit(i32),
}

/// A scope frame for lexical variables
#[derive(Clone)]
struct Scope {
    vars: HashMap<String, Value>,
    arrays: HashMap<String, Vec<Value>>,
    hashes: HashMap<String, HashMap<String, Value>>,
}

impl Scope {
    fn new() -> Self {
        Scope {
            vars: HashMap::new(),
            arrays: HashMap::new(),
            hashes: HashMap::new(),
        }
    }
}

pub struct Interpreter {
    // Scope stack: last is innermost
    scopes: Vec<Scope>,
    // Global scope
    globals: Scope,
    // Subroutines
    subs: HashMap<String, (Vec<String>, Vec<Stmt>)>,
    // BEGIN blocks (already executed)
    // END blocks (deferred)
    end_blocks: Vec<Vec<Stmt>>,
    // Current package
    package: String,
    // Exit code
    pub exit_code: i32,
    // Last expression value (for implicit sub return)
    last_expr_val: Value,
    // Last list value (for sub returning list in list context)
    last_list_val: Option<Vec<Value>>,
    // Saved $@ for eval
    eval_error: String,
    // Saved local variables for restore
    local_saves: Vec<Vec<(String, Value)>>,
    // Local array saves
    local_array_saves: Vec<Vec<(String, Vec<Value>)>>,
    // File handles for reading
    read_handles: HashMap<String, BufReader<File>>,
    // File handles for writing
    write_handles: HashMap<String, BufWriter<File>>,
    // Counter for generating anonymous filehandle names
    fh_counter: usize,
    // Tracks files already loaded via require (like %INC)
    required_files: HashSet<String>,
}

impl Interpreter {
    pub fn new() -> Self {
        let mut globals = Scope::new();
        // Set up default special variables
        globals
            .vars
            .insert("/".to_string(), Value::Str("\n".to_string()));
        globals.vars.insert("\\".to_string(), Value::Undef);
        globals.vars.insert(",".to_string(), Value::Undef);
        globals.vars.insert("_".to_string(), Value::Undef);
        // $" — list separator (default space)
        globals
            .vars
            .insert("\"".to_string(), Value::Str(" ".to_string()));
        // $| — autoflush (default 0)
        globals.vars.insert("|".to_string(), Value::Num(0.0));
        globals
            .vars
            .insert("^O".to_string(), Value::Str("linux".to_string()));
        globals
            .vars
            .insert("@".to_string(), Value::Str(String::new()));
        globals
            .vars
            .insert("0".to_string(), Value::Str("perl".to_string()));
        globals
            .vars
            .insert("!".to_string(), Value::Str(String::new()));
        // $^X — path to the Perl executable (we use our own binary path)
        globals.vars.insert(
            "^X".to_string(),
            Value::Str(
                std::env::current_exe()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|_| "perl".to_string()),
            ),
        );

        Interpreter {
            scopes: Vec::new(),
            globals,
            subs: HashMap::new(),
            end_blocks: Vec::new(),
            package: "main".to_string(),
            exit_code: 0,
            last_expr_val: Value::Undef,
            last_list_val: None,
            eval_error: String::new(),
            local_saves: Vec::new(),
            local_array_saves: Vec::new(),
            read_handles: HashMap::new(),
            write_handles: HashMap::new(),
            fh_counter: 0,
            required_files: HashSet::new(),
        }
    }

    pub fn set_special_var(&mut self, name: &str, val: &str) {
        self.globals
            .vars
            .insert(name.to_string(), Value::Str(val.to_string()));
    }

    pub fn run(&mut self, program: &[Stmt]) {
        // First pass: collect sub definitions and BEGIN blocks
        let mut main_stmts = Vec::new();
        for stmt in program {
            match stmt {
                Stmt::Sub { name, params, body } if !name.is_empty() => {
                    self.subs
                        .insert(name.clone(), (params.clone(), body.clone()));
                }
                Stmt::Begin(body) => {
                    let _flow = self.exec_stmts(body);
                }
                Stmt::End(body) => {
                    self.end_blocks.push(body.clone());
                }
                _ => main_stmts.push(stmt.clone()),
            }
        }

        // Execute main program
        let _flow = self.exec_stmts(&main_stmts);

        // Execute END blocks in reverse order
        let end_blocks: Vec<Vec<Stmt>> = self.end_blocks.clone().into_iter().rev().collect();
        for body in &end_blocks {
            let _flow = self.exec_stmts(body);
        }
    }

    fn exec_stmts(&mut self, stmts: &[Stmt]) -> Flow {
        for stmt in stmts {
            let flow = self.exec_stmt(stmt);
            match flow {
                Flow::None => {}
                other => return other,
            }
        }
        Flow::None
    }

    fn exec_stmt(&mut self, stmt: &Stmt) -> Flow {
        match stmt {
            Stmt::Nop => Flow::None,

            Stmt::Expr(expr) => {
                // Check if expression produces a list (for sub list-context return)
                match expr {
                    Expr::ArrayLit(items) => {
                        let list: Vec<Value> =
                            items.iter().flat_map(|item| self.eval_list(item)).collect();
                        if let Some(last) = list.last() {
                            self.last_expr_val = last.clone();
                        } else {
                            self.last_expr_val = Value::Undef;
                        }
                        self.last_list_val = Some(list);
                    }
                    Expr::ArrayVar(name) => {
                        let list = self.get_array(name);
                        if let Some(last) = list.last() {
                            self.last_expr_val = last.clone();
                        } else {
                            self.last_expr_val = Value::Undef;
                        }
                        self.last_list_val = Some(list);
                    }
                    _ => {
                        self.last_expr_val = self.eval_expr(expr);
                        self.last_list_val = None;
                    }
                }
                Flow::None
            }

            Stmt::Print(fh, args) => {
                self.exec_print(fh, args, false);
                Flow::None
            }

            Stmt::Say(fh, args) => {
                self.exec_print(fh, args, true);
                Flow::None
            }

            Stmt::Printf(fh, args) => {
                if !args.is_empty() {
                    let fmt = self.eval_expr(&args[0]).to_str();
                    let vals: Vec<Value> = args[1..].iter().map(|a| self.eval_expr(a)).collect();
                    let result = self.sprintf_impl(&fmt, &vals);
                    let fh_name = fh.as_ref().map(|e| self.eval_expr(e).to_str());
                    self.write_to_handle(&fh_name, &result);
                }
                Flow::None
            }

            Stmt::If {
                cond,
                then,
                elsifs,
                else_block,
            } => {
                let val = self.eval_expr(cond);
                self.last_expr_val = val.clone();
                if val.to_bool() {
                    return self.exec_stmts(then);
                }
                for (cond, body) in elsifs {
                    let val = self.eval_expr(cond);
                    self.last_expr_val = val.clone();
                    if val.to_bool() {
                        return self.exec_stmts(body);
                    }
                }
                if let Some(body) = else_block {
                    return self.exec_stmts(body);
                }
                Flow::None
            }

            Stmt::Unless {
                cond,
                then,
                else_block,
            } => {
                let val = self.eval_expr(cond);
                self.last_expr_val = val.clone();
                if !val.to_bool() {
                    return self.exec_stmts(then);
                }
                if let Some(body) = else_block {
                    return self.exec_stmts(body);
                }
                Flow::None
            }

            Stmt::While { cond, body, label } => {
                loop {
                    if !self.eval_expr(cond).to_bool() {
                        break;
                    }
                    match self.exec_stmts(body) {
                        Flow::Last(l) if l.is_none() || l == *label => break,
                        Flow::Next(l) if l.is_none() || l == *label => continue,
                        Flow::Last(_) | Flow::Next(_) => {} // Different label
                        Flow::Return(v) => return Flow::Return(v),
                        Flow::Die(msg) => return Flow::Die(msg),
                        Flow::Exit(code) => return Flow::Exit(code),
                        Flow::None => {}
                    }
                }
                Flow::None
            }

            Stmt::Until { cond, body, label } => {
                loop {
                    if self.eval_expr(cond).to_bool() {
                        break;
                    }
                    match self.exec_stmts(body) {
                        Flow::Last(l) if l.is_none() || l == *label => break,
                        Flow::Next(l) if l.is_none() || l == *label => continue,
                        Flow::Return(v) => return Flow::Return(v),
                        Flow::Die(msg) => return Flow::Die(msg),
                        Flow::Exit(code) => return Flow::Exit(code),
                        _ => {}
                    }
                }
                Flow::None
            }

            Stmt::For {
                init,
                cond,
                step,
                body,
                label,
            } => {
                if let Some(init) = init {
                    self.exec_stmt(init);
                }
                loop {
                    if let Some(cond) = cond {
                        if !self.eval_expr(cond).to_bool() {
                            break;
                        }
                    }
                    match self.exec_stmts(body) {
                        Flow::Last(l) if l.is_none() || l == *label => break,
                        Flow::Next(l) if l.is_none() || l == *label => {}
                        Flow::Return(v) => return Flow::Return(v),
                        Flow::Die(msg) => return Flow::Die(msg),
                        Flow::Exit(code) => return Flow::Exit(code),
                        _ => {}
                    }
                    if let Some(step) = step {
                        self.eval_expr(step);
                    }
                }
                Flow::None
            }

            Stmt::Foreach {
                var,
                is_my,
                list,
                body,
                label,
            } => {
                // Detect if iterating over an array variable (for aliasing)
                let source_array = match list {
                    Expr::ArrayVar(name) => Some(name.clone()),
                    _ => None,
                };
                let items = self.eval_list(list);

                // Save the loop variable's current value for restoration
                let saved_var = self.get_var(var);

                self.push_scope();
                for (i, item) in items.into_iter().enumerate() {
                    self.set_var(var, item);
                    let flow = self.exec_stmts(body);

                    // If iterating over an array, write modifications back
                    if let Some(ref arr_name) = source_array {
                        let modified_val = self.get_var(var);
                        let mut arr = self.get_array(arr_name);
                        if i < arr.len() {
                            arr[i] = modified_val;
                            self.set_array(arr_name, arr);
                        }
                    }

                    match flow {
                        Flow::Last(l) if l.is_none() || l == *label => break,
                        Flow::Next(l) if l.is_none() || l == *label => continue,
                        Flow::Return(v) => {
                            self.pop_scope();
                            self.set_var(var, saved_var);
                            return Flow::Return(v);
                        }
                        Flow::Die(msg) => {
                            self.pop_scope();
                            self.set_var(var, saved_var);
                            return Flow::Die(msg);
                        }
                        Flow::Exit(code) => {
                            self.pop_scope();
                            self.set_var(var, saved_var);
                            return Flow::Exit(code);
                        }
                        _ => {}
                    }
                }
                self.pop_scope();
                // Restore the loop variable to its pre-loop value
                self.set_var(var, saved_var);
                Flow::None
            }

            Stmt::Last(label) => Flow::Last(label.clone()),
            Stmt::Next(label) => Flow::Next(label.clone()),
            Stmt::Redo(_) => Flow::None, // TODO
            Stmt::Return(expr) => {
                if let Some(e) = expr {
                    // Store list result for list-context returns
                    let list = self.eval_list(e);
                    self.last_list_val = Some(list.clone());
                    let val = list.last().cloned().unwrap_or(Value::Undef);
                    self.last_expr_val = val.clone();
                    Flow::Return(val)
                } else {
                    Flow::Return(Value::Undef)
                }
            }

            Stmt::Block(stmts) | Stmt::BareBlock(stmts) => {
                self.push_scope();
                let flow = self.exec_stmts(stmts);
                self.pop_scope();
                flow
            }

            Stmt::NamedBlock(label, stmts) => {
                self.push_scope();
                let flow = self.exec_stmts(stmts);
                self.pop_scope();
                match flow {
                    Flow::Last(Some(ref l)) if l == label => Flow::None,
                    Flow::Next(Some(ref l)) if l == label => Flow::None,
                    other => other,
                }
            }

            Stmt::Sub { name, params, body } => {
                if !name.is_empty() {
                    self.subs
                        .insert(name.clone(), (params.clone(), body.clone()));
                }
                Flow::None
            }

            Stmt::My(vars) => {
                // Check for list destructuring: my ($a, $b, $c) = @_
                let has_list_init = vars.len() > 1 && vars[0].1.is_some();
                if has_list_init {
                    let init_expr = vars[0].1.as_ref().unwrap();
                    let items = self.eval_list(init_expr);
                    for (i, (name, _)) in vars.iter().enumerate() {
                        let var_name = name
                            .trim_start_matches('$')
                            .trim_start_matches('@')
                            .trim_start_matches('%');
                        if name.starts_with('@') {
                            let start = i.min(items.len());
                            self.set_my_array(var_name, items[start..].to_vec());
                        } else if name.starts_with('%') {
                            let start = i.min(items.len());
                            self.set_hash_from_list(var_name, items[start..].to_vec());
                        } else {
                            let val = items.get(i).cloned().unwrap_or(Value::Undef);
                            self.set_my_var(var_name, val);
                        }
                    }
                } else {
                    for (name, init) in vars {
                        let var_name = name
                            .trim_start_matches('$')
                            .trim_start_matches('@')
                            .trim_start_matches('%');
                        if name.starts_with('@') {
                            let items = if init.is_some() {
                                self.eval_list(init.as_ref().unwrap())
                            } else {
                                Vec::new()
                            };
                            self.set_my_array(var_name, items);
                        } else if name.starts_with('%') {
                            let items = if init.is_some() {
                                self.eval_list(init.as_ref().unwrap())
                            } else {
                                Vec::new()
                            };
                            self.set_hash_from_list(var_name, items);
                        } else {
                            let val = init
                                .as_ref()
                                .map(|e| self.eval_expr(e))
                                .unwrap_or(Value::Undef);
                            self.set_my_var(var_name, val);
                        }
                    }
                }
                Flow::None
            }

            Stmt::Local(vars) => {
                for (name, init) in vars {
                    let var_name = name
                        .trim_start_matches('$')
                        .trim_start_matches('@')
                        .trim_start_matches('%');
                    // Save current value
                    let old = self.get_var(var_name);
                    if let Some(saves) = self.local_saves.last_mut() {
                        saves.push((var_name.to_string(), old));
                    }
                    let val = init
                        .as_ref()
                        .map(|e| self.eval_expr(e))
                        .unwrap_or(Value::Undef);
                    if name.starts_with('@') {
                        let items = if init.is_some() {
                            self.eval_list(init.as_ref().unwrap())
                        } else {
                            Vec::new()
                        };
                        self.globals.arrays.insert(var_name.to_string(), items);
                    } else {
                        self.globals.vars.insert(var_name.to_string(), val);
                    }
                }
                Flow::None
            }

            Stmt::Our(vars) => {
                for (name, init) in vars {
                    let var_name = name
                        .trim_start_matches('$')
                        .trim_start_matches('@')
                        .trim_start_matches('%');
                    let val = init
                        .as_ref()
                        .map(|e| self.eval_expr(e))
                        .unwrap_or(Value::Undef);
                    self.globals.vars.insert(var_name.to_string(), val);
                }
                Flow::None
            }

            Stmt::Package(name) => {
                self.package = name.clone();
                Flow::None
            }

            Stmt::Use(module, _args) => {
                // Silently ignore use statements for now
                // Could handle 'use strict', 'use warnings' etc.
                let _ = module;
                Flow::None
            }

            Stmt::Require(expr) => {
                let filename = self.eval_expr(expr).to_str();
                let result = self.do_require(&filename);
                if result.is_undef() {
                    // require failed — die
                    let err = self.get_var("@").to_str();
                    if !err.is_empty() {
                        return Flow::Die(err);
                    }
                }
                Flow::None
            }

            Stmt::Begin(body) => {
                self.exec_stmts(body);
                Flow::None
            }
            Stmt::End(body) => {
                self.end_blocks.push(body.clone());
                Flow::None
            }

            Stmt::Die(args) => {
                let msg = if args.is_empty() {
                    "Died".to_string()
                } else {
                    args.iter()
                        .map(|a| self.eval_expr(a).to_str())
                        .collect::<Vec<_>>()
                        .join("")
                };
                Flow::Die(msg)
            }

            Stmt::Warn(args) => {
                let msg = if args.is_empty() {
                    "Warning: something's wrong".to_string()
                } else {
                    args.iter()
                        .map(|a| self.eval_expr(a).to_str())
                        .collect::<Vec<_>>()
                        .join("")
                };
                eprint!("{msg}");
                if !msg.ends_with('\n') {
                    eprintln!();
                }
                Flow::None
            }

            Stmt::Eval(arg) => match arg.as_ref() {
                EvalArg::Block(body) => {
                    self.set_global_var("@", Value::Str(String::new()));
                    self.push_scope();
                    let flow = self.exec_stmts(body);
                    self.pop_scope();
                    match flow {
                        Flow::Die(msg) => {
                            self.set_global_var("@", Value::Str(msg));
                            Flow::None
                        }
                        other => other,
                    }
                }
                EvalArg::Expr(expr) => {
                    let code = self.eval_expr(expr).to_str();
                    self.eval_string(&code);
                    Flow::None
                }
            },

            Stmt::PostfixIf(stmt, cond) => {
                let val = self.eval_expr(cond);
                self.last_expr_val = val.clone();
                if val.to_bool() {
                    self.exec_stmt(stmt)
                } else {
                    Flow::None
                }
            }
            Stmt::PostfixUnless(stmt, cond) => {
                let val = self.eval_expr(cond);
                self.last_expr_val = val.clone();
                if !val.to_bool() {
                    self.exec_stmt(stmt)
                } else {
                    Flow::None
                }
            }
            Stmt::PostfixWhile(stmt, cond) => {
                // Check for do { BLOCK } while COND — always execute once first
                let is_do_block = matches!(stmt.as_ref(), Stmt::Expr(Expr::DoBlock(_)));
                if is_do_block {
                    loop {
                        match self.exec_stmt(stmt) {
                            Flow::Last(_) => break,
                            Flow::Next(_) => {}
                            Flow::None => {}
                            other => return other,
                        }
                        if !self.eval_expr(cond).to_bool() {
                            break;
                        }
                    }
                } else {
                    loop {
                        if !self.eval_expr(cond).to_bool() {
                            break;
                        }
                        match self.exec_stmt(stmt) {
                            Flow::Last(_) => break,
                            Flow::Next(_) => continue,
                            Flow::None => {}
                            other => return other,
                        }
                    }
                }
                Flow::None
            }
            Stmt::PostfixUntil(stmt, cond) => {
                let is_do_block = matches!(stmt.as_ref(), Stmt::Expr(Expr::DoBlock(_)));
                if is_do_block {
                    loop {
                        match self.exec_stmt(stmt) {
                            Flow::Last(_) => break,
                            Flow::None => {}
                            other => return other,
                        }
                        if self.eval_expr(cond).to_bool() {
                            break;
                        }
                    }
                } else {
                    loop {
                        if self.eval_expr(cond).to_bool() {
                            break;
                        }
                        match self.exec_stmt(stmt) {
                            Flow::Last(_) => break,
                            Flow::None => {}
                            other => return other,
                        }
                    }
                }
                Flow::None
            }
            Stmt::PostfixFor(stmt, list) => {
                let items = self.eval_list(list);
                for item in items {
                    self.set_var("_", item);
                    match self.exec_stmt(stmt) {
                        Flow::Last(_) => break,
                        Flow::None => {}
                        other => return other,
                    }
                }
                Flow::None
            }

            _ => Flow::None,
        }
    }

    fn exec_print(&mut self, fh: &Option<Expr>, args: &[Expr], add_newline: bool) {
        let fh_name = fh.as_ref().map(|e| self.eval_expr(e).to_str());

        if args.is_empty() {
            // print with no args prints $_
            let val = self.get_var("_").to_str();
            self.write_to_handle(&fh_name, &val);
        } else {
            let sep = self.get_var(",").to_str();
            // Expand arrays in list context
            let mut values: Vec<String> = Vec::new();
            for arg in args {
                match arg {
                    Expr::ArrayVar(name) => {
                        let arr = self.get_array(name);
                        for v in &arr {
                            values.push(v.to_str());
                        }
                    }
                    _ => {
                        values.push(self.eval_expr(arg).to_str());
                    }
                }
            }
            let mut output = String::new();
            for (i, val) in values.iter().enumerate() {
                if i > 0 && !sep.is_empty() {
                    output.push_str(&sep);
                }
                output.push_str(val);
            }
            self.write_to_handle(&fh_name, &output);
        }

        if add_newline {
            self.write_to_handle(&fh_name, "\n");
        } else {
            // $\ output record separator
            let ors = self.get_var("\\").to_str();
            if !ors.is_empty() {
                self.write_to_handle(&fh_name, &ors);
            }
        }
    }

    fn write_to_handle(&mut self, fh_name: &Option<String>, text: &str) {
        match fh_name.as_deref() {
            Some("STDERR") => {
                let _ = io::stderr().write_all(text.as_bytes());
            }
            Some("STDOUT") | None => {
                let _ = io::stdout().write_all(text.as_bytes());
            }
            Some(name) => {
                // Try writing to a file handle
                if let Some(writer) = self.write_handles.get_mut(name) {
                    let _ = writer.write_all(text.as_bytes());
                } else {
                    // Fall back to stdout
                    let _ = io::stdout().write_all(text.as_bytes());
                }
            }
        }
    }

    // --- Expression evaluation ---

    pub fn eval_expr(&mut self, expr: &Expr) -> Value {
        match expr {
            Expr::IntLit(n) => Value::Num(*n as f64),
            Expr::FloatLit(n) => Value::Num(*n),
            Expr::StringLit(s) => Value::Str(s.clone()),
            Expr::Undef => Value::Undef,
            Expr::QW(words) => {
                // In scalar context, returns last element
                if let Some(last) = words.last() {
                    Value::Str(last.clone())
                } else {
                    Value::Undef
                }
            }

            Expr::ScalarVar(name) => self.get_var(name),
            Expr::ArrayVar(name) => {
                // In scalar context, returns array length
                let arr = self.get_array(name);
                Value::Num(arr.len() as f64)
            }
            Expr::HashVar(name) => {
                // In scalar context, returns hash info string
                Value::Str(String::new())
            }
            Expr::ArrayElement(name, index) => {
                let idx = self.eval_expr(index).to_num() as i64;
                let arr = self.get_array(name);
                let idx = if idx < 0 {
                    (arr.len() as i64 + idx).max(0) as usize
                } else {
                    idx as usize
                };
                arr.get(idx).cloned().unwrap_or(Value::Undef)
            }
            Expr::HashElement(name, key) => {
                let key_str = self.eval_expr(key).to_str();
                self.get_hash_element(name, &key_str)
            }
            Expr::ArrayLen(name) => {
                let arr = self.get_array(name);
                Value::Num((arr.len() as i64 - 1) as f64)
            }

            Expr::BinOp(op, left, right) => self.eval_binop(op, left, right),

            Expr::UnaryOp(op, expr) => self.eval_unaryop(op, expr),

            Expr::PostfixOp(op, expr) => {
                let val = self.eval_expr(expr);
                let num = val.to_num();
                let new_val = match op {
                    PostfixOp::Inc => Value::Num(num + 1.0),
                    PostfixOp::Dec => Value::Num(num - 1.0),
                };
                self.assign_to(expr, new_val);
                Value::Num(num) // return old value
            }

            Expr::Assign(target, value) => {
                // Check if target is an array — need list context for RHS
                if matches!(target.as_ref(), Expr::ArrayVar(_)) {
                    let items = self.eval_list(value);
                    if let Expr::ArrayVar(name) = target.as_ref() {
                        let len = items.len();
                        self.set_array(name, items);
                        return Value::Num(len as f64);
                    }
                }
                // Check if target is a hash — need list context for RHS
                if matches!(target.as_ref(), Expr::HashVar(_)) {
                    let items = self.eval_list(value);
                    if let Expr::HashVar(name) = target.as_ref() {
                        self.set_hash_from_list(name, items);
                        return Value::Num(0.0); // hash in scalar context
                    }
                }
                let val = self.eval_expr(value);
                self.assign_to(target, val.clone());
                val
            }

            Expr::OpAssign(op, target, value) => {
                let left = self.eval_expr(target);
                let right = self.eval_expr(value);
                let result = self.apply_binop(op, &left, &right);
                self.assign_to(target, result.clone());
                result
            }

            Expr::RegexMatch(expr, pat, flags) => {
                let text = self.eval_expr(expr).to_str();
                let matched = self.regex_match(&text, pat, flags);
                Value::Num(if matched { 1.0 } else { 0.0 })
            }

            Expr::RegexNotMatch(expr, pat, flags) => {
                let text = self.eval_expr(expr).to_str();
                let matched = self.regex_match(&text, pat, flags);
                Value::Num(if matched { 0.0 } else { 1.0 })
            }

            Expr::Substitution(target, pat, repl, flags) => {
                let text = self.eval_expr(target).to_str();
                let case_insensitive = flags.contains('i');
                let global = flags.contains('g');
                let pat_str = if case_insensitive {
                    format!("(?i){}", pat)
                } else {
                    pat.clone()
                };
                match regex::Regex::new(&pat_str) {
                    Ok(re) => {
                        // Process replacement: handle \-escaped sequences
                        let mut replacement = String::new();
                        let repl_bytes: Vec<char> = repl.chars().collect();
                        let mut ri = 0;
                        while ri < repl_bytes.len() {
                            if repl_bytes[ri] == '\\' && ri + 1 < repl_bytes.len() {
                                match repl_bytes[ri + 1] {
                                    'n' => {
                                        replacement.push('\n');
                                        ri += 2;
                                    }
                                    't' => {
                                        replacement.push('\t');
                                        ri += 2;
                                    }
                                    '\\' => {
                                        replacement.push('\\');
                                        ri += 2;
                                    }
                                    '#' => {
                                        replacement.push('#');
                                        ri += 2;
                                    }
                                    _ => {
                                        replacement.push(repl_bytes[ri]);
                                        ri += 1;
                                    }
                                }
                            } else {
                                replacement.push(repl_bytes[ri]);
                                ri += 1;
                            }
                        }

                        // Helper closure to expand $N and ${N} references in replacement
                        let expand_replacement = |caps: &regex::Captures,
                                                  replacement: &str|
                         -> String {
                            let mut result = String::new();
                            let repl_chars: Vec<char> = replacement.chars().collect();
                            let mut i = 0;
                            while i < repl_chars.len() {
                                if repl_chars[i] == '$' && i + 1 < repl_chars.len() {
                                    if repl_chars[i + 1] == '{' {
                                        // ${N} reference
                                        let mut num_str = String::new();
                                        i += 2;
                                        while i < repl_chars.len() && repl_chars[i] != '}' {
                                            num_str.push(repl_chars[i]);
                                            i += 1;
                                        }
                                        if i < repl_chars.len() {
                                            i += 1; // skip }
                                        }
                                        if let Ok(n) = num_str.parse::<usize>() {
                                            if let Some(m) = caps.get(n) {
                                                result.push_str(m.as_str());
                                            }
                                        }
                                    } else if repl_chars[i + 1].is_ascii_digit() {
                                        // $N reference (multi-digit)
                                        let mut num_str = String::new();
                                        i += 1;
                                        while i < repl_chars.len() && repl_chars[i].is_ascii_digit()
                                        {
                                            num_str.push(repl_chars[i]);
                                            i += 1;
                                        }
                                        if let Ok(n) = num_str.parse::<usize>() {
                                            if let Some(m) = caps.get(n) {
                                                result.push_str(m.as_str());
                                            }
                                        }
                                    } else {
                                        result.push(repl_chars[i]);
                                        i += 1;
                                    }
                                } else if repl_chars[i] == '&' {
                                    // & means whole match
                                    if let Some(m) = caps.get(0) {
                                        result.push_str(m.as_str());
                                    }
                                    i += 1;
                                } else {
                                    result.push(repl_chars[i]);
                                    i += 1;
                                }
                            }
                            result
                        };

                        // Store capture groups from first match
                        if let Some(caps) = re.captures(&text) {
                            for i in 1..caps.len() {
                                if let Some(m) = caps.get(i) {
                                    self.set_global_var(
                                        &i.to_string(),
                                        Value::Str(m.as_str().to_string()),
                                    );
                                } else {
                                    self.set_global_var(&i.to_string(), Value::Undef);
                                }
                            }
                        }

                        let (new_text, count) = if global {
                            let mut count = 0u64;
                            let new = re.replace_all(&text, |caps: &regex::Captures| {
                                count += 1;
                                expand_replacement(caps, &replacement)
                            });
                            (new.into_owned(), count)
                        } else {
                            if re.is_match(&text) {
                                let new = re.replace(&text, |caps: &regex::Captures| {
                                    expand_replacement(caps, &replacement)
                                });
                                (new.into_owned(), 1)
                            } else {
                                (text, 0)
                            }
                        };
                        // Assign modified text back to the target variable
                        self.assign_to(target, Value::Str(new_text));
                        Value::Num(count as f64)
                    }
                    Err(_) => Value::Num(0.0),
                }
            }

            Expr::Ternary(cond, then, else_) => {
                if self.eval_expr(cond).to_bool() {
                    self.eval_expr(then)
                } else {
                    self.eval_expr(else_)
                }
            }

            Expr::Call(name, args) => self.eval_call(name, args),

            Expr::Defined(expr) => {
                let val = self.eval_expr(expr);
                Value::Num(if val.is_undef() { 0.0 } else { 1.0 })
            }

            Expr::Ref(expr) => {
                // Simplified reference implementation
                Value::Str("REF".to_string())
            }

            Expr::ArrayRef(items) => {
                let vals: Vec<Value> = items.iter().map(|e| self.eval_expr(e)).collect();
                // Store and return ref
                Value::Str("ARRAY_REF".to_string())
            }

            Expr::HashRef(pairs) => Value::Str("HASH_REF".to_string()),

            Expr::ArrayLit(items) => {
                // In scalar context, return last element
                if let Some(last) = items.last() {
                    self.eval_expr(last)
                } else {
                    Value::Undef
                }
            }

            Expr::Range(start, end) => {
                // In scalar context, returns flip-flop (just return end for now)
                self.eval_expr(end)
            }

            Expr::DoBlock(stmts) => {
                self.push_scope();
                for stmt in stmts {
                    match self.exec_stmt(stmt) {
                        Flow::Return(v) => {
                            self.pop_scope();
                            return v;
                        }
                        Flow::Die(msg) => {
                            self.pop_scope();
                            self.set_global_var("@", Value::Str(msg));
                            return Value::Undef;
                        }
                        _ => {}
                    }
                }
                let result = self.last_expr_val.clone();
                self.pop_scope();
                result
            }

            Expr::Diamond(name) => {
                // <FH> reads a line from the named filehandle
                self.readline(name)
            }

            Expr::Backtick(cmd) => self.run_backtick(cmd),

            Expr::BacktickInterp(expr) => {
                let cmd = self.eval_expr(expr).to_str();
                self.run_backtick(&cmd)
            }

            Expr::MyVar(name) => {
                // Declare in current scope, return undef initially
                let clean_name = name.trim_start_matches('$');
                self.declare_my(clean_name);
                Value::Undef
            }

            Expr::LocalVar(name) => {
                let clean_name = name.trim_start_matches('$').trim_start_matches('@');
                Value::Undef
            }

            Expr::FileTest(op, expr) => {
                let path = self.eval_expr(expr).to_str();
                let result = match op.as_str() {
                    "-e" => std::path::Path::new(&path).exists(),
                    "-f" => std::path::Path::new(&path).is_file(),
                    "-d" => std::path::Path::new(&path).is_dir(),
                    "-r" => std::path::Path::new(&path).exists(), // simplified
                    "-w" => std::path::Path::new(&path).exists(), // simplified
                    "-s" => std::fs::metadata(&path)
                        .map(|m| m.len() > 0)
                        .unwrap_or(false),
                    "-c" => false, // char device — simplified
                    _ => false,
                };
                Value::Num(if result { 1.0 } else { 0.0 })
            }

            Expr::Wantarray => Value::Undef,

            Expr::Interp(parts) => {
                let mut result = String::new();
                for part in parts {
                    match part {
                        InterpPart::Lit(s) => result.push_str(s),
                        InterpPart::ScalarVar(name) => {
                            result.push_str(&self.get_var(name).to_str());
                        }
                        InterpPart::ArrayVar(name) => {
                            let arr = self.get_array(name);
                            let sep = self.get_var(" ").to_str(); // $" default is space
                            let s: Vec<String> = arr.iter().map(|v| v.to_str()).collect();
                            result.push_str(&s.join(if sep.is_empty() { " " } else { &sep }));
                        }
                        InterpPart::ArrayElement(name, idx) => {
                            let idx_val = self.eval_expr(idx);
                            let idx = idx_val.to_num() as i64;
                            let arr = self.get_array(name);
                            let idx = if idx < 0 {
                                (arr.len() as i64 + idx).max(0) as usize
                            } else {
                                idx as usize
                            };
                            if let Some(val) = arr.get(idx) {
                                result.push_str(&val.to_str());
                            }
                        }
                        InterpPart::HashElement(name, key) => {
                            let key_str = self.eval_expr(key).to_str();
                            result.push_str(&self.get_hash_element(name, &key_str).to_str());
                        }
                        InterpPart::Expr(expr) => {
                            result.push_str(&self.eval_expr(expr).to_str());
                        }
                    }
                }
                Value::Str(result)
            }

            _ => Value::Undef,
        }
    }

    fn eval_binop(&mut self, op: &BinOp, left: &Expr, right: &Expr) -> Value {
        // Short-circuit operators
        match op {
            BinOp::LogAnd | BinOp::And => {
                let l = self.eval_expr(left);
                if !l.to_bool() {
                    return l;
                }
                return self.eval_expr(right);
            }
            BinOp::LogOr | BinOp::Or => {
                let l = self.eval_expr(left);
                if l.to_bool() {
                    return l;
                }
                return self.eval_expr(right);
            }
            BinOp::DefOr => {
                let l = self.eval_expr(left);
                if !l.is_undef() {
                    return l;
                }
                return self.eval_expr(right);
            }
            _ => {}
        }

        let l = self.eval_expr(left);
        let r = self.eval_expr(right);
        self.apply_binop(op, &l, &r)
    }

    fn apply_binop(&self, op: &BinOp, l: &Value, r: &Value) -> Value {
        match op {
            BinOp::Add => Value::Num(l.to_num() + r.to_num()),
            BinOp::Sub => Value::Num(l.to_num() - r.to_num()),
            BinOp::Mul => Value::Num(l.to_num() * r.to_num()),
            BinOp::Div => {
                let d = r.to_num();
                if d == 0.0 {
                    eprintln!("Illegal division by zero");
                    Value::Undef
                } else {
                    Value::Num(l.to_num() / d)
                }
            }
            BinOp::Mod => {
                let d = r.to_num();
                if d == 0.0 {
                    eprintln!("Illegal modulus zero");
                    Value::Undef
                } else {
                    // Perl's modulo matches sign of right operand for integers
                    let a = l.to_num();
                    let b = d;
                    let result = a % b;
                    // Perl adjusts: if result != 0 and signs differ, add b
                    if result != 0.0 && (result > 0.0) != (b > 0.0) {
                        Value::Num(result + b)
                    } else {
                        Value::Num(result)
                    }
                }
            }
            BinOp::Pow => Value::Num(l.to_num().powf(r.to_num())),
            BinOp::Concat => Value::Str(format!("{}{}", l.to_str(), r.to_str())),
            BinOp::Repeat => {
                let s = l.to_str();
                let n = r.to_num() as usize;
                Value::Str(s.repeat(n))
            }

            BinOp::NumEq => Value::Num(if l.to_num() == r.to_num() { 1.0 } else { 0.0 }),
            BinOp::NumNe => Value::Num(if l.to_num() != r.to_num() { 1.0 } else { 0.0 }),
            BinOp::NumLt => Value::Num(if l.to_num() < r.to_num() { 1.0 } else { 0.0 }),
            BinOp::NumGt => Value::Num(if l.to_num() > r.to_num() { 1.0 } else { 0.0 }),
            BinOp::NumLe => Value::Num(if l.to_num() <= r.to_num() { 1.0 } else { 0.0 }),
            BinOp::NumGe => Value::Num(if l.to_num() >= r.to_num() { 1.0 } else { 0.0 }),
            BinOp::Spaceship => {
                let a = l.to_num();
                let b = r.to_num();
                Value::Num(if a < b {
                    -1.0
                } else if a > b {
                    1.0
                } else {
                    0.0
                })
            }

            BinOp::StrEq => Value::Num(if l.to_str() == r.to_str() { 1.0 } else { 0.0 }),
            BinOp::StrNe => Value::Num(if l.to_str() != r.to_str() { 1.0 } else { 0.0 }),
            BinOp::StrLt => Value::Num(if l.to_str() < r.to_str() { 1.0 } else { 0.0 }),
            BinOp::StrGt => Value::Num(if l.to_str() > r.to_str() { 1.0 } else { 0.0 }),
            BinOp::StrLe => Value::Num(if l.to_str() <= r.to_str() { 1.0 } else { 0.0 }),
            BinOp::StrGe => Value::Num(if l.to_str() >= r.to_str() { 1.0 } else { 0.0 }),
            BinOp::StrCmp => {
                let a = l.to_str();
                let b = r.to_str();
                Value::Num(match a.cmp(&b) {
                    std::cmp::Ordering::Less => -1.0,
                    std::cmp::Ordering::Equal => 0.0,
                    std::cmp::Ordering::Greater => 1.0,
                })
            }

            BinOp::BitAnd => Value::Num((l.to_num() as i64 & r.to_num() as i64) as f64),
            BinOp::BitOr => Value::Num((l.to_num() as i64 | r.to_num() as i64) as f64),
            BinOp::BitXor => Value::Num((l.to_num() as i64 ^ r.to_num() as i64) as f64),
            BinOp::ShiftLeft => Value::Num(((l.to_num() as i64) << (r.to_num() as u32)) as f64),
            BinOp::ShiftRight => Value::Num(((l.to_num() as i64) >> (r.to_num() as u32)) as f64),

            BinOp::Range => {
                // In scalar context, returns the right value
                r.clone()
            }

            _ => Value::Undef,
        }
    }

    fn eval_unaryop(&mut self, op: &UnaryOp, expr: &Expr) -> Value {
        match op {
            UnaryOp::Neg => {
                let val = self.eval_expr(expr);
                Value::Num(-val.to_num())
            }
            UnaryOp::Pos => {
                let val = self.eval_expr(expr);
                Value::Num(val.to_num())
            }
            UnaryOp::LogNot => {
                let val = self.eval_expr(expr);
                Value::Num(if val.to_bool() { 0.0 } else { 1.0 })
            }
            UnaryOp::Not => {
                let val = self.eval_expr(expr);
                Value::Num(if val.to_bool() { 0.0 } else { 1.0 })
            }
            UnaryOp::BitNot => {
                let val = self.eval_expr(expr);
                Value::Num(!(val.to_num() as i64) as f64)
            }
            UnaryOp::PreInc => {
                let val = self.eval_expr(expr);
                let new_val = Value::Num(val.to_num() + 1.0);
                self.assign_to(expr, new_val.clone());
                new_val
            }
            UnaryOp::PreDec => {
                let val = self.eval_expr(expr);
                let new_val = Value::Num(val.to_num() - 1.0);
                self.assign_to(expr, new_val.clone());
                new_val
            }
            UnaryOp::Ref => Value::Str("REF".to_string()),
        }
    }

    fn eval_call(&mut self, name: &str, args: &[Expr]) -> Value {
        match name {
            "print" => {
                // print in expression context
                let mut output = String::new();
                let sep = self.get_var(",").to_str();
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 && !sep.is_empty() {
                        output.push_str(&sep);
                    }
                    output.push_str(&self.eval_expr(arg).to_str());
                }
                self.write_to_handle(&None, &output);
                let ors = self.get_var("\\").to_str();
                if !ors.is_empty() {
                    self.write_to_handle(&None, &ors);
                }
                Value::Num(1.0)
            }
            "say" => {
                let mut output = String::new();
                let sep = self.get_var(",").to_str();
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 && !sep.is_empty() {
                        output.push_str(&sep);
                    }
                    output.push_str(&self.eval_expr(arg).to_str());
                }
                output.push('\n');
                self.write_to_handle(&None, &output);
                Value::Num(1.0)
            }
            "die" => {
                let msg = args
                    .iter()
                    .map(|a| self.eval_expr(a).to_str())
                    .collect::<Vec<_>>()
                    .join("");
                self.set_global_var("@", Value::Str(msg.clone()));
                // In expression context, die should propagate
                Value::Undef // caller should check $@
            }
            "warn" => {
                let msg = args
                    .iter()
                    .map(|a| self.eval_expr(a).to_str())
                    .collect::<Vec<_>>()
                    .join("");
                eprint!("{msg}");
                if !msg.ends_with('\n') {
                    eprintln!();
                }
                Value::Num(1.0)
            }
            "scalar" => {
                // scalar() forces scalar context
                if let Some(arg) = args.first() {
                    self.eval_expr(arg)
                } else {
                    Value::Undef
                }
            }
            "abs" => {
                let val = if args.is_empty() {
                    self.get_var("_")
                } else {
                    self.eval_expr(&args[0])
                };
                Value::Num(val.to_num().abs())
            }
            "int" => {
                let val = if args.is_empty() {
                    self.get_var("_")
                } else {
                    self.eval_expr(&args[0])
                };
                let n = val.to_num();
                Value::Num(if n >= 0.0 { n.floor() } else { n.ceil() })
            }
            "length" => {
                let val = if args.is_empty() {
                    self.get_var("_")
                } else {
                    self.eval_expr(&args[0])
                };
                Value::Num(val.to_str().len() as f64)
            }
            "chr" => {
                let val = if args.is_empty() {
                    self.get_var("_")
                } else {
                    self.eval_expr(&args[0])
                };
                let n = val.to_num() as u32;
                Value::Str(char::from_u32(n).unwrap_or('\0').to_string())
            }
            "ord" => {
                let val = if args.is_empty() {
                    self.get_var("_")
                } else {
                    self.eval_expr(&args[0])
                };
                let s = val.to_str();
                Value::Num(s.chars().next().map(|c| c as u32 as f64).unwrap_or(0.0))
            }
            "lc" => {
                let val = if args.is_empty() {
                    self.get_var("_")
                } else {
                    self.eval_expr(&args[0])
                };
                Value::Str(val.to_str().to_lowercase())
            }
            "uc" => {
                let val = if args.is_empty() {
                    self.get_var("_")
                } else {
                    self.eval_expr(&args[0])
                };
                Value::Str(val.to_str().to_uppercase())
            }
            "chomp" => {
                let val = if args.is_empty() {
                    self.get_var("_")
                } else {
                    self.eval_expr(&args[0])
                };
                let mut s = val.to_str();
                let removed = if s.ends_with('\n') {
                    s.pop();
                    1.0
                } else {
                    0.0
                };
                if args.is_empty() {
                    self.set_var("_", Value::Str(s));
                } else {
                    self.assign_to(&args[0], Value::Str(s));
                }
                Value::Num(removed)
            }
            "chop" => {
                let val = if args.is_empty() {
                    self.get_var("_")
                } else {
                    self.eval_expr(&args[0])
                };
                let mut s = val.to_str();
                let ch = s.pop().map(|c| c.to_string()).unwrap_or_default();
                if args.is_empty() {
                    self.set_var("_", Value::Str(s));
                } else {
                    self.assign_to(&args[0], Value::Str(s));
                }
                Value::Str(ch)
            }
            "substr" => {
                let s = self.eval_expr(&args[0]).to_str();
                let offset = self.eval_expr(&args[1]).to_num() as i64;
                let len = if args.len() > 2 {
                    Some(self.eval_expr(&args[2]).to_num() as usize)
                } else {
                    None
                };

                let start = if offset < 0 {
                    (s.len() as i64 + offset).max(0) as usize
                } else {
                    offset as usize
                };

                let result = if let Some(len) = len {
                    s.get(start..start + len).unwrap_or("").to_string()
                } else {
                    s.get(start..).unwrap_or("").to_string()
                };
                Value::Str(result)
            }
            "index" => {
                let s = self.eval_expr(&args[0]).to_str();
                let substr = self.eval_expr(&args[1]).to_str();
                let pos = if args.len() > 2 {
                    self.eval_expr(&args[2]).to_num() as usize
                } else {
                    0
                };
                Value::Num(
                    s[pos..]
                        .find(&substr)
                        .map(|i| (i + pos) as f64)
                        .unwrap_or(-1.0),
                )
            }
            "rindex" => {
                let s = self.eval_expr(&args[0]).to_str();
                let substr = self.eval_expr(&args[1]).to_str();
                Value::Num(s.rfind(&substr).map(|i| i as f64).unwrap_or(-1.0))
            }
            "join" => {
                let sep = self.eval_expr(&args[0]).to_str();
                let items: Vec<String> = args[1..]
                    .iter()
                    .flat_map(|a| self.eval_list(a))
                    .map(|v| v.to_str())
                    .collect();
                Value::Str(items.join(&sep))
            }
            "split" => {
                let pat = if args.is_empty() {
                    " ".to_string()
                } else {
                    self.eval_expr(&args[0]).to_str()
                };
                let text = if args.len() > 1 {
                    self.eval_expr(&args[1]).to_str()
                } else {
                    self.get_var("_").to_str()
                };
                let limit = if args.len() > 2 {
                    self.eval_expr(&args[2]).to_num() as i64
                } else {
                    -1
                };

                let parts: Vec<Value> = if pat == " " {
                    text.split_whitespace()
                        .map(|s| Value::Str(s.to_string()))
                        .collect()
                } else {
                    text.split(&pat)
                        .map(|s| Value::Str(s.to_string()))
                        .collect()
                };
                Value::Str(parts.len().to_string()) // In scalar context
            }
            "sprintf" => {
                if args.is_empty() {
                    return Value::Str(String::new());
                }
                let fmt = self.eval_expr(&args[0]).to_str();
                let vals: Vec<Value> = args[1..].iter().map(|a| self.eval_expr(a)).collect();
                Value::Str(self.sprintf_impl(&fmt, &vals))
            }
            "push" => {
                if let Some(Expr::ArrayVar(name)) = args.first() {
                    let mut arr = self.get_array(name);
                    for arg in &args[1..] {
                        arr.push(self.eval_expr(arg));
                    }
                    let len = arr.len();
                    self.set_array(name, arr);
                    Value::Num(len as f64)
                } else {
                    Value::Undef
                }
            }
            "pop" => {
                if let Some(Expr::ArrayVar(name)) = args.first() {
                    let mut arr = self.get_array(name);
                    let val = arr.pop().unwrap_or(Value::Undef);
                    self.set_array(name, arr);
                    val
                } else {
                    Value::Undef
                }
            }
            "shift" => {
                if args.is_empty() {
                    // shift @_
                    let mut arr = self.get_array("_");
                    let val = if arr.is_empty() {
                        Value::Undef
                    } else {
                        arr.remove(0)
                    };
                    self.set_array("_", arr);
                    val
                } else if let Some(Expr::ArrayVar(name)) = args.first() {
                    let mut arr = self.get_array(name);
                    let val = if arr.is_empty() {
                        Value::Undef
                    } else {
                        arr.remove(0)
                    };
                    self.set_array(name, arr);
                    val
                } else {
                    Value::Undef
                }
            }
            "unshift" => {
                if let Some(Expr::ArrayVar(name)) = args.first() {
                    let mut arr = self.get_array(name);
                    let vals: Vec<Value> = args[1..].iter().map(|a| self.eval_expr(a)).collect();
                    for val in vals.into_iter().rev() {
                        arr.insert(0, val);
                    }
                    let len = arr.len();
                    self.set_array(name, arr);
                    Value::Num(len as f64)
                } else {
                    Value::Undef
                }
            }
            "reverse" => {
                let items: Vec<Value> = args
                    .iter()
                    .flat_map(|a| match a {
                        Expr::ArrayVar(name) => self.get_array(name),
                        _ => vec![self.eval_expr(a)],
                    })
                    .collect();
                let mut items = items;
                items.reverse();
                if items.len() == 1 {
                    // Scalar context: reverse string
                    Value::Str(items[0].to_str().chars().rev().collect())
                } else {
                    Value::Num(items.len() as f64) // scalar context
                }
            }
            "keys" => {
                if let Some(Expr::HashVar(name)) = args.first() {
                    let hash = self.get_hash(name);
                    Value::Num(hash.len() as f64) // scalar context
                } else {
                    Value::Undef
                }
            }
            "values" => {
                if let Some(Expr::HashVar(name)) = args.first() {
                    let hash = self.get_hash(name);
                    Value::Num(hash.len() as f64)
                } else {
                    Value::Undef
                }
            }
            "exists" => {
                // TODO: proper implementation
                Value::Num(0.0)
            }
            "delete" => Value::Undef,
            "defined" => {
                let val = if args.is_empty() {
                    self.get_var("_")
                } else {
                    self.eval_expr(&args[0])
                };
                Value::Num(if val.is_undef() { 0.0 } else { 1.0 })
            }
            "hex" => {
                let val = if args.is_empty() {
                    self.get_var("_")
                } else {
                    self.eval_expr(&args[0])
                };
                let s = val.to_str();
                let s = s.trim();
                let s = s
                    .strip_prefix("0x")
                    .or_else(|| s.strip_prefix("0X"))
                    .unwrap_or(s);
                Value::Num(i64::from_str_radix(s, 16).unwrap_or(0) as f64)
            }
            "oct" => {
                let val = if args.is_empty() {
                    self.get_var("_")
                } else {
                    self.eval_expr(&args[0])
                };
                let s = val.to_str();
                let s = s.trim();
                if s.starts_with("0x") || s.starts_with("0X") {
                    Value::Num(i64::from_str_radix(&s[2..], 16).unwrap_or(0) as f64)
                } else if s.starts_with("0b") || s.starts_with("0B") {
                    Value::Num(i64::from_str_radix(&s[2..], 2).unwrap_or(0) as f64)
                } else if s.starts_with('0') {
                    Value::Num(
                        i64::from_str_radix(s.trim_start_matches('0'), 8).unwrap_or(0) as f64,
                    )
                } else {
                    Value::Num(s.parse::<f64>().unwrap_or(0.0))
                }
            }
            "ref" => {
                Value::Str(String::new()) // simplified
            }
            "caller" => Value::Undef,
            "eof" => {
                // Check if filehandle is at EOF
                if let Some(arg) = args.first() {
                    let name = self.eval_expr(arg).to_str();
                    if let Some(reader) = self.read_handles.get_mut(&name) {
                        let buf = reader.fill_buf().unwrap_or(&[]);
                        Value::Num(if buf.is_empty() { 1.0 } else { 0.0 })
                    } else {
                        Value::Num(1.0)
                    }
                } else {
                    Value::Num(1.0)
                }
            }
            "open" => self.eval_open(args),
            "close" => self.eval_close(args),
            "binmode" => {
                // No-op — we don't distinguish binary/text mode
                Value::Num(1.0)
            }
            "unlink" => {
                let mut count = 0;
                for arg in args {
                    let path = self.eval_expr(arg).to_str();
                    if std::fs::remove_file(&path).is_ok() {
                        count += 1;
                    }
                }
                Value::Num(count as f64)
            }
            "rmdir" => {
                if let Some(arg) = args.first() {
                    let path = self.eval_expr(arg).to_str();
                    Value::Num(if std::fs::remove_dir(&path).is_ok() {
                        1.0
                    } else {
                        0.0
                    })
                } else {
                    Value::Num(0.0)
                }
            }
            "eval" => {
                if let Some(arg) = args.first() {
                    let code = self.eval_expr(arg).to_str();
                    self.eval_string(&code)
                } else {
                    Value::Undef
                }
            }
            "utf8::native_to_unicode" => {
                // On ASCII/UTF-8 systems, native_to_unicode is identity
                let val = self.eval_expr(&args[0]).to_num();
                Value::Num(val)
            }
            "utf8::unicode_to_native" => {
                let val = self.eval_expr(&args[0]).to_num();
                Value::Num(val)
            }
            "pack" => {
                // Simplified pack - handle "d" format for double
                let fmt = self.eval_expr(&args[0]).to_str();
                if fmt == "d" && args.len() > 1 {
                    let n = self.eval_expr(&args[1]).to_num();
                    let bytes = n.to_ne_bytes();
                    Value::Str(String::from_utf8_lossy(&bytes).to_string())
                } else {
                    Value::Str(String::new())
                }
            }
            "sort" => {
                // Simplified sort — return the list sorted
                let items: Vec<Value> = args
                    .iter()
                    .flat_map(|a| match a {
                        Expr::ArrayVar(name) => self.get_array(name),
                        _ => vec![self.eval_expr(a)],
                    })
                    .collect();
                let mut sorted: Vec<String> = items.iter().map(|v| v.to_str()).collect();
                sorted.sort();
                // In scalar context return count
                Value::Num(sorted.len() as f64)
            }
            "map" => {
                // map { BLOCK } LIST — first arg is the block, rest is the list
                if args.is_empty() {
                    return Value::Undef;
                }
                let block = &args[0];
                let items: Vec<Value> = args[1..]
                    .iter()
                    .flat_map(|a| match a {
                        Expr::ArrayVar(name) => self.get_array(name),
                        _ => vec![self.eval_expr(a)],
                    })
                    .collect();
                let mut results = Vec::new();
                for item in &items {
                    self.set_var("_", item.clone());
                    let result = self.eval_expr(block);
                    results.push(result);
                }
                // In scalar context, return count
                Value::Num(results.len() as f64)
            }
            "grep" => {
                if args.is_empty() {
                    return Value::Undef;
                }
                let block = &args[0];
                let items: Vec<Value> = args[1..]
                    .iter()
                    .flat_map(|a| match a {
                        Expr::ArrayVar(name) => self.get_array(name),
                        _ => vec![self.eval_expr(a)],
                    })
                    .collect();
                let mut results = Vec::new();
                for item in &items {
                    self.set_var("_", item.clone());
                    let result = self.eval_expr(block);
                    if result.to_bool() {
                        results.push(item.clone());
                    }
                }
                Value::Num(results.len() as f64)
            }
            "require" => {
                if let Some(arg) = args.first() {
                    let filename = self.eval_expr(arg).to_str();
                    self.do_require(&filename)
                } else {
                    Value::Undef
                }
            }
            "chdir" => {
                if let Some(arg) = args.first() {
                    let dir = self.eval_expr(arg).to_str();
                    let path = std::path::Path::new(&dir);
                    if std::env::set_current_dir(path).is_ok() {
                        Value::Num(1.0)
                    } else {
                        self.set_global_var("!", Value::Str(format!("No such file or directory")));
                        Value::Num(0.0)
                    }
                } else {
                    Value::Num(0.0)
                }
            }
            "set_up_inc" | "File::Spec::Functions::catdir" => {
                // No-op / stub for test harness helpers
                Value::Undef
            }
            "mkdir" => {
                if let Some(arg) = args.first() {
                    let dir = self.eval_expr(arg).to_str();
                    Value::Num(if std::fs::create_dir_all(&dir).is_ok() {
                        1.0
                    } else {
                        0.0
                    })
                } else {
                    Value::Num(0.0)
                }
            }
            _ => {
                // Check user-defined subs
                if let Some((params, body)) = self.subs.get(name).cloned() {
                    let arg_vals: Vec<Value> =
                        args.iter().flat_map(|a| self.eval_list(a)).collect();
                    self.call_sub(&body, &arg_vals)
                } else {
                    // Try package-qualified name
                    let qualified = format!("{}::{}", self.package, name);
                    if let Some((params, body)) = self.subs.get(&qualified).cloned() {
                        let arg_vals: Vec<Value> =
                            args.iter().flat_map(|a| self.eval_list(a)).collect();
                        self.call_sub(&body, &arg_vals)
                    } else {
                        Value::Undef
                    }
                }
            }
        }
    }

    fn call_sub(&mut self, body: &[Stmt], args: &[Value]) -> Value {
        self.push_scope();
        self.local_saves.push(Vec::new());
        self.local_array_saves.push(Vec::new());

        // Set @_ to args
        self.set_array("_", args.to_vec());

        // Save and reset last_expr_val and last_list_val
        let saved_last = std::mem::replace(&mut self.last_expr_val, Value::Undef);
        let saved_list = std::mem::take(&mut self.last_list_val);

        let mut return_val = None;
        for stmt in body {
            match self.exec_stmt(stmt) {
                Flow::Return(v) => {
                    return_val = Some(v);
                    break;
                }
                Flow::Die(msg) => {
                    self.last_expr_val = saved_last;
                    self.last_list_val = saved_list;
                    self.restore_locals();
                    self.pop_scope();
                    self.set_global_var("@", Value::Str(msg.clone()));
                    return Value::Undef;
                }
                Flow::None => {}
                _ => {}
            }
        }

        let result = return_val.unwrap_or_else(|| self.last_expr_val.clone());
        self.last_expr_val = saved_last;
        self.last_list_val = saved_list;
        self.restore_locals();
        self.pop_scope();
        result
    }

    /// Call a sub and return the list result (for list context)
    fn call_sub_list(&mut self, body: &[Stmt], args: &[Value]) -> Vec<Value> {
        self.push_scope();
        self.local_saves.push(Vec::new());
        self.local_array_saves.push(Vec::new());

        // Set @_ to args
        self.set_array("_", args.to_vec());

        // Save and reset last_expr_val and last_list_val
        let saved_last = std::mem::replace(&mut self.last_expr_val, Value::Undef);
        let saved_list = std::mem::take(&mut self.last_list_val);

        let mut return_val = None;
        for stmt in body {
            match self.exec_stmt(stmt) {
                Flow::Return(v) => {
                    return_val = Some(v);
                    break;
                }
                Flow::Die(msg) => {
                    self.last_expr_val = saved_last;
                    self.last_list_val = saved_list;
                    self.restore_locals();
                    self.pop_scope();
                    self.set_global_var("@", Value::Str(msg.clone()));
                    return vec![Value::Undef];
                }
                Flow::None => {}
                _ => {}
            }
        }

        let result = if return_val.is_some() {
            // Check if the return also set a list value
            if let Some(list) = self.last_list_val.take() {
                list
            } else {
                vec![return_val.unwrap()]
            }
        } else if let Some(list) = self.last_list_val.take() {
            list
        } else {
            vec![self.last_expr_val.clone()]
        };

        self.last_expr_val = saved_last;
        self.last_list_val = saved_list;
        self.restore_locals();
        self.pop_scope();
        result
    }

    fn restore_locals(&mut self) {
        if let Some(saves) = self.local_saves.pop() {
            for (name, val) in saves.into_iter().rev() {
                self.globals.vars.insert(name, val);
            }
        }
        if let Some(saves) = self.local_array_saves.pop() {
            for (name, val) in saves.into_iter().rev() {
                self.globals.arrays.insert(name, val);
            }
        }
    }

    // --- Variable access ---

    fn get_var(&self, name: &str) -> Value {
        // Check lexical scopes from innermost to outermost
        for scope in self.scopes.iter().rev() {
            if let Some(val) = scope.vars.get(name) {
                return val.clone();
            }
        }
        // Check globals
        self.globals.vars.get(name).cloned().unwrap_or(Value::Undef)
    }

    fn set_var(&mut self, name: &str, val: Value) {
        // Set in the innermost scope that has this variable, or create in global scope
        for scope in self.scopes.iter_mut().rev() {
            if scope.vars.contains_key(name) {
                scope.vars.insert(name.to_string(), val);
                return;
            }
        }
        // Variable not found in any lexical scope — set in globals (package variable)
        self.globals.vars.insert(name.to_string(), val);
    }

    /// Declare a `my` variable in the current lexical scope
    fn set_my_var(&mut self, name: &str, val: Value) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.vars.insert(name.to_string(), val);
        } else {
            self.globals.vars.insert(name.to_string(), val);
        }
    }

    /// Declare a `my` array in the current lexical scope
    fn set_my_array(&mut self, name: &str, arr: Vec<Value>) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.arrays.insert(name.to_string(), arr);
        } else {
            self.globals.arrays.insert(name.to_string(), arr);
        }
    }

    fn set_global_var(&mut self, name: &str, val: Value) {
        self.globals.vars.insert(name.to_string(), val);
    }

    fn declare_my(&mut self, name: &str) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.vars.entry(name.to_string()).or_insert(Value::Undef);
        }
    }

    fn get_array(&self, name: &str) -> Vec<Value> {
        for scope in self.scopes.iter().rev() {
            if let Some(arr) = scope.arrays.get(name) {
                return arr.clone();
            }
        }
        self.globals.arrays.get(name).cloned().unwrap_or_default()
    }

    fn set_array(&mut self, name: &str, arr: Vec<Value>) {
        for scope in self.scopes.iter_mut().rev() {
            if scope.arrays.contains_key(name) {
                scope.arrays.insert(name.to_string(), arr);
                return;
            }
        }
        // Not found in lexical scopes — set in globals
        self.globals.arrays.insert(name.to_string(), arr);
    }

    fn get_hash(&self, name: &str) -> HashMap<String, Value> {
        for scope in self.scopes.iter().rev() {
            if let Some(hash) = scope.hashes.get(name) {
                return hash.clone();
            }
        }
        self.globals.hashes.get(name).cloned().unwrap_or_default()
    }

    fn get_hash_element(&self, name: &str, key: &str) -> Value {
        for scope in self.scopes.iter().rev() {
            if let Some(hash) = scope.hashes.get(name) {
                return hash.get(key).cloned().unwrap_or(Value::Undef);
            }
        }
        self.globals
            .hashes
            .get(name)
            .and_then(|h| h.get(key))
            .cloned()
            .unwrap_or(Value::Undef)
    }

    fn set_hash_element(&mut self, name: &str, key: &str, val: Value) {
        for scope in self.scopes.iter_mut().rev() {
            if scope.hashes.contains_key(name) {
                scope
                    .hashes
                    .entry(name.to_string())
                    .or_default()
                    .insert(key.to_string(), val);
                return;
            }
        }
        // Not found in lexical scopes — set in globals
        self.globals
            .hashes
            .entry(name.to_string())
            .or_default()
            .insert(key.to_string(), val);
    }

    fn set_hash_from_list(&mut self, name: &str, items: Vec<Value>) {
        let mut hash = HashMap::new();
        let mut iter = items.into_iter();
        while let Some(key) = iter.next() {
            let val = iter.next().unwrap_or(Value::Undef);
            hash.insert(key.to_str(), val);
        }
        if let Some(scope) = self.scopes.last_mut() {
            scope.hashes.insert(name.to_string(), hash);
        } else {
            self.globals.hashes.insert(name.to_string(), hash);
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(Scope::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    // --- Assignment ---

    fn assign_to(&mut self, target: &Expr, val: Value) {
        match target {
            Expr::ScalarVar(name) => self.set_var(name, val),
            Expr::MyVar(name) => {
                if let Some(scope) = self.scopes.last_mut() {
                    scope.vars.insert(name.clone(), val);
                } else {
                    self.globals.vars.insert(name.clone(), val);
                }
            }
            Expr::LocalVar(name) => {
                self.globals.vars.insert(name.clone(), val);
            }
            Expr::ArrayElement(name, index) => {
                let idx = self.eval_expr(index).to_num() as usize;
                let mut arr = self.get_array(name);
                while arr.len() <= idx {
                    arr.push(Value::Undef);
                }
                arr[idx] = val;
                self.set_array(name, arr);
            }
            Expr::HashElement(name, key) => {
                let key_str = self.eval_expr(key).to_str();
                self.set_hash_element(name, &key_str, val);
            }
            Expr::ArrayVar(name) => {
                // @arr = (list)
                // For now, just set a single element
                self.set_array(name, vec![val]);
            }
            _ => {} // Can't assign to this
        }
    }

    // --- Regex ---

    fn regex_match(&mut self, text: &str, pattern: &str, flags: &str) -> bool {
        let case_insensitive = flags.contains('i');
        let pat = if case_insensitive {
            format!("(?i){}", pattern)
        } else {
            pattern.to_string()
        };
        match regex::Regex::new(&pat) {
            Ok(re) => {
                if let Some(caps) = re.captures(text) {
                    // Store capture groups as $1, $2, etc.
                    for i in 1..caps.len() {
                        if let Some(m) = caps.get(i) {
                            self.set_global_var(&i.to_string(), Value::Str(m.as_str().to_string()));
                        } else {
                            self.set_global_var(&i.to_string(), Value::Undef);
                        }
                    }
                    true
                } else {
                    false
                }
            }
            Err(_) => false,
        }
    }

    // --- List evaluation ---

    fn eval_list(&mut self, expr: &Expr) -> Vec<Value> {
        match expr {
            Expr::ArrayLit(items) => items.iter().flat_map(|item| self.eval_list(item)).collect(),
            Expr::ArrayVar(name) => self.get_array(name),
            Expr::QW(words) => words.iter().map(|w| Value::Str(w.clone())).collect(),
            Expr::Range(start, end) => {
                let s = self.eval_expr(start).to_num() as i64;
                let e = self.eval_expr(end).to_num() as i64;
                (s..=e).map(|n| Value::Num(n as f64)).collect()
            }
            Expr::Call(name, args) => {
                // In list context, map/grep/sort return lists
                match name.as_str() {
                    "map" if !args.is_empty() => {
                        let block = &args[0];
                        let items: Vec<Value> =
                            args[1..].iter().flat_map(|a| self.eval_list(a)).collect();
                        let mut results = Vec::new();
                        for item in &items {
                            self.set_var("_", item.clone());
                            let result = self.eval_expr(block);
                            results.push(result);
                        }
                        results
                    }
                    "grep" if !args.is_empty() => {
                        let block = &args[0];
                        let items: Vec<Value> =
                            args[1..].iter().flat_map(|a| self.eval_list(a)).collect();
                        let mut results = Vec::new();
                        for item in &items {
                            self.set_var("_", item.clone());
                            let result = self.eval_expr(block);
                            if result.to_bool() {
                                results.push(item.clone());
                            }
                        }
                        results
                    }
                    "reverse" => {
                        let mut items: Vec<Value> =
                            args.iter().flat_map(|a| self.eval_list(a)).collect();
                        items.reverse();
                        items
                    }
                    "sort" => {
                        let mut items: Vec<Value> =
                            args.iter().flat_map(|a| self.eval_list(a)).collect();
                        items.sort_by(|a, b| a.to_str().cmp(&b.to_str()));
                        items
                    }
                    "split" => {
                        let pat = if args.is_empty() {
                            " ".to_string()
                        } else {
                            self.eval_expr(&args[0]).to_str()
                        };
                        let text = if args.len() > 1 {
                            self.eval_expr(&args[1]).to_str()
                        } else {
                            self.get_var("_").to_str()
                        };
                        if pat == " " {
                            text.split_whitespace()
                                .map(|s| Value::Str(s.to_string()))
                                .collect()
                        } else {
                            text.split(&pat)
                                .map(|s| Value::Str(s.to_string()))
                                .collect()
                        }
                    }
                    _ => {
                        // For user-defined subs, return list in list context
                        if let Some((_params, body)) = self.subs.get(name.as_str()).cloned() {
                            let arg_vals: Vec<Value> =
                                args.iter().flat_map(|a| self.eval_list(a)).collect();
                            self.call_sub_list(&body, &arg_vals)
                        } else {
                            let qualified = format!("{}::{}", self.package, name);
                            if let Some((_params, body)) = self.subs.get(&qualified).cloned() {
                                let arg_vals: Vec<Value> =
                                    args.iter().flat_map(|a| self.eval_list(a)).collect();
                                self.call_sub_list(&body, &arg_vals)
                            } else {
                                let val = self.eval_call(name, args);
                                vec![val]
                            }
                        }
                    }
                }
            }
            _ => vec![self.eval_expr(expr)],
        }
    }

    // --- I/O ---

    fn readline(&mut self, handle: &str) -> Value {
        // Handle <$fh> — variable containing filehandle name
        let effective_handle = if handle.starts_with('$') {
            let var_name = &handle[1..];
            self.get_var(var_name).to_str()
        } else {
            handle.to_string()
        };

        // <> or <STDIN> reads from stdin
        if effective_handle.is_empty() || effective_handle == "STDIN" {
            let stdin = io::stdin();
            let mut line = String::new();
            return match stdin.lock().read_line(&mut line) {
                Ok(0) => Value::Undef, // EOF
                Ok(_) => Value::Str(line),
                Err(_) => Value::Undef,
            };
        }

        // Read from named filehandle
        if let Some(reader) = self.read_handles.get_mut(&effective_handle) {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => Value::Undef, // EOF
                Ok(_) => Value::Str(line),
                Err(_) => Value::Undef,
            }
        } else {
            Value::Undef
        }
    }

    fn eval_open(&mut self, args: &[Expr]) -> Value {
        if args.is_empty() {
            return Value::Undef;
        }

        // Determine filehandle name and filename
        // Forms:
        //   open(FH, "file")  — bareword FH
        //   open(FH, "<file") — read mode
        //   open(FH, ">file") — write mode
        //   open(FH, ">>file") — append mode
        //   open(FH, "<", "file") — 3-arg form
        //   open(FH, ">", "file") — 3-arg form
        //   open(my $fh, ...) — lexical filehandle

        let fh_name: String;
        let filename: String;
        let mut write_mode = false;
        let mut append_mode = false;

        // First arg: filehandle (can be bareword Ident/StringLit or MyVar)
        match &args[0] {
            Expr::MyVar(name) => {
                // Generate a unique filehandle name and store it in the variable
                self.fh_counter += 1;
                fh_name = format!("__anon_fh_{}", self.fh_counter);
                self.set_var(name, Value::Str(fh_name.clone()));
            }
            Expr::ScalarVar(name) => {
                let val = self.get_var(name);
                if val.is_undef() {
                    // Auto-vivify: generate a name
                    self.fh_counter += 1;
                    fh_name = format!("__anon_fh_{}", self.fh_counter);
                    self.set_var(name, Value::Str(fh_name.clone()));
                } else {
                    fh_name = val.to_str();
                }
            }
            _ => {
                fh_name = self.eval_expr(&args[0]).to_str();
            }
        }

        if args.len() == 1 {
            // open(FH) — not very useful without a filename
            return Value::Undef;
        }

        if args.len() >= 3 {
            // 3-arg form: open(FH, MODE, FILE)
            let mode = self.eval_expr(&args[1]).to_str();
            filename = self.eval_expr(&args[2]).to_str();
            match mode.as_str() {
                ">" => write_mode = true,
                ">>" => {
                    write_mode = true;
                    append_mode = true;
                }
                "<" | "" => {} // read mode (default)
                _ => {}
            }
        } else {
            // 2-arg form: open(FH, "mode+file")
            let raw = self.eval_expr(&args[1]).to_str();
            if let Some(rest) = raw.strip_prefix(">>") {
                filename = rest.to_string();
                write_mode = true;
                append_mode = true;
            } else if let Some(rest) = raw.strip_prefix('>') {
                filename = rest.to_string();
                write_mode = true;
            } else if let Some(rest) = raw.strip_prefix('<') {
                filename = rest.to_string();
            } else {
                filename = raw;
            }
        }

        if write_mode {
            let file = if append_mode {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&filename)
            } else {
                File::create(&filename)
            };
            match file {
                Ok(f) => {
                    self.write_handles.insert(fh_name, BufWriter::new(f));
                    Value::Num(1.0)
                }
                Err(e) => {
                    self.set_global_var("!", Value::Str(e.to_string()));
                    Value::Undef
                }
            }
        } else {
            match File::open(&filename) {
                Ok(f) => {
                    self.read_handles.insert(fh_name, BufReader::new(f));
                    Value::Num(1.0)
                }
                Err(e) => {
                    self.set_global_var("!", Value::Str(e.to_string()));
                    Value::Undef
                }
            }
        }
    }

    fn eval_close(&mut self, args: &[Expr]) -> Value {
        if args.is_empty() {
            return Value::Num(1.0);
        }
        let name = self.eval_expr(&args[0]).to_str();
        // Flush and remove write handles
        if let Some(mut writer) = self.write_handles.remove(&name) {
            let _ = writer.flush();
        }
        self.read_handles.remove(&name);
        Value::Num(1.0)
    }

    fn run_backtick(&self, cmd: &str) -> Value {
        use std::process::Command;
        match Command::new("sh").arg("-c").arg(cmd).output() {
            Ok(output) => Value::Str(String::from_utf8_lossy(&output.stdout).to_string()),
            Err(_) => Value::Undef,
        }
    }

    // --- Require ---

    fn do_require(&mut self, filename: &str) -> Value {
        // Check if already loaded
        if self.required_files.contains(filename) {
            return Value::Num(1.0);
        }

        // Resolve the file path
        let path = if filename.starts_with('/')
            || filename.starts_with("./")
            || filename.starts_with("../")
        {
            std::path::PathBuf::from(filename)
        } else {
            // Search @INC
            let inc = self.get_array("INC");
            let mut found = None;
            for dir in &inc {
                let candidate = std::path::PathBuf::from(dir.to_str()).join(filename);
                if candidate.is_file() {
                    found = Some(candidate);
                    break;
                }
            }
            found.unwrap_or_else(|| std::path::PathBuf::from(filename))
        };

        // Read the file
        let code = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => {
                let msg = format!("Can't locate {} in @INC (@INC contains: .)\n", filename);
                self.set_global_var("@", Value::Str(msg));
                return Value::Undef;
            }
        };

        // Mark as loaded
        let canon = path.to_string_lossy().to_string();
        self.required_files.insert(filename.to_string());
        // Set %INC entry
        self.set_hash_element("INC", filename, Value::Str(canon));

        // Execute the file using the run method (which handles BEGIN, subs, etc.)
        self.eval_file_string(&code)
    }

    /// Execute code from a required file — like eval_string but uses `run`
    /// semantics (collects subs/BEGIN/END, then executes main statements).
    fn eval_file_string(&mut self, code: &str) -> Value {
        use crate::lexer::Lexer;
        use crate::parser::Parser;

        let mut lexer = Lexer::new(code);
        let tokens = lexer.tokenize();
        let mut parser = Parser::new(tokens);
        let stmts = parser.parse_program();

        self.set_global_var("@", Value::Str(String::new()));

        // Process like run(): collect subs and BEGIN blocks first
        let mut main_stmts = Vec::new();
        for stmt in &stmts {
            match stmt {
                Stmt::Sub { name, params, body } if !name.is_empty() => {
                    self.subs
                        .insert(name.clone(), (params.clone(), body.clone()));
                }
                Stmt::Begin(body) => {
                    let _flow = self.exec_stmts(body);
                }
                Stmt::End(body) => {
                    self.end_blocks.push(body.clone());
                }
                _ => main_stmts.push(stmt.clone()),
            }
        }

        // Execute main statements
        for stmt in &main_stmts {
            match self.exec_stmt(stmt) {
                Flow::Return(v) => {
                    return v;
                }
                Flow::Die(msg) => {
                    self.set_global_var("@", Value::Str(msg));
                    return Value::Undef;
                }
                Flow::None => {}
                _ => {}
            }
        }

        // Return last expression value (Perl require expects file to return true)
        let result = self.last_expr_val.clone();
        if result.to_bool() {
            result
        } else {
            Value::Num(1.0)
        }
    }

    // --- Eval string ---

    fn eval_string(&mut self, code: &str) -> Value {
        use crate::lexer::Lexer;
        use crate::parser::Parser;

        let mut lexer = Lexer::new(code);
        let tokens = lexer.tokenize();
        let mut parser = Parser::new(tokens);
        let stmts = parser.parse_program();

        self.set_global_var("@", Value::Str(String::new()));
        self.push_scope();

        for stmt in &stmts {
            match self.exec_stmt(stmt) {
                Flow::Return(v) => {
                    self.pop_scope();
                    return v;
                }
                Flow::Die(msg) => {
                    self.set_global_var("@", Value::Str(msg));
                    self.pop_scope();
                    return Value::Undef;
                }
                Flow::None => {}
                _ => {}
            }
        }

        let result = self.last_expr_val.clone();
        self.pop_scope();
        result
    }

    // --- sprintf ---

    fn sprintf_impl(&self, fmt: &str, args: &[Value]) -> String {
        let mut result = String::new();
        let chars: Vec<char> = fmt.chars().collect();
        let mut i = 0;
        let mut arg_idx = 0;

        while i < chars.len() {
            if chars[i] == '%' {
                i += 1;
                if i >= chars.len() {
                    break;
                }
                if chars[i] == '%' {
                    result.push('%');
                    i += 1;
                    continue;
                }

                // Parse format spec
                let mut flags = String::new();
                while i < chars.len() && matches!(chars[i], '-' | '+' | ' ' | '0' | '#') {
                    flags.push(chars[i]);
                    i += 1;
                }

                let mut width = 0;
                if i < chars.len() && chars[i] == '*' {
                    i += 1;
                    width = args.get(arg_idx).map(|v| v.to_num() as i32).unwrap_or(0);
                    arg_idx += 1;
                } else {
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        width = width * 10 + (chars[i] as i32 - '0' as i32);
                        i += 1;
                    }
                }

                let mut precision = -1i32;
                if i < chars.len() && chars[i] == '.' {
                    i += 1;
                    precision = 0;
                    if i < chars.len() && chars[i] == '*' {
                        i += 1;
                        precision = args.get(arg_idx).map(|v| v.to_num() as i32).unwrap_or(0);
                        arg_idx += 1;
                    } else {
                        while i < chars.len() && chars[i].is_ascii_digit() {
                            precision = precision * 10 + (chars[i] as i32 - '0' as i32);
                            i += 1;
                        }
                    }
                }

                if i >= chars.len() {
                    break;
                }

                let conv = chars[i];
                i += 1;

                let arg = args.get(arg_idx).cloned().unwrap_or(Value::Undef);
                arg_idx += 1;

                let formatted = match conv {
                    'd' | 'i' => format!("{}", arg.to_num() as i64),
                    'u' => format!("{}", arg.to_num() as u64),
                    'o' => format!("{:o}", arg.to_num() as u64),
                    'x' => format!("{:x}", arg.to_num() as u64),
                    'X' => format!("{:X}", arg.to_num() as u64),
                    'e' | 'E' => {
                        let p = if precision >= 0 {
                            precision as usize
                        } else {
                            6
                        };
                        format!("{:.prec$e}", arg.to_num(), prec = p)
                    }
                    'f' | 'F' => {
                        let p = if precision >= 0 {
                            precision as usize
                        } else {
                            6
                        };
                        format!("{:.prec$}", arg.to_num(), prec = p)
                    }
                    'g' | 'G' => {
                        let p = if precision >= 0 {
                            precision as usize
                        } else {
                            6
                        };
                        let n = arg.to_num();
                        format_number(n) // simplified
                    }
                    's' => {
                        let s = arg.to_str();
                        if precision >= 0 {
                            s[..s.len().min(precision as usize)].to_string()
                        } else {
                            s
                        }
                    }
                    'c' => {
                        let n = arg.to_num() as u32;
                        char::from_u32(n).unwrap_or('\0').to_string()
                    }
                    _ => format!("%{conv}"),
                };

                // Apply width and alignment
                let w = width.unsigned_abs() as usize;
                if w > formatted.len() {
                    let pad = w - formatted.len();
                    if flags.contains('-') || width < 0 {
                        result.push_str(&formatted);
                        for _ in 0..pad {
                            result.push(' ');
                        }
                    } else {
                        let pad_char = if flags.contains('0') { '0' } else { ' ' };
                        for _ in 0..pad {
                            result.push(pad_char);
                        }
                        result.push_str(&formatted);
                    }
                } else {
                    result.push_str(&formatted);
                }
            } else {
                result.push(chars[i]);
                i += 1;
            }
        }
        result
    }
}
