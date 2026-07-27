; ModuleID = 'debug.ll'
source_filename = "debug.765ffadcb553e2ad-cgu.0"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@alloc_095a80957e05374feb6aed8ed18b0480 = private unnamed_addr constant [67 x i8] c"/home/overby.me/Work/overby.me/rust/llvm/corpus/rustc/src/debug.rs\00", align 1
@alloc_f5216199ce52a5b2a4f4794d7a967e83 = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_095a80957e05374feb6aed8ed18b0480, [16 x i8] c"B\00\00\00\00\00\00\00(\00\00\00\1C\00\00\00" }>, align 8
@alloc_189f88d3bda44c8fee1b7ce386b44d97 = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_095a80957e05374feb6aed8ed18b0480, [16 x i8] c"B\00\00\00\00\00\00\00(\00\00\00\09\00\00\00" }>, align 8
@alloc_75808ae5424a5c3b72d19972f19735b4 = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_095a80957e05374feb6aed8ed18b0480, [16 x i8] c"B\00\00\00\00\00\00\00)\00\00\00\09\00\00\00" }>, align 8
@alloc_64c0c00f4155190ec6f8d4c545d33d5b = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_095a80957e05374feb6aed8ed18b0480, [16 x i8] c"B\00\00\00\00\00\00\00\1A\00\00\00\05\00\00\00" }>, align 8
@alloc_eb7a5380db50245ee7a1f0b18657375d = private unnamed_addr constant <{ ptr, [16 x i8] }> <{ ptr @alloc_095a80957e05374feb6aed8ed18b0480, [16 x i8] c"B\00\00\00\00\00\00\00\1A\00\00\00\0F\00\00\00" }>, align 8

; Function Attrs: nounwind nonlazybind uwtable
define i64 @_ZN5debug10accumulate17he0d133801b101669E(ptr align 4 %values.0, i64 %values.1) unnamed_addr #0 !dbg !7 {
start:
  %small.dbg.spill.i = alloca [4 x i8], align 4
  %values.dbg.spill = alloca [16 x i8], align 8
  %index = alloca [8 x i8], align 8
  %total = alloca [8 x i8], align 8
  store ptr %values.0, ptr %values.dbg.spill, align 8
  %0 = getelementptr inbounds i8, ptr %values.dbg.spill, i64 8
  store i64 %values.1, ptr %0, align 8
    #dbg_declare(ptr %values.dbg.spill, !23, !DIExpression(), !28)
    #dbg_declare(ptr %total, !24, !DIExpression(), !29)
    #dbg_declare(ptr %index, !26, !DIExpression(), !30)
  store i64 0, ptr %total, align 8, !dbg !31
  store i64 0, ptr %index, align 8, !dbg !32
  br label %bb1, !dbg !33

bb1:                                              ; preds = %bb6, %start
  %_5 = load i64, ptr %index, align 8, !dbg !34
  %_4 = icmp ult i64 %_5, %values.1, !dbg !34
  br i1 %_4, label %bb2, label %bb7, !dbg !34

bb7:                                              ; preds = %bb1
  %_0 = load i64, ptr %total, align 8, !dbg !35
  ret i64 %_0, !dbg !36

bb2:                                              ; preds = %bb1
  %_9 = load i64, ptr %index, align 8, !dbg !37
  %_11 = icmp ult i64 %_9, %values.1, !dbg !38
  br i1 %_11, label %bb3, label %panic, !dbg !38

bb3:                                              ; preds = %bb2
  %1 = getelementptr inbounds nuw i32, ptr %values.0, i64 %_9, !dbg !38
  %_8 = load i32, ptr %1, align 4, !dbg !38
  store i32 %_8, ptr %small.dbg.spill.i, align 4
    #dbg_declare(ptr %small.dbg.spill.i, !39, !DIExpression(), !49)
  %_0.i = sext i32 %_8 to i64, !dbg !51
  %2 = load i64, ptr %total, align 8, !dbg !52
  %3 = call { i64, i1 } @llvm.sadd.with.overflow.i64(i64 %2, i64 %_0.i), !dbg !52
  %_12.0 = extractvalue { i64, i1 } %3, 0, !dbg !52
  %_12.1 = extractvalue { i64, i1 } %3, 1, !dbg !52
  br i1 %_12.1, label %panic1, label %bb5, !dbg !52

