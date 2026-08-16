; ModuleID = 'optimised.ll'
source_filename = "optimised.d3780980f30d9f09-cgu.0"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

; Function Attrs: nofree norecurse nosync nounwind nonlazybind memory(argmem: read) uwtable
define noundef range(i64 0, -9223372036854775808) i64 @find(ptr noalias noundef nonnull readonly align 1 captures(none) %values.0, i64 noundef range(i64 0, -9223372036854775808) %values.1, i8 noundef %needle) unnamed_addr #0 {
start:
  %_34.not = icmp eq i64 %values.1, 0
  br i1 %_34.not, label %bb7, label %bb3

bb7:                                              ; preds = %bb5, %bb3, %start
  %index.sroa.0.1 = phi i64 [ 0, %start ], [ %values.1, %bb5 ], [ %index.sroa.0.05, %bb3 ]
  ret i64 %index.sroa.0.1

bb3:                                              ; preds = %bb5, %start
  %index.sroa.0.05 = phi i64 [ %1, %bb5 ], [ 0, %start ]
  %0 = getelementptr inbounds nuw i8, ptr %values.0, i64 %index.sroa.0.05
  %_7 = load i8, ptr %0, align 1, !noundef !3
  %_6 = icmp eq i8 %_7, %needle
  br i1 %_6, label %bb7, label %bb5

bb5:                                              ; preds = %bb3
  %1 = add nuw nsw i64 %index.sroa.0.05, 1
  %exitcond.not = icmp eq i64 %1, %values.1
  br i1 %exitcond.not, label %bb7, label %bb3
}

; Function Attrs: nofree norecurse nosync nounwind nonlazybind memory(argmem: readwrite) uwtable
define void @saturate(ptr noalias noundef nonnull align 2 captures(none) %values.0, i64 noundef range(i64 0, 4611686018427387904) %values.1) unnamed_addr #1 {
start:
  %_35.not = icmp eq i64 %values.1, 0
  br i1 %_35.not, label %bb5, label %iter.check

iter.check:                                       ; preds = %start
  %min.iters.check = icmp samesign ult i64 %values.1, 4
  br i1 %min.iters.check, label %bb4.preheader, label %vector.main.loop.iter.check

bb4.preheader:                                    ; preds = %vec.epilog.middle.block, %vec.epilog.iter.check, %iter.check
  %index.sroa.0.06.ph = phi i64 [ 0, %iter.check ], [ %n.vec, %vec.epilog.iter.check ], [ %n.vec10, %vec.epilog.middle.block ]
  br label %bb4

vector.main.loop.iter.check:                      ; preds = %iter.check
  %min.iters.check7 = icmp samesign ult i64 %values.1, 16
  br i1 %min.iters.check7, label %vec.epilog.ph, label %vector.ph

vector.ph:                                        ; preds = %vector.main.loop.iter.check
  %n.vec = and i64 %values.1, 4611686018427387888
  br label %vector.body

vector.body:                                      ; preds = %vector.body, %vector.ph
  %index = phi i64 [ 0, %vector.ph ], [ %index.next, %vector.body ]
  %0 = getelementptr inbounds nuw i16, ptr %values.0, i64 %index
  %1 = getelementptr inbounds nuw i8, ptr %0, i64 16
  %wide.load = load <8 x i16>, ptr %0, align 2
  %wide.load8 = load <8 x i16>, ptr %1, align 2
  %2 = tail call <8 x i16> @llvm.sadd.sat.v8i16(<8 x i16> %wide.load, <8 x i16> splat (i16 1))
  %3 = tail call <8 x i16> @llvm.sadd.sat.v8i16(<8 x i16> %wide.load8, <8 x i16> splat (i16 1))
  store <8 x i16> %2, ptr %0, align 2
  store <8 x i16> %3, ptr %1, align 2
  %index.next = add nuw i64 %index, 16
  %4 = icmp eq i64 %index.next, %n.vec
  br i1 %4, label %middle.block, label %vector.body, !llvm.loop !4

middle.block:                                     ; preds = %vector.body
  %cmp.n = icmp eq i64 %values.1, %n.vec
  br i1 %cmp.n, label %bb5, label %vec.epilog.iter.check

vec.epilog.iter.check:                            ; preds = %middle.block
  %n.vec.remaining = and i64 %values.1, 12
  %min.epilog.iters.check = icmp eq i64 %n.vec.remaining, 0
  br i1 %min.epilog.iters.check, label %bb4.preheader, label %vec.epilog.ph

vec.epilog.ph:                                    ; preds = %vec.epilog.iter.check, %vector.main.loop.iter.check
  %vec.epilog.resume.val = phi i64 [ %n.vec, %vec.epilog.iter.check ], [ 0, %vector.main.loop.iter.check ]
  %n.vec10 = and i64 %values.1, 4611686018427387900
  br label %vec.epilog.vector.body

