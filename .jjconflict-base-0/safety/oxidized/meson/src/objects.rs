use std::cell::RefCell;
/// Object types for the Meson interpreter.
/// These represent values that can appear on the VM stack.
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub enum Object {
    None,
    Bool(bool),
    Int(i64),
    String(String),
    Array(Vec<Object>),
    Dict(Vec<(String, Object)>),
    /// A built-in function reference
    BuiltinFunction(String),
    /// Disabler object — propagates through all operations
    Disabler,
    /// Feature option (auto, enabled, disabled)
    Feature(FeatureState),
    /// Build target reference
    BuildTarget(BuildTargetRef),
    /// Custom target reference
    CustomTarget(CustomTargetRef),
    /// Dependency object
    Dependency(DependencyData),
    /// External program found via find_program()
    ExternalProgram(ExternalProgramData),
    /// Compiler object
    Compiler(CompilerData),
    /// Configuration data object
    ConfigurationData(ConfigData),
    /// Environment object
    Environment(EnvData),
    /// Include directories
    IncludeDirs(IncludeDirsData),
    /// Generator object
    Generator(GeneratorData),
    /// Run result from run_command()
    RunResult(RunResultData),
    /// Machine info (build_machine, host_machine, target_machine)
    MachineInfo(MachineInfoData),
    /// Subproject object
    Subproject(SubprojectData),
    /// Meson object (meson.version(), etc.)
    MesonObject,
    /// Range object for range()
    Range(i64, i64, i64), // start, end, step
    /// Structured sources
    StructuredSources(Vec<(String, Vec<String>)>),
    /// File object from files()
    File(FileData),
    /// Module object (returned by import())
    Module(String),
    /// Both libraries (shared + static)
    BothLibraries(Box<Object>, Box<Object>),
    /// Generated list from generator.process()
    GeneratedList(GeneratedListData),
    /// Custom target index (one output of a multi-output custom target)
    CustomTargetIndex(CustomTargetRef, usize),
    /// Source set object from sourceset module
    SourceSet(SourceSetData),
    /// Result of source_set.apply()
    SourceSetResult(SourceSetResultData),
}

impl PartialEq for Object {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Object::None, Object::None) => true,
            (Object::Bool(a), Object::Bool(b)) => a == b,
            (Object::Int(a), Object::Int(b)) => a == b,
            (Object::String(a), Object::String(b)) => a == b,
            (Object::Array(a), Object::Array(b)) => a == b,
            (Object::Dict(a), Object::Dict(b)) => a == b,
            (Object::Disabler, Object::Disabler) => true,
            (Object::Feature(a), Object::Feature(b)) => a == b,
            (Object::BuildTarget(a), Object::BuildTarget(b)) => a.id == b.id,
            (Object::CustomTarget(a), Object::CustomTarget(b)) => a.id == b.id,
            (Object::ExternalProgram(a), Object::ExternalProgram(b)) => {
                a.name == b.name && a.path == b.path
            }
            (Object::Dependency(a), Object::Dependency(b)) => {
                a.name == b.name && a.found == b.found
            }
            (Object::File(a), Object::File(b)) => a.path == b.path,
            _ => false,
        }
    }
}

impl Object {
    pub fn type_name(&self) -> &str {
        match self {
            Object::None => "void",
            Object::Bool(_) => "bool",
            Object::Int(_) => "int",
            Object::String(_) => "str",
            Object::Array(_) => "list",
            Object::Dict(_) => "dict",
            Object::BuiltinFunction(_) => "function",
            Object::Disabler => "disabler",
            Object::Feature(_) => "feature",
            Object::BuildTarget(_) => "build_tgt",
            Object::CustomTarget(_) => "custom_tgt",
            Object::Dependency(_) => "dep",
            Object::ExternalProgram(_) => "external_program",
            Object::Compiler(_) => "compiler",
            Object::ConfigurationData(_) => "cfg_data",
            Object::Environment(_) => "env",
            Object::IncludeDirs(_) => "inc",
            Object::Generator(_) => "generator",
            Object::RunResult(_) => "runresult",
            Object::MachineInfo(_) => "build_machine",
            Object::Subproject(_) => "subproject",
            Object::MesonObject => "meson",
            Object::Range(_, _, _) => "range",
            Object::StructuredSources(_) => "structured_src",
            Object::File(_) => "file",
            Object::Module(_) => "module",
            Object::BothLibraries(_, _) => "both_libs",
            Object::GeneratedList(_) => "generated_list",
            Object::CustomTargetIndex(_, _) => "custom_idx",
            Object::SourceSet(_) => "source_set",
            Object::SourceSetResult(_) => "source_set_result",
        }
    }

