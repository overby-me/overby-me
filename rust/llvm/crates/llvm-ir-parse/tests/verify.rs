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
    // Attributes that describe something only a pointer has.
    (
        "declare void @f(i32 byval(i32) %n)\n",
        "byval on parameter 0, which is not a pointer",
    ),
    (
        "define void @f(i32 writable %p) {\nentry:\n  ret void\n}\n",
        "writable on parameter 0, which is not a pointer",
    ),
    (
        "define void @f(i32 nofpclass(nan) %x) {\nentry:\n  ret void\n}\n",
        "nofpclass on parameter 0, which is not a floating-point type",
    ),
    (
        "define void @f(i32 range(i8 1, 0) %x) {\nentry:\n  ret void\n}\n",
        "range of i8 on parameter 0, which is i32",
    ),
    (
        "declare void @a(ptr sret(i32) %a, ptr sret(i32) %b)\n",
        "2 parameters are sret, which allows one",
    ),
    (
        "declare swifterror void @c(ptr swifterror %a)\n",
        "swifterror on the return value",
    ),
    // A function attribute whose own value is wrong.
    (
        "define void @f() \"frame-pointer\"=\"arst\" {\nentry:\n  ret void\n}\n",
        "invalid value for 'frame-pointer': arst",
    ),
    (
        "define void @f() \"patchable-function-entry\"=\"-1\" {\nentry:\n  ret void\n}\n",
        "'patchable-function-entry' takes an unsigned integer: -1",
    ),
    (
        "define void @f() \"denormal-fp-math\"=\"ieee,ieee,ieee\" {\nentry:\n  ret void\n}\n",
        "invalid value for 'denormal-fp-math': ieee,ieee,ieee",
    ),
    (
        "declare ptr @c(ptr) vscale_range(3, 16)\n",
        "the vscale_range minimum must be a power of two",
    ),
    (
        "define i32 @f() jumptable {\nentry:\n  ret i32 0\n}\n",
        "jumptable requires unnamed_addr",
    ),
    // The DWARF address space says where a pointer points, so a typedef has
    // nowhere to put it.
    (
        "!named = !{!1}\n!0 = !DIBasicType(name: \"n\")\n!1 = !DIDerivedType(tag: DW_TAG_typedef, baseType: !0, dwarfAddressSpace: 1)\n",
        "DWARF address space only applies to pointer or reference types",
    ),
    // A comdat needs a definition to pick and a name the linker can see.
    (
        "$v = comdat any\n@v = available_externally global i32 0, comdat\n",
        "@v is a declaration and may not be in a comdat",
    ),
    (
        "define ptr @resolver() {\nentry:\n  ret ptr null\n}\n\n@f = ifunc void (), ptr getelementptr (i8, ptr @resolver, i32 4)\n",
        "@f must have a function as its resolver",
    ),
    (
        "@a = external global i32\n@g = external global i32, !associated !0\n!0 = !{i32 1}\n",
        "!associated takes one pointer-typed value",
    ),
    (
        "@g = external global i32, !absolute_symbol !0\n!0 = !{}\n",
        "!absolute_symbol takes ranges of two values",
    ),
    // An intrinsic is the compiler's, not the module's.
    (
        "define void @llvm.memcpy.p0.p0.i32(ptr %a, ptr %b, i32 %n, i1 %v) {\nentry:\n  ret void\n}\n",
        "an intrinsic is provided by the compiler and cannot be defined",
    ),
    (
        "declare i32 @llvm.umax.i32(i32, i32)\n@g = global ptr @llvm.umax.i32\n",
        "@g takes the address of @llvm.umax.i32",
    ),
    (
        "declare void @llvm.made.up.name.i32(i32)\n\ndefine void @f() {\nentry:\n  call void @llvm.made.up.name.i32(i32 1, i32 2)\n  ret void\n}\n",
        "calls an intrinsic with an incompatible signature",
    ),
    (
        "declare void @llvm.t.immarg.i32(i32 immarg)\n\ndefine void @f(i32 %x) {\nentry:\n  call void @llvm.t.immarg.i32(i32 %x)\n  ret void\n}\n",
        "passes a non-immediate to an immarg parameter",
    ),
    (
        "declare void @llvm.t.immarg.i32(i32 immarg)\n\ndefine void @f() {\nentry:\n  call void @llvm.t.immarg.i32(i32 undef)\n  ret void\n}\n",
        "passes a non-immediate to an immarg parameter",
    ),
    (
        "@f = external global i32\n@fa = alias i32, ptr @f\n",
        "@fa aliases something this module does not define",
    ),
    (
        "define amdgpu_kernel i32 @f() {\nentry:\n  ret i32 0\n}\n",
        "the amdgpu_kernel convention returns nothing",
    ),
    (
        "define amdgpu_ps void @f(...) {\nentry:\n  ret void\n}\n",
        "the amdgpu_ps convention does not take a variable argument list",
    ),
    (
        "declare amdgpu_cs_chain void @g()\n\ndefine void @f() {\nentry:\n  call amdgpu_cs_chain void @g()\n  ret void\n}\n",
        "uses a convention that does not permit calls",
    ),
    (
        "define <2 x ptr> @f(<4 x ptr> %a) {\nentry:\n  %w = getelementptr i32, <4 x ptr> %a, <2 x i32> <i32 1, i32 2>\n  ret <2 x ptr> %w\n}\n",
        "has vector indices of different widths",
    ),
    (
        "define void @f(<vscale x 2 x ptr> %a) {\nentry:\n  %w = getelementptr {i32, i32}, <vscale x 2 x ptr> %a, <vscale x 2 x i32> zeroinitializer, <vscale x 2 x i32> zeroinitializer\n  ret void\n}\n",
        "has invalid indices",
    ),
    (
        "declare void @llvm.localescape(...)\n\ndefine internal void @f() {\nentry:\n  %a = alloca i8, align 1\n  call void (...) @llvm.localescape(ptr %a)\n  call void (...) @llvm.localescape(ptr %a)\n  ret void\n}\n",
        "2 calls to llvm.localescape in one function",
    ),
    // A subrange is described from one end or the other, never both.
    (
        "!named = !{!0}\n!0 = !DISubrange(count: 20, lowerBound: 1, upperBound: 10)\n",
        "!DISubrange has both a count and an upperBound",
    ),
    (
        "!named = !{!0, !1}\n!0 = !DISubrange(lowerBound: !1, upperBound: 1)\n!1 = !DIBasicType(name: \"n\", size: 64)\n",
        "!DISubrange has a lowerBound that is neither a constant, a variable nor an expression",
    ),
    (
        "!named = !{!0}\n!0 = !DIGenericSubrange(lowerBound: !DIExpression(DW_OP_deref))\n",
        "!DIGenericSubrange has no stride",
    ),
    // Four fields describe an array's shape and one picks a variant's arm.
    (
        "!named = !{!0}\n!0 = !DICompositeType(tag: DW_TAG_structure_type, name: \"A\", size: 64, rank: !DIExpression(DW_OP_deref))\n",
        "rank appears on a type that is not an array",
    ),
    (
        "!named = !{!0, !1}\n!0 = !DICompositeType(tag: DW_TAG_structure_type, name: \"A\", size: 64, discriminator: !1)\n!1 = !DIBasicType(name: \"u64\", size: 64)\n",
        "a discriminator appears on a type that is not a variant part",
    ),
    (
        "!named = !{!0, !1}\n!0 = !DIBasicType(name: \"int\", size: 32)\n!1 = !DICompositeType(tag: DW_TAG_structure_type, name: \"T\", size: 32, templateParams: !0)\n",
        "the template parameters of a composite type are not a tuple",
    ),
    (
        "!named = !{!0, !1, !2}\n!0 = !DIBasicType(name: \"int\", size: 32)\n!1 = !{!0}\n!2 = !DICompositeType(tag: DW_TAG_structure_type, name: \"T\", size: 32, templateParams: !1)\n",
        "a template parameter is not a template parameter node",
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
    // The grammar of specialized metadata nodes, which upstream enforces in
    // its parser rather than its verifier.
    (
        "!named = !{!0}\n!0 = !Invalid(field: 0)\n",
        "expected metadata type",
    ),
    (
        "!named = !{!0}\n!0 = !DILocation(bad: 0)\n",
        "invalid field 'bad'",
    ),
    (
        "!named = !{!0, !1}\n!0 = !{}\n!1 = !DILocation(line: 3, scope: !0, line: 3)\n",
        "field 'line' cannot be specified more than once",
    ),
    (
        "!named = !{!0}\n!0 = !DILocation()\n",
        "missing required field 'scope'",
    ),
    (
        "!named = !{!0}\n!0 = !DILocation(scope: null)\n",
        "'scope' cannot be null",
    ),
    (
        "!named = !{!0}\n!0 = !DIGlobalVariable(name: \"\")\n",
        "'name' cannot be empty",
    ),
    (
        "!named = !{!0, !1}\n!0 = !{}\n!1 = !DILocation(column: 65536, scope: !0)\n",
        "value for 'column' too large, limit is 65535",
    ),
    (
        "!named = !{!0, !1}\n!0 = !{}\n!1 = !DILocalVariable(scope: !0, arg: -1)\n",
        "expected unsigned integer",
    ),
    (
        "!named = !{!0}\n!0 = !DISubrange(count: -2)\n",
        "value for 'count' too small, limit is -1",
    ),
    (
        "!named = !{!0}\n!0 = !GenericDINode(tag: \"string\")\n",
        "expected DWARF tag",
    ),
    (
        "!named = !{!0}\n!0 = !DIExpression(18446744073709551616)\n",
        "element too large, limit is 18446744073709551615",
    ),
    (
        "!named = !{!0}\n!0 = !DICompileUnit(language: DW_LANG_C99, file: !DIFile(filename: \"f\", directory: \"d\"))\n",
        "missing 'distinct', required for !DICompileUnit",
    ),
    (
        "!named = !{!0}\n!0 = !DISubprogram(isDefinition: true)\n",
        "missing 'distinct', required for !DISubprogram that is a Definition",
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
    // The debug-info limits are inclusive, and the values just inside them
    // matter as much as the ones just outside.
    "!named = !{!0, !1}\n!0 = !{}\n!1 = !DILocation(line: 4294967295, column: 65535, scope: !0)\n",
    "!named = !{!0}\n!0 = !DISubrange(count: -1, lowerBound: -9223372036854775808)\n",
    "!named = !{!0}\n!0 = !GenericDINode(tag: 65535)\n",
    // `nofpclass` reaches through arrays as well as vectors.
    "define void @f([8 x [4 x float]] nofpclass(nan) %x) {\nentry:\n  ret void\n}\n",
];

/// Modules that verify clean, which is the half of the verifier a table of
/// broken input cannot check. Each was a false positive first.
const VERIFIES: &[&str] = &[
    // A resolver reached through a cast is still a function.
    "define ptr @resolver() {\nentry:\n  ret ptr null\n}\n\n@f = ifunc void (), ptr addrspacecast (ptr @resolver to ptr)\n",
    // `!absolute_symbol` may carry more than one range.
    "@g = external global i32, !absolute_symbol !0\n!0 = !{i64 1, i64 2, i64 4, i64 8}\n",
    // Opaque pointers put the signature at the call site, so a call that does
    // not match the callee's declaration is not an error.
    "declare void @g(i32)\n\ndefine void @f() {\nentry:\n  call void @g()\n  ret void\n}\n",
    // An immarg argument may be a constant expression, because upstream folds
    // one before the verifier sees it.
    "declare void @llvm.t.immarg.i32(i32 immarg)\n\ndefine void @f() {\nentry:\n  call void @llvm.t.immarg.i32(i32 add (i32 2, i32 3))\n  ret void\n}\n",
    // A composite type's elements may contain a null entry: upstream's own
    // set1.ll does exactly that and llvm-as reads it.
    "!named = !{!0, !1}\n!0 = !{null}\n!1 = !DICompositeType(tag: DW_TAG_class_type, name: \"C\", size: 64, elements: !0)\n",
    // Private linkage in a comdat is a COFF rule, not an IR one: upstream
    // reports it only for a Windows triple and llvm-as reads this.
    "$v = comdat any\n@v = private global i32 0, comdat($v)\n",
    // Dominance says nothing about a block the entry cannot reach.
    "define void @f() {\nentry:\n  ret void\n\ndead:\n  %x = add i32 %x, 1\n  br label %dead\n}\n",
    // A struct may hold scalable vectors, and a vector may hold a target
    // extension type.
    "%t = type { <vscale x 1 x i32>, <vscale x 1 x i32> }\n@g = external global %t\n",
    // An array's shape fields belong on an array.
    "!named = !{!0}\n!0 = !DICompositeType(tag: DW_TAG_array_type, name: \"A\", size: 64, rank: !DIExpression(DW_OP_deref))\n",
    // An alias writes an expression aliasee with no type in front, because
    // the expression says what it produces.
    "@a = global i32 0\n@b = alias i32, getelementptr inbounds (i32, ptr @a, i64 1)\n",
    "@i = global i32 0\n@ia = alias ptr, addrspacecast (ptr @i to ptr addrspace(3))\n",
    // Wrapping flags on a constant expression.
    "@addr = external global i64\n@a = global i64 add nuw nsw (i64 ptrtoint (ptr @addr to i64), i64 91)\n",
    // A struct indexed lanewise, and the sanitizer clauses a global carries.
    "define <2 x ptr> @f(<2 x ptr> %a) {\nentry:\n  %w = getelementptr {i32, i32}, <2 x ptr> %a, <2 x i32> <i32 5, i32 9>, <2 x i32> zeroinitializer\n  ret <2 x ptr> %w\n}\n",
    "@g = global i32 2, no_sanitize_address, no_sanitize_hwaddress, sanitize_memtag, align 4\n",
    // A metadata name escapes what the bare grammar has no room for, and an
    // escape that is not one keeps its backslash.
    "!\\23pragma = !{!0}\n!0 = !{}\n",
    "!\\5Cxfoo = !{!0}\n!0 = !{}\n",
    // An attachment follows the same comma an index list uses.
    "define i32 @f({{i32, i32}, i32} %a) {\nentry:\n  %x = extractvalue {{i32, i32}, i32} %a, 0, 1, !foo !0\n  ret i32 %x\n}\n\n!0 = !{}\n",
    // A block label whose name only looks like a number.
    "define void @f() {\nentry:\n  br label %\"2\"\n\n\"2\":\n  br label %-3\n\n-3:\n  ret void\n}\n",
    // Two conventions we had never heard of.
    "define amdgpu_ps float @f(i32 %x) {\nentry:\n  ret float 0.000000e+00\n}\n",
    "define riscv_vls_cc(32) void @g() {\nentry:\n  ret void\n}\n",
];

#[test]
fn what_upstream_verifies_verifies() {
    for text in VERIFIES {
        let module = llvm_ir_parse::parse_module(text)
            .unwrap_or_else(|error| panic!("upstream accepts this: {error}\n{text}"));
        let errors = llvm_ir::verify_module(&module);
        assert!(errors.is_empty(), "{errors:#?}\nfor:\n{text}");
    }
}

#[test]
fn what_upstream_accepts_parses() {
    for text in ACCEPTED {
        llvm_ir_parse::parse_module(text)
            .unwrap_or_else(|error| panic!("upstream accepts this: {error}\n{text}"));
    }
}

/// The ThinLTO summary index, which is a grammar of its own.
///
/// It cannot be pinned by the corpus the way everything else is, because
/// `llvm-dis` regenerates the index from the bitcode rather than preserving
/// what was written: the module path and hash come from the file it read and
/// a `; guid = N` comment is appended. So the property to hold is ours, that
/// what we print back is what was written.
const SUMMARY: &str = r#"; ModuleID = 'summary.ll'
source_filename = "summary.ll"

define void @f() {
entry:
  ret void
}

^0 = module: (path: "summary.o", hash: (1, 2, 3, 4, 5))
^1 = gv: (name: "f", summaries: (function: (module: ^0, flags: (linkage: external, visibility: default, notEligibleToImport: 0, live: 0, dsoLocal: 1, canAutoHide: 0, importType: definition), insts: 1, funcFlags: (readNone: 0, noRecurse: 1), calls: ((callee: ^1)), allocs: ((versions: (none), memProf: ((type: notcold, stackIds: ())))))))
^2 = typeidCompatibleVTable: (name: "_ZTSN3FooE", summary: ((offset: 16, ^1)))
^3 = flags: 8
^4 = blockcount: 1
"#;

/// The summary index writes a space before a colon and lets a word qualify
/// the value after it, both of which upstream's own thinlto-summary.ll does.
const SUMMARY_SPACING: &str = r#"^0 = module: (path: "a.o", hash: (0, 0, 0, 0, 0))
^1 = gv: (guid: 1, summaries: (function: (module: ^0, noUnwind : 1, refs: (writeonly ^1, readonly ^1, ^1))))
"#;

#[test]
fn a_summary_index_tolerates_upstream_spacing() {
    let module = llvm_ir_parse::parse_module(SUMMARY_SPACING).expect("upstream accepts this");
    let printed = llvm_ir_print::print_module(&module);
    assert!(
        printed.contains("refs: (writeonly ^1, readonly ^1, ^1)"),
        "{printed}"
    );
}

#[test]
fn a_summary_index_round_trips() {
    let module = llvm_ir_parse::parse_module(SUMMARY).expect("upstream accepts this");
    assert!(llvm_ir::verify_module(&module).is_empty());
    assert_eq!(llvm_ir_print::print_module(&module), SUMMARY);
}

#[test]
fn a_summary_index_names_symbols_this_module_has() {
    let text = "^0 = module: (path: \"a.o\", hash: (0, 0, 0, 0, 0))\n^1 = gv: (name: \"absent\")\n";
    let module = llvm_ir_parse::parse_module(text).expect("this case should parse");
    let errors = llvm_ir::verify_module(&module);
    let messages: Vec<String> = errors.iter().map(ToString::to_string).collect();
    assert!(
        messages
            .iter()
            .any(|message| message.contains("the summary index names @absent")),
        "{messages:?}"
    );
}
