; ModuleID = 'instructions.ll'
source_filename = "instructions.ll"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

%pair = type { i32, i64 }

@global_int = global i32 0, align 4
@table = global [4 x ptr] zeroinitializer, align 8

declare void @sink(i32)

declare i32 @source()

declare i32 @personality_v0(...)

define i32 @integer_arithmetic(i32 %a, i32 %b) {
entry:
  %add = add i32 %a, %b
  %add_nuw = add nuw i32 %a, %b
  %add_nsw = add nsw i32 %a, %b
  %add_both = add nuw nsw i32 %a, %b
  %sub = sub nsw i32 %add, %b
  %mul = mul nuw i32 %sub, 3
  %udiv = udiv i32 %mul, 7
  %udiv_exact = udiv exact i32 %mul, 8
  %sdiv = sdiv i32 %mul, -7
  %urem = urem i32 %mul, 5
  %srem = srem i32 %mul, 5
  %shl = shl nuw nsw i32 %urem, 2
  %lshr = lshr exact i32 %shl, 1
  %ashr = ashr i32 %lshr, 1
  %and = and i32 %ashr, 255
  %or = or i32 %and, 16
  %or_disjoint = or disjoint i32 %and, 256
  %xor = xor i32 %or, %or_disjoint
  ret i32 %xor
}

define double @float_arithmetic(double %x, double %y) {
entry:
  %fadd = fadd double %x, %y
  %fsub = fsub fast double %fadd, %y
  %fmul = fmul nnan ninf double %fsub, %x
  %fdiv = fdiv nsz arcp double %fmul, %y
  %frem = frem reassoc contract afn double %fdiv, 2.500000e+00
  %fneg = fneg double %frem
  ret double %fneg
}

define void @conversions(i32 %i, i64 %l, double %d, ptr %p) {
entry:
  %trunc = trunc i32 %i to i8
  %trunc_nuw = trunc nuw i32 %i to i8
  %trunc_nsw = trunc nsw i32 %i to i8
  %zext = zext i32 %i to i64
  %zext_nneg = zext nneg i32 %i to i64
  %sext = sext i32 %i to i64
  %fptrunc = fptrunc double %d to float
  %fpext = fpext float %fptrunc to double
  %fptoui = fptoui double %d to i32
  %fptosi = fptosi double %d to i32
  %uitofp = uitofp i32 %i to double
  %uitofp_nneg = uitofp nneg i32 %i to double
  %sitofp = sitofp i32 %i to double
  %ptrtoint = ptrtoint ptr %p to i64
  %inttoptr = inttoptr i64 %l to ptr
  %bitcast = bitcast double %d to i64
  %addrspacecast = addrspacecast ptr %p to ptr addrspace(1)
  ret void
}

define void @comparisons(i32 %a, i32 %b, double %x, double %y) {
entry:
  %eq = icmp eq i32 %a, %b
  %ne = icmp ne i32 %a, %b
  %ugt = icmp ugt i32 %a, %b
  %uge = icmp uge i32 %a, %b
  %ult = icmp ult i32 %a, %b
  %ule = icmp ule i32 %a, %b
  %sgt = icmp sgt i32 %a, %b
  %sge = icmp samesign sge i32 %a, %b
  %slt = icmp slt i32 %a, %b
  %sle = icmp sle i32 %a, %b
  %ffalse = fcmp false double %x, %y
  %oeq = fcmp oeq double %x, %y
  %ogt = fcmp ogt double %x, %y
  %oge = fcmp oge double %x, %y
  %olt = fcmp olt double %x, %y
  %ole = fcmp ole double %x, %y
  %one = fcmp one double %x, %y
  %ford = fcmp ord double %x, %y
  %ueq = fcmp ueq double %x, %y
  %fugt = fcmp fast ugt double %x, %y
  %fuge = fcmp uge double %x, %y
  %fult = fcmp ult double %x, %y
  %fule = fcmp ule double %x, %y
  %une = fcmp une double %x, %y
  %uno = fcmp uno double %x, %y
  %ftrue = fcmp true double %x, %y
  ret void
}

define i64 @memory_operations(ptr %p, i64 %n) {
entry:
  %slot = alloca i64, align 8
  %array = alloca [8 x i32], align 16
  %counted = alloca i32, i64 %n, align 4
  store i64 0, ptr %slot, align 8
  store volatile i64 1, ptr %slot, align 8
  %loaded = load i64, ptr %slot, align 8
  %loaded_volatile = load volatile i64, ptr %slot, align 8
  %element = getelementptr inbounds [8 x i32], ptr %array, i64 0, i64 3
  %byte = getelementptr i8, ptr %p, i64 %n
  %nusw = getelementptr nusw nuw i8, ptr %p, i64 4
  %field = getelementptr inbounds %pair, ptr %p, i32 0, i32 1
  store i32 7, ptr %element, align 4
  %sum = add i64 %loaded, %loaded_volatile
  ret i64 %sum
}

