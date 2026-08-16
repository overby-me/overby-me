; ModuleID = 'enums.ll'
source_filename = "enums.35725fa9e5af2093-cgu.0"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@alloc_e05d24d8281e788759f08dbea3521ad4 = private unnamed_addr constant [76 x i8] c"/rustc/59807616e1fa2540724bfbac14d7976d7e4a3860/library/core/src/num/mod.rs\00", align 1
@alloc_54c2b0cbeb44e1fb485f7f54185dd08a = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_e05d24d8281e788759f08dbea3521ad4, [16 x i8] c"K\00\00\00\00\00\00\00\8A\01\00\00\05\00\00\00" }>, align 8
@alloc_eea3b4108fd45221c9468d112de34f4d = private unnamed_addr constant [184 x i8] c"unsafe precondition(s) violated: i32::unchecked_shl cannot overflow\0A\0AThis indicates a bug in the program. This Undefined Behavior check is optional, and cannot be relied on for safety.", align 1
@alloc_51faf7dcdededcddf8ff69cfb939d83f = private unnamed_addr constant [184 x i8] c"unsafe precondition(s) violated: i32::unchecked_shr cannot overflow\0A\0AThis indicates a bug in the program. This Undefined Behavior check is optional, and cannot be relied on for safety.", align 1

; Function Attrs: cold nounwind nonlazybind uwtable
define internal void @_ZN4core10intrinsics9cold_path17hbb2c64bfd84ec291E() unnamed_addr #0 {
start:
  ret void
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal i32 @"_ZN4core3num21_$LT$impl$u20$i32$GT$12wrapping_div17h68e998da37544623E"(i32 %self, i32 %rhs) unnamed_addr #1 {
start:
  %0 = call { i32, i1 } @"_ZN4core3num21_$LT$impl$u20$i32$GT$15overflowing_div17h8442c7527431a805E"(i32 %self, i32 %rhs) #4
  %_3.0 = extractvalue { i32, i1 } %0, 0
  %_3.1 = extractvalue { i32, i1 } %0, 1
  ret i32 %_3.0
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal i32 @"_ZN4core3num21_$LT$impl$u20$i32$GT$12wrapping_rem17h75123849ab958ba6E"(i32 %self, i32 %rhs) unnamed_addr #1 {
start:
  %0 = call { i32, i1 } @"_ZN4core3num21_$LT$impl$u20$i32$GT$15overflowing_rem17h72a27ff924d708deE"(i32 %self, i32 %rhs) #4
  %_3.0 = extractvalue { i32, i1 } %0, 0
  %_3.1 = extractvalue { i32, i1 } %0, 1
  ret i32 %_3.0
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal void @"_ZN4core3num21_$LT$impl$u20$i32$GT$13unchecked_shl18precondition_check17h9aa1843b5b5575fcE"(i32 %rhs, ptr align 8 %0) unnamed_addr #1 {
start:
  %_2 = icmp ult i32 %rhs, 32
  br i1 %_2, label %bb1, label %bb2

bb2:                                              ; preds = %start
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking18panic_nounwind_fmt(ptr @alloc_eea3b4108fd45221c9468d112de34f4d, ptr inttoptr (i64 369 to ptr), i1 zeroext false, ptr align 8 %0) #5
  unreachable

bb1:                                              ; preds = %start
  ret void
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal void @"_ZN4core3num21_$LT$impl$u20$i32$GT$13unchecked_shr18precondition_check17hfc89d748a5923779E"(i32 %rhs, ptr align 8 %0) unnamed_addr #1 {
start:
  %_2 = icmp ult i32 %rhs, 32
  br i1 %_2, label %bb1, label %bb2

bb2:                                              ; preds = %start
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking18panic_nounwind_fmt(ptr @alloc_51faf7dcdededcddf8ff69cfb939d83f, ptr inttoptr (i64 369 to ptr), i1 zeroext false, ptr align 8 %0) #5
  unreachable

