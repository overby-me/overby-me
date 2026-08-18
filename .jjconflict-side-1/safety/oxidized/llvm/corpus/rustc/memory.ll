; ModuleID = 'memory.ll'
source_filename = "memory.c98feac3ea0c72fe-cgu.0"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@alloc_854c899e5ca06b77ca7e89923faf1616 = private unnamed_addr constant [214 x i8] c"unsafe precondition(s) violated: ptr::read_volatile requires that the pointer argument is aligned\0A\0AThis indicates a bug in the program. This Undefined Behavior check is optional, and cannot be relied on for safety.", align 1
@alloc_c848f501c9a24e1e115677405b6cf8e4 = private unnamed_addr constant [215 x i8] c"unsafe precondition(s) violated: ptr::write_volatile requires that the pointer argument is aligned\0A\0AThis indicates a bug in the program. This Undefined Behavior check is optional, and cannot be relied on for safety.", align 1
@alloc_b5fc2a6f2f5a9fc4395aada8c7c926bf = private unnamed_addr constant [76 x i8] c"/rustc/59807616e1fa2540724bfbac14d7976d7e4a3860/library/core/src/ptr/mod.rs\00", align 1
@alloc_37f6e60a2902ea1334fb3e649d7c5a34 = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_b5fc2a6f2f5a9fc4395aada8c7c926bf, [16 x i8] c"K\00\00\00\00\00\00\00\0F\02\00\00\05\00\00\00" }>, align 8
@alloc_bd3468a7b96187f70c1ce98a3e7a63bf = private unnamed_addr constant [283 x i8] c"unsafe precondition(s) violated: ptr::copy_nonoverlapping requires that both pointer arguments are aligned and non-null and the specified memory ranges do not overlap\0A\0AThis indicates a bug in the program. This Undefined Behavior check is optional, and cannot be relied on for safety.", align 1
@alloc_fad0cd83b7d1858a846a172eb260e593 = private unnamed_addr constant [42 x i8] c"is_aligned_to: align is not a power-of-two", align 1
@alloc_3d063512bd2a283debbda10df8c730ad = private unnamed_addr constant [82 x i8] c"/rustc/59807616e1fa2540724bfbac14d7976d7e4a3860/library/core/src/ptr/const_ptr.rs\00", align 1
@alloc_180ab55c3ad9891a0d81c48ca2ae33fa = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_3d063512bd2a283debbda10df8c730ad, [16 x i8] c"Q\00\00\00\00\00\00\00^\05\00\00\0D\00\00\00" }>, align 8
@alloc_763310d78c99c2c1ad3f8a9821e942f3 = private unnamed_addr constant [61 x i8] c"is_nonoverlapping: `size_of::<T>() * count` overflows a usize", align 1
@alloc_ce9bcd0b8e6f4a04e9569d41e1aa2a9b = private unnamed_addr constant [68 x i8] c"/home/overby.me/Work/overby.me/rust/llvm/corpus/rustc/src/memory.rs\00", align 1
@alloc_63f80bcd8c70ad91c779026710e70f7d = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_ce9bcd0b8e6f4a04e9569d41e1aa2a9b, [16 x i8] c"C\00\00\00\00\00\00\00.\00\00\00\05\00\00\00" }>, align 8
@alloc_a91c9996004d941559f81d13c0926edc = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_ce9bcd0b8e6f4a04e9569d41e1aa2a9b, [16 x i8] c"C\00\00\00\00\00\00\00;\00\00\00$\00\00\00" }>, align 8
@alloc_6b9cbf0f2808c7a8970daa6d90ba16e8 = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_ce9bcd0b8e6f4a04e9569d41e1aa2a9b, [16 x i8] c"C\00\00\00\00\00\00\00<\00\00\00\09\00\00\00" }>, align 8
@alloc_c481e64023a296d31603aee7a63de78a = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_ce9bcd0b8e6f4a04e9569d41e1aa2a9b, [16 x i8] c"C\00\00\00\00\00\00\00M\00\00\00\11\00\00\00" }>, align 8
@alloc_2c66a06554dbe5d8c54d9fd79679c158 = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_ce9bcd0b8e6f4a04e9569d41e1aa2a9b, [16 x i8] c"C\00\00\00\00\00\00\00N\00\00\00\09\00\00\00" }>, align 8

