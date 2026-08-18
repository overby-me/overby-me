; ModuleID = 'calls.ll'
source_filename = "calls.a8c912beb06f5d98-cgu.0"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@alloc_270d35f65696b300988bbb2c23055154 = private unnamed_addr constant [67 x i8] c"/home/overby.me/Work/overby.me/rust/llvm/corpus/rustc/src/calls.rs\00", align 1
@alloc_162495632b5fcbb9f06bf197bd268d93 = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_270d35f65696b300988bbb2c23055154, [16 x i8] c"B\00\00\00\00\00\00\00\16\00\00\00\09\00\00\00" }>, align 8
@alloc_49306441e5a5f0c25805e0c836b8d6fb = private unnamed_addr constant [6 x i8] c"square", align 1
@alloc_87551382a9de3243abbfdbda2f0b586b = private unnamed_addr constant [4 x i8] c"%d\0A\00", align 1

; Function Attrs: nounwind nonlazybind uwtable
define i64 @"_ZN46_$LT$calls..Square$u20$as$u20$calls..Shape$GT$4area17h8eebee98efa55e83E"(ptr align 4 %self) unnamed_addr #0 {
start:
  %_3 = load i32, ptr %self, align 4
  %_0.i1 = zext i32 %_3 to i64
  %_5 = load i32, ptr %self, align 4
  %_0.i = zext i32 %_5 to i64
  %0 = call { i64, i1 } @llvm.umul.with.overflow.i64(i64 %_0.i1, i64 %_0.i)
  %_6.0 = extractvalue { i64, i1 } %0, 0
  %_6.1 = extractvalue { i64, i1 } %0, 1
  br i1 %_6.1, label %panic, label %bb3

bb3:                                              ; preds = %start
  ret i64 %_6.0

panic:                                            ; preds = %start
  call void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const24panic_const_mul_overflow(ptr align 8 @alloc_162495632b5fcbb9f06bf197bd268d93) #4
  unreachable
}

; Function Attrs: nounwind nonlazybind uwtable
define { ptr, i64 } @"_ZN46_$LT$calls..Square$u20$as$u20$calls..Shape$GT$4name17he5e4e693e4ebeb29E"(ptr align 4 %self) unnamed_addr #0 {
start:
  ret { ptr, i64 } { ptr @alloc_49306441e5a5f0c25805e0c836b8d6fb, i64 6 }
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal ptr @_ZN4core3ffi5c_str4CStr6as_ptr17h24339f3917ac2fdfE(ptr align 1 %self.0, i64 %self.1) unnamed_addr #1 {
start:
  ret ptr %self.0
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal i64 @"_ZN4core3str21_$LT$impl$u20$str$GT$3len17h4d2dc01c6dbaf506E"(ptr align 1 %self.0, i64 %self.1) unnamed_addr #1 {
start:
  ret i64 %self.1
}

; Function Attrs: nounwind nonlazybind uwtable
define i32 @boxed_square(i32 %side) unnamed_addr #0 {
start:
  ret i32 %side
}

; Function Attrs: nounwind nonlazybind uwtable
define i32 @byte_swap(i32 %x) unnamed_addr #0 {
start:
  %0 = alloca [4 x i8], align 4
  %1 = call i32 @llvm.bswap.i32(i32 %x)
  store i32 %1, ptr %0, align 4
  %_0.i = load i32, ptr %0, align 4
  ret i32 %_0.i
}

; Function Attrs: nounwind nonlazybind uwtable
define void @counting(ptr sret([12 x i8]) align 4 %_0, i64 %x) unnamed_addr #0 {
start:
  %0 = alloca [4 x i8], align 4
  %1 = alloca [4 x i8], align 4
  %2 = alloca [4 x i8], align 4
  %3 = call i64 @llvm.ctpop.i64(i64 %x)
  %4 = trunc i64 %3 to i32
  store i32 %4, ptr %2, align 4
  %_0.i = load i32, ptr %2, align 4
  %5 = call i64 @llvm.ctlz.i64(i64 %x, i1 false)
  %6 = trunc i64 %5 to i32
  store i32 %6, ptr %1, align 4
  %_0.i1 = load i32, ptr %1, align 4
  %7 = call i64 @llvm.cttz.i64(i64 %x, i1 false)
  %8 = trunc i64 %7 to i32
  store i32 %8, ptr %0, align 4
  %_0.i2 = load i32, ptr %0, align 4
  store i32 %_0.i, ptr %_0, align 4
  %9 = getelementptr inbounds i8, ptr %_0, i64 4
  store i32 %_0.i1, ptr %9, align 4
  %10 = getelementptr inbounds i8, ptr %_0, i64 8
  store i32 %_0.i2, ptr %10, align 4
  ret void
}

