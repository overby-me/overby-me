; ModuleID = 'control.ll'
source_filename = "control.a1cea36279839f18-cgu.0"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@alloc_5e5b86aafaeba305aaf6dcfe470a71a7 = private unnamed_addr constant [69 x i8] c"/home/overby.me/Work/overby.me/rust/llvm/corpus/rustc/src/control.rs\00", align 1
@alloc_6530c31df4dff1e05ea9faca8080bb26 = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_5e5b86aafaeba305aaf6dcfe470a71a7, [16 x i8] c"D\00\00\00\00\00\00\00\1E\00\00\00\09\00\00\00" }>, align 8
@alloc_cb3db5271dfaee602367c526a28d9776 = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_5e5b86aafaeba305aaf6dcfe470a71a7, [16 x i8] c"D\00\00\00\00\00\00\00=\00\00\00\0C\00\00\00" }>, align 8
@alloc_086ab7ed283010f5bb6b38cfbc47ee17 = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_5e5b86aafaeba305aaf6dcfe470a71a7, [16 x i8] c"D\00\00\00\00\00\00\00@\00\00\00\09\00\00\00" }>, align 8
@alloc_eb178c3b330263de232661629059bdd5 = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_5e5b86aafaeba305aaf6dcfe470a71a7, [16 x i8] c"D\00\00\00\00\00\00\00\12\00\00\00\09\00\00\00" }>, align 8
@alloc_90259a2ae55d451f0da677bd23dc9d82 = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_5e5b86aafaeba305aaf6dcfe470a71a7, [16 x i8] c"D\00\00\00\00\00\00\00\10\00\00\00#\00\00\00" }>, align 8
@alloc_d7186a1fc8a35968f1077b6eb95040ad = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_5e5b86aafaeba305aaf6dcfe470a71a7, [16 x i8] c"D\00\00\00\00\00\00\00\10\00\00\00\14\00\00\00" }>, align 8

; Function Attrs: nounwind nonlazybind uwtable
define i32 @branch(i1 zeroext %c, i32 %x, i32 %y) unnamed_addr #0 {
start:
  %_0 = alloca [4 x i8], align 4
  br i1 %c, label %bb1, label %bb2

bb2:                                              ; preds = %start
  store i32 %y, ptr %_0, align 4
  br label %bb3

bb1:                                              ; preds = %start
  store i32 %x, ptr %_0, align 4
  br label %bb3

bb3:                                              ; preds = %bb1, %bb2
  %0 = load i32, ptr %_0, align 4
  ret i32 %0
}

; Function Attrs: nounwind nonlazybind uwtable
define i32 @counted_loop(i32 %n) unnamed_addr #0 {
start:
  %i = alloca [4 x i8], align 4
  %total = alloca [4 x i8], align 4
  store i32 0, ptr %total, align 4
  store i32 0, ptr %i, align 4
  br label %bb1

bb1:                                              ; preds = %bb4, %start
  %_5 = load i32, ptr %i, align 4
  %_4 = icmp ult i32 %_5, %n
  br i1 %_4, label %bb2, label %bb5

bb5:                                              ; preds = %bb1
  %_0 = load i32, ptr %total, align 4
  ret i32 %_0

bb2:                                              ; preds = %bb1
  %_7 = load i32, ptr %total, align 4
  %_8 = load i32, ptr %i, align 4
  %_0.i = add i32 %_7, %_8
  store i32 %_0.i, ptr %total, align 4
  %0 = load i32, ptr %i, align 4
  %_9.0 = add i32 %0, 1
  %_9.1 = icmp ult i32 %_9.0, %0
  br i1 %_9.1, label %panic, label %bb4

bb4:                                              ; preds = %bb2
  store i32 %_9.0, ptr %i, align 4
  br label %bb1

panic:                                            ; preds = %bb2
  call void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const24panic_const_add_overflow(ptr align 8 @alloc_6530c31df4dff1e05ea9faca8080bb26) #5
  unreachable
}

; Function Attrs: nounwind nonlazybind uwtable
define i32 @dense_match(i8 %x) unnamed_addr #0 {
start:
  %_0 = alloca [4 x i8], align 4
  switch i8 %x, label %bb1 [
    i8 0, label %bb6
    i8 1, label %bb5
    i8 2, label %bb4
    i8 3, label %bb3
    i8 4, label %bb2
  ]