; Function Attrs: cold nounwind nonlazybind uwtable
define internal void @_ZN4core10intrinsics9cold_path17h2506609ce7083b0aE() unnamed_addr #0 {
start:
  ret void
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define i32 @_ZN4core3ptr13read_volatile17h79f166680c81e669E(ptr %src, ptr align 8 %0) unnamed_addr #1 {
start:
  %1 = alloca [4 x i8], align 4
  br label %bb1

bb1:                                              ; preds = %start
  call void @_ZN4core3ptr13read_volatile18precondition_check17he301d0b28f5f125dE(ptr %src, i64 4, ptr align 8 %0) #8
  br label %bb3

bb3:                                              ; preds = %bb1
  %2 = load volatile i32, ptr %src, align 4
  store i32 %2, ptr %1, align 4
  %_0 = load i32, ptr %1, align 4
  ret i32 %_0
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal void @_ZN4core3ptr13read_volatile18precondition_check17he301d0b28f5f125dE(ptr %addr, i64 %align, ptr align 8 %0) unnamed_addr #1 personality ptr @rust_eh_personality {
start:
  %_3 = call zeroext i1 @"_ZN4core3ptr9const_ptr33_$LT$impl$u20$$BP$const$u20$T$GT$13is_aligned_to17he5b814eb1c8b10f9E"(ptr %addr, i64 %align) #8
  br i1 %_3, label %bb1, label %bb2

bb2:                                              ; preds = %start
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking18panic_nounwind_fmt(ptr @alloc_854c899e5ca06b77ca7e89923faf1616, ptr inttoptr (i64 429 to ptr), i1 zeroext false, ptr align 8 %0) #9
  unreachable

bb1:                                              ; preds = %start
  ret void
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define void @_ZN4core3ptr14write_volatile17hba9a9bfd4f554b5cE(ptr %dst, i32 %src, ptr align 8 %0) unnamed_addr #1 {
start:
  br label %bb1

bb1:                                              ; preds = %start
  call void @_ZN4core3ptr14write_volatile18precondition_check17hcd09c992d1ab82e9E(ptr %dst, i64 4, ptr align 8 %0) #8
  br label %bb3

bb3:                                              ; preds = %bb1
  store volatile i32 %src, ptr %dst, align 4
  ret void
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal void @_ZN4core3ptr14write_volatile18precondition_check17hcd09c992d1ab82e9E(ptr %addr, i64 %align, ptr align 8 %0) unnamed_addr #1 personality ptr @rust_eh_personality {
start:
  %_3 = call zeroext i1 @"_ZN4core3ptr9const_ptr33_$LT$impl$u20$$BP$const$u20$T$GT$13is_aligned_to17he5b814eb1c8b10f9E"(ptr %addr, i64 %align) #8
  br i1 %_3, label %bb1, label %bb2

bb2:                                              ; preds = %start
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking18panic_nounwind_fmt(ptr @alloc_c848f501c9a24e1e115677405b6cf8e4, ptr inttoptr (i64 431 to ptr), i1 zeroext false, ptr align 8 %0) #9
  unreachable

bb1:                                              ; preds = %start
  ret void
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal void @_ZN4core3ptr19copy_nonoverlapping18precondition_check17h5262ec09b3b232b7E(ptr %src, ptr %dst, i64 %size, i64 %align, i64 %count, ptr align 8 %0) unnamed_addr #1 personality ptr @rust_eh_personality {
start:
  %zero_size = alloca [1 x i8], align 1
  %1 = icmp eq i64 %count, 0
  br i1 %1, label %bb1, label %bb2

bb1:                                              ; preds = %start
  store i8 1, ptr %zero_size, align 1
  br label %bb3

bb2:                                              ; preds = %start
  %2 = icmp eq i64 %size, 0
  %3 = zext i1 %2 to i8
  store i8 %3, ptr %zero_size, align 1
  br label %bb3

bb3:                                              ; preds = %bb2, %bb1
  %4 = load i8, ptr %zero_size, align 1
  %is_zst = trunc nuw i8 %4 to i1
  %_15 = call zeroext i1 @"_ZN4core3ptr9const_ptr33_$LT$impl$u20$$BP$const$u20$T$GT$13is_aligned_to17he5b814eb1c8b10f9E"(ptr %src, i64 %align) #8
  br i1 %_15, label %bb11, label %bb12

bb12:                                             ; preds = %bb3
  br label %bb7

bb11:                                             ; preds = %bb3
  br i1 %is_zst, label %bb13, label %bb14

bb7:                                              ; preds = %bb14, %bb12
  br label %bb8

bb14:                                             ; preds = %bb11
  %_17 = ptrtoint ptr %src to i64
  %_16 = icmp eq i64 %_17, 0
  %_8 = xor i1 %_16, true
  br i1 %_8, label %bb4, label %bb7