    pub fn is_truthy(&self) -> bool {
        match self {
            Object::None => false,
            Object::Bool(b) => *b,
            Object::Int(n) => *n != 0,
            Object::String(s) => !s.is_empty(),
            Object::Array(a) => !a.is_empty(),
            Object::Dict(d) => !d.is_empty(),
            Object::Disabler => false,
            Object::Feature(f) => *f == FeatureState::Enabled,
            _ => true,
        }
    }

    pub const NON_PRINTABLE_ERROR: &'static str =
        "Value other than strings, integers, bools, options, dictionaries and lists thereof.";

    /// Check if this value is a "printable" type for message()/f-strings.
    /// Allowed: strings, integers, bools, options (Feature), dicts of printable, lists of printable.
    pub fn is_printable_type(&self) -> bool {
        match self {
            Object::String(_)
            | Object::Int(_)
            | Object::Bool(_)
            | Object::Feature(_)
            | Object::None => true,
            Object::Array(arr) => arr.iter().all(|v| v.is_printable_type()),
            Object::Dict(d) => d.iter().all(|(_, v)| v.is_printable_type()),
            _ => false,
        }
    }

    pub fn to_display_string(&self) -> String {
        match self {
            Object::None => String::new(),
            Object::Bool(b) => b.to_string(),
            Object::Int(n) => n.to_string(),
            Object::String(s) => s.clone(),
            Object::Array(a) => {
                let items: Vec<String> = a
                    .iter()
                    .map(|v| match v {
                        Object::String(s) => format!("'{}'", s),
                        other => other.to_display_string(),
                    })
                    .collect();
                format!("[{}]", items.join(", "))
            }
            Object::Dict(d) => {
                let items: Vec<String> = d
                    .iter()
                    .map(|(k, v)| format!("'{}': {}", k, v.to_display_string()))
                    .collect();
                format!("{{{}}}", items.join(", "))
            }
            Object::Disabler => "<disabler>".to_string(),
            Object::Feature(f) => format!("{:?}", f).to_lowercase(),
            Object::BuildTarget(t) => format!("<build target '{}'>", t.name),
            Object::CustomTarget(t) => format!("<custom target '{}'>", t.name),
            Object::Dependency(d) => format!("<dependency '{}'>", d.name),
            Object::ExternalProgram(p) => format!("<program '{}'>", p.name),
            Object::Compiler(c) => format!("<compiler '{}'>", c.id),
            Object::File(f) => f.path.clone(),
            Object::Module(name) => format!("<module '{}'>", name),
            Object::SourceSet(_) => "<source set>".to_string(),
            Object::SourceSetResult(_) => "<source set result>".to_string(),
            _ => format!("<{}>", self.type_name()),
        }
    }

