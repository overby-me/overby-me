//! The builder has to produce what the parser would have.
//!
//! Building a module programmatically and printing it is easy to get
//! self-consistently wrong: a printer and a builder that share a wrong idea of
//! what an instruction looks like will agree with each other forever. This
//! test pins the built module against text written by hand, and then parses
//! that text back and requires the two modules to print identically. The
//! parser is the third opinion that makes the agreement mean something.

use llvm_ir::Module;
use llvm_ir::builder::Builder;
use llvm_ir::instruction::{BinOp, IntFlags, IntPredicate};
use llvm_ir::value::GlobalRef;
use llvm_support::{DataLayout, Triple};

const X86_64_LINUX: &str =
    "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128";

/// A loop that calls a declared function and accumulates its result: enough
/// shape to exercise globals, allocas, a phi with a back edge, a call and a
/// conditional branch.
fn build() -> Module {
    let mut module = Module::new();
    module.module_id = Some("builder-smoke".to_string());
    module.source_filename = Some("builder-smoke".to_string());
    module.data_layout = Some(DataLayout::parse(X86_64_LINUX).unwrap());
    module.triple = Some(Triple::parse("x86_64-unknown-linux-gnu"));

    let i32 = module.ctx.int_type(32);
    let pointer = module.ctx.pointer_type(0);

    let (message_type, message) = module.const_string(b"hello, world\n\0".to_vec());
    let message_global = module.add_private_constant("message", message_type, message);

    let puts = module.declare_function("puts", i32, vec![pointer], false);
    let main = module.declare_function("main", i32, Vec::new(), false);

    let mut builder = Builder::new(&mut module, main);
    let entry = builder.append_block(Some("entry"));
    let total = builder.alloca(i32);
    builder.name(total, "total");
    let zero = builder.int(32, 0);
    builder.store(zero, total);

    let header = builder.append_block(Some("header"));
    builder.position_at_end(entry);
    builder.br(header);

    builder.position_at_end(header);
    let counter = builder.phi(i32, vec![(zero, entry)]);
    builder.name(counter, "i");
    let limit = builder.int(32, 3);
    let done = builder.icmp(IntPredicate::Sge, counter, limit);
    builder.name(done, "done");

    let body = builder.append_block(Some("body"));
    let exit = builder.append_block(Some("exit"));
    builder.position_at_end(header);
    builder.cond_br(done, exit, body);

    builder.position_at_end(body);
    let text = builder.global_ref(GlobalRef::Variable(message_global));
    let printed = builder.call(puts, vec![text]);
    let loaded = builder.load(i32, total);
    builder.name(loaded, "loaded");
    let sum = builder.binary(BinOp::Add, loaded, printed);
    builder.name(sum, "sum");
    builder.store(sum, total);
    let one = builder.int(32, 1);
    let next = builder.binary_with_flags(
        BinOp::Add,
        IntFlags {
            nuw: true,
            nsw: true,
            ..IntFlags::default()
        },
        counter,
        one,
    );
    builder.name(next, "next");
    builder.add_incoming(counter, next, body);
    builder.br(header);

    builder.position_at_end(exit);
    let result = builder.load(i32, total);
    builder.name(result, "result");
    builder.ret(Some(result));

    module
}

/// What upstream would print for the module `build` describes, written by
/// hand rather than captured from our own output.
///
/// This exact text was fed to real `llvm-as`, which accepted it, and to
/// `llvm-dis`, which printed it back unchanged. So it is upstream's spelling
/// of this module and not merely ours, which is what makes comparing against
/// it worth doing.
const EXPECTED: &str = r#"; ModuleID = 'builder-smoke'
source_filename = "builder-smoke"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@message = private unnamed_addr constant [14 x i8] c"hello, world\0A\00", align 1

declare i32 @puts(ptr)

define i32 @main() {
entry:
  %total = alloca i32, align 4
  store i32 0, ptr %total, align 4
  br label %header

header:                                           ; preds = %body, %entry
  %i = phi i32 [ 0, %entry ], [ %next, %body ]
  %done = icmp sge i32 %i, 3
  br i1 %done, label %exit, label %body

body:                                             ; preds = %header
  %0 = call i32 @puts(ptr @message)
  %loaded = load i32, ptr %total, align 4
  %sum = add i32 %loaded, %0
  store i32 %sum, ptr %total, align 4
  %next = add nuw nsw i32 %i, 1
  br label %header

exit:                                             ; preds = %header
  %result = load i32, ptr %total, align 4
  ret i32 %result
}
"#;

#[test]
fn a_built_module_prints_what_was_expected() {
    let module = build();
    let printed = llvm_ir_print::print_module(&module);
    assert_eq!(printed, EXPECTED, "\n--- printed ---\n{printed}");
}

#[test]
fn a_built_module_verifies() {
    let module = build();
    let errors = llvm_ir::verify_module(&module);
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn a_built_module_agrees_with_the_parsed_one() {
    let built = llvm_ir_print::print_module(&build());
    let parsed = llvm_ir_parse::parse_module(&built).expect("our own output parses");
    let reprinted = llvm_ir_print::print_module(&parsed);
    assert_eq!(
        built, reprinted,
        "the builder and the parser disagree about the same module"
    );
    assert!(llvm_ir::verify_module(&parsed).is_empty());
}
