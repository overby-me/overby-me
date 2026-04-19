use crate::ast;
use crate::objects::{FeatureState, Object};
/// Options system: parsing meson_options.txt/meson.options, built-in options,
/// cross/native files, and version comparison.
use std::collections::HashMap;

/// Parse a value string into an Object
pub fn parse_option_value(value: &str) -> Object {
    match value {
        "true" => Object::Bool(true),
        "false" => Object::Bool(false),
        "enabled" => Object::Feature(FeatureState::Enabled),
        "disabled" => Object::Feature(FeatureState::Disabled),
        "auto" => Object::Feature(FeatureState::Auto),
        _ => {
            if let Ok(n) = value.parse::<i64>() {
                Object::Int(n)
            } else if value.starts_with('[') && value.ends_with(']') {
                let inner = &value[1..value.len() - 1];
                let items: Vec<Object> = inner
                    .split(',')
                    .map(|s| {
                        Object::String(s.trim().trim_matches('\'').trim_matches('"').to_string())
                    })
                    .collect();
                Object::Array(items)
            } else {
                Object::String(value.to_string())
            }
        }
    }
}

/// Evaluate a constant expression from the options file AST.
fn eval_const_expr(expr: &ast::Expression) -> Option<Object> {
    match expr {
        ast::Expression::StringLiteral(s, _) => Some(Object::String(s.clone())),
        ast::Expression::IntLiteral(n, _) => Some(Object::Int(*n)),
        ast::Expression::BoolLiteral(b, _) => Some(Object::Bool(*b)),
        ast::Expression::Identifier(s, _) => match s.as_str() {
            "true" => Some(Object::Bool(true)),
            "false" => Some(Object::Bool(false)),
            _ => Some(Object::String(s.clone())),
        },
        ast::Expression::Array(items, _) => {
            let evaluated: Option<Vec<Object>> = items.iter().map(eval_const_expr).collect();
            evaluated.map(Object::Array)
        }
        ast::Expression::UnaryOp(op, inner, _) => {
            let val = eval_const_expr(inner)?;
            match op {
                ast::UnaryOp::Not => {
                    if let Object::Bool(b) = val {
                        Some(Object::Bool(!b))
                    } else {
                        None
                    }
                }
                ast::UnaryOp::Negate => {
                    if let Object::Int(n) = val {
                        Some(Object::Int(-n))
                    } else {
                        None
                    }
                }
            }
        }
        ast::Expression::BinaryOp(op, left, right, _) => {
            let l = eval_const_expr(left)?;
            let r = eval_const_expr(right)?;
            match op {
                ast::BinaryOp::Add => match (l, r) {
                    (Object::String(a), Object::String(b)) => Some(Object::String(a + &b)),
                    (Object::Int(a), Object::Int(b)) => Some(Object::Int(a + b)),
                    _ => None,
                },
                ast::BinaryOp::Sub => match (l, r) {
                    (Object::Int(a), Object::Int(b)) => Some(Object::Int(a - b)),
                    _ => None,
                },
                ast::BinaryOp::Mul => match (l, r) {
                    (Object::Int(a), Object::Int(b)) => Some(Object::Int(a * b)),
                    _ => None,
                },
                _ => None,
            }
        }
        ast::Expression::Dict(entries, _) => {
            let mut map = Vec::new();
            for (k, v) in entries {
                let key = eval_const_expr(k)?;
                let val = eval_const_expr(v)?;
                map.push((obj_to_string(&key), val));
            }
            Some(Object::Dict(map))
        }
        _ => None,
    }
}

