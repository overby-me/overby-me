; ModuleID = 'unwind.ll'
source_filename = "unwind.dbdd7f5e51ae03e2-cgu.0"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@alloc_bf39103a6db665396aab4632362d9353 = private unnamed_addr constant [42 x i8] c"there is no such thing as an acquire store", align 1
@alloc_e17502e153481471943c37e20438384b = private unnamed_addr constant [80 x i8] c"/rustc/59807616e1fa2540724bfbac14d7976d7e4a3860/library/core/src/sync/atomic.rs\00", align 1
@alloc_a574beff59ab58cf5ed2d6bc163e2302 = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_e17502e153481471943c37e20438384b, [16 x i8] c"O\00\00\00\00\00\00\00&\0F\00\00\18\00\00\00" }>, align 8
@alloc_00c0bce0fa6327f8ec8e69d6d765d508 = private unnamed_addr constant [50 x i8] c"there is no such thing as an acquire-release store", align 1
@alloc_aec0388915b72a4f610ad656c35be884 = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_e17502e153481471943c37e20438384b, [16 x i8] c"O\00\00\00\00\00\00\00'\0F\00\00\17\00\00\00" }>, align 8
@_ZN6unwind4FLAG17he8c62a6779c6712eE = global [1 x i8] zeroinitializer, align 1
@alloc_ecad43a203292ffac6e1f13204f48c54 = private unnamed_addr constant [68 x i8] c"/home/overby.me/Work/overby.me/rust/llvm/corpus/rustc/src/unwind.rs\00", align 1
@alloc_2bf3b7bcf230530e2ed6093fedbbcbe5 = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_ecad43a203292ffac6e1f13204f48c54, [16 x i8] c"C\00\00\00\00\00\00\00(\00\00\00\05\00\00\00" }>, align 8

; Function Attrs: nonlazybind uwtable
define void @"_ZN4core3ptr34drop_in_place$LT$unwind..Guard$GT$17hec65ff26918a7594E"(ptr align 1 %_1) unnamed_addr #0 {
start:
  call void @"_ZN55_$LT$unwind..Guard$u20$as$u20$core..ops..drop..Drop$GT$4drop17hfe14188ba54ceae4E"(ptr align 1 %_1)
  ret void
}

; Function Attrs: inlinehint nonlazybind uwtable
define void @_ZN4core4sync6atomic12atomic_store17h4e4b94e0fb38fc11E(ptr %dst, i8 %val, i8 %order) unnamed_addr #1 {
start:
  %_4 = zext i8 %order to i64
  switch i64 %_4, label %bb1 [
    i64 0, label %bb6
    i64 1, label %bb5
    i64 2, label %bb3
    i64 3, label %bb2
    i64 4, label %bb4
  ]

bb1:                                              ; preds = %start
  unreachable

bb6:                                              ; preds = %start
  store atomic i8 %val, ptr %dst monotonic, align 1
  br label %bb7

bb5:                                              ; preds = %start
  store atomic i8 %val, ptr %dst release, align 1
  br label %bb7

bb3:                                              ; preds = %start
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking9panic_fmt(ptr @alloc_bf39103a6db665396aab4632362d9353, ptr inttoptr (i64 85 to ptr), ptr align 8 @alloc_a574beff59ab58cf5ed2d6bc163e2302) #6
  unreachable

bb2:                                              ; preds = %start
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking9panic_fmt(ptr @alloc_00c0bce0fa6327f8ec8e69d6d765d508, ptr inttoptr (i64 101 to ptr), ptr align 8 @alloc_aec0388915b72a4f610ad656c35be884) #6
  unreachable

bb4:                                              ; preds = %start
  store atomic i8 %val, ptr %dst seq_cst, align 1
  br label %bb7

bb7:                                              ; preds = %bb4, %bb5, %bb6
  ret void
}

; Function Attrs: inlinehint nonlazybind uwtable
define internal void @_ZN4core4sync6atomic8AtomicU85store17h4765d1afac650597E(ptr align 1 %self, i8 %val, i8 %order) unnamed_addr #1 {
start:
  call void @_ZN4core4sync6atomic12atomic_store17h4e4b94e0fb38fc11E(ptr %self, i8 %val, i8 %order) #7
  ret void
}

; Function Attrs: nonlazybind uwtable
define void @"_ZN55_$LT$unwind..Guard$u20$as$u20$core..ops..drop..Drop$GT$4drop17hfe14188ba54ceae4E"(ptr align 1 %self) unnamed_addr #0 {
start:
  call void @_ZN4core4sync6atomic8AtomicU85store17h4765d1afac650597E(ptr align 1 @_ZN6unwind4FLAG17he8c62a6779c6712eE, i8 1, i8 4) #7
  ret void
}

