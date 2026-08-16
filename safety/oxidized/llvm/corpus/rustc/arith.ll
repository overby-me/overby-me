; ModuleID = 'arith.ll'
source_filename = "arith.fbb9fb4f363f2942-cgu.0"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@alloc_e05d24d8281e788759f08dbea3521ad4 = private unnamed_addr constant [76 x i8] c"/rustc/59807616e1fa2540724bfbac14d7976d7e4a3860/library/core/src/num/mod.rs\00", align 1
@alloc_54c2b0cbeb44e1fb485f7f54185dd08a = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_e05d24d8281e788759f08dbea3521ad4, [16 x i8] c"K\00\00\00\00\00\00\00\8A\01\00\00\05\00\00\00" }>, align 8
@anon.c9ce595e7610956914f5050e21aaf03e.0 = private unnamed_addr constant <{ [4 x i8], [4 x i8] }> <{ [4 x i8] zeroinitializer, [4 x i8] undef }>, align 4
@alloc_eea3b4108fd45221c9468d112de34f4d = private unnamed_addr constant [184 x i8] c"unsafe precondition(s) violated: i32::unchecked_shl cannot overflow\0A\0AThis indicates a bug in the program. This Undefined Behavior check is optional, and cannot be relied on for safety.", align 1
@alloc_b136cd45a46530f53e851da940bfc0a1 = private unnamed_addr constant [67 x i8] c"/home/overby.me/Work/overby.me/rust/llvm/corpus/rustc/src/arith.rs\00", align 1
@alloc_3b99d3d10b0f179ad7ff3bbd3c887ada = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_b136cd45a46530f53e851da940bfc0a1, [16 x i8] c"B\00\00\00\00\00\00\00\16\00\00\00\05\00\00\00" }>, align 8
@alloc_bd7d402879de66f99c57c4d7408a9538 = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_b136cd45a46530f53e851da940bfc0a1, [16 x i8] c"B\00\00\00\00\00\00\00\11\00\00\00\05\00\00\00" }>, align 8
@alloc_74b618c45b76b75f557e02b1df91986e = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_b136cd45a46530f53e851da940bfc0a1, [16 x i8] c"B\00\00\00\00\00\00\00\11\00\00\00\0F\00\00\00" }>, align 8

; Function Attrs: cold nounwind nonlazybind uwtable
define internal void @_ZN4core10intrinsics9cold_path17h3f15ccfd3fae14a4E() unnamed_addr #0 {
start:
  ret void
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal i64 @"_ZN4core3f6421_$LT$impl$u20$f64$GT$7to_bits17h53daca25f907cd90E"(double %self) unnamed_addr #1 {
start:
  %_0 = bitcast double %self to i64
  ret i64 %_0
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal { i32, i32 } @"_ZN4core3num21_$LT$impl$u20$i32$GT$11checked_add17h2e85421035264bf7E"(i32 %self, i32 %rhs) unnamed_addr #1 {
start:
  %_0 = alloca [8 x i8], align 4
  %0 = call { i32, i1 } @llvm.sadd.with.overflow.i32(i32 %self, i32 %rhs)
  %_5.0 = extractvalue { i32, i1 } %0, 0
  %_5.1 = extractvalue { i32, i1 } %0, 1
  br i1 %_5.1, label %bb2, label %bb4

bb4:                                              ; preds = %start
  %1 = getelementptr inbounds i8, ptr %_0, i64 4
  store i32 %_5.0, ptr %1, align 4
  store i32 1, ptr %_0, align 4
  br label %bb1

bb2:                                              ; preds = %start
  %2 = load i32, ptr @anon.c9ce595e7610956914f5050e21aaf03e.0, align 4
  %3 = load i32, ptr getelementptr inbounds (i8, ptr @anon.c9ce595e7610956914f5050e21aaf03e.0, i64 4), align 4
  store i32 %2, ptr %_0, align 4
  %4 = getelementptr inbounds i8, ptr %_0, i64 4
  store i32 %3, ptr %4, align 4
  br label %bb1

bb1:                                              ; preds = %bb2, %bb4
  %5 = load i32, ptr %_0, align 4
  %6 = getelementptr inbounds i8, ptr %_0, i64 4
  %7 = load i32, ptr %6, align 4
  %8 = insertvalue { i32, i32 } poison, i32 %5, 0
  %9 = insertvalue { i32, i32 } %8, i32 %7, 1
  ret { i32, i32 } %9
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal void @"_ZN4core3num21_$LT$impl$u20$i32$GT$13unchecked_shl18precondition_check17h8d479b640fb5f9adE"(i32 %rhs, ptr align 8 %0) unnamed_addr #1 {
start:
  %_2 = icmp ult i32 %rhs, 32
  br i1 %_2, label %bb1, label %bb2