fn obj_to_string(obj: &Object) -> String {
    match obj {
        Object::String(s) => s.clone(),
        Object::Int(n) => n.to_string(),
        Object::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

/// Parse a meson_options.txt / meson.options file.
pub fn parse_options_file(source: &str, options: &mut HashMap<String, Object>) {
    let mut lexer = crate::lexer::Lexer::new(source);
    let tokens = match lexer.tokenize() {
        Ok(t) => t,
        Err(_) => return,
    };
    let mut parser = crate::parser::Parser::new(tokens);
    let program = match parser.parse() {
        Ok(p) => p,
        Err(_) => return,
    };
    for stmt in &program.statements {
        if let ast::Statement::Expression(ast::Expression::FunctionCall(func, args, _)) = stmt {
            if let ast::Expression::Identifier(name, _) = func.as_ref() {
                if name == "option" {
                    parse_option_call(args, options);
                }
            }
        }
    }
}

fn parse_option_call(args: &[ast::Argument], options: &mut HashMap<String, Object>) {
    let mut name = String::new();
    let mut opt_type = String::new();
    let mut value = None;
    let mut choices = Vec::new();
    for (i, arg) in args.iter().enumerate() {
        match arg.name.as_deref() {
            None if i == 0 => {
                if let Some(obj) = eval_const_expr(&arg.value) {
                    name = obj_to_string(&obj);
                }
            }
            Some("type") => {
                if let Some(obj) = eval_const_expr(&arg.value) {
                    opt_type = obj_to_string(&obj);
                }
            }
            Some("value") => {
                value = eval_const_expr(&arg.value);
            }
            Some("choices") => {
                if let Some(Object::Array(items)) = eval_const_expr(&arg.value) {
                    choices = items.into_iter().map(|o| obj_to_string(&o)).collect();
                }
            }
            _ => {}
        }
    }
    if name.is_empty() {
        return;
    }
    let value = value.map(|v| coerce_option_value(v, &opt_type));
    let default_value = value.unwrap_or_else(|| match opt_type.as_str() {
        "boolean" => Object::Bool(true),
        "integer" => Object::Int(0),
        "string" => Object::String(String::new()),
        "combo" => choices
            .first()
            .map(|f| Object::String(f.clone()))
            .unwrap_or(Object::String(String::new())),
        "array" => {
            if choices.is_empty() {
                Object::Array(Vec::new())
            } else {
                Object::Array(choices.iter().map(|c| Object::String(c.clone())).collect())
            }
        }
        "feature" => Object::Feature(FeatureState::Auto),
        _ => Object::String(String::new()),
    });
    options.entry(name).or_insert(default_value);
}

/// Option definition with metadata (including yield)
pub struct OptionDef {
    pub name: String,
    pub opt_type: String,
    pub default_value: Object,
    pub yield_to_parent: bool,
    pub deprecated: Option<DeprecatedInfo>,
}

/// Information about deprecated option
pub enum DeprecatedInfo {
    /// Renamed to another option
    Renamed(String),
    /// Value remapping
    ValueMap(std::collections::HashMap<String, String>),
}

fn default_for_type(opt_type: &str, choices: &[String]) -> Object {
    match opt_type {
        "boolean" => Object::Bool(true),
        "integer" => Object::Int(0),
        "string" => Object::String(String::new()),
        "combo" => choices
            .first()
            .map(|f| Object::String(f.clone()))
            .unwrap_or(Object::String(String::new())),
        "array" => {
            if choices.is_empty() {
                Object::Array(Vec::new())
            } else {
                Object::Array(choices.iter().map(|c| Object::String(c.clone())).collect())
            }
        }
        "feature" => Object::Feature(FeatureState::Auto),
        _ => Object::String(String::new()),
    }
}

/// Parse a meson_options.txt / meson.options file and return structured defs.
pub fn parse_options_file_defs(source: &str) -> Vec<OptionDef> {
    let mut defs = Vec::new();
    let mut lexer = crate::lexer::Lexer::new(source);
    let tokens = match lexer.tokenize() {
        Ok(t) => t,
        Err(_) => return defs,
    };
    let mut parser = crate::parser::Parser::new(tokens);
    let program = match parser.parse() {
        Ok(p) => p,
        Err(_) => return defs,
    };
    for stmt in &program.statements {
        if let ast::Statement::Expression(ast::Expression::FunctionCall(func, args, _)) = stmt {
            if let ast::Expression::Identifier(name, _) = func.as_ref() {
                if name == "option" {
                    if let Some(def) = parse_option_call_def(args) {
                        defs.push(def);
                    }
                }
            }
        }
    }
    defs
}

fn parse_option_call_def(args: &[ast::Argument]) -> Option<OptionDef> {
    let mut name = String::new();
    let mut opt_type = String::new();
    let mut value = None;
    let mut choices = Vec::new();
    let mut yield_to_parent = false;
    let mut deprecated = None;

    for (i, arg) in args.iter().enumerate() {
        match arg.name.as_deref() {
            None if i == 0 => {
                if let Some(obj) = eval_const_expr(&arg.value) {
                    name = obj_to_string(&obj);
                }
            }
            Some("type") => {
                if let Some(obj) = eval_const_expr(&arg.value) {
                    opt_type = obj_to_string(&obj);
                }
            }
            Some("value") => {
                value = eval_const_expr(&arg.value);
            }
            Some("choices") => {
                if let Some(Object::Array(items)) = eval_const_expr(&arg.value) {
                    choices = items.into_iter().map(|o| obj_to_string(&o)).collect();
                }
            }
            Some("yield") => {
                if let Some(Object::Bool(b)) = eval_const_expr(&arg.value) {
                    yield_to_parent = b;
                }
            }
            Some("deprecated") => match eval_const_expr(&arg.value) {
                Some(Object::String(s)) => {
                    deprecated = Some(DeprecatedInfo::Renamed(s));
                }
                Some(Object::Dict(entries)) => {
                    let map: std::collections::HashMap<String, String> = entries
                        .into_iter()
                        .map(|(k, v)| (k, obj_to_string(&v)))
                        .collect();
                    deprecated = Some(DeprecatedInfo::ValueMap(map));
                }
                _ => {}
            },
            _ => {}
        }
    }
    if name.is_empty() {
        return None;
    }
    let value = value.map(|v| coerce_option_value(v, &opt_type));
    let default_value = value.unwrap_or_else(|| default_for_type(&opt_type, &choices));
    Some(OptionDef {
        name,
        opt_type,
        default_value,
        yield_to_parent,
        deprecated,
    })
}

fn coerce_option_value(value: Object, opt_type: &str) -> Object {
    match opt_type {
        "boolean" => match &value {
            Object::Bool(_) => value,
            Object::String(s) => match s.as_str() {
                "true" => Object::Bool(true),
                "false" => Object::Bool(false),
                _ => value,
            },
            _ => value,
        },
        "integer" => match &value {
            Object::Int(_) => value,
            Object::String(s) => s.parse::<i64>().map(Object::Int).unwrap_or(value),
            _ => value,
        },
        "feature" => match &value {
            Object::Feature(_) => value,
            Object::String(s) => match s.as_str() {
                "enabled" => Object::Feature(FeatureState::Enabled),
                "disabled" => Object::Feature(FeatureState::Disabled),
                "auto" => Object::Feature(FeatureState::Auto),
                _ => value,
            },
            _ => value,
        },
        _ => value,
    }
}

/// Compare a version string against a constraint like ">=1.0", "<2.0", "==1.5"
pub fn version_compare(version: &str, constraint: &str) -> bool {
    let constraint = constraint.trim();
    if constraint.is_empty() {
        return true;
    }

    let (op, required) = if constraint.starts_with(">=") {
        (">=", &constraint[2..])
    } else if constraint.starts_with("<=") {
        ("<=", &constraint[2..])
    } else if constraint.starts_with("!=") {
        ("!=", &constraint[2..])
    } else if constraint.starts_with("==") {
        ("==", &constraint[2..])
    } else if constraint.starts_with('>') {
        (">", &constraint[1..])
    } else if constraint.starts_with('<') {
        ("<", &constraint[1..])
    } else {
        // No operator means >=
        (">=", constraint)
    };

    let required = required.trim();
    let cmp = compare_versions(version, required);

    match op {
        ">=" => cmp >= 0,
        "<=" => cmp <= 0,
        ">" => cmp > 0,
        "<" => cmp < 0,
        "==" => cmp == 0,
        "!=" => cmp != 0,
        _ => false,
    }
}

/// Compare two version strings. Returns -1, 0, or 1.
fn compare_versions(a: &str, b: &str) -> i32 {
    let a_parts: Vec<&str> = a.split('.').collect();
    let b_parts: Vec<&str> = b.split('.').collect();
    let max_len = a_parts.len().max(b_parts.len());

    for i in 0..max_len {
        let a_part = a_parts.get(i).unwrap_or(&"0");
        let b_part = b_parts.get(i).unwrap_or(&"0");

        let a_num = a_part.parse::<i64>().unwrap_or(0);
        let b_num = b_part.parse::<i64>().unwrap_or(0);

        if a_num < b_num {
            return -1;
        } else if a_num > b_num {
            return 1;
        }
    }
    0
}

/// Parse a cross/native file (INI-like format)
pub fn parse_machine_file(path: &str) -> HashMap<String, HashMap<String, String>> {
    let mut sections: HashMap<String, HashMap<String, String>> = HashMap::new();
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return sections,
    };

    let mut current_section = String::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            current_section = line[1..line.len() - 1].to_string();
            sections.entry(current_section.clone()).or_default();
        } else if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim().to_string();
            let value = line[eq_pos + 1..]
                .trim()
                .trim_matches('\'')
                .trim_matches('"')
                .to_string();
            sections
                .entry(current_section.clone())
                .or_default()
                .insert(key, value);
        }
    }

    sections
}
