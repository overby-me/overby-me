/// High-level interpreter that ties together lexer, parser, compiler, and VM.
use std::path::Path;

use crate::builtins;
use crate::compiler::Compiler;
use crate::lexer::Lexer;
use crate::options;
use crate::parser::Parser;
use crate::vm::VM;

pub struct Interpreter {
    pub vm: VM,
}

impl Interpreter {
    pub fn new(source_root: &str, build_root: &str) -> Self {
        let mut vm = VM::new();
        vm.source_root = source_root.to_string();
        vm.build_root = build_root.to_string();
        vm.top_source_root = source_root.to_string();
        vm.top_build_root = build_root.to_string();
        builtins::register_all(&mut vm);
        Self { vm }
    }

    pub fn set_options(&mut self, opts: &[(String, String)]) {
        for (key, value) in opts {
            let obj = options::parse_option_value(value);
            self.vm.options.insert(key.clone(), obj);
        }
    }

    pub fn load_options_file(&mut self, source_root: &str) {
        // Try meson.options first, then meson_options.txt
        let options_file = format!("{}/meson.options", source_root);
        let legacy_options = format!("{}/meson_options.txt", source_root);

        let path = if Path::new(&options_file).exists() {
            Some(options_file)
        } else if Path::new(&legacy_options).exists() {
            Some(legacy_options)
        } else {
            None
        };

        if let Some(path) = path {
            if let Ok(source) = std::fs::read_to_string(&path) {
                options::parse_options_file(&source, &mut self.vm.options);
            }
        }
    }

    pub fn run(&mut self, source: &str) -> Result<(), String> {
        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize()?;
        let mut parser = Parser::new(tokens);
        let program = parser.parse()?;
        let mut compiler = Compiler::new();
        compiler.compile(&program)?;
        match self.vm.execute(&compiler.chunk) {
            Ok(_) => Ok(()),
            Err(e) if e == "SUBDIR_DONE" => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub fn print_summary(&self) {
        if self.vm.summary.is_empty() {
            return;
        }
        eprintln!();
        for (section, items) in &self.vm.summary {
            if !section.is_empty() {
                eprintln!("  {}:", section);
            }
            for (key, value) in items {
                eprintln!("    {:<30} : {}", key, value);
            }
        }
        eprintln!();
    }
}