panic:                                            ; preds = %bb2
  call void @_RNvNtCs4uthzyWeO2a_4core9panicking18panic_bounds_check(i64 %_9, i64 %values.1, ptr align 8 @alloc_f5216199ce52a5b2a4f4794d7a967e83) #4, !dbg !38
  unreachable, !dbg !38

bb5:                                              ; preds = %bb3
  store i64 %_12.0, ptr %total, align 8, !dbg !52
  %4 = load i64, ptr %index, align 8, !dbg !53
  %_13.0 = add i64 %4, 1, !dbg !53
  %_13.1 = icmp ult i64 %_13.0, %4, !dbg !53
  br i1 %_13.1, label %panic2, label %bb6, !dbg !53

panic1:                                           ; preds = %bb3
  call void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const24panic_const_add_overflow(ptr align 8 @alloc_189f88d3bda44c8fee1b7ce386b44d97) #4, !dbg !52
  unreachable, !dbg !52

bb6:                                              ; preds = %bb5
  store i64 %_13.0, ptr %index, align 8, !dbg !53
  br label %bb1, !dbg !33

panic2:                                           ; preds = %bb5
  call void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const24panic_const_add_overflow(ptr align 8 @alloc_75808ae5424a5c3b72d19972f19735b4) #4, !dbg !53
  unreachable, !dbg !53
}

; Function Attrs: nounwind nonlazybind uwtable
define i64 @_ZN5debug4area17h41c862acc5230fecE(ptr align 8 %shape) unnamed_addr #0 !dbg !54 {
start:
  %point.dbg.spill = alloca [8 x i8], align 8
  %shape.dbg.spill = alloca [8 x i8], align 8
  %_0 = alloca [8 x i8], align 8
  store ptr %shape, ptr %shape.dbg.spill, align 8
    #dbg_declare(ptr %shape.dbg.spill, !75, !DIExpression(), !79)
  %_2 = load i64, ptr %shape, align 8, !dbg !80
  %0 = trunc nuw i64 %_2 to i1, !dbg !81
  br i1 %0, label %bb2, label %bb3, !dbg !81

bb2:                                              ; preds = %start
  %point = getelementptr inbounds i8, ptr %shape, i64 8, !dbg !82
  store ptr %point, ptr %point.dbg.spill, align 8, !dbg !82
    #dbg_declare(ptr %point.dbg.spill, !76, !DIExpression(), !83)
  %1 = call i64 @_ZN5debug8distance17h5e3963d485c3e6a0E(ptr align 8 %point) #5, !dbg !84
  store i64 %1, ptr %_0, align 8, !dbg !84
  br label %bb4, !dbg !84

bb3:                                              ; preds = %start
  store i64 0, ptr %_0, align 8, !dbg !85
  br label %bb4, !dbg !85

bb4:                                              ; preds = %bb3, %bb2
  %2 = load i64, ptr %_0, align 8, !dbg !86
  ret i64 %2, !dbg !86

bb1:                                              ; No predecessors!
  unreachable, !dbg !80
}

