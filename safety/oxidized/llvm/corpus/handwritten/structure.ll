; ModuleID = 'structure.ll'
source_filename = "structure.ll"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

module asm "\09.text"
module asm "\09.globl handwritten_asm_symbol"

%pair = type { i32, i64 }

$group = comdat any

@plain = global i32 7, align 4
@zeroed = global [16 x i8] zeroinitializer, align 16
@constant_string = private unnamed_addr constant [6 x i8] c"hello\00", align 1
@in_comdat = global i32 0, comdat($group), align 4
@sectioned = global i64 1, section ".mysection", align 8
@aligned = global i128 0, align 16
@thread_local_var = thread_local global i32 0, align 4
@thread_local_model = thread_local(initialexec) global i32 0, align 4
@external_decl = external global i32, align 4
@weak_var = weak global i32 0, align 4
@internal_var = internal global i32 3, align 4
@addrspace_var = addrspace(1) global i32 0, align 4
@points_at = global ptr @plain, align 8
@offset_into = global ptr getelementptr inbounds ([6 x i8], ptr @constant_string, i64 0, i64 2), align 8

@the_alias = alias i32, ptr @plain
@weak_alias = weak alias i32, ptr @plain

@the_ifunc = ifunc i32 (), ptr @resolver

declare i32 @external_function(i32)

declare void @takes_many(ptr sret(%pair), ptr readonly captures(none), i64)

define i32 @uses_everything(i32 %argument) comdat($group) {
entry:
  %sum = add i32 %argument, 7
  %call = call i32 @external_function(i32 %sum)
  ret i32 %call
}

define internal void @with_section() section ".text.hot" align 16 {
entry:
  ret void
}

define void @with_gc() gc "statepoint-example" {
entry:
  ret void
}

define internal ptr @resolver() {
entry:
  ret ptr @external_function
}

!llvm.module.flags = !{!0, !1}
!llvm.ident = !{!2}
!named.metadata = !{!3, !4}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 2, !"RtLibUseGOT", i32 1}
!2 = !{!"handwritten corpus"}
!3 = !{!"a string operand", i32 42, ptr @plain, null}
!4 = distinct !{!3}