bb13:                                             ; preds = %bb11
  br label %bb4

bb4:                                              ; preds = %bb13, %bb14
  %_18 = call zeroext i1 @"_ZN4core3ptr9const_ptr33_$LT$impl$u20$$BP$const$u20$T$GT$13is_aligned_to17he5b814eb1c8b10f9E"(ptr %dst, i64 %align) #8
  br i1 %_18, label %bb16, label %bb17

bb8:                                              ; preds = %bb6, %bb7
  br label %bb9

bb17:                                             ; preds = %bb4
  br label %bb6

bb16:                                             ; preds = %bb4
  %5 = load i8, ptr %zero_size, align 1
  %6 = trunc nuw i8 %5 to i1
  br i1 %6, label %bb18, label %bb19

bb6:                                              ; preds = %bb19, %bb17
  br label %bb8

bb19:                                             ; preds = %bb16
  %_20 = ptrtoint ptr %dst to i64
  %_19 = icmp eq i64 %_20, 0
  %_10 = xor i1 %_19, true
  br i1 %_10, label %bb5, label %bb6

bb18:                                             ; preds = %bb16
  br label %bb5

bb5:                                              ; preds = %bb18, %bb19
  %_6 = call zeroext i1 @_ZN4core9ub_checks23maybe_is_nonoverlapping7runtime17h9a3a1e3643d57445E(ptr %src, ptr %dst, i64 %size, i64 %count) #8
  br i1 %_6, label %bb10, label %bb9

bb9:                                              ; preds = %bb5, %bb8
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking18panic_nounwind_fmt(ptr @alloc_bd3468a7b96187f70c1ce98a3e7a63bf, ptr inttoptr (i64 567 to ptr), i1 zeroext false, ptr align 8 %0) #9
  unreachable

bb10:                                             ; preds = %bb5
  ret void
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define zeroext i1 @"_ZN4core3ptr9const_ptr33_$LT$impl$u20$$BP$const$u20$T$GT$13is_aligned_to17he5b814eb1c8b10f9E"(ptr %self, i64 %align) unnamed_addr #1 {
start:
  %0 = alloca [4 x i8], align 4
  %1 = call i64 @llvm.ctpop.i64(i64 %align)
  %2 = trunc i64 %1 to i32
  store i32 %2, ptr %0, align 4
  %_8 = load i32, ptr %0, align 4
  %3 = icmp eq i32 %_8, 1
  br i1 %3, label %bb1, label %bb2

bb1:                                              ; preds = %start
  %_6 = ptrtoint ptr %self to i64
  %_7 = sub i64 %align, 1
  %_5 = and i64 %_6, %_7
  %_0 = icmp eq i64 %_5, 0
  ret i1 %_0

bb2:                                              ; preds = %start
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking9panic_fmt(ptr @alloc_fad0cd83b7d1858a846a172eb260e593, ptr inttoptr (i64 85 to ptr), ptr align 8 @alloc_180ab55c3ad9891a0d81c48ca2ae33fa) #9
  unreachable
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal zeroext i1 @_ZN4core9ub_checks23maybe_is_nonoverlapping7runtime17h9a3a1e3643d57445E(ptr %src, ptr %dst, i64 %size, i64 %count) unnamed_addr #1 {
start:
  %diff = alloca [8 x i8], align 8
  %_9 = alloca [16 x i8], align 8
  %src_usize = ptrtoint ptr %src to i64
  %dst_usize = ptrtoint ptr %dst to i64
  %0 = call { i64, i1 } @llvm.umul.with.overflow.i64(i64 %size, i64 %count)
  %_13.0 = extractvalue { i64, i1 } %0, 0
  %_13.1 = extractvalue { i64, i1 } %0, 1
  br i1 %_13.1, label %bb1, label %bb3

bb3:                                              ; preds = %start
  %1 = getelementptr inbounds i8, ptr %_9, i64 8
  store i64 %_13.0, ptr %1, align 8
  store i64 1, ptr %_9, align 8
  %2 = getelementptr inbounds i8, ptr %_9, i64 8
  %size1 = load i64, ptr %2, align 8
  %_21 = icmp ult i64 %src_usize, %dst_usize
  br i1 %_21, label %bb4, label %bb5

bb1:                                              ; preds = %start
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking14panic_nounwind(ptr align 1 @alloc_763310d78c99c2c1ad3f8a9821e942f3, i64 61) #9
  unreachable