bb1:                                              ; preds = %start
  ret void
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal { i32, i1 } @"_ZN4core3num21_$LT$impl$u20$i32$GT$15overflowing_div17h8442c7527431a805E"(i32 %self, i32 %rhs) unnamed_addr #1 {
start:
  %_0 = alloca [8 x i8], align 4
  %_4 = icmp eq i32 %self, -2147483648
  %_5 = icmp eq i32 %rhs, -1
  %b = and i1 %_4, %_5
  br i1 %b, label %bb4, label %bb6

bb6:                                              ; preds = %start
  %_7 = icmp eq i32 %rhs, 0
  br i1 %_7, label %panic, label %bb1

bb4:                                              ; preds = %start
  store i32 %self, ptr %_0, align 4
  %0 = getelementptr inbounds i8, ptr %_0, i64 4
  store i8 1, ptr %0, align 4
  br label %bb3

bb1:                                              ; preds = %bb6
  %_8 = icmp eq i32 %self, -2147483648
  %_9 = and i1 %_5, %_8
  br i1 %_9, label %panic1, label %bb2

panic:                                            ; preds = %bb6
  call void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const23panic_const_div_by_zero(ptr align 8 @alloc_54c2b0cbeb44e1fb485f7f54185dd08a) #5
  unreachable

bb2:                                              ; preds = %bb1
  %_6 = sdiv i32 %self, %rhs
  store i32 %_6, ptr %_0, align 4
  %1 = getelementptr inbounds i8, ptr %_0, i64 4
  store i8 0, ptr %1, align 4
  br label %bb3

panic1:                                           ; preds = %bb1
  call void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const24panic_const_div_overflow(ptr align 8 @alloc_54c2b0cbeb44e1fb485f7f54185dd08a) #5
  unreachable

bb3:                                              ; preds = %bb2, %bb4
  %2 = load i32, ptr %_0, align 4
  %3 = getelementptr inbounds i8, ptr %_0, i64 4
  %4 = load i8, ptr %3, align 4
  %5 = trunc nuw i8 %4 to i1
  %6 = insertvalue { i32, i1 } poison, i32 %2, 0
  %7 = insertvalue { i32, i1 } %6, i1 %5, 1
  ret { i32, i1 } %7
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal { i32, i1 } @"_ZN4core3num21_$LT$impl$u20$i32$GT$15overflowing_rem17h72a27ff924d708deE"(i32 %self, i32 %rhs) unnamed_addr #1 {
start:
  %_0 = alloca [8 x i8], align 4
  %b = icmp eq i32 %rhs, -1
  %0 = icmp eq i32 %rhs, -1
  br i1 %0, label %bb4, label %bb6

bb4:                                              ; preds = %start
  %_4 = icmp eq i32 %self, -2147483648
  store i32 0, ptr %_0, align 4
  %1 = getelementptr inbounds i8, ptr %_0, i64 4
  %2 = zext i1 %_4 to i8
  store i8 %2, ptr %1, align 4
  br label %bb3

bb6:                                              ; preds = %start
  %_6 = icmp eq i32 %rhs, 0
  br i1 %_6, label %panic, label %bb1

bb3:                                              ; preds = %bb2, %bb4
  %3 = load i32, ptr %_0, align 4
  %4 = getelementptr inbounds i8, ptr %_0, i64 4
  %5 = load i8, ptr %4, align 4
  %6 = trunc nuw i8 %5 to i1
  %7 = insertvalue { i32, i1 } poison, i32 %3, 0
  %8 = insertvalue { i32, i1 } %7, i1 %6, 1
  ret { i32, i1 } %8

bb1:                                              ; preds = %bb6
  %_7 = icmp eq i32 %self, -2147483648
  %_8 = and i1 %b, %_7
  br i1 %_8, label %panic1, label %bb2

panic:                                            ; preds = %bb6
  call void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const23panic_const_rem_by_zero(ptr align 8 @alloc_54c2b0cbeb44e1fb485f7f54185dd08a) #5
  unreachable

