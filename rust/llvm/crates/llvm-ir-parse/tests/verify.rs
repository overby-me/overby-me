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
        "define i32 @f(i1 %c) {\nentry:\n  br i1 %c, label %a, label %b\na:\n  %x = add i32 1, 2\n  br label %b\nb:\n  ret i32 %x\n}\n",
        "uses a value defined where it cannot reach",
    ),
    (
        "define internal hidden void @f() {\nentry:\n  ret void\n}\n",
        "symbol with local linkage must have default visibility",
    ),
    (
        "@g = private protected global i32 0, align 4\n",
        "symbol with local linkage must have default visibility",
    ),
    (
        "define void @f(ptr %p, i32 %a, i32 %b) {\nentry:\n  %x = cmpxchg ptr %p, i32 %a, i32 %b unordered monotonic, align 4\n  ret void\n}\n",
        "invalid success ordering",
    ),
    (
        "define void @f(ptr %p, i32 %a, i32 %b) {\nentry:\n  %x = cmpxchg ptr %p, i32 %a, i32 %b seq_cst release, align 4\n  ret void\n}\n",
        "invalid failure ordering",
    ),
    (
        "%s = type { i32, i64 }\n\ndefine ptr @f(ptr %p) {\nentry:\n  %x = getelementptr %s, ptr %p, i32 0, i64 1\n  ret ptr %x\n}\n",
        "invalid indices",
    ),
    (
        "%s = type { i32, i64 }\n\ndefine ptr @f(ptr %p) {\nentry:\n  %x = getelementptr %s, ptr %p, i32 0, i32 7\n  ret ptr %x\n}\n",
        "invalid indices",
    ),
    (
        "define void @f() {\nentry:\n  %x = extractvalue [0 x i32] undef, 0\n  ret void\n}\n",
        "invalid indices for extractvalue",
    ),
    (
        "!llvm.module.flags = !{!0}\n\n!0 = !{i32 1, !\"k\"}\n",
        "module flag must be a MDNode triple",
    ),
    (
        "!llvm.module.flags = !{!0}\n\n!0 = !{i32 99, !\"k\", i32 1}\n",
        "invalid behaviour operand",
    ),
    (
        "!llvm.ident = !{!0}\n\n!0 = !{i32 1}\n",
        "must be a node with one string",
    ),
    (
        "define void @f() {\nentry:\n  %p = alloca token, align 8\n  ret void\n}\n",
        "invalid type for alloca",
    ),
    (
        "@g = external global token\n",
        "invalid type for a global variable",
    ),
    (
        "define token @f() {\nentry:\n  ret token none\n}\n",
        "returns a token but is not an intrinsic",
    ),
    (
        "define void @f(x86_amx %a) {\nentry:\n  ret void\n}\n",
        "takes a x86_amx but is not an intrinsic",
    ),
    (
        "declare token @llvm.thing()\n\ndefine void @f() {\nentry:\n  %t = call token @llvm.thing()\n  %s = select i1 true, token %t, token %t\n  ret void\n}\n",
        "select values cannot have token type",
    ),
    (
        "@llvm.used = appending global [1 x i32] [i32 0], section \"llvm.metadata\"\n",
        "wrong type for an intrinsic global",
    ),
    (
        "@llvm.used = appending global [1 x ptr] zeroinitializer, section \"llvm.metadata\"\n",
        "wrong initialiser for an intrinsic global",
    ),
    (
        "@f = global i32 0\n@llvm.global_ctors = appending global [1 x { i32, ptr }] [{ i32, ptr } { i32 65535, ptr null }]\n",
        "third field of the element type is mandatory",
    ),
    (
        "$v = comdat any\n@v = common global i32 0, comdat($v), align 4\n",
        "common global may not be in a comdat",
    ),
    (
        "define ptr @f(ptr %p) {\nentry:\n  %x = bitcast ptr %p to ptr addrspace(1)\n  ret ptr %p\n}\n",
        "casts between the wrong kinds of type",
    ),
    (
        "define i64 @f(ptr %p) {\nentry:\n  %x = bitcast ptr %p to i64\n  ret i64 %x\n}\n",
        "casts between the wrong kinds of type",
    ),
    (
        "@a = global i32 0\n@b = global ptr addrspace(1) bitcast (ptr @a to ptr addrspace(1)), align 8\n",
        "casts between the wrong kinds of type",
    ),
    (
        "define i16 @f(i32 %x) {\nentry:\n  %y = bitcast i32 %x to i16\n  ret i16 %y\n}\n",
        "changes the size of its operand",
    ),
];