bb5:                                              ; preds = %bb3
  %3 = sub i64 %src_usize, %dst_usize
  store i64 %3, ptr %diff, align 8
  br label %bb6

bb4:                                              ; preds = %bb3
  %4 = sub i64 %dst_usize, %src_usize
  store i64 %4, ptr %diff, align 8
  br label %bb6

bb6:                                              ; preds = %bb4, %bb5
  %5 = load i64, ptr %diff, align 8
  %_0 = icmp uge i64 %5, %size1
  ret i1 %_0
}

; Function Attrs: nounwind nonlazybind uwtable
define void @build(ptr sret([16 x i8]) align 8 %_0, i8 %tag, i32 %length) unnamed_addr #2 {
start:
  store i8 %tag, ptr %_0, align 8
  %0 = getelementptr inbounds i8, ptr %_0, i64 4
  store i32 %length, ptr %0, align 4
  %1 = getelementptr inbounds i8, ptr %_0, i64 8
  store ptr null, ptr %1, align 8
  ret void
}

; Function Attrs: nounwind nonlazybind uwtable
define i32 @index_slice(ptr align 4 %xs.0, i64 %xs.1, i64 %i) unnamed_addr #2 {
start:
  %_4 = icmp ult i64 %i, %xs.1
  br i1 %_4, label %bb1, label %panic

bb1:                                              ; preds = %start
  %0 = getelementptr inbounds nuw i32, ptr %xs.0, i64 %i
  %_0 = load i32, ptr %0, align 4
  ret i32 %_0

panic:                                            ; preds = %start
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking18panic_bounds_check(i64 %i, i64 %xs.1, ptr align 8 @alloc_63f80bcd8c70ad91c779026710e70f7d) #9
  unreachable
}

; Function Attrs: nounwind nonlazybind uwtable
define i16 @nested_field(ptr align 8 %n) unnamed_addr #2 {
start:
  %0 = getelementptr inbounds i8, ptr %n, i64 16
  %1 = getelementptr inbounds nuw i16, ptr %0, i64 1
  %_0 = load i16, ptr %1, align 2
  ret i16 %_0
}

; Function Attrs: nounwind nonlazybind uwtable
define void @raw_copy(ptr %dst, ptr %src, i64 %n) unnamed_addr #2 {
start:
  call void @_ZN4core3ptr19copy_nonoverlapping18precondition_check17h5262ec09b3b232b7E(ptr %src, ptr %dst, i64 1, i64 1, i64 %n, ptr align 8 @alloc_37f6e60a2902ea1334fb3e649d7c5a34) #8
  call void @llvm.memcpy.p0.p0.i64(ptr align 1 %dst, ptr align 1 %src, i64 %n, i1 false)
  ret void
}

; Function Attrs: nounwind nonlazybind uwtable
define i32 @read_field(ptr align 8 %h) unnamed_addr #2 {
start:
  %0 = getelementptr inbounds i8, ptr %h, i64 4
  %_0 = load i32, ptr %0, align 4
  ret i32 %_0
}

; Function Attrs: nounwind nonlazybind uwtable
define i64 @slice_length(ptr align 4 %xs.0, i64 %xs.1) unnamed_addr #2 {
start:
  ret i64 %xs.1
}

; Function Attrs: nounwind nonlazybind uwtable
define i64 @sum_array(ptr align 8 %xs) unnamed_addr #2 {
start:
  %i = alloca [8 x i8], align 8
  %total = alloca [8 x i8], align 8
  store i64 0, ptr %total, align 8
  store i64 0, ptr %i, align 8
  br label %bb1

bb1:                                              ; preds = %bb5, %start
  %_5 = load i64, ptr %i, align 8
  %_4 = icmp ult i64 %_5, 8
  br i1 %_4, label %bb2, label %bb6

bb6:                                              ; preds = %bb1
  %_0 = load i64, ptr %total, align 8
  ret i64 %_0

bb2:                                              ; preds = %bb1
  %_7 = load i64, ptr %total, align 8
  %_9 = load i64, ptr %i, align 8
  %_10 = icmp ult i64 %_9, 8
  br i1 %_10, label %bb3, label %panic

bb3:                                              ; preds = %bb2
  %0 = getelementptr inbounds nuw i64, ptr %xs, i64 %_9
  %_8 = load i64, ptr %0, align 8
  %_0.i = add i64 %_7, %_8
  store i64 %_0.i, ptr %total, align 8
  %1 = load i64, ptr %i, align 8
  %_11.0 = add i64 %1, 1
  %_11.1 = icmp ult i64 %_11.0, %1
  br i1 %_11.1, label %panic1, label %bb5

