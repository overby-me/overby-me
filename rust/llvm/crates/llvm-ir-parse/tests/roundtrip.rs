//! Round-trip fidelity over the corpus.
//!
//! Every file in `corpus/` is canonical `llvm-dis` output, so parsing one and
//! printing it back has to reproduce the input byte for byte. That is a much
//! stronger property than "the parser accepted it": it says we agree with
//! upstream about numbering, ordering, spacing and every default that prints
//! as nothing.
//!
//! A failure prints the first differing line on both sides, which is almost
//! always enough to name the rule that is missing.

use std::path::{Path, PathBuf};

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus")
        .canonicalize()
        .expect("the corpus directory is part of the source tree")
}

fn corpus_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    for directory in ["rustc", "handwritten"] {
        let path = corpus_root().join(directory);
        if !path.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(&path).expect("corpus directory is readable") {
            let entry = entry.expect("directory entry is readable");
            let path = entry.path();
            if path.extension().is_some_and(|extension| extension == "ll") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// The first line that differs, with a little context, as a report.
fn first_difference(expected: &str, actual: &str) -> String {
    let expected_lines: Vec<&str> = expected.lines().collect();
    let actual_lines: Vec<&str> = actual.lines().collect();
    for (index, (want, got)) in expected_lines.iter().zip(actual_lines.iter()).enumerate() {
        if want != got {
            let start = index.saturating_sub(2);
            let mut report = format!("first difference at line {}\n", index + 1);
            for line in &expected_lines[start..index] {
                report.push_str(&format!("  context: {line}\n"));
            }
            report.push_str(&format!("  expected: {want}\n"));
            report.push_str(&format!("  actual:   {got}\n"));
            return report;
        }
    }
    format!(
        "line counts differ: expected {} lines, printed {}\n  expected next: {:?}\n  actual next:   {:?}",
        expected_lines.len(),
        actual_lines.len(),
        expected_lines.get(actual_lines.len()),
        actual_lines.get(expected_lines.len()),
    )
}

#[test]
fn corpus_round_trips_byte_for_byte() {
    let files = corpus_files();
    assert!(!files.is_empty(), "the corpus is empty");

    let mut failures = Vec::new();
    for path in &files {
        let text = std::fs::read_to_string(path).expect("corpus file is readable");
        let name = path.file_name().unwrap().to_string_lossy();
        match llvm_ir_parse::parse_module(&text) {
            Err(error) => failures.push(format!("{name}: parse failed: {error}")),
            Ok(module) => {
                let printed = llvm_ir_print::print_module(&module);
                if printed != text {
                    failures.push(format!("{name}: {}", first_difference(&text, &printed)));
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} corpus files did not round trip:\n\n{}",
        failures.len(),
        files.len(),
        failures.join("\n\n")
    );
}

#[test]
fn printing_is_idempotent() {
    // Even where we diverge from upstream's spelling, our own output has to be
    // a fixed point, or every later differential test inherits the wobble.
    for path in corpus_files() {
        let text = std::fs::read_to_string(&path).expect("corpus file is readable");
        let Ok(module) = llvm_ir_parse::parse_module(&text) else {
            continue;
        };
        let once = llvm_ir_print::print_module(&module);
        let reparsed = llvm_ir_parse::parse_module(&once).unwrap_or_else(|error| {
            panic!("{}: our own output did not parse: {error}", path.display())
        });
        let twice = llvm_ir_print::print_module(&reparsed);
        assert_eq!(
            once,
            twice,
            "{}: printing is not a fixed point\n{}",
            path.display(),
            first_difference(&once, &twice)
        );
    }
}