; Function Attrs: nonlazybind uwtable
define i32 @indexing_can_panic(ptr align 4 %xs.0, i64 %xs.1, i64 %i) unnamed_addr #0 personality ptr @rust_eh_personality {
start:
  %0 = alloca [16 x i8], align 8
  %_3 = alloca [0 x i8], align 1
  %_5 = icmp ult i64 %i, %xs.1
  br i1 %_5, label %bb1, label %panic

bb1:                                              ; preds = %start
  %1 = getelementptr inbounds nuw i32, ptr %xs.0, i64 %i
  %_0 = load i32, ptr %1, align 4
  call void @"_ZN4core3ptr34drop_in_place$LT$unwind..Guard$GT$17hec65ff26918a7594E"(ptr align 1 %_3)
  ret i32 %_0

panic:                                            ; preds = %start
  invoke void @_RNvNtCs4uthzyWeO2a_4core9panicking18panic_bounds_check(i64 %i, i64 %xs.1, ptr align 8 @alloc_2bf3b7bcf230530e2ed6093fedbbcbe5) #8
          to label %unreachable unwind label %cleanup

bb3:                                              ; preds = %cleanup
  invoke void @"_ZN4core3ptr34drop_in_place$LT$unwind..Guard$GT$17hec65ff26918a7594E"(ptr align 1 %_3) #9
          to label %bb4 unwind label %terminate

cleanup:                                          ; preds = %panic
  %2 = landingpad { ptr, i32 }
          cleanup
  %3 = extractvalue { ptr, i32 } %2, 0
  %4 = extractvalue { ptr, i32 } %2, 1
  store ptr %3, ptr %0, align 8
  %5 = getelementptr inbounds i8, ptr %0, i64 8
  store i32 %4, ptr %5, align 8
  br label %bb3

unreachable:                                      ; preds = %panic
  unreachable

terminate:                                        ; preds = %bb3
  %6 = landingpad { ptr, i32 }
          filter [0 x ptr] zeroinitializer
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking16panic_in_cleanup() #10
  unreachable

bb4:                                              ; preds = %bb3
  %7 = load ptr, ptr %0, align 8
  %8 = getelementptr inbounds i8, ptr %0, i64 8
  %9 = load i32, ptr %8, align 8
  %10 = insertvalue { ptr, i32 } poison, ptr %7, 0
  %11 = insertvalue { ptr, i32 } %10, i32 %9, 1
  resume { ptr, i32 } %11
}

; Function Attrs: nonlazybind uwtable
define i32 @two_guards(i32 %x) unnamed_addr #0 personality ptr @rust_eh_personality {
start:
  %0 = alloca [16 x i8], align 8
  %_3 = alloca [0 x i8], align 1
  %_2 = alloca [0 x i8], align 1
  %_0 = invoke i32 @may_panic(i32 %x)
          to label %bb1 unwind label %cleanup

bb4:                                              ; preds = %cleanup
  invoke void @"_ZN4core3ptr34drop_in_place$LT$unwind..Guard$GT$17hec65ff26918a7594E"(ptr align 1 %_3) #9
          to label %bb5 unwind label %terminate

cleanup:                                          ; preds = %start
  %1 = landingpad { ptr, i32 }
          cleanup
  %2 = extractvalue { ptr, i32 } %1, 0
  %3 = extractvalue { ptr, i32 } %1, 1
  store ptr %2, ptr %0, align 8
  %4 = getelementptr inbounds i8, ptr %0, i64 8
  store i32 %3, ptr %4, align 8
  br label %bb4

bb1:                                              ; preds = %start
  invoke void @"_ZN4core3ptr34drop_in_place$LT$unwind..Guard$GT$17hec65ff26918a7594E"(ptr align 1 %_3)
          to label %bb2 unwind label %cleanup1

bb5:                                              ; preds = %cleanup1, %bb4
  invoke void @"_ZN4core3ptr34drop_in_place$LT$unwind..Guard$GT$17hec65ff26918a7594E"(ptr align 1 %_2) #9
          to label %bb6 unwind label %terminate

cleanup1:                                         ; preds = %bb1
  %5 = landingpad { ptr, i32 }
          cleanup
  %6 = extractvalue { ptr, i32 } %5, 0
  %7 = extractvalue { ptr, i32 } %5, 1
  store ptr %6, ptr %0, align 8
  %8 = getelementptr inbounds i8, ptr %0, i64 8
  store i32 %7, ptr %8, align 8
  br label %bb5

