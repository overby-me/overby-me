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

/// What upstream folds an aggregate into when every element says the same
/// thing. Written out longhand on the left, folded on the right, which is a
/// change the corpus cannot pin because `llvm-dis` never writes the longhand.
const UPGRADED: &[(&str, &str)] = &[
    // The four debug-info intrinsics are the older spelling of the debug
    // records, and upstream reads a call to one as the record, taking the
    // location from the call's `!dbg` and dropping the declaration.
    (
        "declare void @llvm.dbg.declare(metadata, metadata, metadata)\n\ndefine void @f(ptr %p) !dbg !5 {\nentry:\n  call void @llvm.dbg.declare(metadata ptr %p, metadata !4, metadata !DIExpression()), !dbg !8\n  ret void\n}\n\n!llvm.module.flags = !{!0}\n!llvm.dbg.cu = !{!1}\n\n!0 = !{i32 2, !\"Debug Info Version\", i32 3}\n!1 = distinct !DICompileUnit(language: DW_LANG_C99, file: !2, producer: \"p\", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug)\n!2 = !DIFile(filename: \"a.c\", directory: \"/\")\n!4 = !DILocalVariable(name: \"v\", scope: !5, file: !2, line: 1)\n!5 = distinct !DISubprogram(name: \"f\", scope: !2, file: !2, line: 1, type: !6, spFlags: DISPFlagDefinition, unit: !1)\n!6 = !DISubroutineType(types: !7)\n!7 = !{null}\n!8 = !DILocation(line: 1, column: 1, scope: !5)\n",
        "    #dbg_declare(ptr %p, !6, !DIExpression(), !7)",
    ),
    // The declaration goes even when nothing called it.
    (
        "declare void @llvm.dbg.value(metadata, metadata, metadata)\n\ndefine void @f() {\nentry:\n  ret void\n}\n",
        "define void @f() {",
    ),
];

#[test]
fn debug_intrinsic_calls_become_records() {
    for (written, expected) in UPGRADED {
        let module = llvm_ir_parse::parse_module(written)
            .unwrap_or_else(|error| panic!("{written} did not parse: {error}"));
        let printed = llvm_ir_print::print_module(&module);
        assert!(
            printed.lines().any(|line| line == *expected),
            "expected a line {expected:?}\n--- printed ---\n{printed}"
        );
        assert!(
            !printed
                .lines()
                .any(|line| line.starts_with("declare") && line.contains("llvm.dbg.")),
            "the declaration should not be printed\n--- printed ---\n{printed}"
        );
    }
}

/// What upstream drops on the way out. `llvm-dis` prints no use-list order
/// directives unless asked to, so a module holding them prints back without
/// them, and an alias whose aliasee is an expression writes no type in front
/// of it.
const DROPPED: &[(&str, &str)] = &[
    (
        "@a = global i32 0\n\nuselistorder ptr @a, { 1, 0 }\n",
        "@a = global i32 0\n",
    ),
    (
        "@a = global [4 x i1] zeroinitializer\n@b = alias i1, getelementptr ([4 x i1], ptr @a, i64 0, i64 2)\n",
        "@a = global [4 x i1] zeroinitializer\n\n@b = alias i1, getelementptr ([4 x i1], ptr @a, i64 0, i64 2)\n",
    ),
    (
        "@a = global i32 0\n@b = alias i32, ptr @a\n",
        "@a = global i32 0\n\n@b = alias i32, ptr @a\n",
    ),
];

#[test]
fn what_upstream_drops_is_dropped() {
    for (written, expected) in DROPPED {
        let module = llvm_ir_parse::parse_module(written)
            .unwrap_or_else(|error| panic!("{written} did not parse: {error}"));
        let printed = llvm_ir_print::print_module(&module);
        assert_eq!(
            printed.trim_start_matches('\n'),
            *expected,
            "\nfrom: {written}"
        );
    }
}

const FOLDED: &[(&str, &str)] = &[
    (
        "@g = global <4 x i16> <i16 -1, i16 -1, i16 -1, i16 -1>\n",
        "@g = global <4 x i16> splat (i16 -1)\n",
    ),
    (
        "@g = global <4 x i16> <i16 0, i16 0, i16 0, i16 0>\n",
        "@g = global <4 x i16> zeroinitializer\n",
    ),
    (
        "@g = global <2 x ptr> <ptr null, ptr null>\n",
        "@g = global <2 x ptr> zeroinitializer\n",
    ),
    (
        "@g = global <4 x i16> <i16 undef, i16 undef, i16 undef, i16 undef>\n",
        "@g = global <4 x i16> undef\n",
    ),
    (
        "@g = global <4 x i16> <i16 poison, i16 poison, i16 poison, i16 poison>\n",
        "@g = global <4 x i16> poison\n",
    ),
    // A negative zero has a bit set, so it splats rather than zeroing.
    (
        "@g = global <2 x float> <float -0.0, float -0.0>\n",
        "@g = global <2 x float> splat (float -0.000000e+00)\n",
    ),
    // An array folds to zero but never to a splat, which is upstream's rule
    // and not one a reader would guess.
    (
        "@g = global [4 x i16] [i16 0, i16 0, i16 0, i16 0]\n",
        "@g = global [4 x i16] zeroinitializer\n",
    ),
    (
        "@g = global [2 x i16] [i16 -1, i16 -1]\n",
        "@g = global [2 x i16] [i16 -1, i16 -1]\n",
    ),
    // A struct is all zero when each field is, which they need not be the
    // same constant to be.
    (
        "@g = global { i16, [2 x i16] } { i16 0, [2 x i16] zeroinitializer }\n",
        "@g = global { i16, [2 x i16] } zeroinitializer\n",
    ),
    // One lane out of step and nothing folds.
    (
        "@g = global <4 x i16> <i16 -1, i16 -1, i16 -1, i16 undef>\n",
        "@g = global <4 x i16> <i16 -1, i16 -1, i16 -1, i16 undef>\n",
    ),
];

#[test]
fn uniform_aggregates_fold_the_way_upstream_folds_them() {
    for (written, expected) in FOLDED {
        let module = llvm_ir_parse::parse_module(written)
            .unwrap_or_else(|error| panic!("{written} did not parse: {error}"));
        let printed = llvm_ir_print::print_module(&module);
        // A module prints with a leading blank line where its header would
        // be, which these fragments have none of.
        assert_eq!(
            printed.trim_start_matches('\n'),
            *expected,
            "\nfrom: {written}"
        );
    }
}
