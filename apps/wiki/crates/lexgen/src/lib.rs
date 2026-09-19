//! Turns the wiki's lexicons into the typed client in `appview-client`.
//!
//! The lexicons are what the AppView says it serves. Generating the client from
//! them, and running that client against the real router with unknown fields
//! refused (`appview-client/tests/contract.rs`), is what holds all three to one
//! another: a field the server sends and no lexicon names fails there, and so
//! does one a lexicon promises and the server does not send.
//!
//! It reads the subset of the lexicon language these files use: objects,
//! arrays, strings, integers, booleans, `unknown`, refs into `defs.json` or the
//! file's own defs, and a union of views told apart by their `node` field.

use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Write;

const NSID: &str = "com.example.wiki.";

/// `fooBar` as `foo_bar`.
fn snake(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_uppercase() {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

/// `fooBar` or `foo_bar` as `FooBar`.
fn pascal(name: &str) -> String {
    let mut out = String::new();
    let mut upper = true;
    for c in name.chars() {
        match c {
            '_' => upper = true,
            c if upper => {
                out.push(c.to_ascii_uppercase());
                upper = false;
            }
            c => out.push(c),
        }
    }
    out
}

fn doc(out: &mut String, indent: &str, schema: &Value) {
    if let Some(text) = schema.get("description").and_then(Value::as_str) {
        for line in text.lines() {
            let _ = writeln!(out, "{indent}/// {}", line.trim_end());
        }
    }
}

/// One module's worth of structs, collected as the schema is walked.
struct Module {
    /// Whether this is `defs`, where a ref to a shared view needs no path.
    shared: bool,
    structs: BTreeMap<String, String>,
}

impl Module {
    fn reference(&self, target: &str) -> String {
        let shared = target.strip_prefix(&format!("{NSID}defs#"));
        match (shared, target.strip_prefix('#')) {
            (Some(name), _) if self.shared => pascal(name),
            (Some(name), _) => format!("defs::{}", pascal(name)),
            (None, Some(name)) => pascal(name),
            _ => "serde_json::Value".to_string(),
        }
    }

    /// The Rust type of `schema`, declaring any struct it needs as `name`.
    fn rust_type(&mut self, name: &str, schema: &Value) -> String {
        match schema
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
        {
            "string" => "String".to_string(),
            "integer" => "i64".to_string(),
            "boolean" => "bool".to_string(),
            "ref" => self.reference(schema["ref"].as_str().unwrap_or_default()),
            "array" => {
                let item = self.rust_type(&format!("{name}Item"), &schema["items"]);
                format!("Vec<{item}>")
            }
            "object" if schema.get("properties").is_some() => {
                self.object(name, schema);
                name.to_string()
            }
            "union" => {
                self.union(name, schema);
                name.to_string()
            }
            _ => "serde_json::Value".to_string(),
        }
    }

    fn object(&mut self, name: &str, schema: &Value) {
        let required: Vec<&str> = schema["required"]
            .as_array()
            .map(|r| r.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        // Absent leaves a field as it is and `null` clears it, where a lexicon
        // says a field is nullable: two things one `Option` cannot say.
        let nullable: Vec<&str> = schema["nullable"]
            .as_array()
            .map(|n| n.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let mut body = String::new();
        doc(&mut body, "", schema);
        let empty = serde_json::Map::new();
        // A union has no default, so neither has what holds one.
        let holds_union = schema["properties"]
            .as_object()
            .unwrap_or(&empty)
            .values()
            .any(|inner| inner["type"] == "union");
        let default = if holds_union { "" } else { "Default, " };
        let _ = writeln!(
            body,
            "#[derive(Debug, Clone, {default}PartialEq, Serialize, Deserialize)]"
        );
        body.push_str("#[cfg_attr(feature = \"strict\", serde(deny_unknown_fields))]\n");
        let _ = writeln!(body, "pub struct {name} {{");
        for (field, inner) in schema["properties"].as_object().unwrap_or(&empty) {
            let ty = self.rust_type(&format!("{name}{}", pascal(field)), inner);
            doc(&mut body, "    ", inner);
            if required.contains(&field.as_str()) {
                let _ = writeln!(body, "    pub {field}: {ty},");
            } else if nullable.contains(&field.as_str()) {
                body.push_str("    #[serde(default, skip_serializing_if = \"Option::is_none\")]\n");
                let _ = writeln!(body, "    pub {field}: Option<Option<{ty}>>,");
            } else {
                body.push_str("    #[serde(default, skip_serializing_if = \"Option::is_none\")]\n");
                let _ = writeln!(body, "    pub {field}: Option<{ty}>,");
            }
        }
        body.push_str("}\n");
        self.structs.insert(name.to_string(), body);
    }

    /// A union of views, told apart by the `node` field the server adds.
    fn union(&mut self, name: &str, schema: &Value) {
        let mut body = String::new();
        doc(&mut body, "", schema);
        body.push_str("#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\n");
        body.push_str("#[serde(tag = \"node\", rename_all = \"snake_case\")]\n");
        let _ = writeln!(body, "pub enum {name} {{");
        for target in schema["refs"].as_array().into_iter().flatten() {
            let target = target.as_str().unwrap_or_default();
            let view = target.rsplit('#').next().unwrap_or_default();
            let variant = pascal(view.strip_suffix("View").unwrap_or(view));
            let _ = writeln!(body, "    {variant}({}),", self.reference(target));
        }
        body.push_str("}\n");
        self.structs.insert(name.to_string(), body);
    }

    fn render(&self, out: &mut String) {
        for body in self.structs.values() {
            for line in body.lines() {
                let _ = writeln!(out, "    {line}");
            }
            out.push('\n');
        }
    }
}

/// A `type Name = ...;` for a body that is a ref, else the struct `name`.
fn body_type(module: &mut Module, name: &str, schema: &Value) -> Option<String> {
    let ty = module.rust_type(name, schema);
    (ty != name).then(|| format!("    pub type {name} = {ty};\n\n"))
}

struct Method {
    fn_name: String,
    nsid: String,
    description: Option<String>,
    query: bool,
    params: bool,
    /// `Some(true)` for a JSON body, `Some(false)` for raw bytes.
    input: Option<bool>,
    /// Whether the answer is JSON. Bytes otherwise.
    json_out: bool,
}

/// Generate the client source from every lexicon in `lexicons`, keyed by file
/// stem. Deterministic: the same lexicons give the same bytes.
pub fn generate(lexicons: &BTreeMap<String, Value>) -> String {
    let mut out = String::new();
    out.push_str(
        "// @generated by `cargo run -p lexgen`, from apps/wiki/lexicons. Do not edit:\n\
         // change the lexicon and run it again. `tests/contract.rs` fails when stale.\n\n\
         #![allow(clippy::derivable_impls, clippy::large_enum_variant)]\n\n\
         use crate::{Binary, Client, Error};\n\n",
    );
    let mut methods = Vec::new();
    for (stem, lexicon) in lexicons {
        let empty = serde_json::Map::new();
        let defs = lexicon["defs"].as_object().unwrap_or(&empty);
        let main = defs.get("main");
        let kind = main.and_then(|m| m["type"].as_str()).unwrap_or_default();
        if kind == "record" {
            continue;
        }
        let mut module = Module {
            shared: stem == "defs",
            structs: BTreeMap::new(),
        };
        for (name, schema) in defs.iter().filter(|(name, _)| *name != "main") {
            module.rust_type(&pascal(name), schema);
        }
        let mut aliases = String::new();
        if let Some(main) = main {
            let mut method = Method {
                fn_name: snake(stem),
                nsid: format!("{NSID}{stem}"),
                description: main["description"].as_str().map(str::to_string),
                query: kind == "query",
                params: false,
                input: None,
                json_out: true,
            };
            if let Some(params) = main.get("parameters") {
                let mut as_object = params.clone();
                as_object["type"] = Value::from("object");
                module.object("Params", &as_object);
                method.params = true;
            }
            if let Some(input) = main.get("input") {
                let json = input["encoding"] == "application/json";
                method.input = Some(json);
                if json {
                    aliases.extend(body_type(&mut module, "Input", &input["schema"]));
                }
            }
            if let Some(output) = main.get("output") {
                method.json_out = output["encoding"] == "application/json";
                if method.json_out {
                    aliases.extend(body_type(&mut module, "Output", &output["schema"]));
                }
            }
            methods.push(method);
        }
        let _ = writeln!(out, "pub mod {} {{", snake(stem));
        if !module.shared {
            out.push_str("    #[allow(unused_imports)]\n    use super::defs;\n");
        }
        out.push_str("    #[allow(unused_imports)]\n    use serde::{Deserialize, Serialize};\n\n");
        out.push_str(&aliases);
        module.render(&mut out);
        out.push_str("}\n\n");
        if let Some(params) = main.and_then(|m| m.get("parameters")) {
            query_pairs(&mut out, &snake(stem), params);
        }
    }
    out.push_str("impl Client {\n");
    for method in &methods {
        render_method(&mut out, method);
    }
    out.push_str("}\n");
    out
}

/// How a `Params` becomes a query string.
fn query_pairs(out: &mut String, module: &str, params: &Value) {
    let required: Vec<&str> = params["required"]
        .as_array()
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let _ = writeln!(out, "impl {module}::Params {{");
    out.push_str("    pub(crate) fn pairs(&self) -> Vec<(&'static str, String)> {\n");
    let empty = serde_json::Map::new();
    let fields = params["properties"].as_object().unwrap_or(&empty);
    let (always, maybe): (Vec<&String>, Vec<&String>) = fields
        .keys()
        .partition(|field| required.contains(&field.as_str()));
    let binding = if maybe.is_empty() { "let" } else { "let mut" };
    let _ = writeln!(out, "        {binding} pairs = vec![");
    for field in always {
        let _ = writeln!(out, "            (\"{field}\", self.{field}.to_string()),");
    }
    out.push_str("        ];\n");
    for field in maybe {
        let _ = writeln!(out, "        if let Some(value) = &self.{field} {{");
        let _ = writeln!(
            out,
            "            pairs.push((\"{field}\", value.to_string()));"
        );
        out.push_str("        }\n");
    }
    out.push_str("        pairs\n    }\n}\n\n");
}

fn render_method(out: &mut String, method: &Method) {
    let module = &method.fn_name;
    if let Some(description) = &method.description {
        for line in description.lines() {
            let _ = writeln!(out, "    /// {}", line.trim_end());
        }
    }
    let mut args = String::from("&self");
    if method.params {
        let _ = write!(args, ", params: &{module}::Params");
    }
    match method.input {
        Some(true) => {
            let _ = write!(args, ", input: &{module}::Input");
        }
        Some(false) => args.push_str(", body: Vec<u8>, content_type: &str"),
        None => {}
    }
    let returns = match method.json_out {
        true => format!("{module}::Output"),
        false => "Binary".to_string(),
    };
    let _ = writeln!(
        out,
        "    pub async fn {module}({args}) -> Result<{returns}, Error> {{"
    );
    let pairs = match method.params {
        true => "params.pairs()",
        false => "Vec::new()",
    };
    let verb = if method.query { "Get" } else { "Post" };
    let body = match method.input {
        Some(true) => "crate::Body::Json(serde_json::to_value(input)?)",
        Some(false) => "crate::Body::Bytes(body, content_type.to_string())",
        None => "crate::Body::None",
    };
    let _ = writeln!(
        out,
        "        let answer = self.call(crate::Verb::{verb}, \"{}\", {pairs}, {body}).await?;",
        method.nsid
    );
    match method.json_out {
        true => out.push_str("        answer.json()\n"),
        false => out.push_str("        Ok(answer.binary())\n"),
    }
    out.push_str("    }\n\n");
}

/// Every lexicon under `dir`, by file stem.
pub fn read_lexicons(dir: &std::path::Path) -> std::io::Result<BTreeMap<String, Value>> {
    let mut lexicons = BTreeMap::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let value = serde_json::from_slice(&std::fs::read(&path)?)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        lexicons.insert(stem, value);
    }
    Ok(lexicons)
}
