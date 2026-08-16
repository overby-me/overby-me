; ModuleID = 'statics.ll'
source_filename = "statics.e1b1ba2e383dba82-cgu.0"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

%Entry = type { i32, [1 x i32], { ptr, i64 } }

@_ZN7statics5FIRST17h008275920c6982c8E = constant ptr @_ZN7statics5TABLE17hac24c39ab780fbccE, align 8
@alloc_63575c3e000160dea70bdb05edf17349 = private unnamed_addr constant [3 x i8] c"one", align 1
@alloc_babee5271e6e42dc125a96e0272fa27d = private unnamed_addr constant [3 x i8] c"two", align 1
@_ZN7statics5TABLE17hac24c39ab780fbccE = constant <{ [4 x i8], [4 x i8], ptr, [12 x i8], [4 x i8], ptr, [8 x i8] }> <{ [4 x i8] c"\01\00\00\00", [4 x i8] undef, ptr @alloc_63575c3e000160dea70bdb05edf17349, [12 x i8] c"\03\00\00\00\00\00\00\00\02\00\00\00", [4 x i8] undef, ptr @alloc_babee5271e6e42dc125a96e0272fa27d, [8 x i8] c"\03\00\00\00\00\00\00\00" }>, align 8
@_ZN7statics6PADDED17h1d8e319a7bbed617E = constant <{ [8 x i8], [56 x i8] }> <{ [8 x i8] zeroinitializer, [56 x i8] undef }>, align 64
@alloc_2ee90561176326de12bb1a252542a79f = private unnamed_addr constant [69 x i8] c"/home/overby.me/Work/overby.me/rust/llvm/corpus/rustc/src/statics.rs\00", align 1
@alloc_b3ffc2f6fdf4ebfa0a74b7af22c3cdd0 = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_2ee90561176326de12bb1a252542a79f, [16 x i8] c"D\00\00\00\00\00\00\00\22\00\00\00\05\00\00\00" }>, align 8
@_ZN7statics7COUNTER17h875a96599a23b902E = global [8 x i8] zeroinitializer, align 8

; Function Attrs: nounwind nonlazybind uwtable
define { ptr, i64 } @_ZN7statics10first_name17h4b5298bd005aa1e2E() unnamed_addr #0 {
start:
  %_2 = load ptr, ptr @_ZN7statics5FIRST17h008275920c6982c8E, align 8
  %0 = getelementptr inbounds i8, ptr %_2, i64 8
  %_0.0 = load ptr, ptr %0, align 8
  %1 = getelementptr inbounds i8, ptr %0, i64 8
  %_0.1 = load i64, ptr %1, align 8
  %2 = insertvalue { ptr, i64 } poison, ptr %_0.0, 0
  %3 = insertvalue { ptr, i64 } %2, i64 %_0.1, 1
  ret { ptr, i64 } %3
}

; Function Attrs: nounwind nonlazybind uwtable
define i32 @_ZN7statics6key_of17h0b9d975004ab6076E(i64 %index) unnamed_addr #0 {
start:
  %_3 = icmp ult i64 %index, 2
  br i1 %_3, label %bb1, label %panic

bb1:                                              ; preds = %start
  %0 = getelementptr inbounds nuw %Entry, ptr @_ZN7statics5TABLE17hac24c39ab780fbccE, i64 %index
  %_0 = load i32, ptr %0, align 8
  ret i32 %_0

panic:                                            ; preds = %start
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking18panic_bounds_check(i64 %index, i64 2, ptr align 8 @alloc_b3ffc2f6fdf4ebfa0a74b7af22c3cdd0) #2
  unreachable
}

; Function Attrs: cold minsize noinline noreturn nounwind nonlazybind optsize uwtable
declare void @_RNvNtCs4uthzyWeO2a_4core9panicking18panic_bounds_check(i64, i64, ptr align 8) unnamed_addr #1

attributes #0 = { nounwind nonlazybind uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #1 = { cold minsize noinline noreturn nounwind nonlazybind optsize uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #2 = { noinline noreturn nounwind }

!llvm.module.flags = !{!0, !1}
!llvm.ident = !{!2}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 2, !"RtLibUseGOT", i32 1}
!2 = !{!"rustc version 1.95.0 (59807616e 2026-04-14) (built from a source tarball)"}
