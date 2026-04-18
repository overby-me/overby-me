use crate::compiler::{Chunk, Constant, OpCode};
use crate::objects::Object;
/// Stack-based virtual machine for executing Meson bytecode.
use std::collections::HashMap;

/// Represents a callable argument (positional or keyword)
#[derive(Debug, Clone)]
pub struct CallArg {
    pub name: Option<String>,
    pub value: Object,
}

/// Iterator state for foreach loops
#[derive(Debug, Clone)]
pub enum IterState {
    Array(Vec<Object>, usize),
    Dict(Vec<(String, Object)>, usize),
    Range(i64, i64, i64), // current, end, step
}

impl IterState {
    pub fn next(&mut self) -> Option<Vec<Object>> {
        match self {
            IterState::Array(arr, idx) => {
                if *idx < arr.len() {
                    let val = arr[*idx].clone();
                    *idx += 1;
                    Some(vec![val])
                } else {
                    None
                }
            }
            IterState::Dict(entries, idx) => {
                if *idx < entries.len() {
                    let (k, v) = entries[*idx].clone();
                    *idx += 1;
                    Some(vec![Object::String(k), v])
                } else {
                    None
                }
            }
            IterState::Range(current, end, step) => {
                if (*step > 0 && *current < *end) || (*step < 0 && *current > *end) {
                    let val = *current;
                    *current += *step;
                    Some(vec![Object::Int(val)])
                } else {
                    None
                }
            }
        }
    }
}

pub type BuiltinFn = fn(&mut VM, &[CallArg]) -> Result<Object, String>;
pub type MethodFn = fn(&mut VM, &Object, &[CallArg]) -> Result<Object, String>;

/// Context for a testcase block that catches expected errors
#[derive(Clone)]
pub struct TestcaseContext {
    pub expected_error: String,
    pub end_ip: usize,
    pub saved_stack_len: usize,
}

/// Result of executing a single VM step
enum StepResult {
    /// Advance ip by 1
    Next,
    /// Jump to the given ip
    Jump(usize),
    /// Halt execution
    Halt,
}
pub struct VM {
    pub stack: Vec<Object>,
    pub variables: HashMap<String, Object>,
    pub globals: HashMap<String, Object>,
    pub builtins: HashMap<String, BuiltinFn>,
    pub method_registry: HashMap<(String, String), MethodFn>,
    pub iter_stack: Vec<IterState>,
    pub arg_names: Vec<Option<String>>,
    pub testcase_stack: Vec<TestcaseContext>,
    /// Source directory for the current project
    pub source_root: String,
    /// Build directory
    pub build_root: String,
    /// Current subdir relative to source root
    pub current_subdir: String,
    /// Callback for subdir() processing
    pub subdir_handler: Option<Box<dyn FnMut(&mut VM, &str) -> Result<(), String>>>,
    /// Collected build targets, tests, install data, etc.
    pub build_data: BuildData,
    /// Current project info
    pub project: Option<ProjectInfo>,
    /// Options
    pub options: HashMap<String, Object>,
    /// Summary data
    pub summary: Vec<(String, Vec<(String, String)>)>,
}