bb2:                                              ; preds = %start
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking18panic_nounwind_fmt(ptr @alloc_eea3b4108fd45221c9468d112de34f4d, ptr inttoptr (i64 369 to ptr), i1 zeroext false, ptr align 8 %0) #5
  unreachable

bb1:                                              ; preds = %start
  ret void
}

; Function Attrs: nounwind nonlazybind uwtable
define i8 @bool_to_int(i1 zeroext %a) unnamed_addr #2 {
start:
  %_0 = zext i1 %a to i8
  ret i8 %_0
}

; Function Attrs: nounwind nonlazybind uwtable
define { i32, i32 } @checked(i32 %a, i32 %b) unnamed_addr #2 {
start:
  %0 = call { i32, i32 } @"_ZN4core3num21_$LT$impl$u20$i32$GT$11checked_add17h2e85421035264bf7E"(i32 %a, i32 %b) #6
  %_0.0 = extractvalue { i32, i32 } %0, 0
  %_0.1 = extractvalue { i32, i32 } %0, 1
  %1 = insertvalue { i32, i32 } poison, i32 %_0.0, 0
  %2 = insertvalue { i32, i32 } %1, i32 %_0.1, 1
  ret { i32, i32 } %2
}

; Function Attrs: nounwind nonlazybind uwtable
define zeroext i1 @compare_floats(double %x, double %y) unnamed_addr #2 {
start:
  %_0 = fcmp ole double %x, %y
  ret i1 %_0
}

; Function Attrs: nounwind nonlazybind uwtable
define zeroext i1 @compare_ints(i32 %a, i32 %b) unnamed_addr #2 {
start:
  %_0 = alloca [1 x i8], align 1
  %_3 = icmp slt i32 %a, %b
  br i1 %_3, label %bb1, label %bb2

bb2:                                              ; preds = %start
  store i8 0, ptr %_0, align 1
  br label %bb3

bb1:                                              ; preds = %start
  %0 = icmp ne i32 %a, %b
  %1 = zext i1 %0 to i8
  store i8 %1, ptr %_0, align 1
  br label %bb3

bb3:                                              ; preds = %bb1, %bb2
  %2 = load i8, ptr %_0, align 1
  %3 = trunc nuw i8 %2 to i1
  ret i1 %3
}

; Function Attrs: nounwind nonlazybind uwtable
define zeroext i1 @compare_unsigned(i32 %a, i32 %b) unnamed_addr #2 {
start:
  %_0 = alloca [1 x i8], align 1
  %_3 = icmp uge i32 %a, %b
  br i1 %_3, label %bb1, label %bb2

bb2:                                              ; preds = %start
  %0 = icmp eq i32 %a, %b
  %1 = zext i1 %0 to i8
  store i8 %1, ptr %_0, align 1
  br label %bb3

bb1:                                              ; preds = %start
  store i8 1, ptr %_0, align 1
  br label %bb3

bb3:                                              ; preds = %bb1, %bb2
  %2 = load i8, ptr %_0, align 1
  %3 = trunc nuw i8 %2 to i1
  ret i1 %3
}

; Function Attrs: nounwind nonlazybind uwtable
define void @conversions(ptr sret([32 x i8]) align 8 %_0, i8 %a, i16 %b, float %x) unnamed_addr #2 {
start:
  %_4 = sext i8 %a to i64
  %_5 = trunc i16 %b to i8
  %_6 = fpext float %x to double
  %_7 = call i32 @llvm.fptosi.sat.i32.f32(float %x)
  %_9 = fpext float %x to double
  %_8 = call i64 @"_ZN4core3f6421_$LT$impl$u20$f64$GT$7to_bits17h53daca25f907cd90E"(double %_9) #6
  store i64 %_4, ptr %_0, align 8
  %0 = getelementptr inbounds i8, ptr %_0, i64 20
  store i8 %_5, ptr %0, align 4
  %1 = getelementptr inbounds i8, ptr %_0, i64 8
  store double %_6, ptr %1, align 8
  %2 = getelementptr inbounds i8, ptr %_0, i64 16
  store i32 %_7, ptr %2, align 8
  %3 = getelementptr inbounds i8, ptr %_0, i64 24
  store i64 %_8, ptr %3, align 8
  ret void
}

; Function Attrs: nounwind nonlazybind uwtable
define double @floats(double %x, double %y) unnamed_addr #2 {
start:
  %_5 = fadd double %x, %y
  %_6 = fsub double %x, %y
  %_4 = fmul double %_5, %_6
  %_7 = fadd double %y, 1.000000e+00
  %_3 = fdiv double %_4, %_7
  %_8 = frem double %x, %y
  %_0 = fsub double %_3, %_8
  ret double %_0
}

