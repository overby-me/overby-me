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

/// Parse a meson_options.txt / meson.options file.
/// These use the Meson DSL but in a restricted form with only option() calls.
pub fn parse_options_file(source: &str, options: &mut HashMap<String, Object>) {
    // Simple parser that handles option() calls
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
        if let crate::ast::Statement::Expression(crate::ast::Expression::FunctionCall(
            func,
            args,
            _,
        )) = stmt
        {
            if let crate::ast::Expression::Identifier(name, _) = func.as_ref() {
                if name == "option" {
                    parse_option_call(args, options);
                }
            }
        }
    }
}

fn parse_option_call(args: &[crate::ast::Argument], options: &mut HashMap<String, Object>) {
    let mut name = String::new();
    let mut opt_type = String::new();
    let mut value = None;
    let mut description = String::new();
    let mut choices = Vec::new();
    let mut min_val = None;
    let mut max_val = None;

    for (i, arg) in args.iter().enumerate() {
        let val_str = expr_to_string(&arg.value);
        match arg.name.as_deref() {
            None if i == 0 => name = val_str,
            Some("type") => opt_type = val_str,
            Some("value") => value = Some(expr_to_object(&arg.value)),
            Some("description") => description = val_str,
            Some("choices") => {
                if let crate::ast::Expression::Array(items, _) = &arg.value {
                    choices = items.iter().map(|e| expr_to_string(e)).collect();
                }
            }
            Some("min") => min_val = expr_to_int(&arg.value),
            Some("max") => max_val = expr_to_int(&arg.value),
            _ => {}
        }
    }

    if name.is_empty() {
        return;
    }

    let default_value = value.unwrap_or_else(|| match opt_type.as_str() {
        "boolean" => Object::Bool(true),
        "integer" => Object::Int(0),
        "string" => Object::String(String::new()),
        "combo" => {
            if let Some(first) = choices.first() {
                Object::String(first.clone())
            } else {
                Object::String(String::new())
            }
        }
        "array" => Object::Array(Vec::new()),
        "feature" => Object::Feature(FeatureState::Auto),
        _ => Object::String(String::new()),
    });

    // Only set if not already overridden
    options.entry(name).or_insert(default_value);
}

fn expr_to_string(expr: &crate::ast::Expression) -> String {
    match expr {
        crate::ast::Expression::StringLiteral(s, _) => s.clone(),
        crate::ast::Expression::IntLiteral(n, _) => n.to_string(),
        crate::ast::Expression::BoolLiteral(b, _) => b.to_string(),
        crate::ast::Expression::Identifier(s, _) => s.clone(),
        _ => String::new(),
    }
}

fn expr_to_object(expr: &crate::ast::Expression) -> Object {
    match expr {
        crate::ast::Expression::StringLiteral(s, _) => Object::String(s.clone()),
        crate::ast::Expression::IntLiteral(n, _) => Object::Int(*n),
        crate::ast::Expression::BoolLiteral(b, _) => Object::Bool(*b),
        crate::ast::Expression::Array(items, _) => {
            Object::Array(items.iter().map(expr_to_object).collect())
        }
        crate::ast::Expression::Identifier(s, _) => match s.as_str() {
            "true" => Object::Bool(true),
            "false" => Object::Bool(false),
            _ => Object::String(s.clone()),
        },
        _ => Object::String(String::new()),
    }
}

fn expr_to_int(expr: &crate::ast::Expression) -> Option<i64> {
    if let crate::ast::Expression::IntLiteral(n, _) = expr {
        Some(*n)
    } else {
        None
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