#[derive(Debug, Clone, Default)]
pub struct ProjectInfo {
    pub name: String,
    pub version: String,
    pub license: Vec<String>,
    pub license_files: Vec<String>,
    pub meson_version: String,
    pub languages: Vec<String>,
    pub subproject_dir: String,
    pub default_options: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct BuildData {
    pub targets: Vec<BuildTarget>,
    pub custom_targets: Vec<CustomTarget>,
    pub tests: Vec<TestDef>,
    pub benchmarks: Vec<TestDef>,
    pub install_headers: Vec<InstallData>,
    pub install_data: Vec<InstallData>,
    pub install_man: Vec<InstallData>,
    pub install_subdirs: Vec<InstallData>,
    pub install_symlinks: Vec<SymlinkData>,
    pub install_empty_dirs: Vec<String>,
    pub configure_files: Vec<ConfigureFile>,
    pub dependencies: HashMap<String, Object>,
    pub subprojects: HashMap<String, Object>,
    pub generators: Vec<GeneratorDef>,
    pub run_targets: Vec<RunTarget>,
}

#[derive(Debug, Clone)]
pub struct BuildTarget {
    pub name: String,
    pub id: String,
    pub target_type: TargetType,
    pub sources: Vec<String>,
    pub objects: Vec<String>,
    pub dependencies: Vec<Object>,
    pub include_dirs: Vec<String>,
    pub link_with: Vec<Object>,
    pub link_whole: Vec<Object>,
    pub link_args: Vec<String>,
    pub c_args: Vec<String>,
    pub cpp_args: Vec<String>,
    pub rust_args: Vec<String>,
    pub install: bool,
    pub install_dir: Option<String>,
    pub install_rpath: String,
    pub build_rpath: String,
    pub pic: Option<bool>,
    pub pie: Option<bool>,
    pub override_options: Vec<String>,
    pub gnu_symbol_visibility: String,
    pub native: bool,
    pub extra_files: Vec<String>,
    pub implicit_include_directories: bool,
    pub win_subsystem: String,
    pub name_prefix: Option<String>,
    pub name_suffix: Option<String>,
    pub rust_crate_type: Option<String>,
    pub build_by_default: bool,
    pub subdir: String,
    pub output_name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TargetType {
    Executable,
    SharedLibrary,
    StaticLibrary,
    SharedModule,
    BothLibraries,
    Jar,
}

#[derive(Debug, Clone)]
pub struct CustomTarget {
    pub name: String,
    pub id: String,
    pub command: Vec<String>,
    pub input: Vec<String>,
    pub output: Vec<String>,
    pub depends: Vec<Object>,
    pub depend_files: Vec<String>,
    pub depfile: Option<String>,
    pub capture: bool,
    pub feed: bool,
    pub install: bool,
    pub install_dir: Vec<String>,
    pub install_tag: Vec<String>,
    pub build_by_default: bool,
    pub build_always_stale: bool,
    pub env: HashMap<String, String>,
    pub subdir: String,
}

#[derive(Debug, Clone)]
pub struct TestDef {
    pub name: String,
    pub exe: Object,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub should_fail: bool,
    pub timeout: i64,
    pub workdir: Option<String>,
    pub protocol: String,
    pub priority: i64,
    pub suite: Vec<String>,
    pub depends: Vec<Object>,
    pub is_parallel: bool,
    pub verbose: bool,
}

#[derive(Debug, Clone)]
pub struct InstallData {
    pub sources: Vec<String>,
    pub install_dir: String,
    pub install_mode: Option<Vec<String>>,
    pub rename: Vec<String>,
    pub subdir: String,
    pub preserve_path: bool,
    pub strip_directory: bool,
    pub exclude_files: Vec<String>,
    pub exclude_directories: Vec<String>,
    pub follow_symlinks: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct SymlinkData {
    pub name: String,
    pub target: String,
    pub install_dir: String,
}

#[derive(Debug, Clone)]
pub struct ConfigureFile {
    pub input: Option<String>,
    pub output: String,
    pub configuration: Option<Object>,
    pub command: Vec<String>,
    pub format: String,
    pub output_format: String,
    pub encoding: String,
    pub install: bool,
    pub install_dir: Option<String>,
    pub install_tag: Option<String>,
    pub capture: bool,
    pub depfile: Option<String>,
    pub subdir: String,
}

#[derive(Debug, Clone)]
pub struct GeneratorDef {
    pub exe: Object,
    pub arguments: Vec<String>,
    pub output: Vec<String>,
    pub depfile: Option<String>,
    pub capture: bool,
}

#[derive(Debug, Clone)]
pub struct RunTarget {
    pub name: String,
    pub command: Vec<String>,
    pub depends: Vec<Object>,
    pub env: HashMap<String, String>,
    pub subdir: String,
}

/// Simple glob matching for testcase error patterns.
/// Supports '*' as a wildcard that matches any sequence of characters.
fn glob_match(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        // No wildcards - exact match
        return pattern == text;
    }
    let mut pos = 0;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if let Some(found) = text[pos..].find(part) {
            if i == 0 && found != 0 {
                // Pattern doesn't start with *, but text has a prefix
                return false;
            }
            pos += found + part.len();
        } else {
            return false;
        }
    }
    // If pattern doesn't end with *, text must end exactly
    if !pattern.ends_with('*') && pos != text.len() {
        return false;
    }
    true
}

impl VM {
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            variables: HashMap::new(),
            globals: HashMap::new(),
            builtins: HashMap::new(),
            method_registry: HashMap::new(),
            iter_stack: Vec::new(),
            arg_names: Vec::new(),
            testcase_stack: Vec::new(),
            source_root: String::new(),
            build_root: String::new(),
            current_subdir: String::new(),
            subdir_handler: None,
            build_data: BuildData::default(),
            project: None,
            options: HashMap::new(),
            summary: Vec::new(),
        }
    }

    pub fn execute(&mut self, chunk: &Chunk) -> Result<Object, String> {
        let mut ip = 0;
        while ip < chunk.code.len() {
            match self.execute_step(chunk, ip) {
                Ok(StepResult::Next) => ip += 1,
                Ok(StepResult::Jump(target)) => ip = target,
                Ok(StepResult::Halt) => break,
                Err(e) => {
                    if let Some(ctx) = self.testcase_stack.last().cloned() {
                        if glob_match(&ctx.expected_error, &e) {
                            self.testcase_stack.pop();
                            self.stack.truncate(ctx.saved_stack_len);
                            ip = ctx.end_ip;
                            continue;
                        }
                    }
                    return Err(e);
                }
            }
        }
        Ok(self.stack.pop().unwrap_or(Object::None))
    }

    fn execute_step(&mut self, chunk: &Chunk, ip: usize) -> Result<StepResult, String> {
        let op = &chunk.code[ip];
        match op {
            OpCode::Constant(idx) => {
                let val = match &chunk.constants[*idx] {
                    Constant::String(s) => Object::String(s.clone()),
                    Constant::Int(n) => Object::Int(*n),
                    Constant::Bool(b) => Object::Bool(*b),
                    Constant::None => Object::None,
                };
                self.stack.push(val);
            }
            OpCode::True => self.stack.push(Object::Bool(true)),
            OpCode::False => self.stack.push(Object::Bool(false)),
            OpCode::Pop => {
                self.stack.pop();
            }
            OpCode::LoadVar(name) => {
                if let Some(val) = self.variables.get(name) {
                    self.stack.push(val.clone());
                } else if let Some(val) = self.globals.get(name) {
                    self.stack.push(val.clone());
                } else {
                    return Err(format!("Undefined variable: '{}'", name));
                }
            }
            OpCode::StoreVar(name) => {
                let val = self.stack.pop().ok_or("Stack underflow on store")?;
                self.variables.insert(name.clone(), val);
            }
            OpCode::PlusAssignVar(name) => {
                let rhs = self.stack.pop().ok_or("Stack underflow on +=")?;
                let lhs = self
                    .variables
                    .get(name)
                    .cloned()
                    .ok_or(format!("Undefined variable '{}' in +=", name))?;
                let result = self.add_values(&lhs, &rhs)?;
                self.variables.insert(name.clone(), result);
            }
            OpCode::MakeArray(n) => {
                let n = *n;
                let start = self.stack.len() - n;
                let elements: Vec<Object> = self.stack.drain(start..).collect();
                self.stack.push(Object::Array(elements));
            }
            OpCode::MakeDict(n) => {
                let n = *n;
                let start = self.stack.len() - n * 2;
                let pairs: Vec<Object> = self.stack.drain(start..).collect();
                let mut map = Vec::new();
                for chunk_pair in pairs.chunks(2) {
                    let key = match &chunk_pair[0] {
                        Object::String(s) => s.clone(),
                        other => {
                            return Err(format!("Dict key must be string, got {:?}", other));
                        }
                    };
                    map.push((key, chunk_pair[1].clone()));
                }
                self.stack.push(Object::Dict(map));
            }
            OpCode::Add => {
                let b = self.stack.pop().ok_or("Stack underflow")?;
                let a = self.stack.pop().ok_or("Stack underflow")?;
                let result = self.add_values(&a, &b)?;
                self.stack.push(result);
            }
            OpCode::Sub => {
                let b = self.stack.pop().ok_or("Stack underflow")?;
                let a = self.stack.pop().ok_or("Stack underflow")?;
                match (&a, &b) {
                    (Object::Int(a), Object::Int(b)) => self.stack.push(Object::Int(a - b)),
                    _ => return Err(format!("Cannot subtract {:?} and {:?}", a, b)),
                }
            }
            OpCode::Mul => {
                let b = self.stack.pop().ok_or("Stack underflow")?;
                let a = self.stack.pop().ok_or("Stack underflow")?;
                match (&a, &b) {
                    (Object::Int(a), Object::Int(b)) => self.stack.push(Object::Int(a * b)),
                    _ => return Err(format!("Cannot multiply {:?} and {:?}", a, b)),
                }
            }
            OpCode::Div => {
                let b = self.stack.pop().ok_or("Stack underflow")?;
                let a = self.stack.pop().ok_or("Stack underflow")?;
                match (&a, &b) {
                    (Object::Int(a), Object::Int(b)) => {
                        if *b == 0 {
                            return Err("Division by zero".to_string());
                        }
                        self.stack.push(Object::Int(a / b));
                    }
                    (Object::String(a), Object::String(b)) => {
                        let result = if b.starts_with('/') {
                            b.clone()
                        } else if b.is_empty() {
                            format!("{}/", a)
                        } else {
                            format!("{}/{}", a, b)
                        };
                        self.stack.push(Object::String(result));
                    }
                    _ => return Err(format!("Cannot divide {:?} and {:?}", a, b)),
                }
            }
            OpCode::Mod => {
                let b = self.stack.pop().ok_or("Stack underflow")?;
                let a = self.stack.pop().ok_or("Stack underflow")?;
                match (&a, &b) {
                    (Object::Int(a), Object::Int(b)) => {
                        if *b == 0 {
                            return Err("Modulo by zero".to_string());
                        }
                        self.stack.push(Object::Int(a % b));
                    }
                    _ => return Err(format!("Cannot modulo {:?} and {:?}", a, b)),
                }
            }
            OpCode::Eq => {
                let b = self.stack.pop().ok_or("Stack underflow")?;
                let a = self.stack.pop().ok_or("Stack underflow")?;
                self.stack.push(Object::Bool(a == b));
            }
            OpCode::Neq => {
                let b = self.stack.pop().ok_or("Stack underflow")?;
                let a = self.stack.pop().ok_or("Stack underflow")?;
                self.stack.push(Object::Bool(a != b));
            }
            OpCode::Lt => {
                let b = self.stack.pop().ok_or("Stack underflow")?;
                let a = self.stack.pop().ok_or("Stack underflow")?;
                self.stack.push(Object::Bool(self.compare_lt(&a, &b)?));
            }
            OpCode::Gt => {
                let b = self.stack.pop().ok_or("Stack underflow")?;
                let a = self.stack.pop().ok_or("Stack underflow")?;
                self.stack.push(Object::Bool(self.compare_lt(&b, &a)?));
            }
            OpCode::Le => {
                let b = self.stack.pop().ok_or("Stack underflow")?;
                let a = self.stack.pop().ok_or("Stack underflow")?;
                let lt = self.compare_lt(&a, &b)?;
                self.stack.push(Object::Bool(lt || a == b));
            }
            OpCode::Ge => {
                let b = self.stack.pop().ok_or("Stack underflow")?;
                let a = self.stack.pop().ok_or("Stack underflow")?;
                let lt = self.compare_lt(&b, &a)?;
                self.stack.push(Object::Bool(lt || a == b));
            }
            OpCode::And | OpCode::Or => {
                unreachable!("And/Or should be compiled as jumps");
            }
            OpCode::In => {
                let container = self.stack.pop().ok_or("Stack underflow")?;
                let item = self.stack.pop().ok_or("Stack underflow")?;
                let result = self.contains(&container, &item)?;
                self.stack.push(Object::Bool(result));
            }
            OpCode::NotIn => {
                let container = self.stack.pop().ok_or("Stack underflow")?;
                let item = self.stack.pop().ok_or("Stack underflow")?;
                let result = self.contains(&container, &item)?;
                self.stack.push(Object::Bool(!result));
            }
            OpCode::Not => {
                let val = self.stack.pop().ok_or("Stack underflow")?;
                match val {
                    Object::Bool(b) => self.stack.push(Object::Bool(!b)),
                    _ => return Err(format!("Cannot negate non-boolean: {:?}", val)),
                }
            }
            OpCode::Negate => {
                let val = self.stack.pop().ok_or("Stack underflow")?;
                match val {
                    Object::Int(n) => self.stack.push(Object::Int(-n)),
                    _ => return Err(format!("Cannot negate non-integer: {:?}", val)),
                }
            }
            OpCode::ArgName(name) => {
                self.arg_names.push(name.clone());
            }
            OpCode::Call(nargs) => {
                let nargs = *nargs;
                let mut args = Vec::new();
                let names_start = self.arg_names.len() - nargs;
                let names: Vec<Option<String>> = self.arg_names.drain(names_start..).collect();
                let values_start = self.stack.len() - nargs;
                let values: Vec<Object> = self.stack.drain(values_start..).collect();
                for (name, value) in names.into_iter().zip(values) {
                    args.push(CallArg { name, value });
                }
                let func = self.stack.pop().ok_or("Stack underflow on call")?;
                let result = match func {
                    Object::BuiltinFunction(name) => {
                        if let Some(f) = self.builtins.get(&name) {
                            let f = *f;
                            f(self, &args)?
                        } else {
                            return Err(format!("Unknown function: '{}'", name));
                        }
                    }
                    _ => return Err(format!("Cannot call {:?}", func)),
                };
                if matches!(result, Object::Disabler) {
                    self.stack.push(Object::Disabler);
                } else {
                    self.stack.push(result);
                }
            }
            OpCode::MethodCall(method, nargs) => {
                let nargs = *nargs;
                let method = method.clone();
                let mut args = Vec::new();
                let names_start = self.arg_names.len() - nargs;
                let names: Vec<Option<String>> = self.arg_names.drain(names_start..).collect();
                let values_start = self.stack.len() - nargs;
                let values: Vec<Object> = self.stack.drain(values_start..).collect();
                for (name, value) in names.into_iter().zip(values) {
                    args.push(CallArg { name, value });
                }
                let obj = self.stack.pop().ok_or("Stack underflow on method call")?;
                if matches!(obj, Object::Disabler) {
                    self.stack.push(Object::Disabler);
                    return Ok(StepResult::Next);
                }
                let type_name = obj.type_name();
                // For modules, try the qualified method name first (e.g., "python.find_python")
                let key = if let Object::Module(ref module_name) = obj {
                    let qualified = format!("{}.{}", module_name, method);
                    (type_name.to_string(), qualified)
                } else {
                    (type_name.to_string(), method.clone())
                };
                if let Some(f) = self.method_registry.get(&key) {
                    let f = *f;
                    let result = f(self, &obj, &args)?;
                    self.stack.push(result);
                } else {
                    // For modules, also try unqualified method name as fallback
                    let fallback_key = (type_name.to_string(), method.clone());
                    if let Some(f) = self.method_registry.get(&fallback_key) {
                        let f = *f;
                        let result = f(self, &obj, &args)?;
                        self.stack.push(result);
                    } else {
                        let generic_key = ("*".to_string(), method.clone());
                        if let Some(f) = self.method_registry.get(&generic_key) {
                            let f = *f;
                            let result = f(self, &obj, &args)?;
                            self.stack.push(result);
                        } else {
                            return Err(format!("No method '{}' on type '{}'", method, type_name));
                        }
                    }
                }
            }
            OpCode::Index => {
                let index = self.stack.pop().ok_or("Stack underflow")?;
                let obj = self.stack.pop().ok_or("Stack underflow")?;
                let result = self.index_into(&obj, &index)?;
                self.stack.push(result);
            }
            OpCode::Jump(target) => {
                return Ok(StepResult::Jump(*target));
            }
            OpCode::JumpIfFalse(target) => {
                let target = *target;
                let val = self.stack.last().ok_or("Stack underflow")?;
                if !val.is_truthy() {
                    return Ok(StepResult::Jump(target));
                }
                self.stack.pop();
            }
            OpCode::JumpIfTrue(target) => {
                let target = *target;
                let val = self.stack.last().ok_or("Stack underflow")?;
                if val.is_truthy() {
                    return Ok(StepResult::Jump(target));
                }
                self.stack.pop();
            }
            OpCode::IterSetup => {
                let obj = self.stack.pop().ok_or("Stack underflow")?;
                let iter = match obj {
                    Object::Array(arr) => IterState::Array(arr, 0),
                    Object::Dict(entries) => IterState::Dict(entries, 0),
                    Object::Range(start, end, step) => IterState::Range(start, end, step),
                    _ => return Err(format!("Cannot iterate over {:?}", obj)),
                };
                self.iter_stack.push(iter);
            }
            OpCode::IterNext(varnames, end_target) => {
                let end_target = *end_target;
                let varnames = varnames.clone();
                let iter = self.iter_stack.last_mut().ok_or("No iterator")?;
                match iter.next() {
                    Some(values) => {
                        for (i, name) in varnames.iter().enumerate() {
                            if i < values.len() {
                                self.variables.insert(name.clone(), values[i].clone());
                            }
                        }
                    }
                    None => {
                        self.iter_stack.pop();
                        return Ok(StepResult::Jump(end_target));
                    }
                }
            }
            OpCode::Break => {
                unreachable!("Break should be compiled as Jump");
            }
            OpCode::Continue => {
                unreachable!("Continue should be compiled as Jump");
            }
            OpCode::FString(template) => {
                let result = self.interpolate_fstring(template)?;
                self.stack.push(Object::String(result));
            }
            OpCode::Nop => {}
            OpCode::TestcaseStart(end_ip) => {
                let end_ip = *end_ip;
                let expected = self.stack.pop().ok_or("Stack underflow on testcase")?;
                let expected_error = match expected {
                    Object::String(s) => s,
                    other => {
                        return Err(format!(
                            "testcase expect_error() requires a string argument, got {:?}",
                            other
                        ));
                    }
                };
                self.testcase_stack.push(TestcaseContext {
                    expected_error,
                    end_ip,
                    saved_stack_len: self.stack.len(),
                });
            }
            OpCode::TestcaseNoError => {
                let ctx = self
                    .testcase_stack
                    .pop()
                    .ok_or("TestcaseNoError without testcase context")?;
                return Err(format!(
                    "Testcase expected error matching '{}' but code succeeded",
                    ctx.expected_error
                ));
            }
            OpCode::Halt => return Ok(StepResult::Halt),
        }
        Ok(StepResult::Next)
    }

    fn add_values(&self, a: &Object, b: &Object) -> Result<Object, String> {
        match (a, b) {
            (Object::Int(a), Object::Int(b)) => Ok(Object::Int(a + b)),
            (Object::String(a), Object::String(b)) => Ok(Object::String(format!("{}{}", a, b))),
            (Object::Array(a), Object::Array(b)) => {
                let mut result = a.clone();
                result.extend(b.iter().cloned());
                Ok(Object::Array(result))
            }
            (Object::Dict(a), Object::Dict(b)) => {
                let mut result = a.clone();
                for (k, v) in b {
                    if let Some(existing) = result.iter_mut().find(|(ek, _)| ek == k) {
                        existing.1 = v.clone();
                    } else {
                        result.push((k.clone(), v.clone()));
                    }
                }
                Ok(Object::Dict(result))
            }
            (Object::Array(a), other) => {
                let mut result = a.clone();
                result.push(other.clone());
                Ok(Object::Array(result))
            }
            _ => Err(format!("Cannot add {:?} and {:?}", a, b)),
        }
    }

    fn compare_lt(&self, a: &Object, b: &Object) -> Result<bool, String> {
        match (a, b) {
            (Object::Int(a), Object::Int(b)) => Ok(a < b),
            (Object::String(a), Object::String(b)) => Ok(a < b),
            _ => Err(format!("Cannot compare {:?} and {:?}", a, b)),
        }
    }

    fn contains(&self, container: &Object, item: &Object) -> Result<bool, String> {
        match container {
            Object::Array(arr) => Ok(arr.contains(item)),
            Object::Dict(entries) => {
                if let Object::String(key) = item {
                    Ok(entries.iter().any(|(k, _)| k == key))
                } else {
                    Err(format!("Dict key must be string, got {:?}", item))
                }
            }
            Object::String(s) => {
                if let Object::String(sub) = item {
                    Ok(s.contains(sub.as_str()))
                } else {
                    Err(format!("'in' on string requires string, got {:?}", item))
                }
            }
            _ => Err(format!("Cannot check 'in' on {:?}", container)),
        }
    }

    fn index_into(&self, obj: &Object, index: &Object) -> Result<Object, String> {
        match obj {
            Object::Array(arr) => {
                if let Object::Int(i) = index {
                    let idx = if *i < 0 {
                        (arr.len() as i64 + i) as usize
                    } else {
                        *i as usize
                    };
                    arr.get(idx).cloned().ok_or(format!(
                        "Array index {} out of bounds (len {})",
                        i,
                        arr.len()
                    ))
                } else {
                    Err(format!("Array index must be int, got {:?}", index))
                }
            }
            Object::Dict(entries) => {
                if let Object::String(key) = index {
                    entries
                        .iter()
                        .find(|(k, _)| k == key)
                        .map(|(_, v)| v.clone())
                        .ok_or(format!("Key '{}' not found in dict", key))
                } else {
                    Err(format!("Dict key must be string, got {:?}", index))
                }
            }
            Object::String(s) => {
                if let Object::Int(i) = index {
                    let idx = if *i < 0 {
                        (s.len() as i64 + i) as usize
                    } else {
                        *i as usize
                    };
                    s.chars()
                        .nth(idx)
                        .map(|c| Object::String(c.to_string()))
                        .ok_or(format!("String index {} out of bounds", i))
                } else {
                    Err(format!("String index must be int, got {:?}", index))
                }
            }
            Object::CustomTarget(ct_ref) => {
                if let Object::Int(i) = index {
                    let idx = if *i < 0 {
                        (ct_ref.outputs.len() as i64 + i) as usize
                    } else {
                        *i as usize
                    };
                    if idx < ct_ref.outputs.len() {
                        Ok(Object::CustomTargetIndex(ct_ref.clone(), idx))
                    } else {
                        Err(format!(
                            "Custom target '{}' index {} out of bounds (has {} outputs)",
                            ct_ref.name,
                            i,
                            ct_ref.outputs.len()
                        ))
                    }
                } else {
                    Err(format!("Custom target index must be int, got {:?}", index))
                }
            }
            Object::Range(start, end, step) => {
                if let Object::Int(i) = index {
                    let elements: Vec<i64> = {
                        let mut v = Vec::new();
                        let mut cur = *start;
                        while (*step > 0 && cur < *end) || (*step < 0 && cur > *end) {
                            v.push(cur);
                            cur += step;
                        }
                        v
                    };
                    let idx = if *i < 0 {
                        (elements.len() as i64 + i) as usize
                    } else {
                        *i as usize
                    };
                    elements.get(idx).map(|n| Object::Int(*n)).ok_or(format!(
                        "Range index {} out of bounds (len {})",
                        i,
                        elements.len()
                    ))
                } else {
                    Err(format!("Range index must be int, got {:?}", index))
                }
            }
            Object::Disabler => Ok(Object::Disabler),
            _ => Err(format!("Cannot index into {:?}", obj)),
        }
    }

    fn interpolate_fstring(&self, template: &str) -> Result<String, String> {
        let mut result = String::new();
        let chars: Vec<char> = template.chars().collect();
        let mut i = 0;
        // Phase 1: Single-pass replacement of @varname@ patterns.
        // @identifier@ takes priority over @@ at each position.
        while i < chars.len() {
            if chars[i] == '@' {
                // Try to match @identifier@ where identifier starts with letter/underscore
                let start = i + 1;
                if start < chars.len() && (chars[start].is_alphabetic() || chars[start] == '_') {
                    let mut j = start + 1;
                    while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                        j += 1;
                    }
                    if j < chars.len() && chars[j] == '@' {
                        let varname: String = chars[start..j].iter().collect();
                        let val = self
                            .variables
                            .get(&varname)
                            .or_else(|| self.globals.get(&varname))
                            .ok_or(format!("Undefined variable '{}' in f-string", varname))?;
                        result.push_str(&val.to_display_string());
                        i = j + 1;
                        continue;
                    }
                }
                // Not a variable substitution, just output @
                result.push('@');
                i += 1;
            } else {
                result.push(chars[i]);
                i += 1;
            }
        }
        // Phase 2: Replace @@ with literal @
        let result = result.replace("@@", "@");
        Ok(result)
    }

    pub fn get_arg_str<'a>(args: &'a [CallArg], name: &str, pos: usize) -> Option<&'a str> {
        // Try keyword first
        for arg in args {
            if arg.name.as_deref() == Some(name) {
                if let Object::String(ref s) = arg.value {
                    return Some(s);
                }
            }
        }
        // Try positional
        let mut positional_idx = 0;
        for arg in args {
            if arg.name.is_none() {
                if positional_idx == pos {
                    if let Object::String(ref s) = arg.value {
                        return Some(s);
                    }
                }
                positional_idx += 1;
            }
        }
        None
    }

    pub fn get_arg_bool(args: &[CallArg], name: &str, default: bool) -> bool {
        for arg in args {
            if arg.name.as_deref() == Some(name) {
                if let Object::Bool(b) = arg.value {
                    return b;
                }
            }
        }
        default
    }

    pub fn get_arg_int(args: &[CallArg], name: &str, default: i64) -> i64 {
        for arg in args {
            if arg.name.as_deref() == Some(name) {
                if let Object::Int(n) = arg.value {
                    return n;
                }
            }
        }
        default
    }

    pub fn get_arg_value<'a>(args: &'a [CallArg], name: &str) -> Option<&'a Object> {
        for arg in args {
            if arg.name.as_deref() == Some(name) {
                return Some(&arg.value);
            }
        }
        None
    }

    pub fn get_positional_args(args: &[CallArg]) -> Vec<&Object> {
        args.iter()
            .filter(|a| a.name.is_none())
            .map(|a| &a.value)
            .collect()
    }

    pub fn get_arg_string_array(args: &[CallArg], name: &str) -> Vec<String> {
        for arg in args {
            if arg.name.as_deref() == Some(name) {
                if let Object::Array(ref arr) = arg.value {
                    return arr
                        .iter()
                        .filter_map(|v| {
                            if let Object::String(s) = v {
                                Some(s.clone())
                            } else {
                                None
                            }
                        })
                        .collect();
                }
            }
        }
        Vec::new()
    }
}
