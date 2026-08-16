; ModuleID = 'asm.ll'
source_filename = "asm.bc4e83fea2fabdb7-cgu.0"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

; Function Attrs: nounwind nonlazybind uwtable
define i64 @_ZN3asm10clobbering17h93689d84ccf402ddE(i64 %x) unnamed_addr #0 {
start:
  %out = alloca [8 x i8], align 8
  %0 = call { i64, i32 } asm sideeffect alignstack inteldialect "mov rax, ${2:q}\0Amov ${0:q}, rax", "=&r,=&{ax},r,~{dirflag},~{fpsr},~{flags},~{memory}"(i64 %x), !srcloc !3
  %1 = extractvalue { i64, i32 } %0, 0
  store i64 %1, ptr %out, align 8
  %_0 = load i64, ptr %out, align 8
  ret i64 %_0
}

; Function Attrs: nounwind nonlazybind uwtable
define i64 @_ZN3asm3add17h202949ffa0f69d60E(i64 %a, i64 %b) unnamed_addr #0 {
start:
  %out = alloca [8 x i8], align 8
  %0 = call i64 asm sideeffect alignstack inteldialect "mov ${0:q}, ${1:q}\0Aadd ${0:q}, ${2:q}", "=&r,r,r,~{dirflag},~{fpsr},~{flags},~{memory}"(i64 %a, i64 %b), !srcloc !4
  store i64 %0, ptr %out, align 8
  %_0 = load i64, ptr %out, align 8
  ret i64 %_0
}

; Function Attrs: nounwind nonlazybind uwtable
define void @_ZN3asm7barrier17h564f4718a66ddd7fE() unnamed_addr #0 {
start:
  call void asm sideeffect inteldialect "", "~{memory}"(), !srcloc !5
  ret void
}

; Function Attrs: nounwind nonlazybind uwtable
define i64 @_ZN3asm8identity17h0b25b6489c738ef0E(i64 %x) unnamed_addr #0 {
start:
  %out = alloca [8 x i8], align 8
  %0 = call i64 asm sideeffect alignstack inteldialect "mov ${0:q}, ${1:q}", "=&r,r,~{dirflag},~{fpsr},~{flags},~{memory}"(i64 %x), !srcloc !6
  store i64 %0, ptr %out, align 8
  %_0 = load i64, ptr %out, align 8
  ret i64 %_0
}

; Function Attrs: nounwind nonlazybind uwtable
define i64 @_ZN3asm9increment17h4de627a6bf243c3bE(i64 %0) unnamed_addr #0 {
start:
  %x = alloca [8 x i8], align 8
  store i64 %0, ptr %x, align 8
  %1 = load i64, ptr %x, align 8
  %2 = call i64 asm sideeffect alignstack inteldialect "inc ${0:q}", "=&r,0,~{dirflag},~{fpsr},~{flags},~{memory}"(i64 %1), !srcloc !7
  store i64 %2, ptr %x, align 8
  %_0 = load i64, ptr %x, align 8
  ret i64 %_0
}

attributes #0 = { nounwind nonlazybind uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }

!llvm.module.flags = !{!0, !1}
!llvm.ident = !{!2}

!0 = !{i32 8, !"PIC Level", i32 2}
!1 = !{i32 2, !"RtLibUseGOT", i32 1}
!2 = !{!"rustc version 1.95.0 (59807616e 2026-04-14) (built from a source tarball)"}
!3 = !{i64 0, i64 4793183503440, i64 4861902980192}
!4 = !{i64 0, i64 3680786973517, i64 3749506450269}
!5 = !{i64 4191888081872}
!6 = !{i64 2632814953049}
!7 = !{i64 3139621094100}