bb2:                                              ; preds = %bb1
  %_5 = srem i32 %self, %rhs
  store i32 %_5, ptr %_0, align 4
  %9 = getelementptr inbounds i8, ptr %_0, i64 4
  store i8 0, ptr %9, align 4
  br label %bb3

panic1:                                           ; preds = %bb1
  call void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const24panic_const_rem_overflow(ptr align 8 @alloc_54c2b0cbeb44e1fb485f7f54185dd08a) #5
  unreachable
}

; Function Attrs: nounwind nonlazybind uwtable
define i32 @_ZN5enums5apply17h3aba049f27046353E(i8 %op, i32 %a, i32 %b) unnamed_addr #2 {
start:
  %_0 = alloca [4 x i8], align 4
  %_4 = zext i8 %op to i64
  switch i64 %_4, label %bb1 [
    i64 0, label %bb8
    i64 1, label %bb7
    i64 2, label %bb6
    i64 3, label %bb5
    i64 4, label %bb4
    i64 5, label %bb3
    i64 6, label %bb2
  ]

bb1:                                              ; preds = %start
  unreachable

bb8:                                              ; preds = %start
  %_0.i = add i32 %a, %b
  store i32 %_0.i, ptr %_0, align 4
  br label %bb9

bb7:                                              ; preds = %start
  %_0.i5 = sub i32 %a, %b
  store i32 %_0.i5, ptr %_0, align 4
  br label %bb9

bb6:                                              ; preds = %start
  %_0.i1 = mul i32 %a, %b
  store i32 %_0.i1, ptr %_0, align 4
  br label %bb9

bb5:                                              ; preds = %start
  %0 = call i32 @"_ZN4core3num21_$LT$impl$u20$i32$GT$12wrapping_div17h68e998da37544623E"(i32 %a, i32 %b) #4
  store i32 %0, ptr %_0, align 4
  br label %bb9

bb4:                                              ; preds = %start
  %1 = call i32 @"_ZN4core3num21_$LT$impl$u20$i32$GT$12wrapping_rem17h75123849ab958ba6E"(i32 %a, i32 %b) #4
  store i32 %1, ptr %_0, align 4
  br label %bb9

bb3:                                              ; preds = %start
  %rhs1.i = and i32 %b, 31
  call void @"_ZN4core3num21_$LT$impl$u20$i32$GT$13unchecked_shl18precondition_check17h9aa1843b5b5575fcE"(i32 %rhs1.i, ptr align 8 @alloc_54c2b0cbeb44e1fb485f7f54185dd08a) #4
  %_0.i2 = shl i32 %a, %rhs1.i
  store i32 %_0.i2, ptr %_0, align 4
  br label %bb9

bb2:                                              ; preds = %start
  %rhs1.i3 = and i32 %b, 31
  call void @"_ZN4core3num21_$LT$impl$u20$i32$GT$13unchecked_shr18precondition_check17hfc89d748a5923779E"(i32 %rhs1.i3, ptr align 8 @alloc_54c2b0cbeb44e1fb485f7f54185dd08a) #4
  %_0.i4 = ashr i32 %a, %rhs1.i3
  store i32 %_0.i4, ptr %_0, align 4
  br label %bb9

bb9:                                              ; preds = %bb2, %bb3, %bb4, %bb5, %bb6, %bb7, %bb8
  %2 = load i32, ptr %_0, align 4
  ret i32 %2
}

; Function Attrs: nounwind nonlazybind uwtable
define i64 @_ZN5enums5widen17h5e9d952137fc2051E(ptr align 4 %value) unnamed_addr #2 {
start:
  %_0 = alloca [8 x i8], align 8
  %0 = load i8, ptr %value, align 4
  %_2 = zext i8 %0 to i64
  switch i64 %_2, label %bb1 [
    i64 0, label %bb5
    i64 1, label %bb4
    i64 2, label %bb3
    i64 3, label %bb2
  ]

bb1:                                              ; preds = %start
  unreachable

bb5:                                              ; preds = %start
  store i64 0, ptr %_0, align 8
  br label %bb9