vec.epilog.vector.body:                           ; preds = %vec.epilog.vector.body, %vec.epilog.ph
  %index11 = phi i64 [ %vec.epilog.resume.val, %vec.epilog.ph ], [ %index.next13, %vec.epilog.vector.body ]
  %5 = getelementptr inbounds nuw i16, ptr %values.0, i64 %index11
  %wide.load12 = load <4 x i16>, ptr %5, align 2
  %6 = tail call <4 x i16> @llvm.sadd.sat.v4i16(<4 x i16> %wide.load12, <4 x i16> splat (i16 1))
  store <4 x i16> %6, ptr %5, align 2
  %index.next13 = add nuw i64 %index11, 4
  %7 = icmp eq i64 %index.next13, %n.vec10
  br i1 %7, label %vec.epilog.middle.block, label %vec.epilog.vector.body, !llvm.loop !7

vec.epilog.middle.block:                          ; preds = %vec.epilog.vector.body
  %cmp.n14 = icmp eq i64 %values.1, %n.vec10
  br i1 %cmp.n14, label %bb5, label %bb4.preheader

bb5:                                              ; preds = %bb4, %vec.epilog.middle.block, %middle.block, %start
  ret void

bb4:                                              ; preds = %bb4, %bb4.preheader
  %index.sroa.0.06 = phi i64 [ %10, %bb4 ], [ %index.sroa.0.06.ph, %bb4.preheader ]
  %8 = getelementptr inbounds nuw i16, ptr %values.0, i64 %index.sroa.0.06
  %_7 = load i16, ptr %8, align 2, !noundef !3
  %9 = tail call i16 @llvm.sadd.sat.i16(i16 %_7, i16 1)
  store i16 %9, ptr %8, align 2
  %10 = add nuw nsw i64 %index.sroa.0.06, 1
  %exitcond.not = icmp eq i64 %10, %values.1
  br i1 %exitcond.not, label %bb5, label %bb4, !llvm.loop !8
}

; Function Attrs: nofree norecurse nosync nounwind nonlazybind memory(argmem: readwrite) uwtable
define void @scale(ptr noalias noundef nonnull align 4 captures(none) %values.0, i64 noundef range(i64 0, 2305843009213693952) %values.1, float noundef %factor) unnamed_addr #1 {
start:
  %_43.not = icmp eq i64 %values.1, 0
  br i1 %_43.not, label %bb4, label %bb3.preheader

bb3.preheader:                                    ; preds = %start
  %min.iters.check = icmp samesign ult i64 %values.1, 8
  br i1 %min.iters.check, label %bb3.preheader6, label %vector.ph

vector.ph:                                        ; preds = %bb3.preheader
  %n.vec = and i64 %values.1, 2305843009213693944
  %broadcast.splatinsert = insertelement <4 x float> poison, float %factor, i64 0
  %broadcast.splat = shufflevector <4 x float> %broadcast.splatinsert, <4 x float> poison, <4 x i32> zeroinitializer
  br label %vector.body

vector.body:                                      ; preds = %vector.body, %vector.ph
  %index = phi i64 [ 0, %vector.ph ], [ %index.next, %vector.body ]
  %0 = getelementptr inbounds nuw float, ptr %values.0, i64 %index
  %1 = getelementptr inbounds nuw i8, ptr %0, i64 16
  %wide.load = load <4 x float>, ptr %0, align 4
  %wide.load5 = load <4 x float>, ptr %1, align 4
  %2 = fmul <4 x float> %broadcast.splat, %wide.load
  %3 = fmul <4 x float> %broadcast.splat, %wide.load5
  store <4 x float> %2, ptr %0, align 4
  store <4 x float> %3, ptr %1, align 4
  %index.next = add nuw i64 %index, 8
  %4 = icmp eq i64 %index.next, %n.vec
  br i1 %4, label %middle.block, label %vector.body, !llvm.loop !9

middle.block:                                     ; preds = %vector.body
  %cmp.n = icmp eq i64 %values.1, %n.vec
  br i1 %cmp.n, label %bb4, label %bb3.preheader6

bb3.preheader6:                                   ; preds = %middle.block, %bb3.preheader
  %index.sroa.0.04.ph = phi i64 [ 0, %bb3.preheader ], [ %n.vec, %middle.block ]
  br label %bb3

bb4:                                              ; preds = %bb3, %middle.block, %start
  ret void

bb3:                                              ; preds = %bb3, %bb3.preheader6
  %index.sroa.0.04 = phi i64 [ %8, %bb3 ], [ %index.sroa.0.04.ph, %bb3.preheader6 ]
  %5 = getelementptr inbounds nuw float, ptr %values.0, i64 %index.sroa.0.04
  %6 = load float, ptr %5, align 4, !noundef !3
  %7 = fmul float %factor, %6
  store float %7, ptr %5, align 4
  %8 = add nuw nsw i64 %index.sroa.0.04, 1
  %exitcond.not = icmp eq i64 %8, %values.1
  br i1 %exitcond.not, label %bb4, label %bb3, !llvm.loop !10
}

