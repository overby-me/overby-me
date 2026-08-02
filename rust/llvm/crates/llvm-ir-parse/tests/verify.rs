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
    // The declared half of the tied-position rule. A declaration nothing
    // calls is never looked at, upstream included, so the mismatch is
    // reported at the call.
    (
        "declare i8 @llvm.umax.i8(i8, i16)\n\ndefine void @t() {\nentry:\n  %r = call i8 @llvm.umax.i8(i8 0, i16 1)\n  ret void\n}\n",
        "calls an intrinsic with two types where it takes one",
    ),
    (
        "define i8 @f(ptr addrspace(42) %p) {\nentry:\n  %r = call i8 %p(i32 0)\n  ret i8 %r\n}\n",
        "calls through address space 42 rather than 0",
    ),
    (
        "@a = alias void (), ptr addrspace(1) @f\n\ndefine void @f() {\nentry:\n  ret void\n}\n",
        "names a symbol in address space 0 through a pointer to address space 1",
    ),
    (
        "%t = type opaque\n\ndefine void @f(%t %a) {\nentry:\n  ret void\n}\n",
        "parameter 0 has a type no caller can pass",
    ),
    (
        "define mustprogress void @f(i8 %a) {\nentry:\n  ret void\n}\n",
        "does not apply to return values",
    ),
    (
        "define nounwind ptr @f() {\nentry:\n  ret ptr null\n}\n",
        "does not apply to return values",
    ),
    (
        "!t = !{!1}\n!1 = !DIDerivedType(tag: DW_TAG_pointer_type, size: 32, baseType: !\"bad\")\n!llvm.module.flags = !{!0}\n\n!0 = !{i32 2, !\"Debug Info Version\", i32 3}\n!9 = !DIBasicType(name: \"int\", size: 32, encoding: DW_ATE_signed)\n",
        "invalid baseType, expected a node",
    ),
    (
        "!t = !{!1}\n!1 = !DISubroutineType(types: !\"bad\")\n!llvm.module.flags = !{!0}\n\n!0 = !{i32 2, !\"Debug Info Version\", i32 3}\n!9 = !DIBasicType(name: \"int\", size: 32, encoding: DW_ATE_signed)\n",
        "invalid types, expected a node",
    ),
    (
        "!named = !{!1}\n!llvm.module.flags = !{!0}\n!llvm.dbg.cu = !{}\n\n!0 = !{i32 2, !\"Debug Info Version\", i32 3}\n!1 = distinct !DICompileUnit(language: DW_LANG_C99, file: !2, producer: \"p\", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug)\n!2 = !DIFile(filename: \"t.c\", directory: \"/\")\n",
        "DICompileUnit not listed in llvm.dbg.cu",
    ),
    (
        "!named = !{!3}\n!3 = !{!1}\n!llvm.module.flags = !{!0}\n!llvm.dbg.cu = !{}\n\n!0 = !{i32 2, !\"Debug Info Version\", i32 3}\n!1 = distinct !DICompileUnit(language: DW_LANG_C99, file: !2, producer: \"p\", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug)\n!2 = !DIFile(filename: \"t.c\", directory: \"/\")\n",
        "DICompileUnit not listed in llvm.dbg.cu",
    ),
    (
        "declare void @llvm.donothing(...)\n\ndefine void @f() {\nentry:\n  call void (...) @llvm.donothing(i64 0)\n  ret void\n}\n",
        "through a variadic signature, which it does not have",
    ),
    (
        "@buf = global [1024 x i8] zeroinitializer, align 16\n\ndeclare void @llvm.x86.tilestored64.internal(i16, i16, ptr, i64, x86_amx)\n\ndefine void @f(i16 %r, i16 %c) {\nentry:\n  call void @llvm.x86.tilestored64.internal(i16 %r, i16 %c, ptr @buf, i64 64, x86_amx undef)\n  ret void\n}\n",
        "is a constant x86_amx",
    ),
    (
        "declare ptr addrspace(1) @llvm.experimental.gc.relocate.p1(token, i32, i32)\n\ndefine i32 @f() gc \"statepoint-example\" {\nentry:\n  %r = call ptr addrspace(1) @llvm.experimental.gc.relocate.p1(token none, i32 0, i32 0)\n  ret i32 0\n}\n",
        "incorrectly tied to the statepoint",
    ),
    (
        "declare token @other()\n\ndeclare ptr addrspace(1) @llvm.experimental.gc.relocate.p1(token, i32, i32)\n\ndefine i32 @f() gc \"statepoint-example\" {\nentry:\n  %t = call token @other()\n  %r = call ptr addrspace(1) @llvm.experimental.gc.relocate.p1(token %t, i32 0, i32 0)\n  ret i32 0\n}\n",
        "incorrectly tied to the statepoint",
    ),
    (
        "declare i64 @foo() #0\n\ndefine void @f() {\nentry:\n  ret void\n}\n\nattributes #0 = { \"vector-function-abi-variant\"=\"_ZGV_LLVM_M4v_foo(vector_foo)\" }\n",
        "invalid name for a VFABI variant",
    ),
    (
        "declare i64 @foo(i64) #0\n\ndefine void @f() {\nentry:\n  ret void\n}\n\nattributes #0 = { \"vector-function-abi-variant\"=\"_ZGVnQ2v_foo(vf)\" }\n",
        "invalid name for a VFABI variant",
    ),
    (
        "declare i64 @foo(i64) #0\n\ndefine void @f() {\nentry:\n  ret void\n}\n\nattributes #0 = { \"vector-function-abi-variant\"=\"_ZGVnN0v_foo(vf)\" }\n",
        "invalid name for a VFABI variant",
    ),
    (
        "declare i64 @foo(i64) #0\n\ndefine void @f() {\nentry:\n  ret void\n}\n\nattributes #0 = { \"vector-function-abi-variant\"=\"_ZGVnN2ls_foo(vf)\" }\n",
        "invalid name for a VFABI variant",
    ),
    (
        "define void @f(ptr %p) {\nentry:\n  store i32 42, ptr %p, align 4, !tbaa !0\n  ret void\n}\n\n!0 = !{!1, !1, i64 0}\n!1 = !{!\"n\", !1, i64 0}\n",
        "access type node must be a valid scalar type",
    ),
    (
        "define void @f(ptr %p) {\nentry:\n  store i32 42, ptr %p, align 4, !tbaa !0\n  ret void\n}\n\n!0 = !{!1, !1, i64 0}\n!1 = !{!\"s\", !2, i64 0, !2, i64 4}\n!2 = !{!\"n\", !3, i64 0}\n!3 = !{!\"root\"}\n",
        "access type node must be a valid scalar type",
    ),
    (
        "define void @f(ptr %p) {\nentry:\n  store i32 42, ptr %p, align 4, !tbaa !0\n  ret void\n}\n\n!0 = !{!1, !2, i64 0}\n!1 = !{!\"a\", !1, i64 0}\n!2 = !{!\"n\", !3, i64 0}\n!3 = !{!\"root\"}\n",
        "base type node reaches no root",
    ),
    (
        "!llvm.module.flags = !{!0}\n\n!0 = !{i32 1, !\"aarch64-elf-pauthabi-platform\", i32 2}\n",
        "either both or no 'aarch64-elf-pauthabi-platform'",
    ),
    (
        "!llvm.module.flags = !{!0}\n\n!0 = !{i32 1, !\"aarch64-elf-pauthabi-version\", i32 3}\n",
        "either both or no 'aarch64-elf-pauthabi-platform'",
    ),
    (
        "declare void @llvm.assume(i1)\n\ndefine void @f(ptr %p) {\nentry:\n  call void @llvm.assume(i1 true) [\"adazdazd\"()]\n  ret void\n}\n",
        "tags must be valid attribute names",
    ),
    (
        "declare void @llvm.assume(i1)\n\ndefine void @f(ptr %p) {\nentry:\n  call void @llvm.assume(i1 true) [\"dereferenceable\"(ptr %p)]\n  ret void\n}\n",
        "dereferenceable assumptions should have 2 arguments",
    ),
    (
        "declare void @llvm.assume(i1)\n\ndefine void @f(ptr %p) {\nentry:\n  call void @llvm.assume(i1 true) [\"dereferenceable\"(ptr %p, float 1.500000e+00)]\n  ret void\n}\n",
        "second argument should be an integer",
    ),
    (
        "declare void @llvm.assume(i1)\n\ndefine void @f(ptr %p) {\nentry:\n  call void @llvm.assume(i1 true) [\"separate_storage\"(ptr %p)]\n  ret void\n}\n",
        "a separate_storage assumption names two allocations",
    ),
    (
        "declare void @g(ptr inalloca(i64))\n\ndefine void @f() {\nentry:\n  %a = alloca i64\n  call void @g(ptr inalloca(i64) %a)\n  ret void\n}\n",
        "inalloca from an alloca that is not one",
    ),
    (
        "%s = type { <vscale x 1 x double>, <vscale x 1 x double> }\n\ndefine void @f(ptr %a) {\nentry:\n  %p = getelementptr %s, ptr %a, i32 0\n  ret void\n}\n",
        "cannot target a structure that contains a scalable vector",
    ),
    (
        "%i = type { <vscale x 1 x double> }\n%s = type { %i }\n\ndefine void @f(ptr %a) {\nentry:\n  %p = getelementptr %s, ptr %a, i32 0\n  ret void\n}\n",
        "cannot target a structure that contains a scalable vector",
    ),
    (
        "declare i32 @g()\n\ndefine i32 @f() {\nentry:\n  %r = call i32 @g() speculatable\n  ret i32 %r\n}\n",
        "carries speculatable, which its callee does not",
    ),
    (
        "declare i32 @g()\n\ndefine i32 @f() {\nentry:\n  %r = call i32 @g() #0\n  ret i32 %r\n}\n\nattributes #0 = { speculatable }\n",
        "carries speculatable, which its callee does not",
    ),
    (
        "define i32 @f(ptr %p) {\nentry:\n  %r = call i32 %p() speculatable\n  ret i32 %r\n}\n",
        "carries speculatable, which its callee does not",
    ),
    (
        "declare void @g(<2147483649 x i16>)\n\ndefine void @f(<2147483649 x i16> %v) {\nentry:\n  call void @g(<2147483649 x i16> %v)\n  ret void\n}\n",
        "passes a type it cannot align",
    ),
    (
        "declare <2147483649 x i16> @g()\n\ndefine void @f() {\nentry:\n  %v = call <2147483649 x i16> @g()\n  ret void\n}\n",
        "returns a type it cannot align",
    ),
    (
        "declare void @g({ <2147483649 x i16> })\n\ndefine void @f({ <2147483649 x i16> } %v) {\nentry:\n  call void @g({ <2147483649 x i16> } %v)\n  ret void\n}\n",
        "passes a type it cannot align",
    ),
    (
        "define x86_intrcc void @f(ptr %p) {\nentry:\n  ret void\n}\n",
        "calling convention parameter requires byval",
    ),
    (
        "declare x86_intrcc void @f(i32)\n",
        "calling convention parameter requires byval",
    ),
    (
        "declare void @f() \"aarch64_pstate_sm_enabled\" \"aarch64_pstate_sm_compatible\"\n",
        "are incompatible",
    ),
    (
        "declare void @f() \"aarch64_new_za\" \"aarch64_in_za\"\n",
        "the attributes describing za state are mutually exclusive",
    ),
    (
        "declare void @f() \"aarch64_inout_zt0\" \"aarch64_za_state_agnostic\"\n",
        "the attributes describing zt0 state are mutually exclusive",
    ),
    (
        "declare void @f() \"aarch64_zt0_undef\"\n",
        "can only be applied to a callsite",
    ),
    (
        "@x = global i32 0\n@llvm.used = appending global [1 x ptr] [ptr @x], section \"llvm.metadata\"\n@p = global ptr @llvm.used\n",
        "invalid uses of intrinsic global variable @llvm.used",
    ),
    (
        "@x = global i32 0\n@llvm.used = appending global [1 x ptr] [ptr @x], section \"llvm.metadata\"\n\ndefine ptr @f() {\nentry:\n  ret ptr @llvm.used\n}\n",
        "invalid uses of intrinsic global variable @llvm.used",
    ),
    (
        "@llvm.global_ctors = appending global [1 x { i32, ptr, ptr }] [{ i32, ptr, ptr } { i32 65535, ptr @c, ptr null }]\n@p = global ptr @llvm.global_ctors\n\ndefine void @c() {\nentry:\n  ret void\n}\n",
        "invalid uses of intrinsic global variable @llvm.global_ctors",
    ),
    (
        "!llvm.module.flags = !{!0}\n\n!0 = !{i32 1, !\"SemanticInterposition\", !\"yes\"}\n",
        "SemanticInterposition is a number rather than a word",
    ),
    (
        "!llvm.module.flags = !{!0}\n\n!0 = !{i32 5, !\"CG Profile\", !1}\n!1 = !{!2}\n!2 = !{ptr null, ptr null}\n",
        "a CG Profile edge names a caller, a callee and a count",
    ),
    (
        "define void @f(ptr %p) {\nentry:\n  %v = load i32, ptr %p, !tbaa !0\n  ret void\n}\n\n!0 = !{!1, !1}\n!1 = !{!\"x\"}\n",
        "a tbaa tag has three operands or four, not 2",
    ),
    (
        "declare void @g(...)\n\ndefine void @f(ptr %p) {\nentry:\n  call void (...) @g(ptr sret(i32) %p)\n  ret void\n}\n",
        "marks a variadic argument sret",
    ),
    (
        "declare void @g(ptr, ...)\n\ndefine void @f(ptr %p) {\nentry:\n  call void (ptr, ...) @g(ptr %p, ptr returned %p)\n  ret void\n}\n",
        "marks a variadic argument returned",
    ),
    (
        "define void @f(ptr %p) naked {\nentry:\n  %g = getelementptr i8, ptr %p, i64 1\n  unreachable\n}\n",
        "a naked function reads an argument it was never given",
    ),
    (
        "define void @f(<4 x i32> %a, <4 x i32> %b) {\nentry:\n  %r = call <8 x i32> @llvm.scmp.v8i32.v4i32(<4 x i32> %a, <4 x i32> %b)\n  ret void\n}\n",
        "answers in a different number of lanes than it compares",
    ),
    (
        "define void @f(i32 %a, i32 %b) {\nentry:\n  %r = call i1 @llvm.scmp.i1.i32(i32 %a, i32 %b)\n  ret void\n}\n",
        "answers three ways in 1 bit, which holds two",
    ),
    (
        "define void @f(<8 x float> %s, <4 x i1> %m, i32 %n) {\nentry:\n  %r = call <4 x i32> @llvm.vp.fptosi.v4i32.v8f32(<8 x float> %s, <4 x i1> %m, i32 %n)\n  ret void\n}\n",
        "casts a different number of lanes than it produces",
    ),
    (
        "@v = global i32 0\n\ndefine void @f() {\nentry:\n  %p = call ptr @llvm.threadlocal.address(ptr @v)\n  ret void\n}\n",
        "takes the address of something that is not thread-local",
    ),
    (
        "declare void @llvm.experimental.noalias.scope.decl(metadata)\n\ndefine void @f() {\nentry:\n  call void @llvm.experimental.noalias.scope.decl(metadata !0)\n  ret void\n}\n\n!0 = !{!1, !2}\n!1 = !{!1}\n!2 = !{!2}\n",
        "declares something other than one scope",
    ),
    (
        "declare tailcc void @g(i32)\n\ndefine tailcc void @f(i32 inreg %x) {\nentry:\n  musttail call tailcc void @g(i32 %x)\n  ret void\n}\n",
        "musttail call in a tail convention, where inreg has nowhere to go",
    ),
    (
        "define void @f(ptr %p) {\nentry:\n  %v = load atomic i24, ptr %p monotonic, align 4\n  ret void\n}\n",
        "moves 24 bits, which is not a size an atomic comes in",
    ),
    (
        "define void @f(ptr %p) {\nentry:\n  %v = load atomic <2 x i32>, ptr %p monotonic, align 8\n  ret void\n}\n",
        "moves a type an atomic cannot move",
    ),
    (
        "define void @f(ptr %p) {\nentry:\n  %r = atomicrmw fadd ptr %p, <3 x half> undef seq_cst, align 4\n  ret void\n}\n",
        "moves 48 bits, which is not a size an atomic comes in",
    ),
    (
        "define void @f(ptr %p) {\nentry:\n  %x = cmpxchg ptr %p, float 0.0, float 1.0 monotonic monotonic\n  ret void\n}\n",
        "compares something other than an integer or a pointer",
    ),
    (
        "define void @f() {\nentry:\n  %x = shufflevector <2 x i32> undef, <2 x i32> undef, <2 x i32> <i32 0, i32 4>\n  ret void\n}\n",
        "picks a lane the two vectors together do not have",
    ),
    (
        "define void @f() {\nentry:\n  %x = shufflevector <2 x i32> undef, <2 x i32> undef, <2 x i32> <i32 0, i32 -1>\n  ret void\n}\n",
        "picks a lane the two vectors together do not have",
    ),
    (
        "define void @f(i1 %c) {\nentry:\n  br i1 %c, label %b, label %b\n\nb:\n  %p = phi i32 [ 0, %entry ]\n  ret void\n}\n",
        "does not name exactly its block's incoming edges",
    ),
    (
        "define void @f(i32 %x) {\nentry:\n  switch i32 %x, label %d [ i32 1, label %b\n i32 2, label %b ]\n\nb:\n  %p = phi i32 [ 0, %entry ], [ 1, %entry ]\n  ret void\n\nd:\n  ret void\n}\n",
        "gives %entry two values for one arrival",
    ),
    (
        "define void @f() personality ptr null {\nentry:\n  %c = catchswitch within none [ label %h ] unwind to caller\n\nh:\n  %p = catchpad within %c []\n  catchret from %p to label %e\n\ne:\n  ret void\n}\n",
        "catchswitch opens the entry block",
    ),
    (
        "define void @f() personality ptr null {\nentry:\n  br label %s\n\ns:\n  %c = catchswitch within none [ label %h ] unwind to caller\n\nh:\n  ret void\n}\n",
        "hands to %h, which has no catchpad",
    ),
    (
        "define void @f() {\nentry:\n  ret void\n\nb:\n  %p = cleanuppad within none []\n  cleanupret from %p unwind to caller\n}\n",
        "cleanuppad in %b needs a personality routine",
    ),
    (
        "@b = global ptr blockaddress(@f, %missing)\n\ndefine void @f() {\nentry:\n  ret void\n}\n",
        "a blockaddress names %missing, which @f does not define",
    ),
    (
        "define void @f() {\nentry:\n  %a = landingpad { ptr, i32 } cleanup\n  ret void\n}\n",
        "landingpad in %entry needs a personality routine",
    ),
    (
        "define void @f() {\nentry:\n  resume { ptr, i32 } undef\n}\n",
        "resume in %entry needs a personality routine",
    ),
    (
        "define void @f() personality ptr null {\nentry:\n  %a = landingpad { ptr, i32 } cleanup\n  ret void\n}\n",
        "a landingpad in %entry sits in a block nothing unwinds to",
    ),
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
        "does not name exactly its block's incoming edges",
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
    // A global holds no scalable type at all once it has an initialiser to
    // lay out, and a struct holds one only when it holds nothing else.
    (
        "%t = type { <vscale x 1 x i32> }\n@g = external global %t\n",
        "has an invalid type for a global variable",
    ),
    (
        "%u = type { i32, <vscale x 1 x i32> }\n\ndefine void @f() {\nentry:\n  %a = alloca %u, align 8\n  ret void\n}\n",
        "has an invalid type for alloca",
    ),
    (
        "define void @f(ptr %p) {\nentry:\n  %a = load i32, ptr %p, align 4\n  ret void\n}\ndeclare void @g(ptr align 3 %p)\n",
        "is not a power of two",
    ),
    (
        "define void @f() sanitize_realtime sanitize_realtime_blocking {\nentry:\n  ret void\n}\n",
        "sanitize_realtime and sanitize_realtime_blocking are incompatible",
    ),
    (
        "%X = type opaque\n\ndefine void @f() {\nentry:\n  %a = alloca %X, align 8\n  ret void\n}\n",
        "has an invalid type for alloca",
    ),
    (
        "define i32 @f(i32 %x) {\nentry:\n  %a = add i32 %x, 42, !mmra !0\n  ret i32 %a\n}\n\n!0 = !{}\n",
        "mmra is attached to an instruction that takes none",
    ),
    (
        "define void @f(ptr %p) {\nentry:\n  store i32 0, ptr %p, align 4, !annotation !0\n  ret void\n}\n\n!0 = !{}\n",
        "annotation needs at least one operand",
    ),
    (
        "declare void @llvm.va_start(ptr)\n\ndefine void @f(ptr %p) {\nentry:\n  call void @llvm.va_start(ptr %p)\n  ret void\n}\n",
        "llvm.va_start is called in a function that takes no variable arguments",
    ),
    (
        "declare void @g(ptr, i32)\n\ndefine void @f(ptr %p) {\nentry:\n  call void @g(ptr inalloca(i32) %p, i32 3)\n  ret void\n}\n",
        "inalloca on an argument that is not the last",
    ),
    (
        "define void @f(ptr %fn) {\nentry:\n  %r = call token %fn()\n  ret void\n}\n",
        "returns a token from an indirect call",
    ),
    (
        "declare void @g()\n\ndefine void @f() {\nentry:\n  call void @g() speculatable\n  ret void\n}\n",
        "carries speculatable, which its callee does not",
    ),
    (
        "define void @f(ptr %fn) {\nentry:\n  call void %fn() [ \"kcfi\"(i32 42), \"kcfi\"(i32 42) ]\n  ret void\n}\n",
        "carries 2 kcfi operand bundles",
    ),
    (
        "define void @f(ptr %p, [4 x i32] %v) {\nentry:\n  store atomic [4 x i32] %v, ptr %p seq_cst, align 16\n  ret void\n}\n",
        "stores a type an atomic cannot move",
    ),
    (
        "define ptr @resolver() {\nentry:\n  ret ptr null\n}\n\n@f = extern_weak ifunc void (), ptr @resolver\n",
        "has a linkage an ifunc may not have",
    ),
    (
        "define void @f() \"no-jump-tables\"=\"yes\" {\nentry:\n  ret void\n}\n",
        "invalid value for 'no-jump-tables' attribute: yes",
    ),
    (
        "declare ptr @a(i32) allocsize(1)\n",
        "allocsize names parameter 1",
    ),
    (
        "declare ptr @a(i32) allockind(\"aligned\")\n",
        "allockind names none or several of alloc, realloc and free",
    ),
    (
        "@a = global i8 42\n@llvm.used = appending global [2 x ptr] [ptr @a, ptr null], section \"llvm.metadata\"\n",
        "has a member that names no symbol",
    ),
    (
        "define i32 @f() !prof !0 {\nentry:\n  ret i32 0\n}\n\n!0 = !{i32 123, i32 3}\n",
        "starts with the annotation's name",
    ),
    (
        "declare x86_stdcallcc void @g()\n\ndefine void @f() {\nentry:\n  musttail call x86_stdcallcc void @g()\n  ret void\n}\n",
        "musttail call whose convention differs from its caller's",
    ),
    (
        "target datalayout = \"e-p:64:64\"\ndefine void @f(ptr byval([2147483648 x i16]) %p) {\nentry:\n  ret void\n}\n",
        "is too large",
    ),
    (
        "!named = !{!0}\n!0 = !DICompositeType(tag: DW_TAG_structure_type, name: \"A\", size: 1, flags: DIFlagTypePassByReference | DIFlagTypePassByValue)\n",
        "passed both by reference and by value",
    ),
    (
        "define void @f() {\nentry:\n  callbr void asm sideeffect \"\", \"!i\"()\n          to label %a [label %b, label %c]\n\na:\n  ret void\n\nb:\n  ret void\n\nc:\n  ret void\n}\n",
        "has 1 label constraints for 2 indirect labels",
    ),
    (
        "define void @f(ptr %p) {\nentry:\n  call void asm \"\", \"=*rm\"(ptr %p)\n  ret void\n}\n",
        "to an indirect constraint without elementtype",
    ),
    (
        "declare half @llvm.fptrunc.round(float, metadata)\n\ndefine void @f(float %a) {\nentry:\n  %r = call half @llvm.fptrunc.round(float %a, metadata !\"round.nonsense\")\n  ret void\n}\n",
        "round.nonsense, which is not one of them",
    ),
    // A module can declare an intrinsic consistently wrongly, and then the
    // declaration it is checked against is wrong too. LangRef is the only
    // thing left that knows llvm.cttz's second argument is i1.
    (
        "declare i32 @llvm.cttz.i32(i32, i32)\n\ndefine void @f(i32 %x) {\nentry:\n  %r = call i32 @llvm.cttz.i32(i32 %x, i32 0)\n  ret void\n}\n",
        "wrong type in argument 1 of an intrinsic",
    ),
    // The rules LangRef states in prose rather than in a declare line.
    (
        "declare i24 @llvm.bswap.i24(i24)\n\ndefine void @f(i24 %x) {\nentry:\n  %r = call i24 @llvm.bswap.i24(i24 %x)\n  ret void\n}\n",
        "not a whole number of byte pairs",
    ),
    (
        "declare <4 x i32> @llvm.masked.load.v4i32.p0(ptr, i32, <4 x i1>, <4 x i32>)\n\ndefine void @f(ptr %p, <4 x i1> %m, <4 x i32> %v) {\nentry:\n  %r = call <4 x i32> @llvm.masked.load.v4i32.p0(ptr %p, i32 3, <4 x i1> %m, <4 x i32> %v)\n  ret void\n}\n",
        "an alignment of 3, which is not a power of two",
    ),
    (
        "declare <4 x i32> @llvm.get.active.lane.mask.v4i32.i32(i32, i32)\n\ndefine void @f(i32 %a, i32 %b) {\nentry:\n  %r = call <4 x i32> @llvm.get.active.lane.mask.v4i32.i32(i32 %a, i32 %b)\n  ret void\n}\n",
        "produces a mask that is not made of i1",
    ),
    (
        "declare i32 @llvm.ptrmask.i32.i32(i32, i32)\n\ndefine void @f(i32 %p, i32 %m) {\nentry:\n  %r = call i32 @llvm.ptrmask.i32.i32(i32 %p, i32 %m)\n  ret void\n}\n",
        "masks something that is not a pointer",
    ),
    (
        "declare i32 @llvm.experimental.get.vector.length.i32(i32, i32, i1)\n\ndefine void @f(i32 %n) {\nentry:\n  %r = call i32 @llvm.experimental.get.vector.length.i32(i32 %n, i32 0, i1 true)\n  ret void\n}\n",
        "asks for a vector factor of zero",
    ),
    (
        "declare <4 x i32> @llvm.vector.splice.v4i32(<4 x i32>, <4 x i32>, i32)\n\ndefine void @f(<4 x i32> %a, <4 x i32> %b) {\nentry:\n  %r = call <4 x i32> @llvm.vector.splice.v4i32(<4 x i32> %a, <4 x i32> %b, i32 9)\n  ret void\n}\n",
        "splices at 9, which is outside a vector of 4",
    ),
    (
        "declare <2 x i32> @llvm.vector.extract.v2i32.v4i32(<4 x i32>, i64)\n\ndefine void @f(<4 x i32> %a) {\nentry:\n  %r = call <2 x i32> @llvm.vector.extract.v2i32.v4i32(<4 x i32> %a, i64 1)\n  ret void\n}\n",
        "starts at 1, which is not a multiple of 2",
    ),
    (
        "declare <4 x i32> @llvm.get.dynamic.area.offset.v4i32()\n\ndefine void @f() {\nentry:\n  %r = call <4 x i32> @llvm.get.dynamic.area.offset.v4i32()\n  ret void\n}\n",
        "other than a scalar integer",
    ),
    (
        "declare i64 @llvm.aarch64.ldxr.p0(ptr)\n\ndefine void @f(ptr %p) {\nentry:\n  %r = call i64 @llvm.aarch64.ldxr.p0(ptr %p)\n  ret void\n}\n",
        "reaches through argument 0 without an elementtype",
    ),
    (
        "declare void @llvm.experimental.deoptimize.isVoid(...)\n\ndefine void @f() {\nentry:\n  call void (...) @llvm.experimental.deoptimize.isVoid() [ \"deopt\"() ]\n  br label %next\n\nnext:\n  ret void\n}\n",
        "is not followed by a return",
    ),
    (
        "declare void @llvm.experimental.guard(i1, ...)\n\ndefine void @f(i1 %c) {\nentry:\n  call void (i1, ...) @llvm.experimental.guard(i1 %c)\n  ret void\n}\n",
        "carries 0 deopt bundles, not one",
    ),
    (
        "%X = type opaque\ndeclare void @g(ptr inalloca(%X) %p)\n",
        "inalloca on parameter 0 names a type with no size",
    ),
    (
        "define void @f(ptr byval([2147483648 x i16]) %p) {\nentry:\n  ret void\n}\n",
        "is too large",
    ),
    (
        "declare ptr @a(i32, i32) allocsize(0, 0)\n",
        "allocsize names parameter 0 twice",
    ),
    (
        "declare <4 x float> @llvm.matrix.transpose.v4f32(<4 x float>, i32, i32)\n\ndefine void @f(<4 x float> %m) {\nentry:\n  %r = call <4 x float> @llvm.matrix.transpose.v4f32(<4 x float> %m, i32 3, i32 2)\n  ret void\n}\n",
        "transposes a 3 by 2 matrix held in 4 lanes",
    ),
    // The operation an atomicrmw performs says what it can perform it on.
    (
        "define void @f(ptr %p) {\nentry:\n  %r = atomicrmw add ptr %p, float 1.000000e+00 seq_cst, align 4\n  ret void\n}\n",
        "operates on a type its operation cannot take",
    ),
    (
        "define void @f(ptr %p) {\nentry:\n  %r = atomicrmw fadd ptr %p, i32 2 seq_cst, align 4\n  ret void\n}\n",
        "operates on a type its operation cannot take",
    ),
    (
        "define void @f(ptr %p, <2 x float> %v) {\nentry:\n  %r = atomicrmw xchg ptr %p, <2 x float> %v seq_cst, align 8\n  ret void\n}\n",
        "operates on a type its operation cannot take",
    ),
    // Every cast but a bitcast works lane by lane.
    (
        "define void @f(<4 x i64> %x) {\nentry:\n  %y = trunc <4 x i64> %x to i8\n  ret void\n}\n",
        "casts between different vector shapes",
    ),
    (
        "define void @f(<4 x i64> %x) {\nentry:\n  %y = trunc <4 x i64> %x to <3 x i8>\n  ret void\n}\n",
        "casts between different vector shapes",
    ),
    (
        "define void @f() {\nentry:\n  %y = alloca i32, align 4, addrspace(16777216)\n  ret void\n}\n",
        "which is too large",
    ),
    (
        "declare dso_local dllimport void @fun()\n",
        "both dllimport and dso_local",
    ),
    // Where an attribute belongs, and what a field can hold.
    (
        "declare void @llvm.f() immarg\n",
        "immarg describes a call site rather than a function",
    ),
    (
        "declare immarg i32 @llvm.g(i32 %x)\n",
        "immarg on the return value",
    ),
    ("@v = global i32 0, comdat($v)\n", "which does not exist"),
    (
        "define void @f() {\nentry:\n  %r = insertvalue { i32, i32 } undef, ptr null, 0\n  ret void\n}\n",
        "writes a value the field cannot hold",
    ),
    (
        "define void @f() builtin {\nentry:\n  ret void\n}\n",
        "builtin describes a call site rather than a function",
    ),
    (
        "declare void @f()\n\ndefine void @g() {\nentry:\n  call void @f() align 8\n  ret void\n}\n",
        "describes an argument rather than a call",
    ),
    // What a value can be, and where an attribute describes one.
    (
        "define void @f(label %bb) {\nentry:\n  ret void\n}\n",
        "parameter 0 has a type no caller can pass",
    ),
    (
        "declare void @foo(i32 safestack %x)\n",
        "safestack on parameter 0",
    ),
    (
        "declare safestack void @foo()\n",
        "safestack on the return value",
    ),
    (
        "define void @f() {\nentry:\n  %p = phi void ()\n  ret void\n}\n",
        "produces a type no register can hold",
    ),
    (
        "define void @f(i8 range(i8 1, 1) %x) {\nentry:\n  ret void\n}\n",
        "an empty range on parameter 0 constrains nothing",
    ),
    // A type that contains itself has no size, and walking it without a
    // trail does not return. This case used to abort the process.
    (
        "%s = type { %s }\n@g = global %s zeroinitializer\n",
        "has an invalid type for a global variable",
    ),
    (
        "define void @f(ptr %p, i32 %v) {\nentry:\n  %r = cmpxchg ptr %p, i32 %v, ptr null seq_cst seq_cst, align 4\n  ret void\n}\n",
        "operand of the wrong type",
    ),
    (
        "declare void @f3() uwtable(unsync)\n",
        "uwtable names unsync, which is not a kind of unwind table",
    ),
    (
        "define void @0() comdat {\nentry:\n  ret void\n}\n",
        "has no name to key a comdat on",
    ),
    (
        "declare void @f(i32 immarg %x)\n",
        "immarg on parameter 0 of a function that is not an intrinsic",
    ),
    (
        "target datalayout = \"P200\"\n\ndefine void @f(ptr %fn) {\nentry:\n  %r = call i8 %fn(i32 0)\n  ret void\n}\n",
        "rather than 200",
    ),
    (
        "define void @f() {\nentry:\n  ret void, !bar !1\n}\n\n!0 = !{}\n!1 = !{metadata !0}\n",
        "holds metadata wrapped in a value",
    ),
    (
        "%myTy = type { %myTy }\n",
        "%myTy contains itself, so it has no size",
    ),
    (
        "$v = comdat any\n$v = comdat any\n",
        "$v is declared more than once",
    ),
    (
        "define void @f(i8 mustprogress %a) {\nentry:\n  ret void\n}\n",
        "mustprogress on parameter 0, which describes a function",
    ),
    (
        "declare void @llvm.f(ptr byval(i32) immarg %p)\n",
        "alongside an attribute that places it",
    ),
    (
        "declare float @llvm.vector.reduce.fadd.f32.v2f64(double, <2 x double>)\n\ndefine void @f(double %a, <2 x double> %v) {\nentry:\n  %r = call float @llvm.vector.reduce.fadd.f32.v2f64(double %a, <2 x double> %v)\n  ret void\n}\n",
        "reduces to a type the vector does not hold",
    ),
    // Neither of these appears in upstream's suites; both were found by
    // writing the case and asking llvm-as.
    (
        "declare i32 @f()\n\ndefine i64 @g() {\nentry:\n  %r = musttail call i32 @f()\n  ret i64 0\n}\n",
        "musttail call that returns something its caller does not",
    ),
    (
        "@g = global i32 0\n@a = alias i32, ptr @a\n",
        "aliases its way back to itself",
    ),
    (
        "define void @f(<4 x i32> %v) {\nentry:\n  %b = insertelement <4 x i32> %v, i64 0, i32 0\n  ret void\n}\n",
        "inserts a type the vector does not hold",
    ),
    (
        "define void @f(ptr %p) {\nentry:\n  fence monotonic\n  ret void\n}\n",
        "ordering that orders nothing",
    ),
    (
        "define void @f(ptr %p) {\nentry:\n  %r = load atomic i32, ptr %p release, align 4\n  ret void\n}\n",
        "ordering a load cannot have",
    ),
    (
        "define void @f(ptr %p) {\nentry:\n  store atomic i32 0, ptr %p acquire, align 4\n  ret void\n}\n",
        "ordering a store cannot have",
    ),
    (
        "define void @f(<4 x i1> %c, <2 x i32> %a, <2 x i32> %b) {\nentry:\n  %r = select <4 x i1> %c, <2 x i32> %a, <2 x i32> %b\n  ret void\n}\n",
        "picks with a condition of another width",
    ),
    (
        "define i32 @f() {\nentry:\n  br label %missing\n}\n",
        "names a block this function does not define",
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
        "declare i32 @v()\n\ndefine i32 @f() {\ne:\n  %r = invoke i32 @v()\n          to label %n unwind label %n\n\nn:\n  ret i32 0\n}\n",
        "does not begin with a pad",
    ),
    (
        "declare void @llvm.gcroot(ptr, ptr)\n\ndefine void @f() {\nentry:\n  %a = alloca ptr, align 8\n  call void @llvm.gcroot(ptr %a, ptr null)\n  ret void\n}\n",
        "a gc barrier is used in a function that names no collector",
    ),
    (
        "define void @f() alwaysinline noinline {\nentry:\n  ret void\n}\n",
        "alwaysinline and noinline are incompatible",
    ),
    (
        "declare void @foo(ptr signext %p)\n",
        "signext on parameter 0, which is not an integer",
    ),
    (
        "define float @f(float %x) {\nentry:\n  %s = fadd float %x, %x\n  %i = add i32 1, 1, !fpmath !0\n  ret float %s\n}\n\n!0 = !{float 2.500000e+00}\n",
        "fpmath requires a floating-point result",
    ),
    (
        "define void @f() {\nentry:\n  %a = alloca i32, align 4, !invariant.group !0\n  ret void\n}\n\n!0 = !{}\n",
        "invariant.group is only for loads and stores",
    ),
    (
        "declare hidden dllexport i32 @f()\n",
        "a dllexport symbol must have default or protected visibility",
    ),
    (
        "define void @f() \"warn-stack-size\"=\"-1\" {\nentry:\n  ret void\n}\n",
        "'warn-stack-size' takes an unsigned integer: -1",
    ),
    (
        "define void @f() \"sign-return-address\"=\"non-leaf-or-something\" {\nentry:\n  ret void\n}\n",
        "invalid value for 'sign-return-address' attribute",
    ),
    (
        "declare ptr @a(i64) \"alloc-variant-zeroed\"=\"\"\n",
        "'alloc-variant-zeroed' must not be empty",
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
    // A local is counted inside its own function, nothing outside one being
    // able to use it. A name the body never defines has no uses at all.
    (
        "define void @f(i32 %x) {\nentry:\n  %a = add i32 %x, %x\n  %b = add i32 %x, %a\n  ret void\n  uselistorder i32 %x, { 1, 0 }\n}\n",
        "wrong number of indexes, expected 3",
    ),
    (
        "define void @f(i32 %x) {\nentry:\n  ret void\n  uselistorder i32 %missing, { 1, 0 }\n}\n",
        "value has no uses",
    ),
    // A body's use-list order directives come last, in a run. An
    // instruction after one is not one, and neither is a label.
    (
        "define void @f(i32 %x) {\nentry:\n  %a = add i32 %x, %x\n  %b = add i32 %x, %a\n  ret void\n  uselistorder i32 %x, { 2, 1, 0 }\n  %c = add i32 1, 1\n}\n",
        "expected uselistorder directive",
    ),
    (
        "define void @f(i32 %x) {\nentry:\n  %a = add i32 %x, %x\n  %b = add i32 %x, %a\n  ret void\n  uselistorder i32 %x, { 2, 1, 0 }\nnext:\n  ret void\n}\n",
        "expected uselistorder directive",
    ),
    // A use-list order directive names its value with the type that value
    // was defined with. A local is reported against its own definition,
    // whether it is a parameter or an instruction result.
    (
        "define void @f(i32 %x, i32 %y) {\nentry:\n  %a = add i32 %x, %y\n  %b = add i32 %x, %a\n  ret void\n  uselistorder float %x, { 1, 0 }\n}\n",
        "is defined with a type a uselistorder directive does not name",
    ),
    (
        "define void @f(i32 %y) {\nentry:\n  %a = add i32 %y, %y\n  %b = add i32 %a, %a\n  ret void\n  uselistorder float %a, { 1, 0 }\n}\n",
        "is defined with a type a uselistorder directive does not name",
    ),
    // A symbol reference has the symbol's own pointer type, so naming one
    // with anything else is not a reference to it.
    (
        "@g = global i32 0\n@a1 = alias i32, ptr @g\n@a2 = alias i32, ptr @g\nuselistorder i32 @g, { 1, 0 }\n",
        "global variable reference must have pointer type",
    ),
    // Positions that share one overloaded type have to agree about what it
    // is, or there is no instantiation for the declaration to be built
    // from. `llvm.umax` ties both arguments to the result.
    (
        "define void @t() {\nentry:\n  %r = call i8 @llvm.umax(i8 0, i16 1)\n  ret void\n}\n",
        "invalid intrinsic signature",
    ),
    (
        "define void @t() {\nentry:\n  %r = call i16 @llvm.umax(i8 0, i8 1)\n  ret void\n}\n",
        "invalid intrinsic signature",
    ),
    (
        "define void @t() {\nentry:\n  %r = call i8 @llvm.fshl(i8 0, i8 1, i16 2)\n  ret void\n}\n",
        "invalid intrinsic signature",
    ),
    // A `flags:` is a set of words and each of them has to be one. The mask
    // names LLVM groups them under are not words a module may write.
    (
        "!named = !{!0}\n!0 = !DIBasicType(name: \"n\", flags: DIFlagUnknown)\n",
        "invalid debug info flag 'DIFlagUnknown'",
    ),
    (
        "!named = !{!0}\n!0 = !DIBasicType(name: \"n\", flags: DIFlagPublic | DIFlagAccessibility)\n",
        "invalid debug info flag 'DIFlagAccessibility'",
    ),
    // Each vocabulary was swept over its whole range, so a word none of the
    // tables has is a word upstream does not know.
    (
        "!named = !{!0}\n!0 = !GenericDINode(tag: DW_TAG_badtag)\n",
        "invalid DWARF tag 'DW_TAG_badtag'",
    ),
    (
        "!named = !{!0}\n!0 = distinct !DICompileUnit(language: DW_LANG_NoSuch, file: !1)\n!1 = !DIFile(filename: \"a\", directory: \"d\")\n",
        "invalid DWARF language 'DW_LANG_NoSuch'",
    ),
    (
        "!named = !{!0}\n!0 = !DIBasicType(name: \"n\", encoding: DW_ATE_nope)\n",
        "invalid DWARF type attribute encoding 'DW_ATE_nope'",
    ),
    (
        "!named = !{!0}\n!0 = !DISubroutineType(types: null, cc: DW_CC_nope)\n",
        "invalid DWARF calling convention 'DW_CC_nope'",
    ),
    // `operands:` holds the node's own operands, written with braces, so a
    // reference to a node that holds them is not what it takes.
    (
        "!named = !{!1}\n!0 = !{}\n!1 = !GenericDINode(tag: DW_TAG_entry_point, operands: !0)\n",
        "expected '{' here",
    ),
    // A promise about a value survives a change of precision and nothing
    // else: an integer has no NaN for the promise to be about, so upstream
    // reads no fast-math word there and reports the type it wanted.
    (
        "define float @f(i32 %i) {\nentry:\n  %r = sitofp nnan i32 %i to float\n  ret float %r\n}\n",
        "expected type",
    ),
    (
        "define i32 @f(float %x) {\nentry:\n  %r = fptosi ninf float %x to i32\n  ret i32 %r\n}\n",
        "expected type",
    ),
    // Seven fields a node cannot be written without, derived by
    // `corpus/md-required-fields.nu` and missing from the schema before it.
    (
        "!llvm.dbg.cu = !{!0}\n!llvm.module.flags = !{!1}\n\n!0 = distinct !DICompileUnit(language: DW_LANG_C99, producer: \"p\", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug)\n!1 = !{i32 2, !\"Debug Info Version\", i32 3}\n",
        "missing required field",
    ),
    (
        "!named = !{!0}\n!0 = !DIModule(name: \"M\")\n",
        "missing required field",
    ),
    (
        "!named = !{!0}\n!0 = !DIMacro(type: DW_MACINFO_define, value: \"v\")\n",
        "missing required field",
    ),
    (
        "@a = global i32 0\n\nuselistorder ptr @a, { 0, 0 }\n",
        "uselistorder indexes are a permutation, so they are distinct",
    ),
    (
        "@a = global i32 0\n\nuselistorder ptr @a, { 0, 1 }\n",
        "uselistorder indexes that are already in order say nothing",
    ),
    (
        "define void @f() {\nentry:\n  ret void\n}\n\nuselistorder_bb @f, %missing, { 1, 0 }\n",
        "uselistorder_bb names a block its function does not define",
    ),
    (
        "declare void @f()\n\nuselistorder_bb @f, %entry, { 1, 0 }\n",
        "uselistorder_bb names a function with no body",
    ),
    (
        "define void @f() {\nentry:\n  ret void\n}\n\nuselistorder_bb @missing, %entry, { 1, 0 }\n",
        "uselistorder_bb names a function this module does not have",
    ),
    (
        "!named = !{!{i32 1}}\n",
        "a named metadata list holds references, not nodes",
    ),
    (
        "declare void @llvm.dbg.value(metadata, metadata, metadata)\n\ndefine void @f(i32 %a) !dbg !5 {\nentry:\n    #dbg_value(i32 %a, !4, !DIExpression(), !8)\n  call void @llvm.dbg.value(metadata i32 %a, metadata !4, metadata !DIExpression()), !dbg !8\n  ret void\n}\n\n!llvm.module.flags = !{!0}\n!llvm.dbg.cu = !{!1}\n\n!0 = !{i32 2, !\"Debug Info Version\", i32 3}\n!1 = distinct !DICompileUnit(language: DW_LANG_C99, file: !2, producer: \"p\", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug)\n!2 = !DIFile(filename: \"a.c\", directory: \"/\")\n!4 = !DILocalVariable(name: \"v\", scope: !5, file: !2, line: 1)\n!5 = distinct !DISubprogram(name: \"f\", scope: !2, file: !2, line: 1, type: !6, spFlags: DISPFlagDefinition, unit: !1)\n!6 = !DISubroutineType(types: !7)\n!7 = !{null}\n!8 = !DILocation(line: 1, column: 1, scope: !5)\n",
        "records in one place and as intrinsic calls in another",
    ),
    (
        "@g = global [2 x x86_amx] zeroinitializer\n",
        "invalid array element type",
    ),
    (
        "@g = global <2 x x86_amx> zeroinitializer\n",
        "invalid vector element type",
    ),
    (
        "define void @f(i32 %a) !dbg !5 {\nentry:\n    #dbg_invalid(i32 %a, !4, !DIExpression(), !8)\n  ret void\n}\n",
        "#dbg_invalid is not a debug record",
    ),
    (
        "define float @f() {\nentry:\n  ret float 0x7FF0000000000001\n}\n",
        "floating point constant invalid for type",
    ),
    (
        "define void @f() {\nentry:\n  call void @llvm.not.a.real.intrinsic()\n  ret void\n}\n",
        "reference to undefined symbol @llvm.not.a.real.intrinsic",
    ),
    (
        "define void @f(ptr %p) {\nentry:\n  %v = load atomic i32, ptr %p monotonic\n  ret void\n}\n",
        "atomic load needs an alignment of its own",
    ),
    (
        "define void @f(ptr %p) {\nentry:\n  store atomic i32 0, ptr %p monotonic\n  ret void\n}\n",
        "atomic store needs an alignment of its own",
    ),
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
        "@\"\0\" = global i32 0\n",
        "NUL character is not allowed in names",
    ),
    (
        "declare ptr @f(ptr, ...)\n\ndefine ptr @g(ptr %this) {\n  %rv = musttail call ptr (ptr, ...) @f(ptr %this, ...)\n  ret ptr %rv\n}\n",
        "musttail call in non-varargs function",
    ),
    (
        "define void @f(ptr* %a) {\nentry:\n  ret void\n}\n",
        "ptr* is invalid - use ptr instead",
    ),
    (
        "@g = global ptr* null\n",
        "ptr* is invalid - use ptr instead",
    ),
    (
        "declare ptr @f(ptr, ...)\n\ndefine ptr @g(ptr %this, ...) {\n  %rv = call ptr (ptr, ...) @f(ptr %this, ...)\n  ret ptr %rv\n}\n",
        "ellipsis in argument list for non-musttail call",
    ),
    (
        "declare ptr @f(ptr, ...)\n\ndefine ptr @g(ptr %this, ...) {\n  %rv = musttail call ptr (ptr, ...) @f(ptr %this)\n  ret ptr %rv\n}\n",
        "expected '...' at end of argument list for musttail call in varargs function",
    ),
    (
        "define void @f(ptr align 8589934592 %p) {\nentry:\n  ret void\n}\n",
        "huge alignment values are unsupported",
    ),
    (
        "define void @f() alignstack(4294967296) {\nentry:\n  ret void\n}\n",
        "huge alignment values are unsupported",
    ),
    (
        "define void @f(<4294967296 x i8> %v) {\nentry:\n  ret void\n}\n",
        "size too large for vector",
    ),
    (
        "define void @f(<vscale x 4294967296 x i8> %v) {\nentry:\n  ret void\n}\n",
        "size too large for vector",
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
    ("@i2 = common global i8388609 0, align 4\n", "is too wide"),
    (
        "define void @f() {\nentry:\n  %a = alloca i32, i32 4, i32 4, align 4\n  ret void\n}\n",
        "counts its elements once",
    ),
    (
        "define void @f() {\nentry:\n  ret void\n\nentry:\n  ret void\n}\n",
        "redefinition of block '%entry'",
    ),
    (
        "define void @f(<4 x i32> %a, <2 x i32> %b) {\nentry:\n  %r = shufflevector <4 x i32> %a, <2 x i32> %b, <4 x i32> zeroinitializer\n  ret void\n}\n",
        "shuffles two vectors of different types",
    ),
    (
        "define void @f() {\nentry:\n  %x = add i32 0, 1\n  %x = add i32 0, 1\n  ret void\n}\n",
        "redefinition of '%x'",
    ),
    (
        "define void @f(target(\"type\", i32, 0, void) %x) {\nentry:\n  ret void\n}\n",
        "writes its types before its integers",
    ),
    (
        "define void @f(i32 %a) !dbg !4 {\nentry:\n  #dbg_value(i32 %a, i32 0, !DIExpression(), !9)\n  ret void\n}\n\n!llvm.dbg.cu = !{!0}\n!llvm.module.flags = !{!3}\n!0 = distinct !DICompileUnit(language: DW_LANG_C99, file: !1)\n!1 = !DIFile(filename: \"a\", directory: \"b\")\n!3 = !{i32 2, !\"Debug Info Version\", i32 3}\n!4 = distinct !DISubprogram(name: \"f\", scope: !1, file: !1, unit: !0)\n!9 = !DILocation(line: 1, column: 1, scope: !4)\n",
        "takes metadata where it expects it",
    ),
    (
        "target datalayout = \"A16777216\"\n",
        "does not fit 24 bits",
    ),
    // Each operand of a signed pointer says something specific.
    (
        "@a = global ptr ptrauth (i32 42, i32 0)\n",
        "a ptrauth base pointer has to be a pointer",
    ),
    (
        "@var = external global i32\n@a = global ptr ptrauth (ptr @var, i32 2, ptr null)\n",
        "a ptrauth integer discriminator has to be an i64 constant",
    ),
    ("/* never closed\n", "unterminated comment"),
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
    // Every word the sweep found, including the two that name a pair of
    // bits and the one that is reserved.
    "!named = !{!0}\n!0 = !DIBasicType(name: \"n\", flags: DIFlagPublic | DIFlagVirtualInheritance | DIFlagReservedBit4)\n",
    // A word each table does have.
    "!named = !{!0}\n!0 = !GenericDINode(tag: DW_TAG_entry_point)\n",
    "!named = !{!0}\n!0 = !DIBasicType(name: \"n\", encoding: DW_ATE_signed)\n",
    // Three vocabularies the sweep cannot finish, because a value equal to
    // the field's own default never prints its word. Those refuse nothing.
    "!named = !{!0}\n!llvm.dbg.cu = !{!0}\n!0 = distinct !DICompileUnit(language: DW_LANG_C99, file: !1, nameTableKind: Default)\n!1 = !DIFile(filename: \"a\", directory: \"d\")\n!llvm.module.flags = !{!9}\n!9 = !{i32 2, !\"Debug Info Version\", i32 3}\n",
    "!named = !{!0}\n!0 = !DISubprogram(name: \"s\", scope: null, type: null, spFlags: 0, virtuality: DW_VIRTUALITY_none)\n",
    "!named = !{!0}\n!0 = !DIFile(filename: \"f\", directory: \"d\", checksumkind: CSK_MD5, checksum: \"0123456789abcdef0123456789abcdef\")\n",
    // Written with braces it is.
    "!named = !{!1}\n!0 = !{}\n!1 = !GenericDINode(tag: DW_TAG_entry_point, operands: {!0})\n",
    // The two casts that change nothing but precision take them.
    "define double @f(float %x) {\nentry:\n  %r = fpext nnan ninf float %x to double\n  ret double %r\n}\n",
    "define half @f(float %x) {\nentry:\n  %r = fptrunc reassoc float %x to half\n  ret half %r\n}\n",
    // The same three with the field they need.
    "!llvm.dbg.cu = !{!0}\n!llvm.module.flags = !{!1}\n\n!0 = distinct !DICompileUnit(language: DW_LANG_C99, file: !2, producer: \"p\", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug)\n!1 = !{i32 2, !\"Debug Info Version\", i32 3}\n!2 = !DIFile(filename: \"a\", directory: \"d\")\n",
    "!named = !{!0}\n!0 = !DIModule(scope: null, name: \"M\")\n",
    "!named = !{!0}\n!0 = !DIMacro(type: DW_MACINFO_define, name: \"m\", value: \"v\")\n",
    // A pointer to a function, in the older spelling, where a return type
    // and where an operand type. `ret void` still returns nothing: what
    // follows the word is what settles it.
    "define void ()* @f() {\nentry:\n  ret void ()* null\n}\n",
    "define void @g() {\nentry:\n  ret void\n}\n",
    // Two calling conventions the Bitcode tree uses.
    "declare hhvmcc void @a()\n\ndeclare hhvm_ccc void @b()\n",
    // `i8*` is the older spelling of `ptr`, which upstream folds as it reads.
    // Nothing downstream can tell the two apart, here or there.
    "define void @f(i8* %p) {\nentry:\n  ret void\n}\n",
    // A use-list order directive sits among the instructions and after the
    // globals, and says what order a value's uses were in. `llvm-dis` prints
    // none of them unless asked to, so reading one and keeping nothing is
    // what reproduces upstream's output.
    // This was three directives upstream refuses, in an "upstream accepts
    // this" table, and nothing had asked it: `uselistorder` may not sit
    // among the instructions, a value with no uses cannot be permuted, and
    // the entry block has no uses because nothing branches to it. What is
    // left is the two `llvm-as` really takes.
    "@a = global i32 0\n@b = alias i32, ptr @a\n@c = alias i32, ptr @a\n\ndefine i32 @f(i32 %x) {\nentry:\n  %p = add i32 %x, 1\n  %q = add i32 %x, 2\n  %r = add i32 %p, %q\n  ret i32 %r\n  uselistorder i32 %x, { 1, 0 }\n}\n\nuselistorder ptr @a, { 1, 0 }\n",
    // `extern_weak` is a linkage rather than the `external` keyword, and it
    // declares just as `external` does, so the next global is the next
    // global rather than this one's initializer.
    "@a = extern_weak global [0 x i8]\n@b = extern_weak global i32\n@c = global i32 7\n",
    // A wide hexadecimal literal is a constant like any other.
    "@g = global i64 u0x00001\n",
    // A named list may hold one of the two node kinds that are written at
    // every use rather than numbered.
    "!named = !{!DIExpression(DW_OP_constu, 48)}\n",
    // A metadata field may be an aggregate constant.
    "!named = !{!0}\n!0 = !DIDerivedType(tag: DW_TAG_member, name: \"v\", baseType: !1, size: 128, extraData: [4 x i32] [i32 23, i32 23, i32 97, i32 108])\n!1 = !DIBasicType(name: \"int\", size: 32, encoding: DW_ATE_signed)\n",
    // An array length written the way a wide integer literal is written.
    "define void @f() {\nentry:\n  %a = alloca [u0xedcba x i8]\n  ret void\n}\n",
    // A backslash that begins no escape keeps itself, as it does in a
    // metadata name: this string holds a backslash and a `t`.
    "@s = constant [7 x i8] c\"c:\\temp\"\n",
    // An intrinsic returning two tiles returns them in a struct, which is
    // the one aggregate `x86_amx` may sit in.
    "declare { x86_amx, x86_amx } @g(i16, i16)\n",
    // A label may hold a hyphen where a keyword may not: the same run of
    // bytes is one label here and a type and a negative number below.
    "define void @f(i1 %c) {\nentry:\n  br i1 %c, label %a-b, label %c-d\n\na-b:\n  ret void\n\nc-d:\n  ret void\n}\n",
    "define <4 x i16> @f(<4 x i16> %b) {\nentry:\n  %r = xor <4 x i16> %b, < i16 -1, i16 -1, i16 -1, i16-1 >\n  ret <4 x i16> %r\n}\n",
    // The eight calling conventions the CodeGen tree uses and the suites do
    // not, two of which upstream prints with a doubled space.
    "declare msp430_intrcc void @a()\n\ndeclare intel_ocl_bicc void @b()\n\ndeclare m68k_rtdcc void @c()\n\ndeclare avr_intrcc void @d()\n\ndeclare avr_signalcc void @e()\n\ndeclare aarch64_sme_preservemost_from_x0 void @g()\n\ndeclare aarch64_sme_preservemost_from_x1 void @h()\n\ndeclare aarch64_sme_preservemost_from_x2 void @i()\n",
    "define void @f() hybrid_patchable {\nentry:\n  ret void\n}\n",
    // An instruction written without a `%N =` still takes the next number,
    // and a phi above it may already have referred to that number. Reserving
    // a second slot rather than reusing the placeholder left the phi pointing
    // at an instruction that never arrived, which aborted rather than failing.
    "define void @f(i32 %n) {\nentry:\n  br label %bb\n\nbb:\n  %j = phi i32 [ %0, %bb ], [ 0, %entry ]\n  add i32 %j, 1\n  icmp slt i32 %0, %n\n  br i1 %1, label %bb, label %out\n\nout:\n  ret void\n}\n",
    // A global's `align` with no comma before it is an attribute rather than
    // the alignment clause, which is how upstream tells the two apart.
    "@g = external global i32 align 4\n",
    // A function attachment may be written in place.
    "define void @f() !prof !{!\"function_entry_count\", i64 0} {\nentry:\n  ret void\n}\n",
    // A NaN narrows to a float when the payload bits that fall off are zero.
    "define float @f() {\nentry:\n  ret float 0x7FF1000000000000\n}\n",
    // A phi may carry an attachment, and the comma before it looks exactly
    // like the comma before another edge.
    "define void @f(i1 %c) {\nentry:\n  br i1 %c, label %a, label %b\n\na:\n  br label %b\n\nb:\n  %p = phi i32 [ 0, %entry ], [ 1, %a ], !dbg !0\n  ret void\n}\n\n!0 = !DILocation(line: 1, column: 1, scope: !1)\n!1 = distinct !DISubprogram(name: \"f\", scope: !2, file: !2, line: 1, type: !3, spFlags: DISPFlagDefinition, unit: !4)\n!2 = !DIFile(filename: \"a.c\", directory: \"/\")\n!3 = !DISubroutineType(types: !5)\n!4 = distinct !DICompileUnit(language: DW_LANG_C99, file: !2, producer: \"p\", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug)\n!5 = !{null}\n",
    // The slot a call may set, which the target keeps in a register.
    "define void @f() {\nentry:\n  %a = alloca swifterror ptr, align 8\n  ret void\n}\n",
    // A global's attachment may be a node written in place.
    "@g = global i32 0, align 4, !type !{i64 0, !\"a\"}, !type !{i64 4, !\"b\"}\n",
    // Past its width an integer literal truncates rather than being refused,
    // which is what upstream does with both of these.
    "define void @f(ptr %p) {\nentry:\n  store i16 65536, ptr %p, align 2\n  store i1 2, ptr %p, align 1\n  ret void\n}\n",
    // The two hexadecimal forms for an integer too wide to write in decimal.
    "define void @f(ptr %p) {\nentry:\n  store i64 u0x1122334455667788, ptr %p, align 8\n  store i64 s0x10, ptr %p, align 8\n  ret void\n}\n",
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
    // A call that writes the space it goes through, and one that goes
    // through the program's own.
    "define i8 @f(ptr addrspace(42) %p) {\nentry:\n  %r = call addrspace(42) i8 %p(i32 0)\n  ret i8 %r\n}\n",
    "define i8 @f(ptr %p) {\nentry:\n  %r = call i8 %p(i32 0)\n  ret i8 %r\n}\n",
    // Crossing address spaces is what addrspacecast is for, and a musttail
    // call that hands its variable arguments over.
    "@i = global i32 0\n@ia = alias ptr, addrspacecast (ptr @i to ptr addrspace(3))\n",
    "declare ptr @f(ptr, ...)\n\ndefine ptr @g(ptr %this, ...) {\n  %rv = musttail call ptr (ptr, ...) @f(ptr %this, ...)\n  ret ptr %rv\n}\n",
    "declare ptr @f(ptr)\n\ndefine ptr @g(ptr %this) {\n  %rv = musttail call ptr @f(ptr %this)\n  ret ptr %rv\n}\n",
    // An opaque struct where only its name is needed, and the attributes
    // that do describe a result.
    "%t = type opaque\n\ndeclare %t @f()\n",
    "%t = type opaque\n\ndeclare void @f(ptr)\n",
    "define noalias ptr @f() {\nentry:\n  ret ptr null\n}\n",
    "define range(i8 0, 8) i8 @f() {\nentry:\n  ret i8 0\n}\n",
    "define void @f(i8 %a) mustprogress {\nentry:\n  ret void\n}\n",
    // `noext` says not to widen, which anything can be told.
    "declare void @f(ptr noext)\n",
    // The fields that do take a metadata string, which is a list rather than
    // a rule: corpus/md-string-fields.nu measured which.
    "!t = !{!1}\n!1 = !DIDerivedType(tag: DW_TAG_member, baseType: !9, extraData: !\"ok\")\n!llvm.module.flags = !{!0}\n\n!0 = !{i32 2, !\"Debug Info Version\", i32 3}\n!9 = !DIBasicType(name: \"int\", size: 32, encoding: DW_ATE_signed)\n",
    "!t = !{!1}\n!1 = !DITemplateValueParameter(name: \"V\", type: !9, value: !\"ok\")\n!llvm.module.flags = !{!0}\n\n!0 = !{i32 2, !\"Debug Info Version\", i32 3}\n!9 = !DIBasicType(name: \"int\", size: 32, encoding: DW_ATE_signed)\n",
    "!t = !{!1}\n!1 = !DIModule(scope: !\"ok\", name: \"M\")\n!llvm.module.flags = !{!0}\n\n!0 = !{i32 2, !\"Debug Info Version\", i32 3}\n!9 = !DIBasicType(name: \"int\", size: 32, encoding: DW_ATE_signed)\n",
    // A compile unit that is listed, and one a named list never reaches:
    // an attachment leading to a unit is not what the rule asks about.
    "!named = !{!1}\n!llvm.module.flags = !{!0}\n!llvm.dbg.cu = !{!1}\n\n!0 = !{i32 2, !\"Debug Info Version\", i32 3}\n!1 = distinct !DICompileUnit(language: DW_LANG_C99, file: !2, producer: \"p\", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug)\n!2 = !DIFile(filename: \"t.c\", directory: \"/\")\n",
    "!llvm.module.flags = !{!0}\n!llvm.dbg.cu = !{}\n\n!0 = !{i32 2, !\"Debug Info Version\", i32 3}\n!1 = distinct !DICompileUnit(language: DW_LANG_C99, file: !2, producer: \"p\", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug)\n!2 = !DIFile(filename: \"t.c\", directory: \"/\")\n",
    // The two intrinsics LangRef does declare variadically.
    "declare void @llvm.localescape(...)\n\ndefine void @f() {\nentry:\n  %a = alloca i32\n  call void (...) @llvm.localescape(ptr %a)\n  ret void\n}\n",
    "declare void @llvm.donothing()\n\ndefine void @f() {\nentry:\n  call void @llvm.donothing()\n  ret void\n}\n",
    // The two tokens that are not `none`: `poison` stands for any value, and
    // a landingpad on a statepoint invoke's unwind edge carries the same
    // token the normal edge does.
    "declare i32 @llvm.experimental.gc.result.i32(token)\n\ndefine i32 @f() gc \"statepoint-example\" {\nentry:\n  %r = call i32 @llvm.experimental.gc.result.i32(token poison)\n  ret i32 %r\n}\n",
    "declare void @h()\n\ndeclare i32 @P(...)\n\ndeclare token @llvm.experimental.gc.statepoint.p0(i64, i32, ptr, i32, i32, ...)\n\ndeclare ptr addrspace(1) @llvm.experimental.gc.relocate.p1(token, i32, i32)\n\ndefine void @f(ptr addrspace(1) %p) gc \"statepoint-example\" personality ptr @P {\nentry:\n  %t = invoke token (i64, i32, ptr, i32, i32, ...) @llvm.experimental.gc.statepoint.p0(i64 0, i32 0, ptr elementtype(void ()) @h, i32 0, i32 0, i32 0, i32 0) [ \"gc-live\"(ptr addrspace(1) %p) ]\n          to label %ok unwind label %bad\n\nok:\n  ret void\n\nbad:\n  %lp = landingpad token\n          cleanup\n  %r = call ptr addrspace(1) @llvm.experimental.gc.relocate.p1(token %lp, i32 0, i32 0)\n  ret void\n}\n",
    // A tile another call produced, and a relocation tied to the statepoint
    // that made the safepoint it asks about.
    "@buf = global [1024 x i8] zeroinitializer, align 16\n\ndeclare void @llvm.x86.tilestored64.internal(i16, i16, ptr, i64, x86_amx)\n\ndeclare x86_amx @llvm.x86.tilezero.internal(i16, i16)\n\ndefine void @f(i16 %r, i16 %c) {\nentry:\n  %t = call x86_amx @llvm.x86.tilezero.internal(i16 %r, i16 %c)\n  call void @llvm.x86.tilestored64.internal(i16 %r, i16 %c, ptr @buf, i64 64, x86_amx %t)\n  ret void\n}\n",
    "declare void @h()\n\ndeclare token @llvm.experimental.gc.statepoint.p0(i64, i32, ptr, i32, i32, ...)\n\ndeclare ptr addrspace(1) @llvm.experimental.gc.relocate.p1(token, i32, i32)\n\ndefine i32 @f(ptr addrspace(1) %p) gc \"statepoint-example\" {\nentry:\n  %t = call token (i64, i32, ptr, i32, i32, ...) @llvm.experimental.gc.statepoint.p0(i64 0, i32 0, ptr elementtype(void ()) @h, i32 0, i32 0, i32 0, i32 0) [ \"gc-live\"(ptr addrspace(1) %p) ]\n  %r = call ptr addrspace(1) @llvm.experimental.gc.relocate.p1(token %t, i32 0, i32 0)\n  ret i32 0\n}\n",
    // A well-formed variant name in each of its two isa spellings, and the
    // linear parameter forms: a constant stride, a negative one, and one
    // held in another parameter.
    "declare i64 @foo(i64) #0\n\ndefine void @f() {\nentry:\n  ret void\n}\n\nattributes #0 = { \"vector-function-abi-variant\"=\"_ZGVnN2v_foo(vf)\" }\n",
    "declare i64 @foo(i64) #0\n\ndefine void @f() {\nentry:\n  ret void\n}\n\nattributes #0 = { \"vector-function-abi-variant\"=\"_ZGV_LLVM_M4v_foo(vf)\" }\n",
    "declare i64 @foo(i64) #0\n\ndefine void @f() {\nentry:\n  ret void\n}\n\nattributes #0 = { \"vector-function-abi-variant\"=\"_ZGVnN2l2a8_foo(vf)\" }\n",
    "declare i64 @foo(i64, i64) #0\n\ndefine void @f() {\nentry:\n  ret void\n}\n\nattributes #0 = { \"vector-function-abi-variant\"=\"_ZGVnN2ln2v_foo(vf)\" }\n",
    "declare i64 @foo(i64, i64) #0\n\ndefine void @f() {\nentry:\n  ret void\n}\n\nattributes #0 = { \"vector-function-abi-variant\"=\"_ZGVnN2ls0v_foo(vf)\" }\n",
    // A tbaa chain that reaches a root, a struct base with a scalar access,
    // and a base chain that goes through a struct: only the access chain is
    // scalars all the way up.
    "define void @f(ptr %p) {\nentry:\n  store i32 42, ptr %p, align 4, !tbaa !0\n  ret void\n}\n\n!0 = !{!1, !1, i64 0}\n!1 = !{!\"a\", !2, i64 0}\n!2 = !{!\"b\", !3, i64 0}\n!3 = !{!\"root\"}\n",
    "define void @f(ptr %p) {\nentry:\n  store i32 42, ptr %p, align 4, !tbaa !0\n  ret void\n}\n\n!0 = !{!1, !2, i64 0}\n!1 = !{!\"s\", !2, i64 0, !2, i64 4}\n!2 = !{!\"n\", !3, i64 0}\n!3 = !{!\"root\"}\n",
    "define void @f(ptr %p) {\nentry:\n  store i32 42, ptr %p, align 4, !tbaa !0\n  ret void\n}\n\n!0 = !{!1, !3, i64 0}\n!1 = !{!\"outer\", !2, i64 0}\n!2 = !{!\"inner\", !3, i64 0, !3, i64 4}\n!3 = !{!\"n\", !4, i64 0}\n!4 = !{!\"root\"}\n",
    // Both pauthabi flags together, and neither.
    "!llvm.module.flags = !{!0, !1}\n\n!0 = !{i32 1, !\"aarch64-elf-pauthabi-platform\", i32 2}\n!1 = !{i32 1, !\"aarch64-elf-pauthabi-version\", i32 3}\n",
    "!llvm.module.flags = !{!0}\n\n!0 = !{i32 1, !\"something-else\", i32 2}\n",
    // The assume tags that are attribute names, the two that are not, and the
    // same unknown tag on a call that is not an assumption at all.
    "declare void @llvm.assume(i1)\n\ndefine void @f(ptr %p, i32 %n) {\nentry:\n  call void @llvm.assume(i1 true) [\"dereferenceable\"(ptr %p, i32 %n)]\n  ret void\n}\n",
    "declare void @llvm.assume(i1)\n\ndefine void @f(ptr %p) {\nentry:\n  call void @llvm.assume(i1 true) [\"nonnull\"(ptr %p)]\n  ret void\n}\n",
    "declare void @llvm.assume(i1)\n\ndefine void @f(ptr %p) {\nentry:\n  call void @llvm.assume(i1 true) [\"align\"(ptr %p, i32 8, i32 4)]\n  ret void\n}\n",
    "declare void @llvm.assume(i1)\n\ndefine void @f(ptr %p) {\nentry:\n  call void @llvm.assume(i1 true) [\"ignore\"()]\n  ret void\n}\n",
    "declare void @llvm.assume(i1)\n\ndefine void @f(ptr %p) {\nentry:\n  call void @llvm.assume(i1 true) [\"separate_storage\"(ptr %p, ptr %p)]\n  ret void\n}\n",
    "declare void @g()\n\ndefine void @f() {\nentry:\n  call void @g() [\"adazdazd\"()]\n  ret void\n}\n",
    // The alloca that was pushed where the callee looks for it, one of a
    // type that does not match (which is not what the rule is about), and a
    // value that is not an alloca at all.
    "declare void @g(ptr inalloca(i64))\n\ndefine void @f() {\nentry:\n  %a = alloca inalloca i64\n  call void @g(ptr inalloca(i64) %a)\n  ret void\n}\n",
    "declare void @g(ptr inalloca(i64))\n\ndefine void @f() {\nentry:\n  %a = alloca inalloca [2 x i32]\n  call void @g(ptr inalloca(i64) %a)\n  ret void\n}\n",
    "declare void @g(ptr inalloca(i64))\n\ndefine void @f(ptr %p) {\nentry:\n  call void @g(ptr inalloca(i64) %p)\n  ret void\n}\n",
    // A gep on an array of such structs, and on a scalable vector itself:
    // the rule is about a struct with no fixed field offsets, not about
    // scalable vectors as such.
    "%s = type { <vscale x 1 x double> }\n\ndefine void @f(ptr %a) {\nentry:\n  %p = getelementptr [2 x %s], ptr %a, i32 0\n  ret void\n}\n",
    "define void @f(ptr %a) {\nentry:\n  %p = getelementptr <vscale x 1 x double>, ptr %a, i32 0\n  ret void\n}\n",
    // A call site repeating a promise its callee makes, written both ways,
    // and a callee that makes it being called without it.
    "declare i32 @g() speculatable\n\ndefine i32 @f() {\nentry:\n  %r = call i32 @g() speculatable\n  ret i32 %r\n}\n",
    "declare i32 @g() #0\n\ndefine i32 @f() {\nentry:\n  %r = call i32 @g() #0\n  ret i32 %r\n}\n\nattributes #0 = { speculatable }\n",
    "declare i32 @g() speculatable\n\ndefine i32 @f() {\nentry:\n  %r = call i32 @g()\n  ret i32 %r\n}\n",
    // A vector wanting exactly the largest alignment there is, the same type
    // in a signature nothing calls, and one crossing a call that is lowered
    // rather than placed.
    "declare void @g(<2147483648 x i16>)\n\ndefine void @f(<2147483648 x i16> %v) {\nentry:\n  call void @g(<2147483648 x i16> %v)\n  ret void\n}\n",
    "define void @f(<2147483649 x i16> %v) {\nentry:\n  ret void\n}\n",
    "declare <2147483649 x i16> @llvm.fshr.v2147483649i16(<2147483649 x i16>, <2147483649 x i16>, <2147483649 x i16>)\n\ndefine <2147483649 x i16> @f(<2147483649 x i16> %l, <2147483649 x i16> %r, <2147483649 x i16> %a) {\nentry:\n  %b = call <2147483649 x i16> @llvm.fshr.v2147483649i16(<2147483649 x i16> %l, <2147483649 x i16> %r, <2147483649 x i16> %a)\n  ret <2147483649 x i16> %b\n}\n",
    // The largest alignment each of the two caps allows.
    "define void @f(ptr align 4294967296 %p) {\nentry:\n  ret void\n}\n",
    "define void @f() alignstack(2147483648) {\nentry:\n  ret void\n}\n",
    // The interrupt frame passed the way the processor left it, a handler
    // taking no frame at all, and the same first parameter under a convention
    // that says nothing about it.
    "define x86_intrcc void @f(ptr byval(i32) %p) {\nentry:\n  ret void\n}\n",
    "define x86_intrcc void @f() {\nentry:\n  ret void\n}\n",
    "define x86_intrcc void @f(ptr byval(i32) %p, i64 %e) {\nentry:\n  ret void\n}\n",
    "define ccc void @f(i32 %p) {\nentry:\n  ret void\n}\n",
    // One answer per register, and one from each of the two groups at once.
    "declare void @f() \"aarch64_pstate_sm_enabled\" \"aarch64_pstate_sm_body\"\n",
    "declare void @f() \"aarch64_in_za\" \"aarch64_in_zt0\"\n",
    "define void @f() {\nentry:\n  call void @g() \"aarch64_zt0_undef\"\n  ret void\n}\n\ndeclare void @g()\n",
    // A reserved global that nothing reads, and the two names next door that
    // are not reserved: the underscore spelling, and any other `llvm.*`.
    "@x = global i32 0\n@llvm.used = appending global [1 x ptr] [ptr @x], section \"llvm.metadata\"\n",
    "@x = global i32 0\n@llvm.compiler_used = appending global [1 x ptr] [ptr @x], section \"llvm.metadata\"\n@p = global ptr @llvm.compiler_used\n",
    "@x = global i32 0\n@llvm.foo = appending global [1 x ptr] [ptr @x]\n@p = global ptr @llvm.foo\n",
    // The same three in the shapes they are meant to have.
    "!llvm.module.flags = !{!0}\n\n!0 = !{i32 1, !\"SemanticInterposition\", i32 1}\n",
    "declare void @a()\n\ndeclare void @b()\n\n!llvm.module.flags = !{!0}\n\n!0 = !{i32 5, !\"CG Profile\", !1}\n!1 = !{!2}\n!2 = !{ptr @a, ptr @b, i64 5}\n",
    "define void @f(ptr %p) {\nentry:\n  %v = load i32, ptr %p, !tbaa !0\n  ret void\n}\n\n!0 = !{!1, !1, i64 0}\n!1 = !{!\"x\", !2}\n!2 = !{!\"root\"}\n",
    // The same attributes on a parameter the callee did declare, variadic
    // signature or not.
    "declare void @g(ptr sret(i32), ...)\n\ndefine void @f(ptr %p) {\nentry:\n  call void (ptr, ...) @g(ptr sret(i32) %p)\n  ret void\n}\n",
    "declare void @g(ptr)\n\ndefine void @f(ptr %p) {\nentry:\n  call void @g(ptr sret(i32) %p)\n  ret void\n}\n",
    // A module writes its debug info as records or as intrinsic calls, and
    // this one keeps to records.
    "define void @f(i32 %a) !dbg !5 {\nentry:\n    #dbg_value(!DIArgList(i32 %a, i64 0), !4, !DIExpression(), !8)\n  ret void\n}\n\n!llvm.module.flags = !{!0}\n!llvm.dbg.cu = !{!1}\n\n!0 = !{i32 2, !\"Debug Info Version\", i32 3}\n!1 = distinct !DICompileUnit(language: DW_LANG_C99, file: !2, producer: \"p\", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug)\n!2 = !DIFile(filename: \"a.c\", directory: \"/\")\n!4 = !DILocalVariable(name: \"v\", scope: !5, file: !2, line: 1)\n!5 = distinct !DISubprogram(name: \"f\", scope: !2, file: !2, line: 1, type: !6, spFlags: DISPFlagDefinition, unit: !1)\n!6 = !DISubroutineType(types: !7)\n!7 = !{null}\n!8 = !DILocation(line: 1, column: 1, scope: !5)\n",
    // A naked function may take arguments; what it may not do is read them.
    "define void @f(ptr %p) naked {\nentry:\n  unreachable\n}\n",
    // An intrinsic LangRef documents needs no declaration: the call says what
    // its signature is. An alias to a thread-local is thread-local, which is
    // what upstream's own threadlocal-pass.ll leans on.
    "@v = thread_local global i32 0\n@a = thread_local alias i32, ptr @v\n\ndefine void @f(<4 x i32> %x, <4 x i32> %y, <8 x float> %s, <8 x i1> %m, i32 %n) {\nentry:\n  %p = call ptr @llvm.threadlocal.address(ptr @a)\n  %c = call <4 x i32> @llvm.scmp.v4i32.v4i32(<4 x i32> %x, <4 x i32> %y)\n  %v = call <8 x i32> @llvm.vp.fptosi.v8i32.v8f32(<8 x float> %s, <8 x i1> %m, i32 %n)\n  ret void\n}\n",
    // One scope is what the declaration of a scope declares, and a tail
    // convention hands the frame over without anything in a register.
    "declare void @llvm.experimental.noalias.scope.decl(metadata)\n\ndeclare tailcc void @g(i32)\n\ndefine tailcc void @f(i32 %x) {\nentry:\n  call void @llvm.experimental.noalias.scope.decl(metadata !0)\n  musttail call tailcc void @g(i32 %x)\n  ret void\n}\n\n!0 = !{!1}\n!1 = !{!1}\n",
    // 128 bits is a size an atomic comes in, and so is a vector of floats
    // whose lanes multiply out to one.
    "define void @f(ptr %p) {\nentry:\n  %v = load atomic i128, ptr %p monotonic, align 16\n  %r = atomicrmw fadd ptr %p, <2 x half> undef seq_cst, align 4\n  ret void\n}\n",
    // The last lane of the second vector is in range, and a lane that does
    // not matter is written rather than indexed.
    "define void @f() {\nentry:\n  %x = shufflevector <2 x i32> undef, <2 x i32> undef, <2 x i32> <i32 0, i32 3>\n  %y = shufflevector <2 x i32> undef, <2 x i32> undef, <2 x i32> <i32 0, i32 poison>\n  ret void\n}\n",
    // Two edges from one block need two entries, and they agree because both
    // arrive at once.
    "define void @f(i1 %c) {\nentry:\n  br i1 %c, label %b, label %b\n\nb:\n  %p = phi i32 [ 0, %entry ], [ 0, %entry ]\n  ret void\n}\n",
    // The shape the funclet rules are the negative of: an invoke unwinding to
    // a catchswitch, whose handler opens with a catchpad.
    "declare void @g()\n\ndefine void @f() personality ptr null {\nentry:\n  invoke void @g() to label %n unwind label %s\n\nn:\n  ret void\n\ns:\n  %c = catchswitch within none [ label %h ] unwind to caller\n\nh:\n  %p = catchpad within %c []\n  catchret from %p to label %n\n}\n",
    // A blockaddress may name a label in a function the module has not read
    // yet, so the check has to wait for the whole module.
    "@b = global ptr blockaddress(@f, %b)\n\ndefine void @f() {\nentry:\n  ret void\n\nb:\n  ret void\n}\n",
    // The shape those three rules are the negative of: an invoke's unwind
    // destination, a landingpad in it, and a personality to read the result.
    "declare void @g()\n\ndefine void @f() personality ptr null {\nentry:\n  invoke void @g() to label %n unwind label %u\n\nn:\n  ret void\n\nu:\n  %p = landingpad { ptr, i32 } cleanup\n  resume { ptr, i32 } %p\n}\n",
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
    // Private linkage in a comdat is a COFF rule, not an IR one: upstream
    // reports it only for a Windows triple and llvm-as reads this.
    "$v = comdat any\n@v = private global i32 0, comdat($v)\n",
    // Dominance says nothing about a block the entry cannot reach.
    "define void @f() {\nentry:\n  ret void\n\ndead:\n  %x = add i32 %x, 1\n  br label %dead\n}\n",
    // A declaration needs no size, which upstream's own
    // 2009-02-28-StripOpaqueName.ll relies on.
    "%A = type opaque\n@g1 = external global %A\n@g2 = global ptr @g1\n",
    // A struct may hold scalable vectors when it holds nothing else, which
    // an alloca of one is the way to say.
    "%t = type { <vscale x 1 x i32>, <vscale x 1 x i32> }\n\ndefine void @f() {\nentry:\n  %a = alloca %t, align 8\n  ret void\n}\n",
    // A null among a composite type's elements is an error where the node
    // is reached and not where it is not, which is what set1.ll leans on.
    "!named = !{!2}\n!0 = !{null}\n!1 = !DICompositeType(tag: DW_TAG_class_type, name: \"C\", size: 64, elements: !0)\n!2 = !{}\n",
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
    // Text that is not UTF-8. A debug-info path is bytes, and both the
    // string and the name it sits in have to survive being read.
    "!named = !{!0}\n!0 = !DIFile(filename: \"\\00\\01\\02\\80\\81\\82\\FD\\FE\\FF\", directory: \"/dir\")\n",
    "!\\FFfoo = !{!0}\n!0 = !{}\n",
    "!named = !{!0}\n!0 = !{!\"\\FF\"}\n",
    // A vector written as one value, a signed pointer, a comment in the
    // other spelling, a brace-delimited operand list, and an enumerator
    // wider than 128 bits.
    "@g = constant <5 x i32> splat (i32 7)\n",
    "@var = external global i32\n@disc = external global i32\n@a = global ptr ptrauth (ptr @var, i32 2, i64 1234, ptr @disc)\n",
    "/* a comment */\n@g = external global i32\n",
    "!named = !{!1}\n!0 = !{}\n!1 = !GenericDINode(tag: 3, header: \"h\", operands: {!0, !0})\n",
    "!named = !{!0}\n!0 = !DIEnumerator(name: \"D\", value: 2722258935367507707706996859454145691648, isUnsigned: true)\n",
    // Whether a target extension type can be a global is the target's
    // business, and upstream reads this one.
    "@g = global target(\"spirv.DeviceEvent\") zeroinitializer\n",
    // The address space follows the alignment, which is where the grammar
    // puts it.
    "define void @f() {\nentry:\n  %y = alloca i32, align 4, addrspace(3)\n  ret void\n}\n",
    // A call may write the address space it goes through, and then that is
    // what has to match rather than the program's.
    "target datalayout = \"P42\"\n\ndefine i8 @f(ptr %p0) {\nentry:\n  %r = call addrspace(0) i8 %p0(i32 0)\n  ret i8 %r\n}\n",
    // A load acquires and a store releases, which are the directions each
    // can order in.
    "define void @f(ptr %p) {\nentry:\n  %r = load atomic i32, ptr %p acquire, align 4\n  store atomic i32 0, ptr %p release, align 4\n  ret void\n}\n",
    // An unnamed block takes a slot from the same counter unnamed values
    // use, and `%N` names it by that number whether the reference comes
    // before the block or after it.
    "define i32 @a() {\n  br label %BB1\n\nBB1:\n  %r = phi i32 [ 1, %0 ]\n  ret i32 %r\n}\n",
    "define i32 @f() {\n  br label %10\n\n10:\n  br label %11\n\n  ret i32 0\n}\n",
    // A pointer to a type does not make the type contain itself, which is
    // what makes a linked list legal.
    "%node = type { i32, ptr }\n@head = global %node zeroinitializer\n",
    // The floating-point atomics are the ones a target does lane by lane.
    "define void @f(ptr %p, <2 x half> %v) {\nentry:\n  %r = atomicrmw fadd ptr %p, <2 x half> %v seq_cst, align 4\n  ret void\n}\n",
    // An elementtype says what the pointer reaches through.
    "declare i64 @llvm.aarch64.ldxr.p0(ptr)\n\ndefine void @f(ptr %p) {\nentry:\n  %r = call i64 @llvm.aarch64.ldxr.p0(ptr elementtype(i64) %p)\n  ret void\n}\n",
    // The older spelling of an intrinsic has fewer arguments than LangRef
    // documents, and upstream upgrades it rather than refusing it.
    "declare i64 @llvm.objectsize.i64.p0(ptr, i1)\n\ndefine void @f(ptr %p) {\nentry:\n  %r = call i64 @llvm.objectsize.i64.p0(ptr %p, i1 false)\n  ret void\n}\n",
    // Two conventions we had never heard of.
    "define amdgpu_ps float @f(i32 %x) {\nentry:\n  ret float 0.000000e+00\n}\n",
    "define riscv_vls_cc(32) void @g() {\nentry:\n  ret void\n}\n",
    // A declaration whose tied positions disagree and that nothing calls is
    // a module upstream reads, the verifier never reaching a declaration
    // nobody uses. Only the call is refused.
    "declare i8 @llvm.umax.i8(i8, i16)\n",
    // The other direction of the local count, at each way a local can be
    // named: a parameter, a numbered parameter, an unnamed instruction.
    "define void @f(i32 %x) {\nentry:\n  %a = add i32 %x, %x\n  %b = add i32 %x, %a\n  ret void\n  uselistorder i32 %x, { 2, 1, 0 }\n}\n",
    "define void @f(i32) {\nentry:\n  %a = add i32 %0, %0\n  %b = add i32 %0, %a\n  ret void\n  uselistorder i32 %0, { 2, 1, 0 }\n}\n",
    "define void @f(i32 %x) {\nentry:\n  %1 = add i32 %x, %x\n  %2 = add i32 %1, %1\n  ret void\n  uselistorder i32 %1, { 1, 0 }\n}\n",
    // The other direction of the placement rule: a second directive after
    // the first is one, so the run may be any length.
    "define void @f(i32 %x) {\nentry:\n  %a = add i32 %x, %x\n  %b = add i32 %x, %a\n  ret void\n  uselistorder i32 %x, { 2, 1, 0 }\n  uselistorder i32 %x, { 1, 2, 0 }\n}\n",
    // The other direction of the use-list type rule: naming the type the
    // value really has is fine, for a parameter and for a symbol.
    "define void @f(i32 %x, i32 %y) {\nentry:\n  %a = add i32 %x, %y\n  %b = add i32 %x, %a\n  ret void\n  uselistorder i32 %x, { 1, 0 }\n}\n",
    "@g = global i32 0\n@a1 = alias i32, ptr @g\n@a2 = alias i32, ptr @g\nuselistorder ptr @g, { 1, 0 }\n",
    // A name is reduced by dropping mangling-shaped components only.
    // `llvm.vp.cttz.elts` counts into an `i32` where `llvm.vp.cttz` returns
    // its operand's type, and reading the first as the second refused this,
    // which is what a CodeGen file caught.
    "define void @t(<vscale x 16 x i1> %m, <vscale x 16 x i1> %k, i32 %n) {\nentry:\n  %r = call i32 @llvm.vp.cttz.elts.i32.nxv16i1(<vscale x 16 x i1> %m, i1 false, <vscale x 16 x i1> %k, i32 %n)\n  ret void\n}\n",
    // The other direction of the tied-position rule. Agreeing is fine at
    // any type, including a vector, and a position that is fixed rather
    // than tied is not asked to agree with anything: `llvm.ctlz` returns
    // what its first argument is and takes an `i1` second whatever that is.
    "define void @t() {\nentry:\n  %r = call i8 @llvm.umax(i8 0, i8 1)\n  ret void\n}\n",
    "define void @t() {\nentry:\n  %r = call <4 x i32> @llvm.umax(<4 x i32> zeroinitializer, <4 x i32> zeroinitializer)\n  ret void\n}\n",
    "define void @t() {\nentry:\n  %r = call i8 @llvm.ctlz(i8 0, i1 true)\n  ret void\n}\n",
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

/// Bytes that are not UTF-8 survive a round trip rather than being mangled
/// or refused, which is the whole of what B6 was about.
#[test]
fn text_outside_utf8_round_trips() {
    let text = "!named = !{!0}\n!0 = !DIFile(filename: \"\\00\\80\\FF\", directory: \"/dir\")\n";
    let module = llvm_ir_parse::parse_module(text).expect("upstream accepts this");
    let printed = llvm_ir_print::print_module(&module);
    assert!(
        printed.contains("filename: \"\\00\\80\\FF\""),
        "the bytes were mangled: {printed}"
    );
    let again = llvm_ir_parse::parse_module(&printed).expect("our own output parses");
    assert_eq!(llvm_ir_print::print_module(&again), printed);
}

/// The use count agrees with the number upstream reports.
///
/// A `uselistorder` directive whose index list is the wrong length makes
/// `llvm-as` say "wrong number of indexes, expected N", which names the
/// count it took. Every expectation below is that N, read off the oracle
/// rather than reasoned about, and the two that have no N are the messages
/// upstream uses for nought and one.
#[test]
fn a_constants_use_count_is_the_number_upstream_expects() {
    // (module, name of the global, uses upstream counted)
    let cases: &[(&str, usize)] = &[
        // Three aliases, one aliasee slot each.
        (
            "@g = global i32 0\n@a1 = alias i32, ptr @g\n@a2 = alias i32, ptr @g\n@a3 = alias i32, ptr @g\n",
            3,
        ),
        // One instruction reading it twice is two uses, not one.
        (
            "@g = global i32 0\ndefine void @f() {\nentry:\n  %r = icmp eq ptr @g, @g\n  ret void\n}\n",
            2,
        ),
        // Two distinct expressions name it, and a third global names it
        // directly.
        (
            "@g = global i32 0\n@p = global ptr getelementptr (i32, ptr @g, i64 1)\n@q = global ptr getelementptr (i32, ptr @g, i64 2)\n@r = global ptr @g\n",
            3,
        ),
        // Constants are interned, so the same expression written into two
        // globals is one constant naming it once.
        (
            "@g = global i32 0\n@p = global ptr getelementptr (i32, ptr @g, i64 1)\n@q = global ptr getelementptr (i32, ptr @g, i64 1)\n",
            1,
        ),
        ("@g = global i32 0\n@p = global ptr @g\n", 1),
        ("@g = global i32 0\n", 0),
    ];
    for (text, expected) in cases {
        let module = llvm_ir_parse::parse_module(text).expect("upstream accepts this");
        let target = (0..module.ctx.constant_count())
            .map(|index| llvm_ir::constant::ConstId(index as u32))
            .find(|id| match module.ctx.constant(*id) {
                llvm_ir::constant::Constant::Global { target, .. } => {
                    module.global_name(*target) == &llvm_ir::value::Name::Named("g".to_string())
                }
                _ => false,
            });
        let counted = match target {
            Some(id) => module.use_count(id),
            // Nothing names it, so there is no reference constant to count.
            None => 0,
        };
        assert_eq!(counted, *expected, "for:\n{text}");
    }
}