    pub fn to_string_value(&self) -> String {
        match self {
            Object::String(s) => s.clone(),
            other => other.to_display_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FeatureState {
    Auto,
    Enabled,
    Disabled,
}

#[derive(Debug, Clone)]
pub struct BuildTargetRef {
    pub name: String,
    pub id: String,
    pub target_type: String,
    pub subdir: String,
    pub outputs: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CustomTargetRef {
    pub name: String,
    pub id: String,
    pub outputs: Vec<String>,
    pub subdir: String,
}

#[derive(Debug, Clone)]
pub struct DependencyData {
    pub name: String,
    pub found: bool,
    pub version: String,
    pub compile_args: Vec<String>,
    pub link_args: Vec<String>,
    pub sources: Vec<String>,
    pub include_dirs: Vec<String>,
    pub dependencies: Vec<Object>,
    pub variables: HashMap<String, String>,
    pub is_internal: bool,
    /// Origin kind: "" (pkgconfig), "library" (compiler.find_library),
    /// "internal", "system", etc. Used by type_name().
    pub kind: String,
}

impl DependencyData {
    pub fn not_found(name: &str) -> Self {
        Self {
            name: name.to_string(),
            found: false,
            version: String::new(),
            compile_args: Vec::new(),
            link_args: Vec::new(),
            sources: Vec::new(),
            include_dirs: Vec::new(),
            dependencies: Vec::new(),
            variables: HashMap::new(),
            is_internal: false,
            kind: String::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExternalProgramData {
    pub name: String,
    pub path: String,
    pub found: bool,
    pub version: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CompilerData {
    pub id: String,
    pub language: String,
    pub version: String,
    pub cmd: Vec<String>,
    pub linker_id: String,
    pub full_version: String,
}

#[derive(Debug, Clone)]
pub struct ConfigData {
    pub values: Rc<RefCell<HashMap<String, (Object, Option<String>)>>>,
}

impl ConfigData {
    pub fn new() -> Self {
        Self {
            values: Rc::new(RefCell::new(HashMap::new())),
        }
    }
}

#[derive(Debug, Clone)]
pub struct EnvData {
    pub values: Rc<RefCell<Vec<(String, EnvOp)>>>,
}

#[derive(Debug, Clone)]
pub enum EnvOp {
    Set(String),
    Prepend(String, String), // value, separator
    Append(String, String),  // value, separator
    Unset,
}

impl EnvData {
    pub fn new() -> Self {
        Self {
            values: Rc::new(RefCell::new(Vec::new())),
        }
    }

    pub fn to_map(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();
        let values = self.values.borrow();
        for (k, op) in values.iter() {
            match op {
                EnvOp::Set(v) => {
                    map.insert(k.clone(), v.clone());
                }
                EnvOp::Prepend(v, sep) => {
                    let existing = map.get(k).cloned().unwrap_or_default();
                    if existing.is_empty() {
                        map.insert(k.clone(), v.clone());
                    } else {
                        map.insert(k.clone(), format!("{}{}{}", v, sep, existing));
                    }
                }
                EnvOp::Append(v, sep) => {
                    let existing = map.get(k).cloned().unwrap_or_default();
                    if existing.is_empty() {
                        map.insert(k.clone(), v.clone());
                    } else {
                        map.insert(k.clone(), format!("{}{}{}", existing, sep, v));
                    }
                }
                EnvOp::Unset => {
                    map.remove(k);
                }
            }
        }
        map
    }
}

#[derive(Debug, Clone)]
pub struct IncludeDirsData {
    pub dirs: Vec<String>,
    pub is_system: bool,
}

#[derive(Debug, Clone)]
pub struct GeneratorData {
    pub exe: Box<Object>,
    pub arguments: Vec<String>,
    pub output: Vec<String>,
    pub depfile: Option<String>,
    pub capture: bool,
}

#[derive(Debug, Clone)]
pub struct GeneratedListData {
    pub generator: GeneratorData,
    pub sources: Vec<String>,
    pub extra_args: Vec<String>,
    pub preserve_path_from: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RunResultData {
    pub returncode: i64,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone)]
pub struct MachineInfoData {
    pub system: String,
    pub cpu_family: String,
    pub cpu: String,
    pub endian: String,
    pub kernel: String,
    pub subsystem: String,
}

impl MachineInfoData {
    pub fn detect() -> Self {
        let system = if cfg!(target_os = "linux") {
            "linux"
        } else if cfg!(target_os = "macos") {
            "darwin"
        } else if cfg!(target_os = "windows") {
            "windows"
        } else if cfg!(target_os = "freebsd") {
            "freebsd"
        } else if cfg!(target_os = "netbsd") {
            "netbsd"
        } else if cfg!(target_os = "openbsd") {
            "openbsd"
        } else if cfg!(target_os = "dragonfly") {
            "dragonfly"
        } else {
            "unknown"
        };

        let cpu_family = if cfg!(target_arch = "x86_64") {
            "x86_64"
        } else if cfg!(target_arch = "x86") {
            "x86"
        } else if cfg!(target_arch = "aarch64") {
            "aarch64"
        } else if cfg!(target_arch = "arm") {
            "arm"
        } else if cfg!(target_arch = "riscv64") {
            "riscv64"
        } else if cfg!(target_arch = "riscv32") {
            "riscv32"
        } else if cfg!(target_arch = "powerpc64") {
            "ppc64"
        } else if cfg!(target_arch = "powerpc") {
            "ppc"
        } else if cfg!(target_arch = "mips64") {
            "mips64"
        } else if cfg!(target_arch = "mips") {
            "mips"
        } else if cfg!(target_arch = "s390x") {
            "s390x"
        } else {
            "unknown"
        };

        let endian = if cfg!(target_endian = "little") {
            "little"
        } else {
            "big"
        };

        Self {
            system: system.to_string(),
            cpu_family: cpu_family.to_string(),
            cpu: cpu_family.to_string(),
            endian: endian.to_string(),
            kernel: if system == "linux" {
                "linux".to_string()
            } else {
                system.to_string()
            },
            subsystem: String::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SubprojectData {
    pub name: String,
    pub version: String,
    pub found: bool,
    pub variables: HashMap<String, Object>,
}

#[derive(Debug, Clone)]
pub struct FileData {
    pub path: String,
    pub subdir: String,
    pub is_built: bool,
}

#[derive(Debug, Clone)]
pub struct SourceSetData {
    pub rules: Rc<RefCell<Vec<SourceSetRule>>>,
}

#[derive(Debug, Clone)]
pub struct SourceSetRule {
    pub when: Vec<Object>,
    pub if_true: Vec<Object>,
    pub if_false: Vec<Object>,
}

impl SourceSetData {
    pub fn new() -> Self {
        Self {
            rules: Rc::new(RefCell::new(Vec::new())),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SourceSetResultData {
    pub sources: Vec<Object>,
    pub dependencies: Vec<Object>,
}