bb1:                                              ; preds = %start
  store i32 0, ptr %_0, align 4
  br label %bb7

bb6:                                              ; preds = %start
  store i32 10, ptr %_0, align 4
  br label %bb7

bb5:                                              ; preds = %start
  store i32 20, ptr %_0, align 4
  br label %bb7

bb4:                                              ; preds = %start
  store i32 30, ptr %_0, align 4
  br label %bb7

bb3:                                              ; preds = %start
  store i32 40, ptr %_0, align 4
  br label %bb7

bb2:                                              ; preds = %start
  store i32 50, ptr %_0, align 4
  br label %bb7

bb7:                                              ; preds = %bb2, %bb3, %bb4, %bb5, %bb6, %bb1
  %0 = load i32, ptr %_0, align 4
  ret i32 %0
}

; Function Attrs: nounwind nonlazybind uwtable
define zeroext i1 @early_exit(ptr align 1 %xs.0, i64 %xs.1, i8 %needle) unnamed_addr #0 {
start:
  %i = alloca [8 x i8], align 8
  %_0 = alloca [1 x i8], align 1
  store i64 0, ptr %i, align 8
  br label %bb1

bb1:                                              ; preds = %bb6, %start
  %_5 = load i64, ptr %i, align 8
  %_4 = icmp ult i64 %_5, %xs.1
  br i1 %_4, label %bb2, label %bb7

bb7:                                              ; preds = %bb1
  store i8 0, ptr %_0, align 1
  br label %bb8

bb2:                                              ; preds = %bb1
  %_9 = load i64, ptr %i, align 8
  %_11 = icmp ult i64 %_9, %xs.1
  br i1 %_11, label %bb3, label %panic

bb8:                                              ; preds = %bb4, %bb7
  %0 = load i8, ptr %_0, align 1
  %1 = trunc nuw i8 %0 to i1
  ret i1 %1

bb3:                                              ; preds = %bb2
  %2 = getelementptr inbounds nuw i8, ptr %xs.0, i64 %_9
  %_8 = load i8, ptr %2, align 1
  %_7 = icmp eq i8 %_8, %needle
  br i1 %_7, label %bb4, label %bb5

panic:                                            ; preds = %bb2
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking18panic_bounds_check(i64 %_9, i64 %xs.1, ptr align 8 @alloc_cb3db5271dfaee602367c526a28d9776) #5
  unreachable

bb5:                                              ; preds = %bb3
  %3 = load i64, ptr %i, align 8
  %_12.0 = add i64 %3, 1
  %_12.1 = icmp ult i64 %_12.0, %3
  br i1 %_12.1, label %panic1, label %bb6

bb4:                                              ; preds = %bb3
  store i8 1, ptr %_0, align 1
  br label %bb8

bb6:                                              ; preds = %bb5
  store i64 %_12.0, ptr %i, align 8
  br label %bb1

panic1:                                           ; preds = %bb5
  call void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const24panic_const_add_overflow(ptr align 8 @alloc_086ab7ed283010f5bb6b38cfbc47ee17) #5
  unreachable
}

; Function Attrs: nounwind nonlazybind uwtable
define i32 @nested(i32 %a, i32 %b) unnamed_addr #0 {
start:
  %_0 = alloca [4 x i8], align 4
  %_3 = icmp sgt i32 %a, 0
  br i1 %_3, label %bb1, label %bb6

bb6:                                              ; preds = %start
  %_7 = icmp sgt i32 %b, 0
  br i1 %_7, label %bb7, label %bb9

bb1:                                              ; preds = %start
  %_4 = icmp sgt i32 %b, 0
  br i1 %_4, label %bb2, label %bb4

bb9:                                              ; preds = %bb6
  store i32 0, ptr %_0, align 4
  br label %bb10

bb7:                                              ; preds = %bb6
  %0 = call { i32, i1 } @llvm.ssub.with.overflow.i32(i32 %b, i32 %a)
  %_8.0 = extractvalue { i32, i1 } %0, 0
  %_8.1 = extractvalue { i32, i1 } %0, 1
  br i1 %_8.1, label %panic, label %bb8

