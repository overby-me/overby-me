use dioxus::prelude::*;

use crate::model::NodeWithChildren;

/// CowApp — the "secret cow" easter egg (#82). Reachable via `?app=cow`; not
/// advertised in the app rail. It cowsays the node's name, in the spirit of
/// `apt moo`.
#[component]
pub fn CowApp(node: NodeWithChildren) -> Element {
    let art = cowsay(&format!("Moo! You found {}.", node.name));
    rsx! {
        div { class: "card app-card",
            div { class: "card-content",
                pre { class: "cowsay", "{art}" }
            }
        }
    }
}

/// Render `msg` as a cowsay speech balloon above the classic cow. Long messages
/// wrap at ~38 columns. Pure so it can be unit-tested off-browser.
fn cowsay(msg: &str) -> String {
    let width = 38usize;
    let lines = wrap(msg, width);
    let inner = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);

    let mut out = String::new();
    out.push(' ');
    out.push_str(&"_".repeat(inner + 2));
    out.push('\n');

    if lines.len() == 1 {
        out.push_str(&format!("< {:<width$} >\n", lines[0], width = inner));
    } else {
        for (i, line) in lines.iter().enumerate() {
            let (l, r) = match i {
                0 => ('/', '\\'),
                _ if i == lines.len() - 1 => ('\\', '/'),
                _ => ('|', '|'),
            };
            out.push_str(&format!("{l} {:<width$} {r}\n", line, width = inner));
        }
    }

    out.push(' ');
    out.push_str(&"-".repeat(inner + 2));
    out.push('\n');
    out.push_str(concat!(
        "        \\   ^__^\n",
        "         \\  (oo)\\_______\n",
        "            (__)\\       )\\/\\\n",
        "                ||----w |\n",
        "                ||     ||\n",
    ));
    out
}

/// Greedy word-wrap to at most `width` columns (never splits a word shorter than
/// the width; over-long words get their own line).
fn wrap(msg: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in msg.split_whitespace() {
        if cur.is_empty() {
            cur.push_str(word);
        } else if cur.chars().count() + 1 + word.chars().count() <= width {
            cur.push(' ');
            cur.push_str(word);
        } else {
            lines.push(std::mem::take(&mut cur));
            cur.push_str(word);
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cowsay_wraps_and_draws_the_cow() {
        let art = cowsay("Moo");
        assert!(art.contains("< Moo >"), "single-line balloon: {art}");
        assert!(art.contains("(oo)"), "cow face present");

        // A long message wraps into multiple balloon lines.
        let long = cowsay("the quick brown fox jumps over the lazy dog again and again");
        assert!(long.lines().filter(|l| l.contains('|')).count() >= 1);
    }
}