; Function Attrs: nounwind nonlazybind uwtable
define i32 @direct(i32 %x) unnamed_addr #0 {
start:
  %_0 = call i32 @plain(i32 %x) #5
  ret i32 %_0
}

; Function Attrs: nounwind nonlazybind uwtable
define i64 @dynamic(ptr align 1 %shape.0, ptr align 8 %shape.1) unnamed_addr #0 {
start:
  %0 = getelementptr inbounds i8, ptr %shape.1, i64 24
  %1 = load ptr, ptr %0, align 8, !invariant.load !3, !nonnull !3
  %_0 = call i64 %1(ptr align 1 %shape.0) #6
  ret i64 %_0
}

; Function Attrs: nounwind nonlazybind uwtable
define i64 @dynamic_name(ptr align 1 %shape.0, ptr align 8 %shape.1) unnamed_addr #0 {
start:
  %0 = getelementptr inbounds i8, ptr %shape.1, i64 32
  %1 = load ptr, ptr %0, align 8, !invariant.load !3, !nonnull !3
  %2 = call { ptr, i64 } %1(ptr align 1 %shape.0) #6
  %_2.0 = extractvalue { ptr, i64 } %2, 0
  %_2.1 = extractvalue { ptr, i64 } %2, 1
  %_0 = call i64 @"_ZN4core3str21_$LT$impl$u20$str$GT$3len17h4d2dc01c6dbaf506E"(ptr align 1 %_2.0, i64 %_2.1) #6
  ret i64 %_0
}

; Function Attrs: nounwind nonlazybind uwtable
define i32 @indirect(ptr %f, i32 %x) unnamed_addr #0 {
start:
  %_0 = call i32 %f(i32 %x) #5
  ret i32 %_0
}

; Function Attrs: nounwind nonlazybind uwtable
define { i32, i1 } @overflowing(i32 %a, i32 %b) unnamed_addr #0 {
start:
  %_0.0.i = add i32 %a, %b
  %_0.1.i = icmp ult i32 %_0.0.i, %a
  %0 = insertvalue { i32, i1 } poison, i32 %_0.0.i, 0
  %1 = insertvalue { i32, i1 } %0, i1 %_0.1.i, 1
  %_0.0 = extractvalue { i32, i1 } %1, 0
  %_0.1 = extractvalue { i32, i1 } %1, 1
  %2 = insertvalue { i32, i1 } poison, i32 %_0.0, 0
  %3 = insertvalue { i32, i1 } %2, i1 %_0.1, 1
  ret { i32, i1 } %3
}

; Function Attrs: nounwind nonlazybind uwtable
define i16 @saturating(i16 %a, i16 %b) unnamed_addr #0 {
start:
  %0 = alloca [2 x i8], align 2
  %1 = call i16 @llvm.ssub.sat.i16(i16 %a, i16 %b)
  store i16 %1, ptr %0, align 2
  %_0.i = load i16, ptr %0, align 2
  ret i16 %_0.i
}

; Function Attrs: nounwind nonlazybind uwtable
define i32 @variadic(i32 %x) unnamed_addr #0 {
start:
  %_3 = call ptr @_ZN4core3ffi5c_str4CStr6as_ptr17h24339f3917ac2fdfE(ptr align 1 @alloc_87551382a9de3243abbfdbda2f0b586b, i64 4) #6
  %_0 = call i32 (ptr, ...) @printf(ptr %_3, i32 %x) #5
  ret i32 %_0
}

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare i32 @llvm.bswap.i32(i32) #2

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare i64 @llvm.ctpop.i64(i64) #2

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare i64 @llvm.ctlz.i64(i64, i1 immarg) #2

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare i64 @llvm.cttz.i64(i64, i1 immarg) #2

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare i16 @llvm.ssub.sat.i16(i16, i16) #2

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare { i64, i1 } @llvm.umul.with.overflow.i64(i64, i64) #2

; Function Attrs: cold noinline noreturn nounwind nonlazybind uwtable
declare void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const24panic_const_mul_overflow(ptr align 8) unnamed_addr #3

; Function Attrs: nounwind nonlazybind uwtable
declare i32 @plain(i32) unnamed_addr #0

; Function Attrs: nounwind nonlazybind uwtable
declare i32 @printf(ptr, ...) unnamed_addr #0

attributes #0 = { nounwind nonlazybind uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #1 = { inlinehint nounwind nonlazybind uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #2 = { nocallback nofree nosync nounwind speculatable willreturn memory(none) }
attributes #3 = { cold noinline noreturn nounwind nonlazybind uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #4 = { noinline noreturn nounwind }
attributes #5 = { nounwind }
attributes #6 = { inlinehint nounwind }

!llvm.module.flags = !{!0, !1}
!llvm.ident = !{!2}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 2, !"RtLibUseGOT", i32 1}
!2 = !{!"rustc version 1.95.0 (59807616e 2026-04-14) (built from a source tarball)"}
!3 = !{}