panic:                                            ; preds = %bb2
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking18panic_bounds_check(i64 %_9, i64 8, ptr align 8 @alloc_a91c9996004d941559f81d13c0926edc) #9
  unreachable

bb5:                                              ; preds = %bb3
  store i64 %_11.0, ptr %i, align 8
  br label %bb1

panic1:                                           ; preds = %bb3
  call void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const24panic_const_add_overflow(ptr align 8 @alloc_6b9cbf0f2808c7a8970daa6d90ba16e8) #9
  unreachable
}

; Function Attrs: nounwind nonlazybind uwtable
define { i32, i64 } @tuple_return(i32 %a, i64 %b) unnamed_addr #2 {
start:
  %0 = insertvalue { i32, i64 } poison, i32 %a, 0
  %1 = insertvalue { i32, i64 } %0, i64 %b, 1
  ret { i32, i64 } %1
}

; Function Attrs: nounwind nonlazybind uwtable
define i32 @volatile_touch(ptr %p) unnamed_addr #2 {
start:
  %v = call i32 @_ZN4core3ptr13read_volatile17h79f166680c81e669E(ptr %p, ptr align 8 @alloc_c481e64023a296d31603aee7a63de78a) #8
  %_0.i = add i32 %v, 1
  call void @_ZN4core3ptr14write_volatile17hba9a9bfd4f554b5cE(ptr %p, i32 %_0.i, ptr align 8 @alloc_2c66a06554dbe5d8c54d9fd79679c158) #8
  ret i32 %v
}

; Function Attrs: nounwind nonlazybind uwtable
define void @write_field(ptr align 8 %h, i32 %value) unnamed_addr #2 {
start:
  %0 = getelementptr inbounds i8, ptr %h, i64 4
  store i32 %value, ptr %0, align 4
  ret void
}

; Function Attrs: nonlazybind
declare i32 @rust_eh_personality(...) unnamed_addr #3

; Function Attrs: cold noinline noreturn nounwind nonlazybind uwtable
declare void @_RNvNtCs4uthzyWeO2a_4core9panicking18panic_nounwind_fmt(ptr, ptr, i1 zeroext, ptr align 8) unnamed_addr #4

; Function Attrs: nocallback nofree nounwind willreturn memory(argmem: readwrite)
declare void @llvm.memcpy.p0.p0.i64(ptr noalias writeonly captures(none), ptr noalias readonly captures(none), i64, i1 immarg) #5

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare i64 @llvm.ctpop.i64(i64) #6

; Function Attrs: cold noinline noreturn nounwind nonlazybind uwtable
declare void @_RNvNtCs4uthzyWeO2a_4core9panicking9panic_fmt(ptr, ptr, ptr align 8) unnamed_addr #4

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare { i64, i1 } @llvm.umul.with.overflow.i64(i64, i64) #6

; Function Attrs: cold noinline noreturn nounwind nonlazybind uwtable
declare void @_RNvNtCs4uthzyWeO2a_4core9panicking14panic_nounwind(ptr align 1, i64) unnamed_addr #4

; Function Attrs: cold minsize noinline noreturn nounwind nonlazybind optsize uwtable
declare void @_RNvNtCs4uthzyWeO2a_4core9panicking18panic_bounds_check(i64, i64, ptr align 8) unnamed_addr #7

; Function Attrs: cold noinline noreturn nounwind nonlazybind uwtable
declare void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const24panic_const_add_overflow(ptr align 8) unnamed_addr #4

attributes #0 = { cold nounwind nonlazybind uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #1 = { inlinehint nounwind nonlazybind uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #2 = { nounwind nonlazybind uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #3 = { nonlazybind "target-cpu"="x86-64" }
attributes #4 = { cold noinline noreturn nounwind nonlazybind uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #5 = { nocallback nofree nounwind willreturn memory(argmem: readwrite) }
attributes #6 = { nocallback nofree nosync nounwind speculatable willreturn memory(none) }
attributes #7 = { cold minsize noinline noreturn nounwind nonlazybind optsize uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #8 = { inlinehint nounwind }
attributes #9 = { noinline noreturn nounwind }

!llvm.module.flags = !{!0, !1}
!llvm.ident = !{!2}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 2, !"RtLibUseGOT", i32 1}
!2 = !{!"rustc version 1.95.0 (59807616e 2026-04-14) (built from a source tarball)"}