define void @atomics(ptr %p, i64 %value) {
entry:
  %xchg = atomicrmw xchg ptr %p, i64 %value monotonic, align 8
  %add = atomicrmw add ptr %p, i64 %value acquire, align 8
  %sub = atomicrmw sub ptr %p, i64 %value release, align 8
  %and = atomicrmw and ptr %p, i64 %value acq_rel, align 8
  %nand = atomicrmw nand ptr %p, i64 %value seq_cst, align 8
  %or = atomicrmw or ptr %p, i64 %value monotonic, align 8
  %xor = atomicrmw xor ptr %p, i64 %value monotonic, align 8
  %max = atomicrmw max ptr %p, i64 %value monotonic, align 8
  %min = atomicrmw min ptr %p, i64 %value monotonic, align 8
  %umax = atomicrmw umax ptr %p, i64 %value monotonic, align 8
  %umin = atomicrmw umin ptr %p, i64 %value monotonic, align 8
  %uinc = atomicrmw uinc_wrap ptr %p, i64 %value monotonic, align 8
  %udec = atomicrmw udec_wrap ptr %p, i64 %value monotonic, align 8
  %volatile = atomicrmw volatile add ptr %p, i64 %value seq_cst, align 8
  %scoped = atomicrmw add ptr %p, i64 %value syncscope("singlethread") seq_cst, align 8
  %pair = cmpxchg ptr %p, i64 0, i64 %value seq_cst acquire, align 8
  %weak = cmpxchg weak ptr %p, i64 0, i64 %value release monotonic, align 8
  %both = cmpxchg weak volatile ptr %p, i64 0, i64 %value syncscope("agent") acq_rel monotonic, align 8
  fence seq_cst
  fence syncscope("singlethread") acquire
  %atomic_load = load atomic i64, ptr %p seq_cst, align 8
  store atomic i64 0, ptr %p release, align 8
  ret void
}

define <4 x i32> @vectors(<4 x i32> %v, <4 x float> %f, i32 %scalar) {
entry:
  %element = extractelement <4 x i32> %v, i32 2
  %inserted = insertelement <4 x i32> %v, i32 %scalar, i32 1
  %shuffled = shufflevector <4 x i32> %v, <4 x i32> %inserted, <4 x i32> <i32 0, i32 5, i32 2, i32 7>
  %narrowed = shufflevector <4 x i32> %v, <4 x i32> poison, <2 x i32> <i32 0, i32 1>
  %widened = shufflevector <2 x i32> %narrowed, <2 x i32> poison, <4 x i32> <i32 0, i32 1, i32 0, i32 1>
  %compared = icmp slt <4 x i32> %v, %shuffled
  %selected = select <4 x i1> %compared, <4 x i32> %v, <4 x i32> %shuffled
  %added = add <4 x i32> %selected, %widened
  ret <4 x i32> %added
}

define %pair @aggregates(%pair %p, i32 %a, i64 %b) {
entry:
  %first = extractvalue %pair %p, 0
  %second = extractvalue %pair %p, 1
  %with_first = insertvalue %pair %p, i32 %a, 0
  %with_both = insertvalue %pair %with_first, i64 %b, 1
  ret %pair %with_both
}

define i32 @control_flow(i32 %x) {
entry:
  switch i32 %x, label %default [
    i32 0, label %zero
    i32 1, label %one
    i32 100, label %many
  ]

zero:                                             ; preds = %entry
  br label %join

one:                                              ; preds = %entry
  br label %join

many:                                             ; preds = %entry
  br label %join

default:                                          ; preds = %entry
  unreachable

join:                                             ; preds = %many, %one, %zero
  %merged = phi i32 [ 0, %zero ], [ 1, %one ], [ 100, %many ]
  %frozen = freeze i32 %merged
  %chosen = select i1 true, i32 %frozen, i32 0
  ret i32 %chosen
}

define i32 @loop_with_phi(i32 %n) {
entry:
  br label %header

header:                                           ; preds = %body, %entry
  %i = phi i32 [ 0, %entry ], [ %next, %body ]
  %total = phi i32 [ 0, %entry ], [ %sum, %body ]
  %done = icmp sge i32 %i, %n
  br i1 %done, label %exit, label %body

body:                                             ; preds = %header
  %sum = add i32 %total, %i
  %next = add nuw nsw i32 %i, 1
  br label %header

exit:                                             ; preds = %header
  ret i32 %total
}

define void @calls(ptr %callee, i32 %x) {
entry:
  %direct = call i32 @source()
  %with_args = call i32 @source() #0
  call void @sink(i32 %x)
  %indirect = call i32 %callee(i32 %x)
  %tail = tail call i32 @source()
  %notail = notail call i32 @source()
  call void @sink(i32 %direct) [ "deopt"(i32 1, i64 2) ]
  ret void
}

define i32 @with_landing_pad(i32 %x) personality ptr @personality_v0 {
entry:
  %result = invoke i32 @source()
          to label %normal unwind label %cleanup

normal:                                           ; preds = %entry
  ret i32 %result

cleanup:                                          ; preds = %entry
  %exception = landingpad { ptr, i32 }
          cleanup
          catch ptr @global_int
          filter [1 x ptr] [ptr @global_int]
  resume { ptr, i32 } %exception
}

define void @indirect_branch(ptr %target) {
entry:
  indirectbr ptr %target, [label %first, label %second]

first:                                            ; preds = %entry
  ret void

second:                                           ; preds = %entry
  ret void
}

attributes #0 = { nounwind }