; Function Attrs: nounwind nonlazybind uwtable
define i64 @signed_shift(i64 %a, i32 %b) unnamed_addr #2 {
start:
  %_3 = icmp ult i32 %b, 64
  br i1 %_3, label %bb1, label %panic

bb1:                                              ; preds = %start
  %0 = and i32 %b, 63
  %1 = zext i32 %0 to i64
  %_0 = ashr i64 %a, %1
  ret i64 %_0

panic:                                            ; preds = %start
  call void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const24panic_const_shr_overflow(ptr align 8 @alloc_3b99d3d10b0f179ad7ff3bbd3c887ada) #5
  unreachable
}

; Function Attrs: nounwind nonlazybind uwtable
define float @single(float %x) unnamed_addr #2 {
start:
  %_2 = fneg float %x
  %_0 = fmul float %_2, 5.000000e-01
  ret float %_0
}

; Function Attrs: nounwind nonlazybind uwtable
define i64 @unsigned_ops(i64 %a, i64 %b) unnamed_addr #2 {
start:
  %_7 = icmp eq i64 %b, 0
  br i1 %_7, label %panic, label %bb1

bb1:                                              ; preds = %start
  %_6 = udiv i64 %a, %b
  %_9 = icmp eq i64 %b, 0
  br i1 %_9, label %panic1, label %bb2

panic:                                            ; preds = %start
  call void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const23panic_const_div_by_zero(ptr align 8 @alloc_bd7d402879de66f99c57c4d7408a9538) #5
  unreachable

bb2:                                              ; preds = %bb1
  %_8 = urem i64 %a, %b
  %_5 = xor i64 %_6, %_8
  %_10 = and i64 %a, %b
  %_4 = or i64 %_5, %_10
  %_11 = lshr i64 %a, 2
  %_3 = or i64 %_4, %_11
  %_14 = shl i64 %a, 1
  %_0 = or i64 %_3, %_14
  ret i64 %_0

panic1:                                           ; preds = %bb1
  call void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const23panic_const_rem_by_zero(ptr align 8 @alloc_74b618c45b76b75f557e02b1df91986e) #5
  unreachable
}

; Function Attrs: nounwind nonlazybind uwtable
define i128 @wide_math(i128 %a, i128 %b) unnamed_addr #2 {
start:
  %_0.i1 = mul i128 %a, %b
  %_0.i = add i128 %_0.i1, 1
  ret i128 %_0.i
}

; Function Attrs: nounwind nonlazybind uwtable
define i32 @wrapping(i32 %a, i32 %b) unnamed_addr #2 {
start:
  %_0.i = add i32 %a, %b
  %_0.i1 = mul i32 %a, %a
  call void @"_ZN4core3num21_$LT$impl$u20$i32$GT$13unchecked_shl18precondition_check17h8d479b640fb5f9adE"(i32 3, ptr align 8 @alloc_54c2b0cbeb44e1fb485f7f54185dd08a) #6
  %_0.i2 = shl i32 %_0.i1, 3
  ret i32 %_0.i2
}

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare { i32, i1 } @llvm.sadd.with.overflow.i32(i32, i32) #3

; Function Attrs: cold noinline noreturn nounwind nonlazybind uwtable
declare void @_RNvNtCs4uthzyWeO2a_4core9panicking18panic_nounwind_fmt(ptr, ptr, i1 zeroext, ptr align 8) unnamed_addr #4

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare i32 @llvm.fptosi.sat.i32.f32(float) #3

; Function Attrs: cold noinline noreturn nounwind nonlazybind uwtable
declare void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const24panic_const_shr_overflow(ptr align 8) unnamed_addr #4

; Function Attrs: cold noinline noreturn nounwind nonlazybind uwtable
declare void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const23panic_const_div_by_zero(ptr align 8) unnamed_addr #4

; Function Attrs: cold noinline noreturn nounwind nonlazybind uwtable
declare void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const23panic_const_rem_by_zero(ptr align 8) unnamed_addr #4

attributes #0 = { cold nounwind nonlazybind uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #1 = { inlinehint nounwind nonlazybind uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #2 = { nounwind nonlazybind uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #3 = { nocallback nofree nosync nounwind speculatable willreturn memory(none) }
attributes #4 = { cold noinline noreturn nounwind nonlazybind uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #5 = { noinline noreturn nounwind }
attributes #6 = { inlinehint nounwind }

!llvm.module.flags = !{!0, !1}
!llvm.ident = !{!2}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 2, !"RtLibUseGOT", i32 1}
!2 = !{!"rustc version 1.95.0 (59807616e 2026-04-14) (built from a source tarball)"}
