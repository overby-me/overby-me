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

/// The older pointer spellings, which upstream folds to `ptr` as it reads.
const POINTERS: &[(&str, &str)] = &[
    ("@g = global i8* null\n", "@g = global ptr null\n"),
    ("@g = global i8** null\n", "@g = global ptr null\n"),
    (
        "@g = global i8 addrspace(3)* null\n",
        "@g = global ptr addrspace(3) null\n",
    ),
    ("@g = global void (i32)* null\n", "@g = global ptr null\n"),
    ("declare i8* @f()\n", "declare ptr @f()\n"),
    ("declare void @f(i32*)\n", "declare void @f(ptr)\n"),
];

#[test]
fn typed_pointers_fold_to_opaque_ones() {
    for (written, expected) in POINTERS {
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

/// Which identified structs get written back. Upstream prints the ones its
/// type finder reaches from the module and drops the rest, and the order is
/// the order the walk meets them rather than the order they were written.
const TYPES: &[(&str, &str)] = &[
    ("%Ty = type opaque\n", ""),
    ("%Ty = type { i32 }\n", ""),
    (
        "%Ty = type { i32 }\n@g = global %Ty zeroinitializer\n",
        "%Ty = type { i32 }\n\n@g = global %Ty zeroinitializer\n",
    ),
    // `%B` is what the global led to, so `%B` is written first.
    (
        "%A = type { i32 }\n%B = type { %A }\n@g = global %B zeroinitializer\n",
        "%B = type { %A }\n%A = type { i32 }\n\n@g = global %B zeroinitializer\n",
    ),
    // A chain nothing reaches goes whole.
    ("%A = type { i32 }\n%B = type { %A }\n", ""),
    // An alloca names a type its result does not, and so does an attribute.
    (
        "%Ty = type { i32 }\n\ndefine void @f() {\nentry:\n  %a = alloca %Ty, align 8\n  ret void\n}\n",
        "%Ty = type { i32 }\n\ndefine void @f() {\nentry:\n  %a = alloca %Ty, align 8\n  ret void\n}\n",
    ),
    (
        "%Ty = type { i32 }\n\ndeclare void @f(ptr sret(%Ty))\n",
        "%Ty = type { i32 }\n\ndeclare void @f(ptr sret(%Ty))\n",
    ),
];

#[test]
fn only_the_types_the_module_reaches_are_written() {
    for (written, expected) in TYPES {
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

/// What else upstream folds or leaves out as it reads and prints.
/// The order upstream writes an attribute set in, which is not the order the
/// text used and not alphabetical either.
/// The alignment an alloca gets when it writes none. A struct takes the
/// larger of what its fields need and what the layout prefers for an
/// aggregate, which with the default `a:0:64` is eight even for `{ i8 }`.
/// `nocapture` is the older spelling of `captures(none)`, and a parameter's
/// attributes are written in upstream's order the same way a function's are.
/// A pointer operand is written with the address space it points through
/// rather than a bare `ptr`.
/// A metadata field written with the value it would have had anyway is not
/// written back, which also uniques two nodes that differ only in one.
const METADATA_DEFAULTS: &[(&str, &str)] = &[
    (
        "!DIBasicType(tag: DW_TAG_base_type, name: \"n\", size: 8, align: 0)",
        "!DIBasicType(name: \"n\", size: 8)",
    ),
    (
        "!DIBasicType(tag: DW_TAG_unspecified_type, name: \"n\")",
        "!DIBasicType(tag: DW_TAG_unspecified_type, name: \"n\")",
    ),
    (
        "!DIDerivedType(tag: DW_TAG_member, baseType: null, size: 0, offset: 0)",
        "!DIDerivedType(tag: DW_TAG_member, baseType: null)",
    ),
    (
        "!DIEnumerator(name: \"e\", value: 1, isUnsigned: false)",
        "!DIEnumerator(name: \"e\", value: 1)",
    ),
    // A metadata operand keeps a written zero: it is a constant there, not
    // an absence.
    (
        "!DISubrange(count: 3, lowerBound: 0)",
        "!DISubrange(count: 3, lowerBound: 0)",
    ),
    (
        "!DIEnumerator(name: \"e\", value: 0)",
        "!DIEnumerator(name: \"e\", value: 0)",
    ),
    // A required field stays whatever it holds.
    (
        "!DIFile(filename: \"f\", directory: \"\")",
        "!DIFile(filename: \"f\", directory: \"\")",
    ),
];

#[test]
fn a_metadata_field_at_its_default_is_not_written() {
    for (written, expected) in METADATA_DEFAULTS {
        let text = format!("!named = !{{!0}}\n!0 = {written}\n");
        let module = llvm_ir_parse::parse_module(&text)
            .unwrap_or_else(|error| panic!("{text} did not parse: {error}"));
        let printed = llvm_ir_print::print_module(&module);
        let line = printed
            .split("!0 = ")
            .nth(1)
            .unwrap_or_else(|| panic!("no node\n{printed}"));
        let line = line.split('\n').next().unwrap_or_default();
        assert_eq!(line, *expected, "\nfrom: {written}");
    }
}

/// Two nodes that differ only in a defaulted field are one node.
#[test]
fn dropping_a_default_uniques_two_nodes() {
    let text = "!named = !{!0, !1}\n!0 = !DIBasicType(name: \"n\")\n!1 = !DIBasicType(name: \"n\", align: 0)\n";
    let module =
        llvm_ir_parse::parse_module(text).unwrap_or_else(|error| panic!("did not parse: {error}"));
    let printed = llvm_ir_print::print_module(&module);
    assert!(
        printed.contains("!named = !{!0, !0}"),
        "the two should have uniqued\n--- printed ---\n{printed}"
    );
}

const POINTER_OPERANDS: &[&str] = &[
    "  %v = load i32, ptr addrspace(42) @in, align 4",
    "  store i32 1, ptr addrspace(42) @in, align 4",
    "  %c = cmpxchg ptr addrspace(42) @in, i32 0, i32 1 monotonic monotonic, align 4",
    // Address space zero is the same either way, which is why this went
    // unnoticed until a module used another one.
    "  %w = load i32, ptr @zero, align 4",
];

#[test]
fn a_pointer_operand_says_which_space_it_points_through() {
    let text = "@in = external addrspace(42) global i32\n@zero = external global i32\n\ndefine void @f() {\nentry:\n  %v = load i32, ptr addrspace(42) @in, align 4\n  store i32 1, ptr addrspace(42) @in, align 4\n  %c = cmpxchg ptr addrspace(42) @in, i32 0, i32 1 monotonic monotonic, align 4\n  %w = load i32, ptr @zero, align 4\n  ret void\n}\n";
    let module =
        llvm_ir_parse::parse_module(text).unwrap_or_else(|error| panic!("did not parse: {error}"));
    let printed = llvm_ir_print::print_module(&module);
    for expected in POINTER_OPERANDS {
        assert!(
            printed.lines().any(|line| line == *expected),
            "expected {expected:?}\n--- printed ---\n{printed}"
        );
    }
}

const PARAMETERS: &[(&str, &str)] = &[
    ("ptr nocapture", "ptr captures(none)"),
    ("ptr nocapture readonly", "ptr readonly captures(none)"),
    ("ptr readonly nocapture", "ptr readonly captures(none)"),
    (
        "ptr noalias nocapture nonnull",
        "ptr noalias nonnull captures(none)",
    ),
    ("ptr align 8 noalias", "ptr noalias align 8"),
    // The other three access keywords keep their spelling on a parameter,
    // where on a function they become `memory(...)`.
    ("ptr readonly", "ptr readonly"),
    ("ptr writeonly", "ptr writeonly"),
];

#[test]
fn a_parameters_attributes_are_written_in_upstreams_order() {
    for (written, expected) in PARAMETERS {
        let text = format!("declare void @f({written})\n");
        let module = llvm_ir_parse::parse_module(&text)
            .unwrap_or_else(|error| panic!("{text} did not parse: {error}"));
        let printed = llvm_ir_print::print_module(&module);
        assert!(
            printed.contains(&format!("declare void @f({expected})")),
            "expected {expected:?}\n--- printed ---\n{printed}"
        );
    }
}

const ALLOCA_ALIGN: &[(&str, &str, &str)] = &[
    ("e", "{ i32, i32 }", "align 8"),
    ("e", "{ i8 }", "align 8"),
    ("e-a:0:32", "{ i32, i32 }", "align 4"),
    ("e-a:16:128", "{ i8 }", "align 16"),
    // An array takes its element's, and a scalar its own.
    ("e", "[4 x i8]", "align 1"),
    ("e", "[2 x i64]", "align 8"),
    ("e", "i32", "align 4"),
];

#[test]
fn an_alloca_takes_the_alignment_upstream_gives_it() {
    for (layout, ty, expected) in ALLOCA_ALIGN {
        let text = format!(
            "target datalayout = \"{layout}\"\n\ndefine void @f() {{\nentry:\n  %a = alloca {ty}\n  ret void\n}}\n"
        );
        let module = llvm_ir_parse::parse_module(&text)
            .unwrap_or_else(|error| panic!("{text} did not parse: {error}"));
        let printed = llvm_ir_print::print_module(&module);
        let line = printed
            .split("%a = alloca ")
            .nth(1)
            .unwrap_or_else(|| panic!("no alloca\n{printed}"));
        let line = line.split('\n').next().unwrap_or_default();
        assert!(
            line.ends_with(expected),
            "expected {expected:?} at the end of {line:?}\nlayout {layout}, type {ty}"
        );
    }
}

const ATTRIBUTE_ORDER: &[(&str, &str)] = &[
    // The plain keywords go in the order LLVM declares them, which is why
    // `nounwind` comes before `nonlazybind` and `optsize` before `ssp`.
    ("uwtable optsize ssp", "{ optsize ssp uwtable }"),
    ("nonlazybind nounwind cold", "{ cold nounwind nonlazybind }"),
    // Then the ones that take an argument, in upstream's own order, with
    // `uwtable` among them whether or not it carries a kind.
    (
        "nounwind memory(none) alwaysinline",
        "{ alwaysinline nounwind memory(none) }",
    ),
    (
        "nonlazybind memory(argmem: read) uwtable nofree",
        "{ nofree nonlazybind memory(argmem: read) uwtable }",
    ),
    // Then the quoted ones, by key.
    (
        "\"zed\"=\"1\" \"abc\"=\"2\" nounwind",
        "{ nounwind \"abc\"=\"2\" \"zed\"=\"1\" }",
    ),
];

#[test]
fn an_attribute_set_is_written_in_upstreams_order() {
    for (written, expected) in ATTRIBUTE_ORDER {
        let text = format!("define void @f(i32 %n) {written} {{\nentry:\n  ret void\n}}\n");
        let module = llvm_ir_parse::parse_module(&text)
            .unwrap_or_else(|error| panic!("{text} did not parse: {error}"));
        let printed = llvm_ir_print::print_module(&module);
        let group = printed
            .split("attributes #0 = ")
            .nth(1)
            .unwrap_or_else(|| panic!("no attribute group\n{printed}"));
        let group = group.split('\n').next().unwrap_or_default();
        assert_eq!(group, *expected, "\nfrom: {written}");
    }
}

const PRINTED: &[(&str, &str)] = &[
    // A struct with no fields is all zero; an array with no elements is
    // poison. Neither is a guess a reader would make.
    (
        "define void @f(ptr %x) {\nentry:\n  store {} {}, ptr %x, align 4\n  ret void\n}\n",
        "  store {} zeroinitializer, ptr %x, align 4",
    ),
    (
        "define void @f(ptr %x) {\nentry:\n  store [0 x i32] [], ptr %x, align 4\n  ret void\n}\n",
        "  store [0 x i32] poison, ptr %x, align 4",
    ),
    // The default address space is not written.
    (
        "define void @f() {\nentry:\n  %a = alloca i32, align 4, addrspace(0)\n  ret void\n}\n",
        "  %a = alloca i32, align 4",
    ),
    (
        "define void @f() {\nentry:\n  %a = alloca i32, align 4, addrspace(3)\n  ret void\n}\n",
        "  %a = alloca i32, align 4, addrspace(3)",
    ),
    // A count of one goes only when it is written in the width a count
    // defaults to: dropping `i64 1` would change the width back to i32.
    (
        "define void @f() {\nentry:\n  %a = alloca i1, i32 1, align 8\n  ret void\n}\n",
        "  %a = alloca i1, align 8",
    ),
    (
        "define void @f() {\nentry:\n  %a = alloca i1, i64 1, align 8\n  ret void\n}\n",
        "  %a = alloca i1, i64 1, align 8",
    ),
    // A getelementptr that moves nowhere is the pointer it started from.
    (
        "@a = global [4 x i32] zeroinitializer\n@g = constant ptr getelementptr inbounds ([4 x i32], ptr @a, i64 0, i64 0)\n",
        "@g = constant ptr @a",
    ),
    (
        "@a = global [4 x i32] zeroinitializer\n@g = constant ptr getelementptr ([4 x i32], ptr @a, i64 0, i64 2)\n",
        "@g = constant ptr getelementptr ([4 x i32], ptr @a, i64 0, i64 2)",
    ),
];

#[test]
fn what_upstream_folds_as_it_reads_is_folded() {
    for (written, expected) in PRINTED {
        let module = llvm_ir_parse::parse_module(written)
            .unwrap_or_else(|error| panic!("{written} did not parse: {error}"));
        let printed = llvm_ir_print::print_module(&module);
        assert!(
            printed.lines().any(|line| line == *expected),
            "expected a line {expected:?}\n--- printed ---\n{printed}"
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