; Function Attrs: nounwind nonlazybind uwtable
define i64 @_ZN5debug8distance17h5e3963d485c3e6a0E(ptr align 8 %p) unnamed_addr #0 !dbg !87 {
start:
  %small.dbg.spill.i = alloca [4 x i8], align 4
  %dy.dbg.spill = alloca [8 x i8], align 8
  %dx.dbg.spill = alloca [8 x i8], align 8
  %p.dbg.spill = alloca [8 x i8], align 8
  store ptr %p, ptr %p.dbg.spill, align 8
    #dbg_declare(ptr %p.dbg.spill, !91, !DIExpression(), !96)
  %0 = getelementptr inbounds i8, ptr %p, i64 8, !dbg !97
  %_3 = load i32, ptr %0, align 8, !dbg !97
  store i32 %_3, ptr %small.dbg.spill.i, align 4
    #dbg_declare(ptr %small.dbg.spill.i, !39, !DIExpression(), !98)
  %_0.i = sext i32 %_3 to i64, !dbg !100
  store i64 %_0.i, ptr %dx.dbg.spill, align 8, !dbg !101
    #dbg_declare(ptr %dx.dbg.spill, !92, !DIExpression(), !102)
  %dy = load i64, ptr %p, align 8, !dbg !103
  store i64 %dy, ptr %dy.dbg.spill, align 8, !dbg !103
    #dbg_declare(ptr %dy.dbg.spill, !94, !DIExpression(), !104)
  %1 = call { i64, i1 } @llvm.smul.with.overflow.i64(i64 %_0.i, i64 %_0.i), !dbg !105
  %_6.0 = extractvalue { i64, i1 } %1, 0, !dbg !105
  %_6.1 = extractvalue { i64, i1 } %1, 1, !dbg !105
  br i1 %_6.1, label %panic, label %bb2, !dbg !105

bb2:                                              ; preds = %start
  %2 = call { i64, i1 } @llvm.smul.with.overflow.i64(i64 %dy, i64 %dy), !dbg !106
  %_8.0 = extractvalue { i64, i1 } %2, 0, !dbg !106
  %_8.1 = extractvalue { i64, i1 } %2, 1, !dbg !106
  br i1 %_8.1, label %panic1, label %bb3, !dbg !106

panic:                                            ; preds = %start
  call void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const24panic_const_mul_overflow(ptr align 8 @alloc_64c0c00f4155190ec6f8d4c545d33d5b) #4, !dbg !105
  unreachable, !dbg !105

bb3:                                              ; preds = %bb2
  %3 = call { i64, i1 } @llvm.sadd.with.overflow.i64(i64 %_6.0, i64 %_8.0), !dbg !105
  %_9.0 = extractvalue { i64, i1 } %3, 0, !dbg !105
  %_9.1 = extractvalue { i64, i1 } %3, 1, !dbg !105
  br i1 %_9.1, label %panic2, label %bb4, !dbg !105

panic1:                                           ; preds = %bb2
  call void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const24panic_const_mul_overflow(ptr align 8 @alloc_eb7a5380db50245ee7a1f0b18657375d) #4, !dbg !106
  unreachable, !dbg !106

bb4:                                              ; preds = %bb3
  ret i64 %_9.0, !dbg !107

panic2:                                           ; preds = %bb3
  call void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const24panic_const_add_overflow(ptr align 8 @alloc_64c0c00f4155190ec6f8d4c545d33d5b) #4, !dbg !105
  unreachable, !dbg !105
}

; Function Attrs: cold minsize noinline noreturn nounwind nonlazybind optsize uwtable
declare void @_RNvNtCs4uthzyWeO2a_4core9panicking18panic_bounds_check(i64, i64, ptr align 8) unnamed_addr #1

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare { i64, i1 } @llvm.sadd.with.overflow.i64(i64, i64) #2

; Function Attrs: cold noinline noreturn nounwind nonlazybind uwtable
declare void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const24panic_const_add_overflow(ptr align 8) unnamed_addr #3

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare { i64, i1 } @llvm.smul.with.overflow.i64(i64, i64) #2

; Function Attrs: cold noinline noreturn nounwind nonlazybind uwtable
declare void @_RNvNtNtCs4uthzyWeO2a_4core9panicking11panic_const24panic_const_mul_overflow(ptr align 8) unnamed_addr #3

attributes #0 = { nounwind nonlazybind uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #1 = { cold minsize noinline noreturn nounwind nonlazybind optsize uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #2 = { nocallback nofree nosync nounwind speculatable willreturn memory(none) }
attributes #3 = { cold noinline noreturn nounwind nonlazybind uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #4 = { noinline noreturn nounwind }
attributes #5 = { nounwind }

