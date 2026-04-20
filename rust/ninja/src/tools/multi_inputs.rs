//! `-t multi-inputs` — for each target, list its direct (explicit + implicit
//! + order-only) inputs, one per line, prefixed with the target.
//!
//! Output format is `<target><delim><input>` per line. Default delimiter
//! is a tab; `-d <char>` overrides; `--print0` switches the line
//! terminator to NUL.

use crate::graph::State;

#[derive(Default)]
struct Args {
    delim: Option<String>,
    print0: bool,
    targets: Vec<String>,
}

pub fn run(state: &State, args: &[String]) -> Result<u8, String> {
    let parsed = parse_args(args)?;
    let delim = parsed.delim.as_deref().unwrap_or("\t");
    let term = if parsed.print0 { '\0' } else { '\n' };
    let mut out = String::new();
    for t in &parsed.targets {
        if let Some(&idx) = state.producers.get(t) {
            let edge = &state.edges[idx];
            for inp in edge
                .inputs
                .iter()
                .chain(&edge.implicit_inputs)
                .chain(&edge.order_only_inputs)
            {
                out.push_str(t);
                out.push_str(delim);
                out.push_str(inp);
                out.push(term);
            }
        }
    }
    print!("{out}");
    Ok(0)
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut a = Args::default();
    let mut i = 0;
    while i < args.len() {
        let s = args[i].as_str();
        match s {
            "--print0" => a.print0 = true,
            "-d" => {
                i += 1;
                a.delim = Some(
                    args.get(i)
                        .cloned()
                        .ok_or_else(|| "-d needs an argument".to_string())?,
                );
            }
            s2 if s2.starts_with("-d") => a.delim = Some(s2[2..].to_string()),
            s2 if s2.starts_with('-') => return Err(format!("multi-inputs: unknown flag {s2}")),
            _ => a.targets.push(args[i].clone()),
        }
        i += 1;
    }
    Ok(a)
}