; Function Attrs: nofree norecurse nosync nounwind nonlazybind memory(argmem: read) uwtable
define noundef i32 @sum(ptr noalias noundef nonnull readonly align 4 captures(none) %values.0, i64 noundef range(i64 0, 2305843009213693952) %values.1) unnamed_addr #0 {
start:
  %_34.not = icmp eq i64 %values.1, 0
  br i1 %_34.not, label %bb4, label %bb3.preheader

bb3.preheader:                                    ; preds = %start
  %min.iters.check = icmp samesign ult i64 %values.1, 8
  br i1 %min.iters.check, label %bb3.preheader9, label %vector.ph

vector.ph:                                        ; preds = %bb3.preheader
  %n.vec = and i64 %values.1, 2305843009213693944
  br label %vector.body

vector.body:                                      ; preds = %vector.body, %vector.ph
  %index = phi i64 [ 0, %vector.ph ], [ %index.next, %vector.body ]
  %vec.phi = phi <4 x i32> [ zeroinitializer, %vector.ph ], [ %2, %vector.body ]
  %vec.phi7 = phi <4 x i32> [ zeroinitializer, %vector.ph ], [ %3, %vector.body ]
  %0 = getelementptr inbounds nuw i32, ptr %values.0, i64 %index
  %1 = getelementptr inbounds nuw i8, ptr %0, i64 16
  %wide.load = load <4 x i32>, ptr %0, align 4
  %wide.load8 = load <4 x i32>, ptr %1, align 4
  %2 = add <4 x i32> %wide.load, %vec.phi
  %3 = add <4 x i32> %wide.load8, %vec.phi7
  %index.next = add nuw i64 %index, 8
  %4 = icmp eq i64 %index.next, %n.vec
  br i1 %4, label %middle.block, label %vector.body, !llvm.loop !11

middle.block:                                     ; preds = %vector.body
  %bin.rdx = add <4 x i32> %3, %2
  %5 = tail call i32 @llvm.vector.reduce.add.v4i32(<4 x i32> %bin.rdx)
  %cmp.n = icmp eq i64 %values.1, %n.vec
  br i1 %cmp.n, label %bb4, label %bb3.preheader9

bb3.preheader9:                                   ; preds = %middle.block, %bb3.preheader
  %total.sroa.0.06.ph = phi i32 [ 0, %bb3.preheader ], [ %5, %middle.block ]
  %index.sroa.0.05.ph = phi i64 [ 0, %bb3.preheader ], [ %n.vec, %middle.block ]
  br label %bb3

bb4:                                              ; preds = %bb3, %middle.block, %start
  %total.sroa.0.0.lcssa = phi i32 [ 0, %start ], [ %5, %middle.block ], [ %_6, %bb3 ]
  ret i32 %total.sroa.0.0.lcssa

bb3:                                              ; preds = %bb3, %bb3.preheader9
  %total.sroa.0.06 = phi i32 [ %_6, %bb3 ], [ %total.sroa.0.06.ph, %bb3.preheader9 ]
  %index.sroa.0.05 = phi i64 [ %7, %bb3 ], [ %index.sroa.0.05.ph, %bb3.preheader9 ]
  %6 = getelementptr inbounds nuw i32, ptr %values.0, i64 %index.sroa.0.05
  %_7 = load i32, ptr %6, align 4, !noundef !3
  %_6 = add i32 %_7, %total.sroa.0.06
  %7 = add nuw nsw i64 %index.sroa.0.05, 1
  %exitcond.not = icmp eq i64 %7, %values.1
  br i1 %exitcond.not, label %bb4, label %bb3, !llvm.loop !12
}

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare i16 @llvm.sadd.sat.i16(i16, i16) #2

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare <8 x i16> @llvm.sadd.sat.v8i16(<8 x i16>, <8 x i16>) #2

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare <4 x i16> @llvm.sadd.sat.v4i16(<4 x i16>, <4 x i16>) #2

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare i32 @llvm.vector.reduce.add.v4i32(<4 x i32>) #2

attributes #0 = { nofree norecurse nosync nounwind nonlazybind memory(argmem: read) uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #1 = { nofree norecurse nosync nounwind nonlazybind memory(argmem: readwrite) uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }
attributes #2 = { nocallback nofree nosync nounwind speculatable willreturn memory(none) }

!llvm.module.flags = !{!0, !1}
!llvm.ident = !{!2}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 2, !"RtLibUseGOT", i32 1}
!2 = !{!"rustc version 1.95.0 (59807616e 2026-04-14) (built from a source tarball)"}
!3 = !{}
!4 = distinct !{!4, !5, !6}
!5 = !{!"llvm.loop.isvectorized", i32 1}
!6 = !{!"llvm.loop.unroll.runtime.disable"}
!7 = distinct !{!7, !5, !6}
!8 = distinct !{!8, !6, !5}
!9 = distinct !{!9, !5, !6}
!10 = distinct !{!10, !6, !5}
!11 = distinct !{!11, !5, !6}
!12 = distinct !{!12, !6, !5}
