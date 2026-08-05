//! What the parser makes of a document's paragraphs, for checking a rendering
//! against Word: the resolved indent, spacing, face and size of each one.
fn main() {
    let path = std::env::args().nth(1).expect("usage: dump <file.docx>");
    let data = std::fs::read(&path).expect("read the document");
    let json = docx_parser::parse_docx_native(&data).expect("parse the document");
    // The whole parse, for a tool that wants to lay the document out itself.
    if std::env::args().any(|a| a == "--json") {
        println!("{json}");
        return;
    }
    let doc: serde_json::Value = serde_json::from_str(&json).expect("valid json");
    fn walk(v: &serde_json::Value, n: &mut usize) {
        match v {
            serde_json::Value::Array(items) => items.iter().for_each(|i| walk(i, n)),
            serde_json::Value::Object(o) => {
                if o.contains_key("runs") && o.contains_key("indentLeft") {
                    let text: String = o["runs"].as_array().map(|rs| rs.iter()
                        .filter_map(|r| r.get("text").and_then(|t| t.as_str()))
                        .collect()).unwrap_or_default();
                    if *n < 30 {
                        println!(
                            "{n:3} ind {:>6.1}/{:>6.1}pt after {:>5.1} line {:?} lvl {:?} font {:?}/{:?} style {:?} {:?}",
                            o["indentLeft"].as_f64().unwrap_or(0.0),
                            o["indentFirst"].as_f64().unwrap_or(0.0),
                            o["spaceAfter"].as_f64().unwrap_or(0.0),
                            o.get("lineSpacing"),
                            o.get("numbering").and_then(|x| x.get("level")),
                            o.get("defaultFontFamily").and_then(|x| x.as_str()),
                            o.get("defaultFontSize").and_then(|x| x.as_f64()),
                            o.get("styleId").and_then(|x| x.as_str()),
                            text.chars().take(30).collect::<String>(),
                        );
                    }
                    *n += 1;
                }
                o.values().for_each(|i| walk(i, n));
            }
            _ => {}
        }
    }
    let mut n = 0;
    walk(&doc, &mut n);
    println!("{n} paragraphs");
}
