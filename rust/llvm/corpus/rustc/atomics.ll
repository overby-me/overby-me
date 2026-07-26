; ModuleID = 'atomics.ll'
source_filename = "atomics.16703449c27540f0-cgu.0"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@alloc_9cd20c3e415f4d39f0ceb012cb758628 = private unnamed_addr constant [40 x i8] c"there is no such thing as a release load", align 1
@alloc_e17502e153481471943c37e20438384b = private unnamed_addr constant [80 x i8] c"/rustc/59807616e1fa2540724bfbac14d7976d7e4a3860/library/core/src/sync/atomic.rs\00", align 1
@alloc_b7d26e4bc52b7f02cc25e37f0bf633e4 = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_e17502e153481471943c37e20438384b, [16 x i8] c"O\00\00\00\00\00\00\005\0F\00\00\18\00\00\00" }>, align 8
@alloc_96ab912d0054b46da785b206a96c9a45 = private unnamed_addr constant [49 x i8] c"there is no such thing as an acquire-release load", align 1
@alloc_738c866d826467e54726b360a2118a62 = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_e17502e153481471943c37e20438384b, [16 x i8] c"O\00\00\00\00\00\00\006\0F\00\00\17\00\00\00" }>, align 8
@alloc_bf39103a6db665396aab4632362d9353 = private unnamed_addr constant [42 x i8] c"there is no such thing as an acquire store", align 1
@alloc_a574beff59ab58cf5ed2d6bc163e2302 = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_e17502e153481471943c37e20438384b, [16 x i8] c"O\00\00\00\00\00\00\00&\0F\00\00\18\00\00\00" }>, align 8
@alloc_00c0bce0fa6327f8ec8e69d6d765d508 = private unnamed_addr constant [50 x i8] c"there is no such thing as an acquire-release store", align 1
@alloc_aec0388915b72a4f610ad656c35be884 = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_e17502e153481471943c37e20438384b, [16 x i8] c"O\00\00\00\00\00\00\00'\0F\00\00\17\00\00\00" }>, align 8
@alloc_929e9b2e7b7429614ca4fc017efff666 = private unnamed_addr constant [41 x i8] c"there is no such thing as a relaxed fence", align 1
@alloc_8bb455cbc96bda1beb58cc2846df1309 = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_e17502e153481471943c37e20438384b, [16 x i8] c"O\00\00\00\00\00\00\00g\11\00\00\18\00\00\00" }>, align 8
@alloc_5a43f8d94dd4505c1dba43832ce73af8 = private unnamed_addr constant [52 x i8] c"there is no such thing as a release failure ordering", align 1
@alloc_9358981a0abd66965a2745829d6abd34 = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_e17502e153481471943c37e20438384b, [16 x i8] c"O\00\00\00\00\00\00\00\AB\0F\00\00\1D\00\00\00" }>, align 8
@alloc_7adef5546d83b439c7829602020737c6 = private unnamed_addr constant [61 x i8] c"there is no such thing as an acquire-release failure ordering", align 1
@alloc_5d2b33ea973198a6afb110fdf3882ee7 = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_e17502e153481471943c37e20438384b, [16 x i8] c"O\00\00\00\00\00\00\00\AA\0F\00\00\1C\00\00\00" }>, align 8
@alloc_1dbbdb33ce56ab0f629d5d081745ad16 = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_e17502e153481471943c37e20438384b, [16 x i8] c"O\00\00\00\00\00\00\00\EC\0F\00\00\1D\00\00\00" }>, align 8
@alloc_11a7b6bb3417a8fba26ae9c100d23a1b = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_e17502e153481471943c37e20438384b, [16 x i8] c"O\00\00\00\00\00\00\00\EB\0F\00\00\1C\00\00\00" }>, align 8
@alloc_ea5b21d5fc560c6358071a0315535da4 = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_e17502e153481471943c37e20438384b, [16 x i8] c"O\00\00\00\00\00\00\00\19\11\00\00\18\00\00\00" }>, align 8
@_ZN7atomics4FLAG17ha641e63b27ffe2ecE = global [1 x i8] zeroinitializer, align 1
@_ZN7atomics4SIZE17h8375f3d670919c9fE = global [8 x i8] zeroinitializer, align 8
@_ZN7atomics6SIGNED17h98aac63ff57bf7f3E = global [4 x i8] c"\FF\FF\FF\FF", align 4
@_ZN7atomics7COUNTER17h0bd1eb976af6a3e2E = global [8 x i8] zeroinitializer, align 8
@_ZN7atomics7POINTER17hef8592fb3d4a7165E = global [8 x i8] zeroinitializer, align 8

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal void @_ZN4core4sync6atomic11AtomicIsize5store17h30b3333c67ca0704E(ptr align 8 %self, i64 %val, i8 %order) unnamed_addr #0 {
start:
  call void @_ZN4core4sync6atomic12atomic_store17hd200c696e87f2538E(ptr %self, i64 %val, i8 %order) #3
  ret void
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define ptr @_ZN4core4sync6atomic11atomic_load17habe85c91c4399fa5E(ptr %dst, i8 %order) unnamed_addr #0 {
start:
  %_0 = alloca [8 x i8], align 8
  %_3 = zext i8 %order to i64
  switch i64 %_3, label %bb1 [
    i64 0, label %bb6
    i64 1, label %bb3
    i64 2, label %bb5
    i64 3, label %bb2
    i64 4, label %bb4
  ]

bb1:                                              ; preds = %start
  unreachable

bb6:                                              ; preds = %start
  %0 = load atomic ptr, ptr %dst monotonic, align 8
  store ptr %0, ptr %_0, align 8
  br label %bb7

bb3:                                              ; preds = %start
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking9panic_fmt(ptr @alloc_9cd20c3e415f4d39f0ceb012cb758628, ptr inttoptr (i64 81 to ptr), ptr align 8 @alloc_b7d26e4bc52b7f02cc25e37f0bf633e4) #4
  unreachable

bb5:                                              ; preds = %start
  %1 = load atomic ptr, ptr %dst acquire, align 8
  store ptr %1, ptr %_0, align 8
  br label %bb7

bb2:                                              ; preds = %start
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking9panic_fmt(ptr @alloc_96ab912d0054b46da785b206a96c9a45, ptr inttoptr (i64 99 to ptr), ptr align 8 @alloc_738c866d826467e54726b360a2118a62) #4
  unreachable

bb4:                                              ; preds = %start
  %2 = load atomic ptr, ptr %dst seq_cst, align 8
  store ptr %2, ptr %_0, align 8
  br label %bb7

bb7:                                              ; preds = %bb4, %bb5, %bb6
  %3 = load ptr, ptr %_0, align 8
  ret ptr %3
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define void @_ZN4core4sync6atomic12atomic_store17h5a8be9d899bb4a5fE(ptr %dst, ptr %val, i8 %order) unnamed_addr #0 {
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
  store atomic ptr %val, ptr %dst monotonic, align 8
  br label %bb7

bb5:                                              ; preds = %start
  store atomic ptr %val, ptr %dst release, align 8
  br label %bb7

bb3:                                              ; preds = %start
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking9panic_fmt(ptr @alloc_bf39103a6db665396aab4632362d9353, ptr inttoptr (i64 85 to ptr), ptr align 8 @alloc_a574beff59ab58cf5ed2d6bc163e2302) #4
  unreachable

