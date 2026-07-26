//! The corpus has to verify clean.
//!
//! Real `llvm-as` verified every one of these files when the corpus was
//! generated, so anything the verifier reports here is a rule we have wrong
//! rather than a module that is broken. That makes the corpus a check on the
//! verifier as much as the verifier is a check on the corpus.

use std::path::{Path, PathBuf};

fn corpus_files() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    let mut files = Vec::new();
    for directory in ["rustc", "handwritten"] {
        let path = root.join(directory);
        if !path.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(&path).expect("corpus directory is readable") {
            let path = entry.expect("directory entry is readable").path();
            if path.extension().is_some_and(|extension| extension == "ll") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

#[test]
fn the_corpus_verifies() {
    let files = corpus_files();
    assert!(!files.is_empty(), "the corpus is empty");

    let mut failures = Vec::new();
    for path in &files {
        let text = std::fs::read_to_string(path).expect("corpus file is readable");
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let module =
            llvm_ir_parse::parse_module(&text).unwrap_or_else(|error| panic!("{name}: {error}"));
        let errors = llvm_ir::verify_module(&module);
        for error in errors.iter().take(5) {
            failures.push(format!("{name}: {error}"));
        }
        if errors.len() > 5 {
            failures.push(format!("{name}: and {} more", errors.len() - 5));
        }
    }

    assert!(
        failures.is_empty(),
        "the verifier rejects {} things in a corpus real llvm-as accepted:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Modules the parser accepts and the verifier must not.
///
/// Each case pairs textual IR with a fragment of the message it has to
/// produce, so a rule that stops firing fails loudly rather than quietly
/// widening what we accept.
const BROKEN: &[(&str, &str)] = &[
    (
        "define void @f() {\nentry:\n  %x = add i32 1, 2\n}\n",
        "does not end in a terminator",
    ),
    (
        "define void @f() {\nentry:\n  ret void\n  ret void\n}\n",
        "terminator (ret) before its end",
    ),
    (
        "define i32 @f(i32 %a) {\nentry:\n  br label %next\nnext:\n  %x = add i32 %a, 1\n  %p = phi i32 [ 0, %entry ]\n  ret i32 %x\n}\n",
        "phi after a non-phi",
    ),
    (
        "define i32 @f(i1 %c) {\nentry:\n  br i1 %c, label %a, label %b\na:\n  br label %join\nb:\n  br label %join\njoin:\n  %p = phi i32 [ 0, %a ]\n  ret i32 %p\n}\n",
        "does not name exactly its block's predecessors",
    ),
    (
        "define void @f() {\nentry:\n  ret i32 0\n}\n",
        "returns the wrong type",
    ),
    (
        "define i32 @f() {\nentry:\n  ret void\n}\n",
        "returns nothing from a function that returns a value",
    ),
    (
        "define i32 @f(i32 %a) {\nentry:\n  %x = or nuw i32 %a, 1\n  ret i32 %x\n}\n",
        "carries a flag it does not define",
    ),
    (
        "define i32 @f(i64 %a) {\nentry:\n  %x = zext i64 %a to i32\n  ret i32 %x\n}\n",
        "does not widen",
    ),
    (
        "define i32 @f(double %a) {\nentry:\n  %x = trunc double %a to i32\n  ret i32 %x\n}\n",
        "casts between the wrong kinds of type",
    ),
    (
        "define double @f(double %a) {\nentry:\n  %x = add double %a, %a\n  ret double %x\n}\n",
        "needs an integer type",
    ),
    (
        "define i32 @f(i32 %a) {\nentry:\n  %x = fadd i32 %a, %a\n  ret i32 %x\n}\n",
        "needs a floating-point type",
    ),
    (
        "define void @f(i32 %c) {\nentry:\n  br i1 %c, label %a, label %b\na:\n  ret void\nb:\n  ret void\n}\n",
        "operand of the wrong type",
    ),
    (
        "define void @f(i32 %x) {\nentry:\n  switch i32 %x, label %d [\n    i32 1, label %a\n    i32 1, label %b\n  ]\na:\n  ret void\nb:\n  ret void\nd:\n  ret void\n}\n",
        "duplicated case",
    ),
    (
        "define void @f() {\nentry:\n  ret void, !dbg !9\n}\n",
        "undefined metadata !9",
    ),
    (
        "define void @f() #7 {\nentry:\n  ret void\n}\n",
        "undefined attribute group #7",
    ),
    (
        "declare void @g(i32)\n\ndefine void @f() {\nentry:\n  call void @g()\n  ret void\n}\n",
        "does not match the signature of the function it calls",
    ),
    (
        "define i32 @f(i1 %c) {\nentry:\n  br i1 %c, label %a, label %b\na:\n  %x = add i32 1, 2\n  br label %b\nb:\n  ret i32 %x\n}\n",
        "uses a value defined where it cannot reach",
    ),
    (
        "@g = global i32 0\n\ndefine i32 @f() {\nentry:\n  %x = load i32, ptr @g\n  ret i32 %x\n}\n",
        "has no alignment",
    ),
];

#[test]
fn broken_modules_are_rejected_with_the_expected_message() {
    for (text, expected) in BROKEN {
        let module = llvm_ir_parse::parse_module(text)
            .unwrap_or_else(|error| panic!("this case should parse: {error}\n{text}"));
        let errors = llvm_ir::verify_module(&module);
        let messages: Vec<String> = errors.iter().map(ToString::to_string).collect();
        assert!(
            messages.iter().any(|message| message.contains(expected)),
            "expected an error containing {expected:?}, got {messages:?}\nfor:\n{text}"
        );
    }
}

#[test]
fn a_well_formed_module_produces_no_errors() {
    let text = "\
define i32 @sum(i32 %n) {
entry:
  br label %header

header:                                           ; preds = %body, %entry
  %i = phi i32 [ 0, %entry ], [ %next, %body ]
  %total = phi i32 [ 0, %entry ], [ %sum, %body ]
  %done = icmp sge i32 %i, %n
  br i1 %done, label %exit, label %body

body:                                             ; preds = %header
  %sum = add nuw nsw i32 %total, %i
  %next = add nuw nsw i32 %i, 1
  br label %header

exit:                                             ; preds = %header
  ret i32 %total
}
";
    let module = llvm_ir_parse::parse_module(text).expect("this module is well formed");
    let errors = llvm_ir::verify_module(&module);
    assert!(errors.is_empty(), "{errors:?}");
}