!llvm.module.flags = !{!0, !1, !2, !3}
!llvm.ident = !{!4}
!llvm.dbg.cu = !{!5}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 2, !"RtLibUseGOT", i32 1}
!2 = !{i32 7, !"Dwarf Version", i32 4}
!3 = !{i32 2, !"Debug Info Version", i32 3}
!4 = !{!"rustc version 1.95.0 (59807616e 2026-04-14) (built from a source tarball)"}
!5 = distinct !DICompileUnit(language: DW_LANG_Rust, file: !6, producer: "clang LLVM (rustc version 1.95.0 (59807616e 2026-04-14) (built from a source tarball))", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug, splitDebugInlining: false, nameTableKind: None)
!6 = !DIFile(filename: "/home/overby.me/Work/overby.me/rust/llvm/corpus/rustc/src/debug.rs/@/debug.765ffadcb553e2ad-cgu.0", directory: "/home/overby.me/Work/overby.me/rust/llvm")
!7 = distinct !DISubprogram(name: "accumulate", linkageName: "_ZN5debug10accumulate17he0d133801b101669E", scope: !9, file: !8, line: 36, type: !10, scopeLine: 36, flags: DIFlagPrototyped, spFlags: DISPFlagDefinition, unit: !5, templateParams: !21, retainedNodes: !22)
!8 = !DIFile(filename: "corpus/rustc/src/debug.rs", directory: "/home/overby.me/Work/overby.me/rust/llvm", checksumkind: CSK_MD5, checksum: "3ab57fe285b6779a3071306453653492")
!9 = !DINamespace(name: "debug", scope: null)
!10 = !DISubroutineType(types: !11)
!11 = !{!12, !13}
!12 = !DIBasicType(name: "i64", size: 64, encoding: DW_ATE_signed)
!13 = !DICompositeType(tag: DW_TAG_structure_type, name: "&[i32]", file: !14, size: 128, align: 64, elements: !15, templateParams: !21, identifier: "6b57523f38171cc87b38da8cc3de4ac3")
!14 = !DIFile(filename: "<unknown>", directory: "")
!15 = !{!16, !19}
!16 = !DIDerivedType(tag: DW_TAG_member, name: "data_ptr", scope: !13, file: !14, baseType: !17, size: 64, align: 64)
!17 = !DIDerivedType(tag: DW_TAG_pointer_type, baseType: !18, size: 64, align: 64, dwarfAddressSpace: 0)
!18 = !DIBasicType(name: "i32", size: 32, encoding: DW_ATE_signed)
!19 = !DIDerivedType(tag: DW_TAG_member, name: "length", scope: !13, file: !14, baseType: !20, size: 64, align: 64, offset: 64)
!20 = !DIBasicType(name: "usize", size: 64, encoding: DW_ATE_unsigned)
!21 = !{}
!22 = !{!23, !24, !26}
!23 = !DILocalVariable(name: "values", arg: 1, scope: !7, file: !8, line: 36, type: !13)
!24 = !DILocalVariable(name: "total", scope: !25, file: !8, line: 37, type: !12, align: 64)
!25 = distinct !DILexicalBlock(scope: !7, file: !8, line: 37, column: 5)
!26 = !DILocalVariable(name: "index", scope: !27, file: !8, line: 38, type: !20, align: 64)
!27 = distinct !DILexicalBlock(scope: !25, file: !8, line: 38, column: 5)
!28 = !DILocation(line: 36, column: 19, scope: !7)
!29 = !DILocation(line: 37, column: 9, scope: !25)
!30 = !DILocation(line: 38, column: 9, scope: !27)
!31 = !DILocation(line: 37, column: 26, scope: !7)
!32 = !DILocation(line: 38, column: 21, scope: !25)
!33 = !DILocation(line: 39, column: 5, scope: !27)
!34 = !DILocation(line: 39, column: 11, scope: !27)
!35 = !DILocation(line: 43, column: 5, scope: !27)
!36 = !DILocation(line: 44, column: 2, scope: !7)
!37 = !DILocation(line: 40, column: 35, scope: !27)
!38 = !DILocation(line: 40, column: 28, scope: !27)
!39 = !DILocalVariable(name: "small", arg: 1, scope: !40, file: !41, line: 79, type: !18)
!40 = distinct !DISubprogram(name: "from", linkageName: "_RNvXs1j_NtNtCs4uthzyWeO2a_4core7convert3numxINtB8_4FromlE4from", scope: !42, file: !41, line: 79, type: !46, scopeLine: 79, flags: DIFlagPrototyped, spFlags: DISPFlagLocalToUnit | DISPFlagDefinition, unit: !5, templateParams: !21, retainedNodes: !48)
!41 = !DIFile(filename: "library/core/src/convert/num.rs", directory: "/rustc/59807616e1fa2540724bfbac14d7976d7e4a3860", checksumkind: CSK_MD5, checksum: "5c89b4a3c13ad525a8a6672c69b7f326")
!42 = !DINamespace(name: "{impl#83}", scope: !43)
!43 = !DINamespace(name: "num", scope: !44)
!44 = !DINamespace(name: "convert", scope: !45)
!45 = !DINamespace(name: "core", scope: null)
!46 = !DISubroutineType(types: !47)
!47 = !{!12, !18}
!48 = !{!39}
!49 = !DILocation(line: 79, column: 21, scope: !40, inlinedAt: !50)
!50 = distinct !DILocation(line: 40, column: 18, scope: !27)
!51 = !DILocation(line: 82, column: 17, scope: !40, inlinedAt: !50)
!52 = !DILocation(line: 40, column: 9, scope: !27)
!53 = !DILocation(line: 41, column: 9, scope: !27)
!54 = distinct !DISubprogram(name: "area", linkageName: "_ZN5debug4area17h41c862acc5230fecE", scope: !9, file: !8, line: 29, type: !55, scopeLine: 29, flags: DIFlagPrototyped, spFlags: DISPFlagDefinition, unit: !5, templateParams: !21, retainedNodes: !74)
!55 = !DISubroutineType(types: !56)
!56 = !{!12, !57}
!57 = !DIDerivedType(tag: DW_TAG_pointer_type, name: "&debug::Shape", baseType: !58, size: 64, align: 64, dwarfAddressSpace: 0)
!58 = !DICompositeType(tag: DW_TAG_structure_type, name: "Shape", scope: !9, file: !14, size: 192, align: 64, flags: DIFlagPublic, elements: !59, templateParams: !21, identifier: "7ca115c7fa561d03eac0845ee73de8bd")
!59 = !{!60}
!60 = !DICompositeType(tag: DW_TAG_variant_part, scope: !58, file: !14, size: 192, align: 64, elements: !61, templateParams: !21, identifier: "1044ed537d1b09172a64cc42791a1567", discriminator: !72)
!61 = !{!62, !64}
!62 = !DIDerivedType(tag: DW_TAG_member, name: "Empty", scope: !60, file: !14, baseType: !63, size: 192, align: 64, extraData: i64 0)
!63 = !DICompositeType(tag: DW_TAG_structure_type, name: "Empty", scope: !58, file: !14, size: 192, align: 64, flags: DIFlagPublic, elements: !21, identifier: "d1305e5f7256781550b37f548b745768")
!64 = !DIDerivedType(tag: DW_TAG_member, name: "Dot", scope: !60, file: !14, baseType: !65, size: 192, align: 64, extraData: i64 1)
!65 = !DICompositeType(tag: DW_TAG_structure_type, name: "Dot", scope: !58, file: !14, size: 192, align: 64, flags: DIFlagPublic, elements: !66, templateParams: !21, identifier: "83b476eae14e03dd263059d72e715036")
!66 = !{!67}
!67 = !DIDerivedType(tag: DW_TAG_member, name: "__0", scope: !65, file: !14, baseType: !68, size: 128, align: 64, offset: 64, flags: DIFlagPublic)
!68 = !DICompositeType(tag: DW_TAG_structure_type, name: "Point", scope: !9, file: !14, size: 128, align: 64, flags: DIFlagPublic, elements: !69, templateParams: !21, identifier: "6ccae0b5a7283fae8ce02f9e9562b99a")
!69 = !{!70, !71}
!70 = !DIDerivedType(tag: DW_TAG_member, name: "x", scope: !68, file: !14, baseType: !18, size: 32, align: 32, offset: 64, flags: DIFlagPublic)
!71 = !DIDerivedType(tag: DW_TAG_member, name: "y", scope: !68, file: !14, baseType: !12, size: 64, align: 64, flags: DIFlagPublic)
!72 = !DIDerivedType(tag: DW_TAG_member, scope: !58, file: !14, baseType: !73, size: 64, align: 64, flags: DIFlagArtificial)
!73 = !DIBasicType(name: "u64", size: 64, encoding: DW_ATE_unsigned)
!74 = !{!75, !76}
!75 = !DILocalVariable(name: "shape", arg: 1, scope: !54, file: !8, line: 29, type: !57)
!76 = !DILocalVariable(name: "point", scope: !77, file: !8, line: 32, type: !78, align: 64)
!77 = distinct !DILexicalBlock(scope: !54, file: !8, line: 32, column: 9)
!78 = !DIDerivedType(tag: DW_TAG_pointer_type, name: "&debug::Point", baseType: !68, size: 64, align: 64, dwarfAddressSpace: 0)
!79 = !DILocation(line: 29, column: 13, scope: !54)
!80 = !DILocation(line: 30, column: 11, scope: !54)
!81 = !DILocation(line: 30, column: 5, scope: !54)
!82 = !DILocation(line: 32, column: 20, scope: !54)
!83 = !DILocation(line: 32, column: 20, scope: !77)
!84 = !DILocation(line: 32, column: 30, scope: !77)
!85 = !DILocation(line: 31, column: 25, scope: !54)
!86 = !DILocation(line: 34, column: 2, scope: !54)
!87 = distinct !DISubprogram(name: "distance", linkageName: "_ZN5debug8distance17h5e3963d485c3e6a0E", scope: !9, file: !8, line: 23, type: !88, scopeLine: 23, flags: DIFlagPrototyped, spFlags: DISPFlagDefinition, unit: !5, templateParams: !21, retainedNodes: !90)
!88 = !DISubroutineType(types: !89)
!89 = !{!12, !78}
!90 = !{!91, !92, !94}
!91 = !DILocalVariable(name: "p", arg: 1, scope: !87, file: !8, line: 23, type: !78)
!92 = !DILocalVariable(name: "dx", scope: !93, file: !8, line: 24, type: !12, align: 64)
!93 = distinct !DILexicalBlock(scope: !87, file: !8, line: 24, column: 5)
!94 = !DILocalVariable(name: "dy", scope: !95, file: !8, line: 25, type: !12, align: 64)
!95 = distinct !DILexicalBlock(scope: !93, file: !8, line: 25, column: 5)
!96 = !DILocation(line: 23, column: 17, scope: !87)
!97 = !DILocation(line: 24, column: 24, scope: !87)
!98 = !DILocation(line: 79, column: 21, scope: !40, inlinedAt: !99)
!99 = distinct !DILocation(line: 24, column: 14, scope: !87)
!100 = !DILocation(line: 82, column: 17, scope: !40, inlinedAt: !99)
!101 = !DILocation(line: 24, column: 14, scope: !87)
!102 = !DILocation(line: 24, column: 9, scope: !93)
!103 = !DILocation(line: 25, column: 14, scope: !93)
!104 = !DILocation(line: 25, column: 9, scope: !95)
!105 = !DILocation(line: 26, column: 5, scope: !95)
!106 = !DILocation(line: 26, column: 15, scope: !95)
!107 = !DILocation(line: 27, column: 2, scope: !87)