bb2:                                              ; preds = %start
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking9panic_fmt(ptr @alloc_00c0bce0fa6327f8ec8e69d6d765d508, ptr inttoptr (i64 101 to ptr), ptr align 8 @alloc_aec0388915b72a4f610ad656c35be884) #4
  unreachable

bb4:                                              ; preds = %start
  store atomic ptr %val, ptr %dst seq_cst, align 8
  br label %bb7

bb7:                                              ; preds = %bb4, %bb5, %bb6
  ret void
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define void @_ZN4core4sync6atomic12atomic_store17hd200c696e87f2538E(ptr %dst, i64 %val, i8 %order) unnamed_addr #0 {
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
  store atomic i64 %val, ptr %dst monotonic, align 8
  br label %bb7

bb5:                                              ; preds = %start
  store atomic i64 %val, ptr %dst release, align 8
  br label %bb7

bb3:                                              ; preds = %start
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking9panic_fmt(ptr @alloc_bf39103a6db665396aab4632362d9353, ptr inttoptr (i64 85 to ptr), ptr align 8 @alloc_a574beff59ab58cf5ed2d6bc163e2302) #4
  unreachable

bb2:                                              ; preds = %start
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking9panic_fmt(ptr @alloc_00c0bce0fa6327f8ec8e69d6d765d508, ptr inttoptr (i64 101 to ptr), ptr align 8 @alloc_aec0388915b72a4f610ad656c35be884) #4
  unreachable

bb4:                                              ; preds = %start
  store atomic i64 %val, ptr %dst seq_cst, align 8
  br label %bb7

bb7:                                              ; preds = %bb4, %bb5, %bb6
  ret void
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal void @_ZN4core4sync6atomic14compiler_fence17h6cff920f5dfa0f6aE(i8 %order) unnamed_addr #0 {
start:
  %_2 = zext i8 %order to i64
  switch i64 %_2, label %bb1 [
    i64 0, label %bb2
    i64 1, label %bb5
    i64 2, label %bb6
    i64 3, label %bb4
    i64 4, label %bb3
  ]

bb1:                                              ; preds = %start
  unreachable

bb2:                                              ; preds = %start
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking9panic_fmt(ptr @alloc_929e9b2e7b7429614ca4fc017efff666, ptr inttoptr (i64 83 to ptr), ptr align 8 @alloc_8bb455cbc96bda1beb58cc2846df1309) #4
  unreachable

bb5:                                              ; preds = %start
  fence syncscope("singlethread") release
  br label %bb7

bb6:                                              ; preds = %start
  fence syncscope("singlethread") acquire
  br label %bb7

bb4:                                              ; preds = %start
  fence syncscope("singlethread") acq_rel
  br label %bb7

bb3:                                              ; preds = %start
  fence syncscope("singlethread") seq_cst
  br label %bb7

bb7:                                              ; preds = %bb3, %bb4, %bb6, %bb5
  ret void
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define ptr @"_ZN4core4sync6atomic18AtomicPtr$LT$T$GT$4load17hf5950fa70c23e0e3E"(ptr align 8 %self, i8 %order) unnamed_addr #0 {
start:
  %_0 = call ptr @_ZN4core4sync6atomic11atomic_load17habe85c91c4399fa5E(ptr %self, i8 %order) #3
  ret ptr %_0
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define void @"_ZN4core4sync6atomic18AtomicPtr$LT$T$GT$5store17h0024fedcf9a48c91E"(ptr align 8 %self, ptr %ptr, i8 %order) unnamed_addr #0 {
start:
  call void @_ZN4core4sync6atomic12atomic_store17h5a8be9d899bb4a5fE(ptr %self, ptr %ptr, i8 %order) #3
  ret void
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define { i64, i64 } @_ZN4core4sync6atomic23atomic_compare_exchange17heedd5910b8b17177E(ptr %dst, i64 %old, i64 %new, i8 %success, i8 %failure) unnamed_addr #0 {
start:
  %_8 = alloca [16 x i8], align 8
  %_0 = alloca [16 x i8], align 8
  %_14 = zext i8 %success to i64
  switch i64 %_14, label %bb7 [
    i64 0, label %bb2
    i64 1, label %bb4
    i64 2, label %bb3
    i64 3, label %bb5
    i64 4, label %bb6
  ]

bb7:                                              ; preds = %start
  unreachable

bb2:                                              ; preds = %start
  %_9 = zext i8 %failure to i64
  switch i64 %_9, label %bb1 [
    i64 0, label %bb24
    i64 2, label %bb23
    i64 4, label %bb22
  ]

bb4:                                              ; preds = %start
  %_11 = zext i8 %failure to i64
  switch i64 %_11, label %bb1 [
    i64 0, label %bb18
    i64 2, label %bb17
    i64 4, label %bb16
  ]

bb3:                                              ; preds = %start
  %_10 = zext i8 %failure to i64
  switch i64 %_10, label %bb1 [
    i64 0, label %bb21
    i64 2, label %bb20
    i64 4, label %bb19
  ]

bb5:                                              ; preds = %start
  %_12 = zext i8 %failure to i64
  switch i64 %_12, label %bb1 [
    i64 0, label %bb15
    i64 2, label %bb14
    i64 4, label %bb13
  ]

bb6:                                              ; preds = %start
  %_13 = zext i8 %failure to i64
  switch i64 %_13, label %bb1 [
    i64 0, label %bb12
    i64 2, label %bb11
    i64 4, label %bb10
  ]

bb1:                                              ; preds = %bb6, %bb5, %bb3, %bb4, %bb2
  %_15 = zext i8 %failure to i64
  %0 = icmp eq i64 %_15, 1
  br i1 %0, label %bb8, label %bb9

bb24:                                             ; preds = %bb2
  %1 = cmpxchg ptr %dst, i64 %old, i64 %new monotonic monotonic, align 8
  %2 = extractvalue { i64, i1 } %1, 0
  %3 = extractvalue { i64, i1 } %1, 1
  %4 = zext i1 %3 to i8
  store i64 %2, ptr %_8, align 8
  %5 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %4, ptr %5, align 8
  br label %bb25

bb23:                                             ; preds = %bb2
  %6 = cmpxchg ptr %dst, i64 %old, i64 %new monotonic acquire, align 8
  %7 = extractvalue { i64, i1 } %6, 0
  %8 = extractvalue { i64, i1 } %6, 1
  %9 = zext i1 %8 to i8
  store i64 %7, ptr %_8, align 8
  %10 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %9, ptr %10, align 8
  br label %bb25

bb22:                                             ; preds = %bb2
  %11 = cmpxchg ptr %dst, i64 %old, i64 %new monotonic seq_cst, align 8
  %12 = extractvalue { i64, i1 } %11, 0
  %13 = extractvalue { i64, i1 } %11, 1
  %14 = zext i1 %13 to i8
  store i64 %12, ptr %_8, align 8
  %15 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %14, ptr %15, align 8
  br label %bb25

bb25:                                             ; preds = %bb10, %bb11, %bb12, %bb13, %bb14, %bb15, %bb19, %bb20, %bb21, %bb16, %bb17, %bb18, %bb22, %bb23, %bb24
  %val = load i64, ptr %_8, align 8
  %16 = getelementptr inbounds i8, ptr %_8, i64 8
  %17 = load i8, ptr %16, align 8
  %ok = trunc nuw i8 %17 to i1
  br i1 %ok, label %bb26, label %bb27