/// Input the parser itself has to refuse, with the message it owes.
const REJECTED: &[(&str, &str)] = &[
    (
        "@g = global i32 0\n@g = global i32 1\n",
        "redefinition of global '@g'",
    ),
    (
        "define void @f() {\nentry:\n  %p = alloca i1, align 8589934592\n  ret void\n}\n",
        "huge alignment values are unsupported",
    ),
    (
        "define void @f() {\nentry:\n  %p = alloca i1, align 3\n  ret void\n}\n",
        "not a power of two",
    ),
    (
        "@g = global [4 x token] zeroinitializer\n",
        "invalid array element type",
    ),
    (
        "@g = global <4 x label> zeroinitializer\n",
        "invalid vector element type",
    ),
    ("%s = type { void }\n", "invalid structure element type"),
    (
        "define void @f(i8* %p) {\nentry:\n  ret void\n}\n",
        "opaque",
    ),
];

#[test]
fn the_parser_refuses_what_it_should() {
    for (text, expected) in REJECTED {
        match llvm_ir_parse::parse_module(text) {
            Ok(_) => panic!("this should not have parsed:\n{text}"),
            Err(error) => assert!(
                error.to_string().contains(expected),
                "expected an error containing {expected:?}, got {error}\nfor:\n{text}"
            ),
        }
    }
}

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

#[test]
fn an_instruction_after_a_terminator_opens_a_new_block() {
    // Upstream's parser does this rather than rejecting it, which is why five
    // invokes written in a row are five blocks. Checked against real llvm-as,
    // which prints exactly this.
    let text = "define void @f() {\nentry:\n  ret void\n  ret void\n}\n";
    let module = llvm_ir_parse::parse_module(text).expect("upstream accepts this");
    assert!(llvm_ir::verify_module(&module).is_empty());
    assert_eq!(
        llvm_ir_print::print_module(&module),
        "\ndefine void @f() {\nentry:\n  ret void\n\n0:                                                ; No predecessors!\n  ret void\n}\n"
    );
}

/// Syntax upstream accepts that we used to refuse. Each was found by the
/// upstream suites rather than by reading LangRef.
const ACCEPTED: &[&str] = &[
    // A struct named by number rather than by word.
    "%0 = type { i32, i64 }\n@g = global %0 zeroinitializer, align 8\n",
    // The deprecated sized spelling of the aggregate alignment.
    "target datalayout = \"e-a0:0:32\"\n",
    // `align(4)` as well as `align 4` in a parameter list.
    "define void @f(ptr align(4) %p) {\nentry:\n  ret void\n}\n",
    // A phi with no edges, in a block nothing reaches.
    "define void @f() {\nentry:\n  ret void\n\ndead:\n  %p = phi i32\n  ret void\n}\n",
    // The three arithmetic constant expressions that outlived the others.
    "@a = global i32 add (i32 2, i32 3)\n@b = global i32 sub (i32 9, i32 4)\n@c = global i32 xor (i32 1, i32 3)\n",
    // Inline string attributes on a global, with no comma before them.
    "@g = global i32 7 \"key\" = \"value\"\n",
    // A calling convention only one target has.
    "declare amdgpu_cs_chain void @f()\n",
    // The address space written before the type rather than after it.
    "declare void @g()\n\ndefine void @f() {\nentry:\n  call addrspace(0) void @g()\n  ret void\n}\n",
    // A metadata integer that does not fit 64 bits.
    "!named = !{!0}\n!0 = !DIEnumerator(name: \"x\", value: 170141183460469231731687303715884105727)\n",
];

#[test]
fn what_upstream_accepts_parses() {
    for text in ACCEPTED {
        llvm_ir_parse::parse_module(text)
            .unwrap_or_else(|error| panic!("upstream accepts this: {error}\n{text}"));
    }
}