bb10:                                             ; preds = %bb3, %bb5, %bb8, %bb9
  %1 = load i32, ptr %_0, align 4
  ret i32 %1

bb8:                                              ; preds = %bb7
  store i32 %_8.0, ptr %_0, align 4
  br label %bb10

panic:                                            ; preds = %bb7
  call void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const24panic_const_sub_overflow(ptr align 8 @alloc_eb178c3b330263de232661629059bdd5) #5
  unreachable

bb4:                                              ; preds = %bb1
  %2 = call { i32, i1 } @llvm.ssub.with.overflow.i32(i32 %a, i32 %b)
  %_6.0 = extractvalue { i32, i1 } %2, 0
  %_6.1 = extractvalue { i32, i1 } %2, 1
  br i1 %_6.1, label %panic1, label %bb5

bb2:                                              ; preds = %bb1
  %3 = call { i32, i1 } @llvm.sadd.with.overflow.i32(i32 %a, i32 %b)
  %_5.0 = extractvalue { i32, i1 } %3, 0
  %_5.1 = extractvalue { i32, i1 } %3, 1
  br i1 %_5.1, label %panic2, label %bb3

bb5:                                              ; preds = %bb4
  store i32 %_6.0, ptr %_0, align 4
  br label %bb10

panic1:                                           ; preds = %bb4
  call void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const24panic_const_sub_overflow(ptr align 8 @alloc_90259a2ae55d451f0da677bd23dc9d82) #5
  unreachable

bb3:                                              ; preds = %bb2
  store i32 %_5.0, ptr %_0, align 4
  br label %bb10

panic2:                                           ; preds = %bb2
  call void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const24panic_const_add_overflow(ptr align 8 @alloc_d7186a1fc8a35968f1077b6eb95040ad) #5
  unreachable
}

; Function Attrs: noreturn nounwind nonlazybind uwtable
define void @never_returns() unnamed_addr #1 {
start:
  br label %bb1

bb1:                                              ; preds = %bb1, %start
  br label %bb1
}

; Function Attrs: nounwind nonlazybind uwtable
define i32 @sparse_match(i32 %x) unnamed_addr #0 {
start:
  %_0 = alloca [4 x i8], align 4
  switch i32 %x, label %bb1 [
    i32 1, label %bb4
    i32 1000, label %bb3
    i32 1000000, label %bb2
  ]

bb1:                                              ; preds = %start
  store i32 0, ptr %_0, align 4
  br label %bb5

bb4:                                              ; preds = %start
  store i32 1, ptr %_0, align 4
  br label %bb5

bb3:                                              ; preds = %start
  store i32 2, ptr %_0, align 4
  br label %bb5

bb2:                                              ; preds = %start
  store i32 3, ptr %_0, align 4
  br label %bb5

bb5:                                              ; preds = %bb2, %bb3, %bb4, %bb1
  %0 = load i32, ptr %_0, align 4
  ret i32 %0
}

; Function Attrs: cold noinline noreturn nounwind nonlazybind uwtable
declare void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const24panic_const_add_overflow(ptr align 8) unnamed_addr #2

; Function Attrs: cold minsize noinline noreturn nounwind nonlazybind optsize uwtable
declare void @_RNvNtCs4uthzyWeO2a_4core9panicking18panic_bounds_check(i64, i64, ptr align 8) unnamed_addr #3

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare { i32, i1 } @llvm.ssub.with.overflow.i32(i32, i32) #4

; Function Attrs: cold noinline noreturn nounwind nonlazybind uwtable
declare void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const24panic_const_sub_overflow(ptr align 8) unnamed_addr #2

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare { i32, i1 } @llvm.sadd.with.overflow.i32(i32, i32) #4

attributes #0 = { nounwind nonlazybind uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #1 = { noreturn nounwind nonlazybind uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #2 = { cold noinline noreturn nounwind nonlazybind uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #3 = { cold minsize noinline noreturn nounwind nonlazybind optsize uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #4 = { nocallback nofree nosync nounwind speculatable willreturn memory(none) }
attributes #5 = { noinline noreturn nounwind }

!llvm.module.flags = !{!0, !1}
!llvm.ident = !{!2}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 2, !"RtLibUseGOT", i32 1}
!2 = !{!"rustc version 1.95.0 (59807616e 2026-04-14) (built from a source tarball)"}