bb18:                                             ; preds = %bb4
  %18 = cmpxchg ptr %dst, i64 %old, i64 %new release monotonic, align 8
  %19 = extractvalue { i64, i1 } %18, 0
  %20 = extractvalue { i64, i1 } %18, 1
  %21 = zext i1 %20 to i8
  store i64 %19, ptr %_8, align 8
  %22 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %21, ptr %22, align 8
  br label %bb25

bb17:                                             ; preds = %bb4
  %23 = cmpxchg ptr %dst, i64 %old, i64 %new release acquire, align 8
  %24 = extractvalue { i64, i1 } %23, 0
  %25 = extractvalue { i64, i1 } %23, 1
  %26 = zext i1 %25 to i8
  store i64 %24, ptr %_8, align 8
  %27 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %26, ptr %27, align 8
  br label %bb25

bb16:                                             ; preds = %bb4
  %28 = cmpxchg ptr %dst, i64 %old, i64 %new release seq_cst, align 8
  %29 = extractvalue { i64, i1 } %28, 0
  %30 = extractvalue { i64, i1 } %28, 1
  %31 = zext i1 %30 to i8
  store i64 %29, ptr %_8, align 8
  %32 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %31, ptr %32, align 8
  br label %bb25

bb21:                                             ; preds = %bb3
  %33 = cmpxchg ptr %dst, i64 %old, i64 %new acquire monotonic, align 8
  %34 = extractvalue { i64, i1 } %33, 0
  %35 = extractvalue { i64, i1 } %33, 1
  %36 = zext i1 %35 to i8
  store i64 %34, ptr %_8, align 8
  %37 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %36, ptr %37, align 8
  br label %bb25

bb20:                                             ; preds = %bb3
  %38 = cmpxchg ptr %dst, i64 %old, i64 %new acquire acquire, align 8
  %39 = extractvalue { i64, i1 } %38, 0
  %40 = extractvalue { i64, i1 } %38, 1
  %41 = zext i1 %40 to i8
  store i64 %39, ptr %_8, align 8
  %42 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %41, ptr %42, align 8
  br label %bb25

bb19:                                             ; preds = %bb3
  %43 = cmpxchg ptr %dst, i64 %old, i64 %new acquire seq_cst, align 8
  %44 = extractvalue { i64, i1 } %43, 0
  %45 = extractvalue { i64, i1 } %43, 1
  %46 = zext i1 %45 to i8
  store i64 %44, ptr %_8, align 8
  %47 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %46, ptr %47, align 8
  br label %bb25

bb15:                                             ; preds = %bb5
  %48 = cmpxchg ptr %dst, i64 %old, i64 %new acq_rel monotonic, align 8
  %49 = extractvalue { i64, i1 } %48, 0
  %50 = extractvalue { i64, i1 } %48, 1
  %51 = zext i1 %50 to i8
  store i64 %49, ptr %_8, align 8
  %52 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %51, ptr %52, align 8
  br label %bb25

bb14:                                             ; preds = %bb5
  %53 = cmpxchg ptr %dst, i64 %old, i64 %new acq_rel acquire, align 8
  %54 = extractvalue { i64, i1 } %53, 0
  %55 = extractvalue { i64, i1 } %53, 1
  %56 = zext i1 %55 to i8
  store i64 %54, ptr %_8, align 8
  %57 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %56, ptr %57, align 8
  br label %bb25

bb13:                                             ; preds = %bb5
  %58 = cmpxchg ptr %dst, i64 %old, i64 %new acq_rel seq_cst, align 8
  %59 = extractvalue { i64, i1 } %58, 0
  %60 = extractvalue { i64, i1 } %58, 1
  %61 = zext i1 %60 to i8
  store i64 %59, ptr %_8, align 8
  %62 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %61, ptr %62, align 8
  br label %bb25

bb12:                                             ; preds = %bb6
  %63 = cmpxchg ptr %dst, i64 %old, i64 %new seq_cst monotonic, align 8
  %64 = extractvalue { i64, i1 } %63, 0
  %65 = extractvalue { i64, i1 } %63, 1
  %66 = zext i1 %65 to i8
  store i64 %64, ptr %_8, align 8
  %67 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %66, ptr %67, align 8
  br label %bb25

bb11:                                             ; preds = %bb6
  %68 = cmpxchg ptr %dst, i64 %old, i64 %new seq_cst acquire, align 8
  %69 = extractvalue { i64, i1 } %68, 0
  %70 = extractvalue { i64, i1 } %68, 1
  %71 = zext i1 %70 to i8
  store i64 %69, ptr %_8, align 8
  %72 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %71, ptr %72, align 8
  br label %bb25

bb10:                                             ; preds = %bb6
  %73 = cmpxchg ptr %dst, i64 %old, i64 %new seq_cst seq_cst, align 8
  %74 = extractvalue { i64, i1 } %73, 0
  %75 = extractvalue { i64, i1 } %73, 1
  %76 = zext i1 %75 to i8
  store i64 %74, ptr %_8, align 8
  %77 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %76, ptr %77, align 8
  br label %bb25

bb27:                                             ; preds = %bb25
  %78 = getelementptr inbounds i8, ptr %_0, i64 8
  store i64 %val, ptr %78, align 8
  store i64 1, ptr %_0, align 8
  br label %bb28

bb26:                                             ; preds = %bb25
  %79 = getelementptr inbounds i8, ptr %_0, i64 8
  store i64 %val, ptr %79, align 8
  store i64 0, ptr %_0, align 8
  br label %bb28

bb28:                                             ; preds = %bb26, %bb27
  %80 = load i64, ptr %_0, align 8
  %81 = getelementptr inbounds i8, ptr %_0, i64 8
  %82 = load i64, ptr %81, align 8
  %83 = insertvalue { i64, i64 } poison, i64 %80, 0
  %84 = insertvalue { i64, i64 } %83, i64 %82, 1
  ret { i64, i64 } %84

bb8:                                              ; preds = %bb1
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking9panic_fmt(ptr @alloc_5a43f8d94dd4505c1dba43832ce73af8, ptr inttoptr (i64 105 to ptr), ptr align 8 @alloc_9358981a0abd66965a2745829d6abd34) #4
  unreachable

bb9:                                              ; preds = %bb1
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking9panic_fmt(ptr @alloc_7adef5546d83b439c7829602020737c6, ptr inttoptr (i64 123 to ptr), ptr align 8 @alloc_5d2b33ea973198a6afb110fdf3882ee7) #4
  unreachable
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define { i64, i64 } @_ZN4core4sync6atomic28atomic_compare_exchange_weak17h38f1d7afc9aa7672E(ptr %dst, i64 %old, i64 %new, i8 %success, i8 %failure) unnamed_addr #0 {
start:
  %_8 = alloca [16 x i8], align 8
  %_0 = alloca [16 x i8], align 8
  %_14 = zext i8 %success to i64
  switch i64 %_14, label %bb7 [
    i64 0, label %bb2
    i64 1, label %bb4
    i64 2, label %bb3
    i64 3, label %bb5
    i64 4, label %bb6
  ]

bb7:                                              ; preds = %start
  unreachable

bb2:                                              ; preds = %start
  %_9 = zext i8 %failure to i64
  switch i64 %_9, label %bb1 [
    i64 0, label %bb24
    i64 2, label %bb23
    i64 4, label %bb22
  ]