bb4:                                              ; preds = %start
  %b = getelementptr inbounds i8, ptr %value, i64 1
  %_4 = load i8, ptr %b, align 1
  %_0.i = zext i8 %_4 to i64
  store i64 %_0.i, ptr %_0, align 8
  br label %bb9

bb3:                                              ; preds = %start
  %w = getelementptr inbounds i8, ptr %value, i64 4
  %_6 = load i32, ptr %w, align 4
  %_0.i4 = zext i32 %_6 to i64
  store i64 %_0.i4, ptr %_0, align 8
  br label %bb9

bb2:                                              ; preds = %start
  %a = getelementptr inbounds i8, ptr %value, i64 4
  %b1 = getelementptr inbounds i8, ptr %value, i64 8
  %_11 = load i32, ptr %a, align 4
  %_0.i3 = zext i32 %_11 to i64
  %_9 = shl i64 %_0.i3, 32
  %_15 = load i32, ptr %b1, align 4
  %_0.i2 = zext i32 %_15 to i64
  %1 = or i64 %_9, %_0.i2
  store i64 %1, ptr %_0, align 8
  br label %bb9

bb9:                                              ; preds = %bb2, %bb3, %bb4, %bb5
  %2 = load i64, ptr %_0, align 8
  ret i64 %2
}

; Function Attrs: nounwind nonlazybind uwtable
define i32 @_ZN5enums8deref_or17h8a07c04d2203731cE(ptr align 4 %0, i32 %fallback) unnamed_addr #2 {
start:
  %_0 = alloca [4 x i8], align 4
  %option = alloca [8 x i8], align 8
  store ptr %0, ptr %option, align 8
  %1 = load ptr, ptr %option, align 8
  %2 = ptrtoint ptr %1 to i64
  %3 = icmp eq i64 %2, 0
  %_3 = select i1 %3, i64 0, i64 1
  %4 = trunc nuw i64 %_3 to i1
  br i1 %4, label %bb3, label %bb2

bb3:                                              ; preds = %start
  %value = load ptr, ptr %option, align 8
  %5 = load i32, ptr %value, align 4
  store i32 %5, ptr %_0, align 4
  br label %bb4

bb2:                                              ; preds = %start
  store i32 %fallback, ptr %_0, align 4
  br label %bb4

bb4:                                              ; preds = %bb2, %bb3
  %6 = load i32, ptr %_0, align 4
  ret i32 %6

bb1:                                              ; No predecessors!
  unreachable
}

; Function Attrs: cold noinline noreturn nounwind nonlazybind uwtable
declare void @_RNvNtCs4uthzyWeO2a_4core9panicking18panic_nounwind_fmt(ptr, ptr, i1 zeroext, ptr align 8) unnamed_addr #3

; Function Attrs: cold noinline noreturn nounwind nonlazybind uwtable
declare void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const23panic_const_div_by_zero(ptr align 8) unnamed_addr #3

; Function Attrs: cold noinline noreturn nounwind nonlazybind uwtable
declare void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const24panic_const_div_overflow(ptr align 8) unnamed_addr #3

; Function Attrs: cold noinline noreturn nounwind nonlazybind uwtable
declare void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const23panic_const_rem_by_zero(ptr align 8) unnamed_addr #3

; Function Attrs: cold noinline noreturn nounwind nonlazybind uwtable
declare void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const24panic_const_rem_overflow(ptr align 8) unnamed_addr #3

attributes #0 = { cold nounwind nonlazybind uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #1 = { inlinehint nounwind nonlazybind uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #2 = { nounwind nonlazybind uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #3 = { cold noinline noreturn nounwind nonlazybind uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #4 = { inlinehint nounwind }
attributes #5 = { noinline noreturn nounwind }

!llvm.module.flags = !{!0, !1}
!llvm.ident = !{!2}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 2, !"RtLibUseGOT", i32 1}
!2 = !{!"rustc version 1.95.0 (59807616e 2026-04-14) (built from a source tarball)"}
