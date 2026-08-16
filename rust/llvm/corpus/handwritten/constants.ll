; ModuleID = 'constants.ll'
source_filename = "constants.ll"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

%literal = type { i32, { i8, i8 }, [2 x i16] }
%packed = type <{ i8, i32 }>

@integers = global { i1, i8, i16, i32, i64, i128 } { i1 true, i8 -128, i16 -32768, i32 -2147483648, i64 -9223372036854775808, i128 -170141183460469231731687303715884105728 }, align 8
@unsigned_extremes = global { i8, i16, i32, i64 } { i8 -1, i16 -1, i32 -1, i64 -1 }, align 8
@bools = global [2 x i1] [i1 true, i1 false], align 1
@halves = global [4 x half] [half 0xH3C00, half 0xHBC00, half 0xH7C00, half 0xH7E00], align 8
@bfloats = global [2 x bfloat] [bfloat 0xR3F80, bfloat 0xR40C0], align 4
@singles = global [4 x float] [float 1.000000e+00, float -2.500000e-01, float 0x3FB99999A0000000, float 0x7FF8000000000000], align 16
@doubles = global [5 x double] [double 0.000000e+00, double -0.000000e+00, double 1.000000e+00, double 0x400921FB54442D18, double 0x7FF0000000000000], align 8
@quads = global [2 x fp128] [fp128 0xL00000000000000003FFF000000000000, fp128 0xL00000000000000018000000000000000], align 16
@extended = global x86_fp80 0xK4001E000000000000000, align 16
@double_double = global ppc_fp128 0xM80000000000000000000000000000000, align 16
@zeroed_struct = global %literal zeroinitializer, align 4
@undefined = global i32 undef, align 4
@poisoned = global i32 poison, align 4
@null_pointer = global ptr null, align 8
@empty_array = global [0 x i32] zeroinitializer, align 4
@nested = global %literal { i32 1, { i8, i8 } { i8 2, i8 3 }, [2 x i16] [i16 4, i16 5] }, align 4
@packed_value = global %packed <{ i8 1, i32 2 }>, align 1
@string = global [12 x i8] c"hello\00world\0A", align 1
@escaped = global [4 x i8] c"\22\\\7F\01", align 1
@vector = global <4 x i32> <i32 1, i32 2, i32 3, i32 4>, align 16
@scalable = global i32 0, align 4
@self_reference = global ptr @self_reference, align 8
@forward_reference = global ptr @defined_later, align 8
@defined_later = global i32 9, align 4
@gep_expression = global ptr getelementptr inbounds ([12 x i8], ptr @string, i64 0, i64 6), align 8
@gep_nusw = global ptr getelementptr nusw ([12 x i8], ptr @string, i64 0, i64 1), align 8
@int_to_pointer = global ptr inttoptr (i64 4096 to ptr), align 8
@pointer_to_int = global i64 ptrtoint (ptr @string to i64), align 8
@truncated = global i16 4464, align 2
@cast_pointer = global ptr addrspace(1) addrspacecast (ptr @string to ptr addrspace(1)), align 8
@vector_element = global i32 3, align 4
@equivalent = global ptr dso_local_equivalent @a_function, align 8
@no_control_flow_integrity = global ptr no_cfi @a_function, align 8
@block_address_table = global ptr blockaddress(@block_addresses, %here), align 8

@aliased = alias i32, ptr @defined_later

define void @a_function() {
entry:
  ret void
}

define void @block_addresses() {
entry:
  %target = load ptr, ptr @block_address_table, align 8
  indirectbr ptr %target, [label %here]

here:                                             ; preds = %entry
  ret void
}

define void @scalable_vectors(<vscale x 4 x i32> %v) {
entry:
  %doubled = add <vscale x 4 x i32> %v, %v
  ret void
}

define void @exotic_types(ptr addrspace(1) %p, target("spirv.Image") %image) {
entry:
  %cast = addrspacecast ptr addrspace(1) %p to ptr
  ret void
}

define void @wide_integers(i128 %x, i256 %y) {
entry:
  %sum = add i128 %x, 170141183460469231731687303715884105727
  %big = add i256 %y, 57896044618658097711785492504343953926634992332820282019728792003956564819967
  ret void
}