bb4:                                              ; preds = %start
  %_11 = zext i8 %failure to i64
  switch i64 %_11, label %bb1 [
    i64 0, label %bb18
    i64 2, label %bb17
    i64 4, label %bb16
  ]

bb3:                                              ; preds = %start
  %_10 = zext i8 %failure to i64
  switch i64 %_10, label %bb1 [
    i64 0, label %bb21
    i64 2, label %bb20
    i64 4, label %bb19
  ]

bb5:                                              ; preds = %start
  %_12 = zext i8 %failure to i64
  switch i64 %_12, label %bb1 [
    i64 0, label %bb15
    i64 2, label %bb14
    i64 4, label %bb13
  ]

bb6:                                              ; preds = %start
  %_13 = zext i8 %failure to i64
  switch i64 %_13, label %bb1 [
    i64 0, label %bb12
    i64 2, label %bb11
    i64 4, label %bb10
  ]

bb1:                                              ; preds = %bb6, %bb5, %bb3, %bb4, %bb2
  %_15 = zext i8 %failure to i64
  %0 = icmp eq i64 %_15, 1
  br i1 %0, label %bb8, label %bb9

bb24:                                             ; preds = %bb2
  %1 = cmpxchg weak ptr %dst, i64 %old, i64 %new monotonic monotonic, align 8
  %2 = extractvalue { i64, i1 } %1, 0
  %3 = extractvalue { i64, i1 } %1, 1
  %4 = zext i1 %3 to i8
  store i64 %2, ptr %_8, align 8
  %5 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %4, ptr %5, align 8
  br label %bb25

bb23:                                             ; preds = %bb2
  %6 = cmpxchg weak ptr %dst, i64 %old, i64 %new monotonic acquire, align 8
  %7 = extractvalue { i64, i1 } %6, 0
  %8 = extractvalue { i64, i1 } %6, 1
  %9 = zext i1 %8 to i8
  store i64 %7, ptr %_8, align 8
  %10 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %9, ptr %10, align 8
  br label %bb25

bb22:                                             ; preds = %bb2
  %11 = cmpxchg weak ptr %dst, i64 %old, i64 %new monotonic seq_cst, align 8
  %12 = extractvalue { i64, i1 } %11, 0
  %13 = extractvalue { i64, i1 } %11, 1
  %14 = zext i1 %13 to i8
  store i64 %12, ptr %_8, align 8
  %15 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %14, ptr %15, align 8
  br label %bb25

bb25:                                             ; preds = %bb10, %bb11, %bb12, %bb13, %bb14, %bb15, %bb19, %bb20, %bb21, %bb16, %bb17, %bb18, %bb22, %bb23, %bb24
  %val = load i64, ptr %_8, align 8
  %16 = getelementptr inbounds i8, ptr %_8, i64 8
  %17 = load i8, ptr %16, align 8
  %ok = trunc nuw i8 %17 to i1
  br i1 %ok, label %bb26, label %bb27

bb18:                                             ; preds = %bb4
  %18 = cmpxchg weak ptr %dst, i64 %old, i64 %new release monotonic, align 8
  %19 = extractvalue { i64, i1 } %18, 0
  %20 = extractvalue { i64, i1 } %18, 1
  %21 = zext i1 %20 to i8
  store i64 %19, ptr %_8, align 8
  %22 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %21, ptr %22, align 8
  br label %bb25

bb17:                                             ; preds = %bb4
  %23 = cmpxchg weak ptr %dst, i64 %old, i64 %new release acquire, align 8
  %24 = extractvalue { i64, i1 } %23, 0
  %25 = extractvalue { i64, i1 } %23, 1
  %26 = zext i1 %25 to i8
  store i64 %24, ptr %_8, align 8
  %27 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %26, ptr %27, align 8
  br label %bb25

bb16:                                             ; preds = %bb4
  %28 = cmpxchg weak ptr %dst, i64 %old, i64 %new release seq_cst, align 8
  %29 = extractvalue { i64, i1 } %28, 0
  %30 = extractvalue { i64, i1 } %28, 1
  %31 = zext i1 %30 to i8
  store i64 %29, ptr %_8, align 8
  %32 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %31, ptr %32, align 8
  br label %bb25

bb21:                                             ; preds = %bb3
  %33 = cmpxchg weak ptr %dst, i64 %old, i64 %new acquire monotonic, align 8
  %34 = extractvalue { i64, i1 } %33, 0
  %35 = extractvalue { i64, i1 } %33, 1
  %36 = zext i1 %35 to i8
  store i64 %34, ptr %_8, align 8
  %37 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %36, ptr %37, align 8
  br label %bb25

bb20:                                             ; preds = %bb3
  %38 = cmpxchg weak ptr %dst, i64 %old, i64 %new acquire acquire, align 8
  %39 = extractvalue { i64, i1 } %38, 0
  %40 = extractvalue { i64, i1 } %38, 1
  %41 = zext i1 %40 to i8
  store i64 %39, ptr %_8, align 8
  %42 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %41, ptr %42, align 8
  br label %bb25

bb19:                                             ; preds = %bb3
  %43 = cmpxchg weak ptr %dst, i64 %old, i64 %new acquire seq_cst, align 8
  %44 = extractvalue { i64, i1 } %43, 0
  %45 = extractvalue { i64, i1 } %43, 1
  %46 = zext i1 %45 to i8
  store i64 %44, ptr %_8, align 8
  %47 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %46, ptr %47, align 8
  br label %bb25

bb15:                                             ; preds = %bb5
  %48 = cmpxchg weak ptr %dst, i64 %old, i64 %new acq_rel monotonic, align 8
  %49 = extractvalue { i64, i1 } %48, 0
  %50 = extractvalue { i64, i1 } %48, 1
  %51 = zext i1 %50 to i8
  store i64 %49, ptr %_8, align 8
  %52 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %51, ptr %52, align 8
  br label %bb25

bb14:                                             ; preds = %bb5
  %53 = cmpxchg weak ptr %dst, i64 %old, i64 %new acq_rel acquire, align 8
  %54 = extractvalue { i64, i1 } %53, 0
  %55 = extractvalue { i64, i1 } %53, 1
  %56 = zext i1 %55 to i8
  store i64 %54, ptr %_8, align 8
  %57 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %56, ptr %57, align 8
  br label %bb25

bb13:                                             ; preds = %bb5
  %58 = cmpxchg weak ptr %dst, i64 %old, i64 %new acq_rel seq_cst, align 8
  %59 = extractvalue { i64, i1 } %58, 0
  %60 = extractvalue { i64, i1 } %58, 1
  %61 = zext i1 %60 to i8
  store i64 %59, ptr %_8, align 8
  %62 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %61, ptr %62, align 8
  br label %bb25

bb12:                                             ; preds = %bb6
  %63 = cmpxchg weak ptr %dst, i64 %old, i64 %new seq_cst monotonic, align 8
  %64 = extractvalue { i64, i1 } %63, 0
  %65 = extractvalue { i64, i1 } %63, 1
  %66 = zext i1 %65 to i8
  store i64 %64, ptr %_8, align 8
  %67 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %66, ptr %67, align 8
  br label %bb25