bb2:                                              ; preds = %bb1
  call void @"_ZN4core3ptr34drop_in_place$LT$unwind..Guard$GT$17hec65ff26918a7594E"(ptr align 1 %_2)
  ret i32 %_0

terminate:                                        ; preds = %bb5, %bb4
  %9 = landingpad { ptr, i32 }
          filter [0 x ptr] zeroinitializer
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking16panic_in_cleanup() #10
  unreachable

bb6:                                              ; preds = %bb5
  %10 = load ptr, ptr %0, align 8
  %11 = getelementptr inbounds i8, ptr %0, i64 8
  %12 = load i32, ptr %11, align 8
  %13 = insertvalue { ptr, i32 } poison, ptr %10, 0
  %14 = insertvalue { ptr, i32 } %13, i32 %12, 1
  resume { ptr, i32 } %14
}

; Function Attrs: nonlazybind uwtable
define i32 @with_guard(i32 %x) unnamed_addr #0 personality ptr @rust_eh_personality {
start:
  %0 = alloca [16 x i8], align 8
  %_2 = alloca [0 x i8], align 1
  %_0 = invoke i32 @may_panic(i32 %x)
          to label %bb1 unwind label %cleanup

bb3:                                              ; preds = %cleanup
  invoke void @"_ZN4core3ptr34drop_in_place$LT$unwind..Guard$GT$17hec65ff26918a7594E"(ptr align 1 %_2) #9
          to label %bb4 unwind label %terminate

cleanup:                                          ; preds = %start
  %1 = landingpad { ptr, i32 }
          cleanup
  %2 = extractvalue { ptr, i32 } %1, 0
  %3 = extractvalue { ptr, i32 } %1, 1
  store ptr %2, ptr %0, align 8
  %4 = getelementptr inbounds i8, ptr %0, i64 8
  store i32 %3, ptr %4, align 8
  br label %bb3

bb1:                                              ; preds = %start
  call void @"_ZN4core3ptr34drop_in_place$LT$unwind..Guard$GT$17hec65ff26918a7594E"(ptr align 1 %_2)
  ret i32 %_0

terminate:                                        ; preds = %bb3
  %5 = landingpad { ptr, i32 }
          filter [0 x ptr] zeroinitializer
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking16panic_in_cleanup() #10
  unreachable

bb4:                                              ; preds = %bb3
  %6 = load ptr, ptr %0, align 8
  %7 = getelementptr inbounds i8, ptr %0, i64 8
  %8 = load i32, ptr %7, align 8
  %9 = insertvalue { ptr, i32 } poison, ptr %6, 0
  %10 = insertvalue { ptr, i32 } %9, i32 %8, 1
  resume { ptr, i32 } %10
}

; Function Attrs: cold noinline noreturn nonlazybind uwtable
declare void @_RNvNtCs4uthzyWeO2a_4core9panicking9panic_fmt(ptr, ptr, ptr align 8) unnamed_addr #2

; Function Attrs: nonlazybind
declare i32 @rust_eh_personality(...) unnamed_addr #3

; Function Attrs: cold minsize noinline noreturn nonlazybind optsize uwtable
declare void @_RNvNtCs4uthzyWeO2a_4core9panicking18panic_bounds_check(i64, i64, ptr align 8) unnamed_addr #4

; Function Attrs: cold minsize noinline noreturn nounwind nonlazybind optsize uwtable
declare void @_RNvNtCs4uthzyWeO2a_4core9panicking16panic_in_cleanup() unnamed_addr #5

; Function Attrs: nonlazybind uwtable
declare i32 @may_panic(i32) unnamed_addr #0

attributes #0 = { nonlazybind uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #1 = { inlinehint nonlazybind uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #2 = { cold noinline noreturn nonlazybind uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #3 = { nonlazybind "target-cpu"="x86-64" }
attributes #4 = { cold minsize noinline noreturn nonlazybind optsize uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #5 = { cold minsize noinline noreturn nounwind nonlazybind optsize uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #6 = { noinline noreturn }
attributes #7 = { inlinehint }
attributes #8 = { noreturn }
attributes #9 = { cold }
attributes #10 = { cold noreturn nounwind }

!llvm.module.flags = !{!0, !1}
!llvm.ident = !{!2}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 2, !"RtLibUseGOT", i32 1}
!2 = !{!"rustc version 1.95.0 (59807616e 2026-04-14) (built from a source tarball)"}