bb11:                                             ; preds = %bb6
  %68 = cmpxchg weak ptr %dst, i64 %old, i64 %new seq_cst acquire, align 8
  %69 = extractvalue { i64, i1 } %68, 0
  %70 = extractvalue { i64, i1 } %68, 1
  %71 = zext i1 %70 to i8
  store i64 %69, ptr %_8, align 8
  %72 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %71, ptr %72, align 8
  br label %bb25

bb10:                                             ; preds = %bb6
  %73 = cmpxchg weak ptr %dst, i64 %old, i64 %new seq_cst seq_cst, align 8
  %74 = extractvalue { i64, i1 } %73, 0
  %75 = extractvalue { i64, i1 } %73, 1
  %76 = zext i1 %75 to i8
  store i64 %74, ptr %_8, align 8
  %77 = getelementptr inbounds i8, ptr %_8, i64 8
  store i8 %76, ptr %77, align 8
  br label %bb25

bb27:                                             ; preds = %bb25
  %78 = getelementptr inbounds i8, ptr %_0, i64 8
  store i64 %val, ptr %78, align 8
  store i64 1, ptr %_0, align 8
  br label %bb28

bb26:                                             ; preds = %bb25
  %79 = getelementptr inbounds i8, ptr %_0, i64 8
  store i64 %val, ptr %79, align 8
  store i64 0, ptr %_0, align 8
  br label %bb28

bb28:                                             ; preds = %bb26, %bb27
  %80 = load i64, ptr %_0, align 8
  %81 = getelementptr inbounds i8, ptr %_0, i64 8
  %82 = load i64, ptr %81, align 8
  %83 = insertvalue { i64, i64 } poison, i64 %80, 0
  %84 = insertvalue { i64, i64 } %83, i64 %82, 1
  ret { i64, i64 } %84

bb8:                                              ; preds = %bb1
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking9panic_fmt(ptr @alloc_5a43f8d94dd4505c1dba43832ce73af8, ptr inttoptr (i64 105 to ptr), ptr align 8 @alloc_1dbbdb33ce56ab0f629d5d081745ad16) #4
  unreachable

bb9:                                              ; preds = %bb1
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking9panic_fmt(ptr @alloc_7adef5546d83b439c7829602020737c6, ptr inttoptr (i64 123 to ptr), ptr align 8 @alloc_11a7b6bb3417a8fba26ae9c100d23a1b) #4
  unreachable
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal void @_ZN4core4sync6atomic5fence17h14342f8259fb0c07E(i8 %order) unnamed_addr #0 {
start:
  %_2 = zext i8 %order to i64
  switch i64 %_2, label %bb1 [
    i64 0, label %bb2
    i64 1, label %bb5
    i64 2, label %bb6
    i64 3, label %bb4
    i64 4, label %bb3
  ]

bb1:                                              ; preds = %start
  unreachable

bb2:                                              ; preds = %start
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking9panic_fmt(ptr @alloc_929e9b2e7b7429614ca4fc017efff666, ptr inttoptr (i64 83 to ptr), ptr align 8 @alloc_ea5b21d5fc560c6358071a0315535da4) #4
  unreachable

bb5:                                              ; preds = %start
  fence release
  br label %bb7

bb6:                                              ; preds = %start
  fence acquire
  br label %bb7

bb4:                                              ; preds = %start
  fence acq_rel
  br label %bb7

bb3:                                              ; preds = %start
  fence seq_cst
  br label %bb7

bb7:                                              ; preds = %bb3, %bb4, %bb6, %bb5
  ret void
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal i8 @_ZN4core4sync6atomic8AtomicU810fetch_nand17h329d03648db7aa18E(ptr align 1 %self, i8 %val, i8 %order) unnamed_addr #0 {
start:
  %_0 = alloca [1 x i8], align 1
  %_7 = zext i8 %order to i64
  switch i64 %_7, label %bb2 [
    i64 0, label %bb7
    i64 1, label %bb5
    i64 2, label %bb6
    i64 3, label %bb4
    i64 4, label %bb3
  ]

bb2:                                              ; preds = %start
  unreachable

bb7:                                              ; preds = %start
  %0 = atomicrmw nand ptr %self, i8 %val monotonic, align 1
  store i8 %0, ptr %_0, align 1
  br label %bb1

bb5:                                              ; preds = %start
  %1 = atomicrmw nand ptr %self, i8 %val release, align 1
  store i8 %1, ptr %_0, align 1
  br label %bb1

bb6:                                              ; preds = %start
  %2 = atomicrmw nand ptr %self, i8 %val acquire, align 1
  store i8 %2, ptr %_0, align 1
  br label %bb1

bb4:                                              ; preds = %start
  %3 = atomicrmw nand ptr %self, i8 %val acq_rel, align 1
  store i8 %3, ptr %_0, align 1
  br label %bb1

bb3:                                              ; preds = %start
  %4 = atomicrmw nand ptr %self, i8 %val seq_cst, align 1
  store i8 %4, ptr %_0, align 1
  br label %bb1

bb1:                                              ; preds = %bb3, %bb4, %bb6, %bb5, %bb7
  %5 = load i8, ptr %_0, align 1
  ret i8 %5
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal i8 @_ZN4core4sync6atomic8AtomicU88fetch_or17h1a9543d7f2bb286dE(ptr align 1 %self, i8 %val, i8 %order) unnamed_addr #0 {
start:
  %_0 = alloca [1 x i8], align 1
  %_7 = zext i8 %order to i64
  switch i64 %_7, label %bb2 [
    i64 0, label %bb3
    i64 1, label %bb5
    i64 2, label %bb6
    i64 3, label %bb4
    i64 4, label %bb7
  ]

bb2:                                              ; preds = %start
  unreachable

bb3:                                              ; preds = %start
  %0 = atomicrmw or ptr %self, i8 %val monotonic, align 1
  store i8 %0, ptr %_0, align 1
  br label %bb1

bb5:                                              ; preds = %start
  %1 = atomicrmw or ptr %self, i8 %val release, align 1
  store i8 %1, ptr %_0, align 1
  br label %bb1

bb6:                                              ; preds = %start
  %2 = atomicrmw or ptr %self, i8 %val acquire, align 1
  store i8 %2, ptr %_0, align 1
  br label %bb1

bb4:                                              ; preds = %start
  %3 = atomicrmw or ptr %self, i8 %val acq_rel, align 1
  store i8 %3, ptr %_0, align 1
  br label %bb1

bb7:                                              ; preds = %start
  %4 = atomicrmw or ptr %self, i8 %val seq_cst, align 1
  store i8 %4, ptr %_0, align 1
  br label %bb1

bb1:                                              ; preds = %bb7, %bb4, %bb6, %bb5, %bb3
  %5 = load i8, ptr %_0, align 1
  ret i8 %5
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal i8 @_ZN4core4sync6atomic8AtomicU89fetch_and17h9910080cf9131d10E(ptr align 1 %self, i8 %val, i8 %order) unnamed_addr #0 {
start:
  %_0 = alloca [1 x i8], align 1
  %_7 = zext i8 %order to i64
  switch i64 %_7, label %bb2 [
    i64 0, label %bb7
    i64 1, label %bb5
    i64 2, label %bb6
    i64 3, label %bb4
    i64 4, label %bb3
  ]

bb2:                                              ; preds = %start
  unreachable

bb7:                                              ; preds = %start
  %0 = atomicrmw and ptr %self, i8 %val monotonic, align 1
  store i8 %0, ptr %_0, align 1
  br label %bb1

bb5:                                              ; preds = %start
  %1 = atomicrmw and ptr %self, i8 %val release, align 1
  store i8 %1, ptr %_0, align 1
  br label %bb1

bb6:                                              ; preds = %start
  %2 = atomicrmw and ptr %self, i8 %val acquire, align 1
  store i8 %2, ptr %_0, align 1
  br label %bb1

bb4:                                              ; preds = %start
  %3 = atomicrmw and ptr %self, i8 %val acq_rel, align 1
  store i8 %3, ptr %_0, align 1
  br label %bb1

bb3:                                              ; preds = %start
  %4 = atomicrmw and ptr %self, i8 %val seq_cst, align 1
  store i8 %4, ptr %_0, align 1
  br label %bb1

bb1:                                              ; preds = %bb3, %bb4, %bb6, %bb5, %bb7
  %5 = load i8, ptr %_0, align 1
  ret i8 %5
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal i8 @_ZN4core4sync6atomic8AtomicU89fetch_xor17ha0f2be2b6cd13f68E(ptr align 1 %self, i8 %val, i8 %order) unnamed_addr #0 {
start:
  %_0 = alloca [1 x i8], align 1
  %_7 = zext i8 %order to i64
  switch i64 %_7, label %bb2 [
    i64 0, label %bb3
    i64 1, label %bb5
    i64 2, label %bb6
    i64 3, label %bb4
    i64 4, label %bb7
  ]

bb2:                                              ; preds = %start
  unreachable

bb3:                                              ; preds = %start
  %0 = atomicrmw xor ptr %self, i8 %val monotonic, align 1
  store i8 %0, ptr %_0, align 1
  br label %bb1

bb5:                                              ; preds = %start
  %1 = atomicrmw xor ptr %self, i8 %val release, align 1
  store i8 %1, ptr %_0, align 1
  br label %bb1

bb6:                                              ; preds = %start
  %2 = atomicrmw xor ptr %self, i8 %val acquire, align 1
  store i8 %2, ptr %_0, align 1
  br label %bb1

bb4:                                              ; preds = %start
  %3 = atomicrmw xor ptr %self, i8 %val acq_rel, align 1
  store i8 %3, ptr %_0, align 1
  br label %bb1

bb7:                                              ; preds = %start
  %4 = atomicrmw xor ptr %self, i8 %val seq_cst, align 1
  store i8 %4, ptr %_0, align 1
  br label %bb1

bb1:                                              ; preds = %bb7, %bb4, %bb6, %bb5, %bb3
  %5 = load i8, ptr %_0, align 1
  ret i8 %5
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal i32 @_ZN4core4sync6atomic9AtomicI329fetch_max17hedc26ecce657a6d2E(ptr align 4 %self, i32 %val, i8 %order) unnamed_addr #0 {
start:
  %_0 = alloca [4 x i8], align 4
  %_7 = zext i8 %order to i64
  switch i64 %_7, label %bb2 [
    i64 0, label %bb7
    i64 1, label %bb5
    i64 2, label %bb6
    i64 3, label %bb4
    i64 4, label %bb3
  ]

bb2:                                              ; preds = %start
  unreachable

bb7:                                              ; preds = %start
  %0 = atomicrmw max ptr %self, i32 %val monotonic, align 4
  store i32 %0, ptr %_0, align 4
  br label %bb1

bb5:                                              ; preds = %start
  %1 = atomicrmw max ptr %self, i32 %val release, align 4
  store i32 %1, ptr %_0, align 4
  br label %bb1

bb6:                                              ; preds = %start
  %2 = atomicrmw max ptr %self, i32 %val acquire, align 4
  store i32 %2, ptr %_0, align 4
  br label %bb1

bb4:                                              ; preds = %start
  %3 = atomicrmw max ptr %self, i32 %val acq_rel, align 4
  store i32 %3, ptr %_0, align 4
  br label %bb1

bb3:                                              ; preds = %start
  %4 = atomicrmw max ptr %self, i32 %val seq_cst, align 4
  store i32 %4, ptr %_0, align 4
  br label %bb1

bb1:                                              ; preds = %bb3, %bb4, %bb6, %bb5, %bb7
  %5 = load i32, ptr %_0, align 4
  ret i32 %5
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal i32 @_ZN4core4sync6atomic9AtomicI329fetch_min17h688c282213d05ff1E(ptr align 4 %self, i32 %val, i8 %order) unnamed_addr #0 {
start:
  %_0 = alloca [4 x i8], align 4
  %_7 = zext i8 %order to i64
  switch i64 %_7, label %bb2 [
    i64 0, label %bb7
    i64 1, label %bb5
    i64 2, label %bb6
    i64 3, label %bb4
    i64 4, label %bb3
  ]

bb2:                                              ; preds = %start
  unreachable

bb7:                                              ; preds = %start
  %0 = atomicrmw min ptr %self, i32 %val monotonic, align 4
  store i32 %0, ptr %_0, align 4
  br label %bb1

bb5:                                              ; preds = %start
  %1 = atomicrmw min ptr %self, i32 %val release, align 4
  store i32 %1, ptr %_0, align 4
  br label %bb1

bb6:                                              ; preds = %start
  %2 = atomicrmw min ptr %self, i32 %val acquire, align 4
  store i32 %2, ptr %_0, align 4
  br label %bb1

bb4:                                              ; preds = %start
  %3 = atomicrmw min ptr %self, i32 %val acq_rel, align 4
  store i32 %3, ptr %_0, align 4
  br label %bb1

bb3:                                              ; preds = %start
  %4 = atomicrmw min ptr %self, i32 %val seq_cst, align 4
  store i32 %4, ptr %_0, align 4
  br label %bb1

bb1:                                              ; preds = %bb3, %bb4, %bb6, %bb5, %bb7
  %5 = load i32, ptr %_0, align 4
  ret i32 %5
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal { i64, i64 } @_ZN4core4sync6atomic9AtomicU6416compare_exchange17h76a0df849c524025E(ptr align 8 %self, i64 %current, i64 %new, i8 %success, i8 %failure) unnamed_addr #0 {
start:
  %0 = call { i64, i64 } @_ZN4core4sync6atomic23atomic_compare_exchange17heedd5910b8b17177E(ptr %self, i64 %current, i64 %new, i8 %success, i8 %failure) #3
  %_0.0 = extractvalue { i64, i64 } %0, 0
  %_0.1 = extractvalue { i64, i64 } %0, 1
  %1 = insertvalue { i64, i64 } poison, i64 %_0.0, 0
  %2 = insertvalue { i64, i64 } %1, i64 %_0.1, 1
  ret { i64, i64 } %2
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal { i64, i64 } @_ZN4core4sync6atomic9AtomicU6421compare_exchange_weak17h1777c6eacfd5c4a5E(ptr align 8 %self, i64 %current, i64 %new, i8 %success, i8 %failure) unnamed_addr #0 {
start:
  %0 = call { i64, i64 } @_ZN4core4sync6atomic28atomic_compare_exchange_weak17h38f1d7afc9aa7672E(ptr %self, i64 %current, i64 %new, i8 %success, i8 %failure) #3
  %_0.0 = extractvalue { i64, i64 } %0, 0
  %_0.1 = extractvalue { i64, i64 } %0, 1
  %1 = insertvalue { i64, i64 } poison, i64 %_0.0, 0
  %2 = insertvalue { i64, i64 } %1, i64 %_0.1, 1
  ret { i64, i64 } %2
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal i64 @_ZN4core4sync6atomic9AtomicU644swap17h26187f49dc5a170dE(ptr align 8 %self, i64 %val, i8 %order) unnamed_addr #0 {
start:
  %_0 = alloca [8 x i8], align 8
  %_7 = zext i8 %order to i64
  switch i64 %_7, label %bb2 [
    i64 0, label %bb7
    i64 1, label %bb5
    i64 2, label %bb6
    i64 3, label %bb4
    i64 4, label %bb3
  ]

bb2:                                              ; preds = %start
  unreachable

bb7:                                              ; preds = %start
  %0 = atomicrmw xchg ptr %self, i64 %val monotonic, align 8
  store i64 %0, ptr %_0, align 8
  br label %bb1

bb5:                                              ; preds = %start
  %1 = atomicrmw xchg ptr %self, i64 %val release, align 8
  store i64 %1, ptr %_0, align 8
  br label %bb1

bb6:                                              ; preds = %start
  %2 = atomicrmw xchg ptr %self, i64 %val acquire, align 8
  store i64 %2, ptr %_0, align 8
  br label %bb1

bb4:                                              ; preds = %start
  %3 = atomicrmw xchg ptr %self, i64 %val acq_rel, align 8
  store i64 %3, ptr %_0, align 8
  br label %bb1

bb3:                                              ; preds = %start
  %4 = atomicrmw xchg ptr %self, i64 %val seq_cst, align 8
  store i64 %4, ptr %_0, align 8
  br label %bb1

bb1:                                              ; preds = %bb3, %bb4, %bb6, %bb5, %bb7
  %5 = load i64, ptr %_0, align 8
  ret i64 %5
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal i64 @_ZN4core4sync6atomic9AtomicU649fetch_add17hca885779f84b6327E(ptr align 8 %self, i64 %val, i8 %order) unnamed_addr #0 {
start:
  %_0 = alloca [8 x i8], align 8
  %_7 = zext i8 %order to i64
  switch i64 %_7, label %bb2 [
    i64 0, label %bb7
    i64 1, label %bb5
    i64 2, label %bb6
    i64 3, label %bb4
    i64 4, label %bb3
  ]

bb2:                                              ; preds = %start
  unreachable

bb7:                                              ; preds = %start
  %0 = atomicrmw add ptr %self, i64 %val monotonic, align 8
  store i64 %0, ptr %_0, align 8
  br label %bb1

bb5:                                              ; preds = %start
  %1 = atomicrmw add ptr %self, i64 %val release, align 8
  store i64 %1, ptr %_0, align 8
  br label %bb1

bb6:                                              ; preds = %start
  %2 = atomicrmw add ptr %self, i64 %val acquire, align 8
  store i64 %2, ptr %_0, align 8
  br label %bb1

bb4:                                              ; preds = %start
  %3 = atomicrmw add ptr %self, i64 %val acq_rel, align 8
  store i64 %3, ptr %_0, align 8
  br label %bb1

bb3:                                              ; preds = %start
  %4 = atomicrmw add ptr %self, i64 %val seq_cst, align 8
  store i64 %4, ptr %_0, align 8
  br label %bb1

bb1:                                              ; preds = %bb3, %bb4, %bb6, %bb5, %bb7
  %5 = load i64, ptr %_0, align 8
  ret i64 %5
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal i64 @_ZN4core4sync6atomic9AtomicU649fetch_max17h7e3ceca170491fc8E(ptr align 8 %self, i64 %val, i8 %order) unnamed_addr #0 {
start:
  %_0 = alloca [8 x i8], align 8
  %_7 = zext i8 %order to i64
  switch i64 %_7, label %bb2 [
    i64 0, label %bb7
    i64 1, label %bb5
    i64 2, label %bb6
    i64 3, label %bb4
    i64 4, label %bb3
  ]

bb2:                                              ; preds = %start
  unreachable

bb7:                                              ; preds = %start
  %0 = atomicrmw umax ptr %self, i64 %val monotonic, align 8
  store i64 %0, ptr %_0, align 8
  br label %bb1

bb5:                                              ; preds = %start
  %1 = atomicrmw umax ptr %self, i64 %val release, align 8
  store i64 %1, ptr %_0, align 8
  br label %bb1

bb6:                                              ; preds = %start
  %2 = atomicrmw umax ptr %self, i64 %val acquire, align 8
  store i64 %2, ptr %_0, align 8
  br label %bb1

bb4:                                              ; preds = %start
  %3 = atomicrmw umax ptr %self, i64 %val acq_rel, align 8
  store i64 %3, ptr %_0, align 8
  br label %bb1

bb3:                                              ; preds = %start
  %4 = atomicrmw umax ptr %self, i64 %val seq_cst, align 8
  store i64 %4, ptr %_0, align 8
  br label %bb1

bb1:                                              ; preds = %bb3, %bb4, %bb6, %bb5, %bb7
  %5 = load i64, ptr %_0, align 8
  ret i64 %5
}

; Function Attrs: inlinehint nounwind nonlazybind uwtable
define internal i64 @_ZN4core4sync6atomic9AtomicU649fetch_sub17hd7e8cb6cc017bd87E(ptr align 8 %self, i64 %val, i8 %order) unnamed_addr #0 {
start:
  %_0 = alloca [8 x i8], align 8
  %_7 = zext i8 %order to i64
  switch i64 %_7, label %bb2 [
    i64 0, label %bb7
    i64 1, label %bb5
    i64 2, label %bb6
    i64 3, label %bb4
    i64 4, label %bb3
  ]

bb2:                                              ; preds = %start
  unreachable

bb7:                                              ; preds = %start
  %0 = atomicrmw sub ptr %self, i64 %val monotonic, align 8
  store i64 %0, ptr %_0, align 8
  br label %bb1

bb5:                                              ; preds = %start
  %1 = atomicrmw sub ptr %self, i64 %val release, align 8
  store i64 %1, ptr %_0, align 8
  br label %bb1

bb6:                                              ; preds = %start
  %2 = atomicrmw sub ptr %self, i64 %val acquire, align 8
  store i64 %2, ptr %_0, align 8
  br label %bb1

bb4:                                              ; preds = %start
  %3 = atomicrmw sub ptr %self, i64 %val acq_rel, align 8
  store i64 %3, ptr %_0, align 8
  br label %bb1

bb3:                                              ; preds = %start
  %4 = atomicrmw sub ptr %self, i64 %val seq_cst, align 8
  store i64 %4, ptr %_0, align 8
  br label %bb1

bb1:                                              ; preds = %bb3, %bb4, %bb6, %bb5, %bb7
  %5 = load i64, ptr %_0, align 8
  ret i64 %5
}

; Function Attrs: nounwind nonlazybind uwtable
define i64 @add_relaxed(i64 %n) unnamed_addr #1 {
start:
  %_0 = call i64 @_ZN4core4sync6atomic9AtomicU649fetch_add17hca885779f84b6327E(ptr align 8 @_ZN7atomics7COUNTER17h0bd1eb976af6a3e2E, i64 %n, i8 0) #3
  ret i64 %_0
}

; Function Attrs: nounwind nonlazybind uwtable
define i8 @and_acquire(i8 %n) unnamed_addr #1 {
start:
  %_0 = call i8 @_ZN4core4sync6atomic8AtomicU89fetch_and17h9910080cf9131d10E(ptr align 1 @_ZN7atomics4FLAG17ha641e63b27ffe2ecE, i8 %n, i8 2) #3
  ret i8 %_0
}

; Function Attrs: nounwind nonlazybind uwtable
define void @fences() unnamed_addr #1 {
start:
  call void @_ZN4core4sync6atomic5fence17h14342f8259fb0c07E(i8 4) #3
  call void @_ZN4core4sync6atomic5fence17h14342f8259fb0c07E(i8 2) #3
  call void @_ZN4core4sync6atomic14compiler_fence17h6cff920f5dfa0f6aE(i8 1) #3
  ret void
}

; Function Attrs: nounwind nonlazybind uwtable
define void @load_store(ptr %p) unnamed_addr #1 {
start:
  call void @"_ZN4core4sync6atomic18AtomicPtr$LT$T$GT$5store17h0024fedcf9a48c91E"(ptr align 8 @_ZN7atomics7POINTER17hef8592fb3d4a7165E, ptr %p, i8 1) #3
  %_8 = call ptr @"_ZN4core4sync6atomic18AtomicPtr$LT$T$GT$4load17hf5950fa70c23e0e3E"(ptr align 8 @_ZN7atomics7POINTER17hef8592fb3d4a7165E, i8 2) #3
  %_7 = ptrtoint ptr %_8 to i64
  call void @_ZN4core4sync6atomic11AtomicIsize5store17h30b3333c67ca0704E(ptr align 8 @_ZN7atomics4SIZE17h8375f3d670919c9fE, i64 %_7, i8 0) #3
  ret void
}

; Function Attrs: nounwind nonlazybind uwtable
define i8 @nand(i8 %n) unnamed_addr #1 {
start:
  %_0 = call i8 @_ZN4core4sync6atomic8AtomicU810fetch_nand17h329d03648db7aa18E(ptr align 1 @_ZN7atomics4FLAG17ha641e63b27ffe2ecE, i8 %n, i8 4) #3
  ret i8 %_0
}

; Function Attrs: nounwind nonlazybind uwtable
define i8 @or_acqrel(i8 %n) unnamed_addr #1 {
start:
  %_0 = call i8 @_ZN4core4sync6atomic8AtomicU88fetch_or17h1a9543d7f2bb286dE(ptr align 1 @_ZN7atomics4FLAG17ha641e63b27ffe2ecE, i8 %n, i8 3) #3
  ret i8 %_0
}

; Function Attrs: nounwind nonlazybind uwtable
define i32 @signed_max(i32 %n) unnamed_addr #1 {
start:
  %_0 = call i32 @_ZN4core4sync6atomic9AtomicI329fetch_max17hedc26ecce657a6d2E(ptr align 4 @_ZN7atomics6SIGNED17h98aac63ff57bf7f3E, i32 %n, i8 4) #3
  ret i32 %_0
}

; Function Attrs: nounwind nonlazybind uwtable
define i32 @signed_min(i32 %n) unnamed_addr #1 {
start:
  %_0 = call i32 @_ZN4core4sync6atomic9AtomicI329fetch_min17h688c282213d05ff1E(ptr align 4 @_ZN7atomics6SIGNED17h98aac63ff57bf7f3E, i32 %n, i8 4) #3
  ret i32 %_0
}

; Function Attrs: nounwind nonlazybind uwtable
define { i64, i64 } @strong_exchange(i64 %old, i64 %new) unnamed_addr #1 {
start:
  %0 = call { i64, i64 } @_ZN4core4sync6atomic9AtomicU6416compare_exchange17h76a0df849c524025E(ptr align 8 @_ZN7atomics7COUNTER17h0bd1eb976af6a3e2E, i64 %old, i64 %new, i8 4, i8 2) #3
  %_0.0 = extractvalue { i64, i64 } %0, 0
  %_0.1 = extractvalue { i64, i64 } %0, 1
  %1 = insertvalue { i64, i64 } poison, i64 %_0.0, 0
  %2 = insertvalue { i64, i64 } %1, i64 %_0.1, 1
  ret { i64, i64 } %2
}

; Function Attrs: nounwind nonlazybind uwtable
define i64 @sub_release(i64 %n) unnamed_addr #1 {
start:
  %_0 = call i64 @_ZN4core4sync6atomic9AtomicU649fetch_sub17hd7e8cb6cc017bd87E(ptr align 8 @_ZN7atomics7COUNTER17h0bd1eb976af6a3e2E, i64 %n, i8 1) #3
  ret i64 %_0
}

; Function Attrs: nounwind nonlazybind uwtable
define i64 @swap(i64 %n) unnamed_addr #1 {
start:
  %_0 = call i64 @_ZN4core4sync6atomic9AtomicU644swap17h26187f49dc5a170dE(ptr align 8 @_ZN7atomics7COUNTER17h0bd1eb976af6a3e2E, i64 %n, i8 4) #3
  ret i64 %_0
}

; Function Attrs: nounwind nonlazybind uwtable
define i64 @unsigned_max(i64 %n) unnamed_addr #1 {
start:
  %_0 = call i64 @_ZN4core4sync6atomic9AtomicU649fetch_max17h7e3ceca170491fc8E(ptr align 8 @_ZN7atomics7COUNTER17h0bd1eb976af6a3e2E, i64 %n, i8 4) #3
  ret i64 %_0
}

; Function Attrs: nounwind nonlazybind uwtable
define { i64, i64 } @weak_exchange(i64 %old, i64 %new) unnamed_addr #1 {
start:
  %0 = call { i64, i64 } @_ZN4core4sync6atomic9AtomicU6421compare_exchange_weak17h1777c6eacfd5c4a5E(ptr align 8 @_ZN7atomics7COUNTER17h0bd1eb976af6a3e2E, i64 %old, i64 %new, i8 1, i8 0) #3
  %_0.0 = extractvalue { i64, i64 } %0, 0
  %_0.1 = extractvalue { i64, i64 } %0, 1
  %1 = insertvalue { i64, i64 } poison, i64 %_0.0, 0
  %2 = insertvalue { i64, i64 } %1, i64 %_0.1, 1
  ret { i64, i64 } %2
}

; Function Attrs: nounwind nonlazybind uwtable
define i8 @xor_seqcst(i8 %n) unnamed_addr #1 {
start:
  %_0 = call i8 @_ZN4core4sync6atomic8AtomicU89fetch_xor17ha0f2be2b6cd13f68E(ptr align 1 @_ZN7atomics4FLAG17ha641e63b27ffe2ecE, i8 %n, i8 4) #3
  ret i8 %_0
}

; Function Attrs: cold noinline noreturn nounwind nonlazybind uwtable
declare void @_RNvNtCs4uthzyWeO2a_4core9panicking9panic_fmt(ptr, ptr, ptr align 8) unnamed_addr #2

attributes #0 = { inlinehint nounwind nonlazybind uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #1 = { nounwind nonlazybind uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #2 = { cold noinline noreturn nounwind nonlazybind uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #3 = { inlinehint nounwind }
attributes #4 = { noinline noreturn nounwind }

!llvm.module.flags = !{!0, !1}
!llvm.ident = !{!2}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 2, !"RtLibUseGOT", i32 1}
!2 = !{!"rustc version 1.95.0 (59807616e 2026-04-14) (built from a source tarball)"}
