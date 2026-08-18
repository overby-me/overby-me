//! Maps GCC __builtin_* function names to their libc/standard equivalents.
//!
//! Many C programs use GCC builtins (e.g., __builtin_abort, __builtin_memcpy).
//! We map these to their standard library equivalents so the linker can resolve them.

use crate::common::fx_hash::FxHashMap;
use std::sync::LazyLock;

/// Static mapping of __builtin_* names to their libc equivalents.
static BUILTIN_MAP: LazyLock<FxHashMap<&'static str, BuiltinInfo>> = LazyLock::new(|| {
    let mut m = FxHashMap::default();

    // Abort/exit
    // Note: __builtin_trap and __builtin_unreachable are handled directly in
    // expr_builtins.rs as Terminator::Unreachable (emitting ud2/brk/ebreak),
    // not as calls to abort(). This is critical for kernel code where abort()
    // doesn't exist.
    m.insert("__builtin_abort", BuiltinInfo::simple("abort"));
    m.insert("__builtin_exit", BuiltinInfo::simple("exit"));

    // Memory functions
    m.insert("__builtin_memcpy", BuiltinInfo::simple("memcpy"));
    m.insert("__builtin_memmove", BuiltinInfo::simple("memmove"));
    m.insert("__builtin_memset", BuiltinInfo::simple("memset"));
    m.insert("__builtin_memcmp", BuiltinInfo::simple("memcmp"));
    m.insert("__builtin_strlen", BuiltinInfo::simple("strlen"));
    m.insert("__builtin_strcpy", BuiltinInfo::simple("strcpy"));
    m.insert("__builtin_strncpy", BuiltinInfo::simple("strncpy"));
    m.insert("__builtin_strcmp", BuiltinInfo::simple("strcmp"));
    m.insert("__builtin_strncmp", BuiltinInfo::simple("strncmp"));
    m.insert("__builtin_strcat", BuiltinInfo::simple("strcat"));
    m.insert("__builtin_strchr", BuiltinInfo::simple("strchr"));
    m.insert("__builtin_strrchr", BuiltinInfo::simple("strrchr"));
    m.insert("__builtin_strstr", BuiltinInfo::simple("strstr"));

    // Math functions
    m.insert("__builtin_abs", BuiltinInfo::simple("abs"));
    m.insert("__builtin_labs", BuiltinInfo::simple("labs"));
    m.insert("__builtin_llabs", BuiltinInfo::simple("llabs"));
    m.insert("__builtin_fabs", BuiltinInfo::simple("fabs"));
    m.insert("__builtin_fabsf", BuiltinInfo::simple("fabsf"));
    m.insert("__builtin_fabsl", BuiltinInfo::simple("fabsl"));
    m.insert("__builtin_sqrt", BuiltinInfo::simple("sqrt"));
    m.insert("__builtin_sqrtf", BuiltinInfo::simple("sqrtf"));
    m.insert("__builtin_sin", BuiltinInfo::simple("sin"));
    m.insert("__builtin_sinf", BuiltinInfo::simple("sinf"));
    m.insert("__builtin_cos", BuiltinInfo::simple("cos"));
    m.insert("__builtin_cosf", BuiltinInfo::simple("cosf"));
    m.insert("__builtin_log", BuiltinInfo::simple("log"));
    m.insert("__builtin_logf", BuiltinInfo::simple("logf"));
    m.insert("__builtin_log2", BuiltinInfo::simple("log2"));
    m.insert("__builtin_exp", BuiltinInfo::simple("exp"));
    m.insert("__builtin_expf", BuiltinInfo::simple("expf"));
    m.insert("__builtin_pow", BuiltinInfo::simple("pow"));
    m.insert("__builtin_powf", BuiltinInfo::simple("powf"));
    m.insert("__builtin_floor", BuiltinInfo::simple("floor"));
    m.insert("__builtin_floorf", BuiltinInfo::simple("floorf"));
    m.insert("__builtin_ceil", BuiltinInfo::simple("ceil"));
    m.insert("__builtin_ceilf", BuiltinInfo::simple("ceilf"));
    m.insert("__builtin_round", BuiltinInfo::simple("round"));
    m.insert("__builtin_roundf", BuiltinInfo::simple("roundf"));
    m.insert("__builtin_fmin", BuiltinInfo::simple("fmin"));
    m.insert("__builtin_fmax", BuiltinInfo::simple("fmax"));
    m.insert("__builtin_copysign", BuiltinInfo::simple("copysign"));
    m.insert("__builtin_copysignf", BuiltinInfo::simple("copysignf"));
    m.insert("__builtin_nextafter", BuiltinInfo::simple("nextafter"));
    m.insert("__builtin_nextafterf", BuiltinInfo::simple("nextafterf"));
    m.insert("__builtin_nextafterl", BuiltinInfo::simple("nextafterl"));
    // TODO: __builtin_nan(s) ignores the string payload argument (NaN payload).
    // For common usage with "" this is correct; full payload support needs custom lowering.
    m.insert("__builtin_nan", BuiltinInfo::constant_f64(f64::NAN));
    m.insert("__builtin_nanf", BuiltinInfo::constant_f64(f64::NAN));
    m.insert("__builtin_inf", BuiltinInfo::constant_f64(f64::INFINITY));
    m.insert("__builtin_inff", BuiltinInfo::constant_f64(f64::INFINITY));
    m.insert("__builtin_infl", BuiltinInfo::constant_f64(f64::INFINITY));
    m.insert(
        "__builtin_huge_val",
        BuiltinInfo::constant_f64(f64::INFINITY),
    );
    m.insert(
        "__builtin_huge_valf",
        BuiltinInfo::constant_f64(f64::INFINITY),
    );
    m.insert(
        "__builtin_huge_vall",
        BuiltinInfo::constant_f64(f64::INFINITY),
    );
    m.insert("__builtin_nanl", BuiltinInfo::constant_f64(f64::NAN));

    // I/O
    m.insert("__builtin_printf", BuiltinInfo::simple("printf"));
    m.insert("__builtin_fprintf", BuiltinInfo::simple("fprintf"));
    m.insert("__builtin_sprintf", BuiltinInfo::simple("sprintf"));
    m.insert("__builtin_snprintf", BuiltinInfo::simple("snprintf"));
    m.insert("__builtin_puts", BuiltinInfo::simple("puts"));
    m.insert("__builtin_putchar", BuiltinInfo::simple("putchar"));

    // Allocation
    m.insert("__builtin_malloc", BuiltinInfo::simple("malloc"));
    m.insert("__builtin_calloc", BuiltinInfo::simple("calloc"));
    m.insert("__builtin_realloc", BuiltinInfo::simple("realloc"));
    m.insert("__builtin_free", BuiltinInfo::simple("free"));

    // Stack allocation - handled specially in try_lower_builtin_call as DynAlloca
    m.insert(
        "__builtin_alloca",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Alloca),
    );
    m.insert(
        "__builtin_alloca_with_align",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Alloca),
    );

    // Return address / frame address / thread pointer
    m.insert(
        "__builtin_return_address",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ReturnAddress),
    );
    m.insert(
        "__builtin_frame_address",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FrameAddress),
    );
    m.insert("__builtin_extract_return_addr", BuiltinInfo::identity());
    m.insert(
        "__builtin_thread_pointer",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ThreadPointer),
    );

    // Compiler hints (these become no-ops or identity)
    m.insert("__builtin_expect", BuiltinInfo::identity()); // returns first arg
    m.insert("__builtin_expect_with_probability", BuiltinInfo::identity());
    m.insert("__builtin_assume_aligned", BuiltinInfo::identity());

    // Type queries (compile-time constants)
    m.insert(
        "__builtin_constant_p",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ConstantP),
    );
    m.insert(
        "__builtin_object_size",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ObjectSize),
    );
    m.insert(
        "__builtin_dynamic_object_size",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ObjectSize),
    );
    m.insert(
        "__builtin_classify_type",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ClassifyType),
    );

    // Fortification builtins: __builtin___*_chk variants used by glibc's _FORTIFY_SOURCE.
    // These forward all arguments to the glibc __*_chk runtime function.
    m.insert(
        "__builtin___memcpy_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___memmove_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___memset_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___strcpy_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___strncpy_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___strcat_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___strncat_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___sprintf_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___snprintf_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___vsprintf_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___vsnprintf_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___printf_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___fprintf_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___vprintf_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___vfprintf_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___mempcpy_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___stpcpy_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin___stpncpy_chk",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FortifyChk),
    );
    m.insert(
        "__builtin_va_arg_pack",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::VaArgPack),
    );
    m.insert(
        "__builtin_va_arg_pack_len",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::VaArgPack),
    );
    // Note: __builtin_types_compatible_p is handled as a special AST node (BuiltinTypesCompatibleP),
    // parsed directly in the parser and evaluated at compile-time in the lowerer.

    // Floating-point comparison builtins
    m.insert(
        "__builtin_isgreater",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FpCompare),
    );
    m.insert(
        "__builtin_isgreaterequal",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FpCompare),
    );
    m.insert(
        "__builtin_isless",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FpCompare),
    );
    m.insert(
        "__builtin_islessequal",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FpCompare),
    );
    m.insert(
        "__builtin_islessgreater",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FpCompare),
    );
    m.insert(
        "__builtin_isunordered",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FpCompare),
    );

    // Floating-point classification builtins
    m.insert(
        "__builtin_fpclassify",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::FpClassify),
    );
    m.insert(
        "__builtin_isnan",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::IsNan),
    );
    m.insert(
        "__builtin_isinf",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::IsInf),
    );
    m.insert(
        "__builtin_isfinite",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::IsFinite),
    );
    m.insert(
        "__builtin_isnormal",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::IsNormal),
    );
    m.insert(
        "__builtin_signbit",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::SignBit),
    );
    m.insert(
        "__builtin_signbitf",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::SignBit),
    );
    m.insert(
        "__builtin_signbitl",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::SignBit),
    );
    m.insert(
        "__builtin_isinf_sign",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::IsInfSign),
    );

    // Bit manipulation
    m.insert(
        "__builtin_clz",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Clz),
    );
    m.insert(
        "__builtin_clzl",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Clz),
    );
    m.insert(
        "__builtin_clzll",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Clz),
    );
    m.insert(
        "__builtin_ctz",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Ctz),
    );
    m.insert(
        "__builtin_ctzl",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Ctz),
    );
    m.insert(
        "__builtin_ctzll",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Ctz),
    );
    m.insert(
        "__builtin_popcount",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Popcount),
    );
    m.insert(
        "__builtin_popcountl",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Popcount),
    );
    m.insert(
        "__builtin_popcountll",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Popcount),
    );
    m.insert(
        "__builtin_bswap16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Bswap),
    );
    m.insert(
        "__builtin_bswap32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Bswap),
    );
    m.insert(
        "__builtin_bswap64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Bswap),
    );
    m.insert(
        "__builtin_ffs",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Ffs),
    );
    m.insert(
        "__builtin_ffsl",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Ffs),
    );
    m.insert(
        "__builtin_ffsll",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Ffs),
    );
    m.insert(
        "__builtin_parity",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Parity),
    );
    m.insert(
        "__builtin_parityl",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Parity),
    );
    m.insert(
        "__builtin_parityll",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Parity),
    );
    m.insert(
        "__builtin_clrsb",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Clrsb),
    );
    m.insert(
        "__builtin_clrsbl",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Clrsb),
    );
    m.insert(
        "__builtin_clrsbll",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Clrsb),
    );

    // Overflow-checking arithmetic builtins
    // Generic (type-deduced from arguments):
    m.insert(
        "__builtin_add_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::AddOverflow),
    );
    m.insert(
        "__builtin_sub_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::SubOverflow),
    );
    m.insert(
        "__builtin_mul_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::MulOverflow),
    );
    // Signed int variants:
    m.insert(
        "__builtin_sadd_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::AddOverflow),
    );
    m.insert(
        "__builtin_saddl_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::AddOverflow),
    );
    m.insert(
        "__builtin_saddll_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::AddOverflow),
    );
    m.insert(
        "__builtin_ssub_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::SubOverflow),
    );
    m.insert(
        "__builtin_ssubl_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::SubOverflow),
    );
    m.insert(
        "__builtin_ssubll_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::SubOverflow),
    );
    m.insert(
        "__builtin_smul_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::MulOverflow),
    );
    m.insert(
        "__builtin_smull_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::MulOverflow),
    );
    m.insert(
        "__builtin_smulll_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::MulOverflow),
    );
    // Unsigned int variants:
    m.insert(
        "__builtin_uadd_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::AddOverflow),
    );
    m.insert(
        "__builtin_uaddl_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::AddOverflow),
    );
    m.insert(
        "__builtin_uaddll_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::AddOverflow),
    );
    m.insert(
        "__builtin_usub_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::SubOverflow),
    );
    m.insert(
        "__builtin_usubl_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::SubOverflow),
    );
    m.insert(
        "__builtin_usubll_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::SubOverflow),
    );
    m.insert(
        "__builtin_umul_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::MulOverflow),
    );
    m.insert(
        "__builtin_umull_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::MulOverflow),
    );
    m.insert(
        "__builtin_umulll_overflow",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::MulOverflow),
    );

    // Overflow-checking predicate builtins (GCC 7+):
    // __builtin_{add,sub,mul}_overflow_p(a, b, (T)0) -> 1 if op overflows type T
    // These are like the non-_p variants but don't store a result.
    m.insert(
        "__builtin_add_overflow_p",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::AddOverflowP),
    );
    m.insert(
        "__builtin_sub_overflow_p",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::SubOverflowP),
    );
    m.insert(
        "__builtin_mul_overflow_p",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::MulOverflowP),
    );

    // Atomics (map to libc atomic helpers for now)
    m.insert(
        "__sync_synchronize",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Fence),
    );

    // Complex number functions (C99 <complex.h>)
    m.insert(
        "creal",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexReal),
    );
    m.insert(
        "crealf",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexReal),
    );
    m.insert(
        "creall",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexReal),
    );
    m.insert(
        "cimag",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexImag),
    );
    m.insert(
        "cimagf",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexImag),
    );
    m.insert(
        "cimagl",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexImag),
    );
    m.insert(
        "__builtin_creal",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexReal),
    );
    m.insert(
        "__builtin_crealf",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexReal),
    );
    m.insert(
        "__builtin_creall",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexReal),
    );
    m.insert(
        "__builtin_cimag",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexImag),
    );
    m.insert(
        "__builtin_cimagf",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexImag),
    );
    m.insert(
        "__builtin_cimagl",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexImag),
    );
    m.insert(
        "conj",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexConj),
    );
    m.insert(
        "conjf",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexConj),
    );
    m.insert(
        "conjl",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexConj),
    );
    m.insert(
        "__builtin_conj",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexConj),
    );
    m.insert(
        "__builtin_conjf",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexConj),
    );
    m.insert(
        "__builtin_conjl",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexConj),
    );

    // Complex construction
    m.insert(
        "__builtin_complex",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::ComplexConstruct),
    );

    // Variadic argument builtins - these are handled specially in IR lowering
    // (expr.rs try_lower_builtin_call), but must be registered here so sema
    // does not emit "implicit declaration" warnings. Those warnings break
    // configure scripts that check stderr for errors (e.g., zlib).
    m.insert(
        "__builtin_va_start",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::VaStart),
    );
    m.insert(
        "__builtin_va_end",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::VaEnd),
    );
    m.insert(
        "__builtin_va_copy",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::VaCopy),
    );

    // Prefetch (no-op, handled separately in lowering)
    m.insert(
        "__builtin_prefetch",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Nop),
    );

    // CPU feature detection builtins
    // __builtin_cpu_init() initializes CPU feature detection; on glibc systems
    // this is automatic, so we emit it as a no-op.
    m.insert(
        "__builtin_cpu_init",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::CpuInit),
    );
    // __builtin_cpu_supports("feature") returns nonzero if the CPU supports
    // the named feature. We conservatively return 0 (not supported) so that
    // code always takes the safe fallback path.
    m.insert(
        "__builtin_cpu_supports",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::CpuSupports),
    );

    // Cache flush - maps to __clear_cache runtime function (provided by libgcc/glibc).
    // On x86 this is a no-op (cache coherent), on ARM/RISC-V it flushes icache.
    m.insert(
        "__builtin___clear_cache",
        BuiltinInfo::simple("__clear_cache"),
    );

    // Vector construction builtins used by SSE header wrapper functions.
    // The actual _mm_set1_* calls are intercepted as direct builtins, but the
    // function bodies in emmintrin.h still reference these, so register as Nop
    // to avoid linker errors from the compiled (but never called) wrappers.
    m.insert(
        "__builtin_ia32_vec_init_v16qi",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Nop),
    );
    m.insert(
        "__builtin_ia32_vec_init_v4si",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Nop),
    );
    m.insert(
        "__builtin_ia32_vec_init_v8hi",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::Nop),
    );

    // x86 SSE/SSE2/SSE4.2 intrinsic builtins (__builtin_ia32_* names)
    m.insert(
        "__builtin_ia32_lfence",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Lfence),
    );
    m.insert(
        "__builtin_ia32_mfence",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Mfence),
    );
    m.insert(
        "__builtin_ia32_sfence",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Sfence),
    );
    m.insert(
        "__builtin_ia32_clflush",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Clflush),
    );
    m.insert(
        "__builtin_ia32_pause",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pause),
    );
    m.insert(
        "__builtin_ia32_movnti",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Movnti),
    );
    m.insert(
        "__builtin_ia32_movnti64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Movnti64),
    );
    m.insert(
        "__builtin_ia32_movntdq",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Movntdq),
    );
    m.insert(
        "__builtin_ia32_movntpd",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Movntpd),
    );
    m.insert(
        "__builtin_ia32_loaddqu",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Loaddqu),
    );
    m.insert(
        "__builtin_ia32_storedqu",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Storedqu),
    );
    m.insert(
        "__builtin_ia32_pcmpeqb128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pcmpeqb128),
    );
    m.insert(
        "__builtin_ia32_pcmpeqd128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pcmpeqd128),
    );
    m.insert(
        "__builtin_ia32_psubusb128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubusb128),
    );
    m.insert(
        "__builtin_ia32_psubsb128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubsb128),
    );
    m.insert(
        "__builtin_ia32_por128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Por128),
    );
    m.insert(
        "__builtin_ia32_pand128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pand128),
    );
    m.insert(
        "__builtin_ia32_pxor128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pxor128),
    );
    m.insert(
        "__builtin_ia32_pmovmskb128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmovmskb128),
    );
    m.insert(
        "__builtin_ia32_set1_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Set1Epi8),
    );
    m.insert(
        "__builtin_ia32_set1_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Set1Epi32),
    );
    m.insert(
        "__builtin_ia32_crc32qi",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Crc32_8),
    );
    m.insert(
        "__builtin_ia32_crc32hi",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Crc32_16),
    );
    m.insert(
        "__builtin_ia32_crc32si",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Crc32_32),
    );
    m.insert(
        "__builtin_ia32_crc32di",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Crc32_64),
    );

    // x86 AES-NI builtins
    m.insert(
        "__builtin_ia32_aesenc128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aesenc128),
    );
    m.insert(
        "__builtin_ia32_aesenclast128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aesenclast128),
    );
    m.insert(
        "__builtin_ia32_aesdec128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aesdec128),
    );
    m.insert(
        "__builtin_ia32_aesdeclast128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aesdeclast128),
    );
    m.insert(
        "__builtin_ia32_aesimc128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aesimc128),
    );
    m.insert(
        "__builtin_ia32_aeskeygenassist128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aeskeygenassist128),
    );
    // x86 CLMUL
    m.insert(
        "__builtin_ia32_pclmulqdq128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pclmulqdq128),
    );
    // x86 SSE2 shift/shuffle builtins
    m.insert(
        "__builtin_ia32_pslldqi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pslldqi128),
    );
    m.insert(
        "__builtin_ia32_psrldqi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psrldqi128),
    );
    m.insert(
        "__builtin_ia32_psllqi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psllqi128),
    );
    m.insert(
        "__builtin_ia32_psrlqi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psrlqi128),
    );
    m.insert(
        "__builtin_ia32_pshufd128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pshufd128),
    );
    m.insert(
        "__builtin_ia32_loadldi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Loadldi128),
    );

    // Direct _mm_* function name mappings (bypass wrapper functions, avoid ABI issues)
    m.insert(
        "_mm_loadu_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Loaddqu),
    );
    m.insert(
        "_mm_load_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Loaddqu),
    );
    m.insert(
        "_mm_storeu_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Storedqu),
    );
    m.insert(
        "_mm_store_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Storedqu),
    );
    m.insert(
        "_mm_set1_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Set1Epi8),
    );
    m.insert(
        "_mm_set1_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Set1Epi32),
    );
    // _mm_setzero_si128 is handled by its header inline function (returns compound literal)
    // Do NOT map it here -- it takes 0 args, unlike Set1Epi8 which takes 1.
    m.insert(
        "_mm_cmpeq_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pcmpeqb128),
    );
    m.insert(
        "_mm_cmpeq_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pcmpeqd128),
    );
    m.insert(
        "_mm_subs_epu8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubusb128),
    );
    m.insert(
        "_mm_subs_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubsb128),
    );
    m.insert(
        "_mm_or_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Por128),
    );
    m.insert(
        "_mm_and_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pand128),
    );
    m.insert(
        "_mm_xor_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pxor128),
    );
    m.insert(
        "_mm_movemask_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmovmskb128),
    );
    m.insert(
        "_mm_stream_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Movntdq),
    );
    m.insert(
        "_mm_stream_si64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Movnti64),
    );
    m.insert(
        "_mm_stream_si32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Movnti),
    );
    m.insert(
        "_mm_stream_pd",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Movntpd),
    );
    m.insert(
        "_mm_lfence",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Lfence),
    );
    m.insert(
        "_mm_mfence",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Mfence),
    );
    m.insert(
        "_mm_sfence",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Sfence),
    );
    m.insert(
        "_mm_clflush",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Clflush),
    );
    m.insert(
        "_mm_pause",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pause),
    );
    m.insert(
        "_mm_crc32_u8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Crc32_8),
    );
    m.insert(
        "_mm_crc32_u16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Crc32_16),
    );
    m.insert(
        "_mm_crc32_u32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Crc32_32),
    );
    m.insert(
        "_mm_crc32_u64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Crc32_64),
    );
    // Direct _mm_* AES-NI/CLMUL/shift mappings
    m.insert(
        "_mm_aesenc_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aesenc128),
    );
    m.insert(
        "_mm_aesenclast_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aesenclast128),
    );
    m.insert(
        "_mm_aesdec_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aesdec128),
    );
    m.insert(
        "_mm_aesdeclast_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aesdeclast128),
    );
    m.insert(
        "_mm_aesimc_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aesimc128),
    );
    m.insert(
        "_mm_aeskeygenassist_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Aeskeygenassist128),
    );
    m.insert(
        "_mm_clmulepi64_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pclmulqdq128),
    );
    m.insert(
        "_mm_slli_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pslldqi128),
    );
    m.insert(
        "_mm_srli_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psrldqi128),
    );
    m.insert(
        "_mm_slli_epi64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psllqi128),
    );
    m.insert(
        "_mm_srli_epi64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psrlqi128),
    );
    m.insert(
        "_mm_shuffle_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pshufd128),
    );
    m.insert(
        "_mm_loadl_epi64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Loadldi128),
    );

    // New SSE2 packed 16-bit _mm_* mappings
    m.insert(
        "_mm_add_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Paddw128),
    );
    m.insert(
        "_mm_sub_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubw128),
    );
    m.insert(
        "_mm_mulhi_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmulhw128),
    );
    m.insert(
        "_mm_madd_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmaddwd128),
    );
    m.insert(
        "_mm_cmpgt_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pcmpgtw128),
    );
    m.insert(
        "_mm_cmpgt_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pcmpgtb128),
    );
    m.insert(
        "_mm_slli_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psllwi128),
    );
    m.insert(
        "_mm_srli_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psrlwi128),
    );
    m.insert(
        "_mm_srai_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psrawi128),
    );
    m.insert(
        "_mm_srai_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psradi128),
    );
    m.insert(
        "_mm_slli_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pslldi128),
    );
    m.insert(
        "_mm_srli_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psrldi128),
    );
    // New SSE2 packed 32-bit _mm_* mappings
    m.insert(
        "_mm_add_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Paddd128),
    );
    m.insert(
        "_mm_sub_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubd128),
    );
    // New SSE2 pack/unpack _mm_* mappings
    m.insert(
        "_mm_packs_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Packssdw128),
    );
    m.insert(
        "_mm_packs_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Packsswb128),
    );
    m.insert(
        "_mm_packus_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Packuswb128),
    );
    m.insert(
        "_mm_unpacklo_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Punpcklbw128),
    );
    m.insert(
        "_mm_unpackhi_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Punpckhbw128),
    );
    m.insert(
        "_mm_unpacklo_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Punpcklwd128),
    );
    m.insert(
        "_mm_unpackhi_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Punpckhwd128),
    );
    // New SSE2 set/insert/extract/convert _mm_* mappings
    m.insert(
        "_mm_set1_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Set1Epi16),
    );
    m.insert(
        "_mm_insert_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pinsrw128),
    );
    m.insert(
        "_mm_extract_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pextrw128),
    );
    m.insert(
        "_mm_storel_epi64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Storeldi128),
    );
    m.insert(
        "_mm_cvtsi128_si32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Cvtsi128Si32),
    );
    m.insert(
        "_mm_cvtsi32_si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Cvtsi32Si128),
    );
    m.insert(
        "_mm_cvtsi128_si64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Cvtsi128Si64),
    );
    m.insert(
        "_mm_cvtsi128_si64x",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Cvtsi128Si64),
    );
    m.insert(
        "_mm_shufflelo_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pshuflw128),
    );
    m.insert(
        "_mm_shufflehi_epi16",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pshufhw128),
    );
    // SSE4.1 _mm_* direct name mappings
    m.insert(
        "_mm_insert_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pinsrd128),
    );
    m.insert(
        "_mm_extract_epi32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pextrd128),
    );
    m.insert(
        "_mm_insert_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pinsrb128),
    );
    m.insert(
        "_mm_extract_epi8",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pextrb128),
    );
    m.insert(
        "_mm_insert_epi64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pinsrq128),
    );
    m.insert(
        "_mm_extract_epi64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pextrq128),
    );
    // __builtin_ia32_* names for the new SSE2 operations
    m.insert(
        "__builtin_ia32_paddw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Paddw128),
    );
    m.insert(
        "__builtin_ia32_psubw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubw128),
    );
    m.insert(
        "__builtin_ia32_pmulhw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmulhw128),
    );
    m.insert(
        "__builtin_ia32_pmaddwd128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pmaddwd128),
    );
    m.insert(
        "__builtin_ia32_pcmpgtw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pcmpgtw128),
    );
    m.insert(
        "__builtin_ia32_pcmpgtb128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pcmpgtb128),
    );
    m.insert(
        "__builtin_ia32_psllwi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psllwi128),
    );
    m.insert(
        "__builtin_ia32_psrlwi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psrlwi128),
    );
    m.insert(
        "__builtin_ia32_psrawi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psrawi128),
    );
    m.insert(
        "__builtin_ia32_psradi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psradi128),
    );
    m.insert(
        "__builtin_ia32_pslldi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pslldi128),
    );
    m.insert(
        "__builtin_ia32_psrldi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psrldi128),
    );
    m.insert(
        "__builtin_ia32_paddd128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Paddd128),
    );
    m.insert(
        "__builtin_ia32_psubd128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Psubd128),
    );
    m.insert(
        "__builtin_ia32_packssdw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Packssdw128),
    );
    m.insert(
        "__builtin_ia32_packsswb128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Packsswb128),
    );
    m.insert(
        "__builtin_ia32_packuswb128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Packuswb128),
    );
    m.insert(
        "__builtin_ia32_punpcklbw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Punpcklbw128),
    );
    m.insert(
        "__builtin_ia32_punpckhbw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Punpckhbw128),
    );
    m.insert(
        "__builtin_ia32_punpcklwd128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Punpcklwd128),
    );
    m.insert(
        "__builtin_ia32_punpckhwd128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Punpckhwd128),
    );
    m.insert(
        "__builtin_ia32_pinsrw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pinsrw128),
    );
    m.insert(
        "__builtin_ia32_pextrw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pextrw128),
    );
    m.insert(
        "__builtin_ia32_storeldi128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Storeldi128),
    );
    m.insert(
        "__builtin_ia32_cvtsi128si32",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Cvtsi128Si32),
    );
    m.insert(
        "__builtin_ia32_cvtsi32si128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Cvtsi32Si128),
    );
    m.insert(
        "__builtin_ia32_cvtsi128si64",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Cvtsi128Si64),
    );
    m.insert(
        "__builtin_ia32_pshuflw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pshuflw128),
    );
    m.insert(
        "__builtin_ia32_pshufhw128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pshufhw128),
    );

    // x86 SSE4.1 insert/extract builtins (__builtin_ia32_* names)
    m.insert(
        "__builtin_ia32_pinsrd128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pinsrd128),
    );
    m.insert(
        "__builtin_ia32_pextrd128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pextrd128),
    );
    m.insert(
        "__builtin_ia32_pinsrb128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pinsrb128),
    );
    m.insert(
        "__builtin_ia32_pextrb128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pextrb128),
    );
    m.insert(
        "__builtin_ia32_pinsrq128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pinsrq128),
    );
    m.insert(
        "__builtin_ia32_pextrq128",
        BuiltinInfo::intrinsic(BuiltinIntrinsic::X86Pextrq128),
    );

    m
});

/// How a builtin should be handled during lowering.
#[derive(Debug, Clone)]
pub struct BuiltinInfo {
    pub kind: BuiltinKind,
}

/// The kind of builtin behavior.
#[derive(Debug, Clone)]
pub enum BuiltinKind {
    /// Map directly to a libc function call.
    LibcAlias(String),
    /// Return the first argument unchanged (__builtin_expect).
    Identity,
    /// Evaluate to a compile-time float constant.
    ConstantF64(f64),
    /// Requires special codegen (CLZ, CTZ, popcount, bswap, etc.).
    Intrinsic(BuiltinIntrinsic),
}

/// Intrinsics that need target-specific codegen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinIntrinsic {
    Clz,
    Ctz,
    /// ffs(x) = x == 0 ? 0 : ctz(x) + 1 — find first set bit (1-indexed)
    Ffs,
    Clrsb,
    Popcount,
    Bswap,
    Fence,
    FpCompare,
    Parity,
    /// creal/crealf/creall: extract real part of complex number
    ComplexReal,
    /// cimag/cimagf/cimagl: extract imaginary part of complex number
    ComplexImag,
    /// conj/conjf/conjl: compute complex conjugate
    ComplexConj,
    /// __builtin_fpclassify(nan, inf, norm, subnorm, zero, x) -> int
    FpClassify,
    /// __builtin_isnan(x) -> int (1 if NaN, 0 otherwise)
    IsNan,
    /// __builtin_isinf(x) -> int (1 if +/-inf, 0 otherwise)
    IsInf,
    /// __builtin_isfinite(x) -> int (1 if finite, 0 otherwise)
    IsFinite,
    /// __builtin_isnormal(x) -> int (1 if normal, 0 otherwise)
    IsNormal,
    /// __builtin_signbit(x) -> int (nonzero if sign bit set)
    SignBit,
    /// __builtin_isinf_sign(x) -> int (-1 if -inf, 1 if +inf, 0 otherwise)
    IsInfSign,
    /// __builtin_alloca(size) -> dynamic stack allocation
    Alloca,
    /// __builtin_complex(real, imag) -> construct complex number
    ComplexConstruct,
    /// __builtin_va_start(ap, last) -> initialize va_list (lowered specially in IR)
    VaStart,
    /// __builtin_va_end(ap) -> cleanup va_list (lowered specially in IR)
    VaEnd,
    /// __builtin_va_copy(dest, src) -> copy va_list (lowered specially in IR)
    VaCopy,
    /// __builtin_constant_p(expr) -> 1 if expr is a compile-time constant, 0 otherwise
    ConstantP,
    /// __builtin_object_size(ptr, type) -> size of object ptr points to, or -1/0 if unknown
    ObjectSize,
    /// __builtin_classify_type(expr) -> integer type class of the expression's type
    ClassifyType,
    /// No-op builtin (evaluates args, returns 0)
    Nop,
    /// __builtin_cpu_init() - no-op, glibc handles CPU feature detection init
    CpuInit,
    /// __builtin_cpu_supports("feature") - conservatively returns 0 (unsupported)
    CpuSupports,
    /// __builtin_add_overflow(a, b, result_ptr) -> bool (1 if overflow)
    AddOverflow,
    /// __builtin_sub_overflow(a, b, result_ptr) -> bool (1 if overflow)
    SubOverflow,
    /// __builtin_mul_overflow(a, b, result_ptr) -> bool (1 if overflow)
    MulOverflow,
    /// __builtin_add_overflow_p(a, b, (T)0) -> bool (1 if a+b overflows type T)
    AddOverflowP,
    /// __builtin_sub_overflow_p(a, b, (T)0) -> bool (1 if a-b overflows type T)
    SubOverflowP,
    /// __builtin_mul_overflow_p(a, b, (T)0) -> bool (1 if a*b overflows type T)
    MulOverflowP,
    /// __builtin_frame_address(level) -> returns frame pointer
    FrameAddress,
    /// __builtin_return_address(level) -> returns return address
    ReturnAddress,
    // X86 SSE intrinsics
    X86Lfence,
    X86Mfence,
    X86Sfence,
    X86Pause,
    X86Clflush,
    X86Movnti,
    X86Movnti64,
    X86Movntdq,
    X86Movntpd,
    X86Loaddqu,
    X86Storedqu,
    X86Pcmpeqb128,
    X86Pcmpeqd128,
    X86Psubusb128,
    X86Psubsb128,
    X86Por128,
    X86Pand128,
    X86Pxor128,
    X86Pmovmskb128,
    X86Set1Epi8,
    X86Set1Epi32,
    X86Crc32_8,
    X86Crc32_16,
    X86Crc32_32,
    X86Crc32_64,
    // AES-NI intrinsics
    X86Aesenc128,
    X86Aesenclast128,
    X86Aesdec128,
    X86Aesdeclast128,
    X86Aesimc128,
    X86Aeskeygenassist128,
    // CLMUL
    X86Pclmulqdq128,
    // SSE2 shift/shuffle
    X86Pslldqi128, // _mm_slli_si128 (byte shift left)
    X86Psrldqi128, // _mm_srli_si128 (byte shift right)
    X86Psllqi128,  // _mm_slli_epi64 (bit shift left per 64-bit lane)
    X86Psrlqi128,  // _mm_srli_epi64 (bit shift right per 64-bit lane)
    X86Pshufd128,  // _mm_shuffle_epi32
    X86Loadldi128, // _mm_loadl_epi64 (load low 64 bits)
    // SSE2 packed 16-bit operations
    X86Paddw128,   // _mm_add_epi16 (PADDW)
    X86Psubw128,   // _mm_sub_epi16 (PSUBW)
    X86Pmulhw128,  // _mm_mulhi_epi16 (PMULHW)
    X86Pmaddwd128, // _mm_madd_epi16 (PMADDWD)
    X86Pcmpgtw128, // _mm_cmpgt_epi16 (PCMPGTW)
    X86Pcmpgtb128, // _mm_cmpgt_epi8 (PCMPGTB)
    X86Psllwi128,  // _mm_slli_epi16 (PSLLW imm)
    X86Psrlwi128,  // _mm_srli_epi16 (PSRLW imm)
    X86Psrawi128,  // _mm_srai_epi16 (PSRAW imm)
    X86Psradi128,  // _mm_srai_epi32 (PSRAD imm)
    X86Pslldi128,  // _mm_slli_epi32 (PSLLD imm)
    X86Psrldi128,  // _mm_srli_epi32 (PSRLD imm)
    // SSE2 packed 32-bit operations
    X86Paddd128, // _mm_add_epi32 (PADDD)
    X86Psubd128, // _mm_sub_epi32 (PSUBD)
    // SSE2 pack/unpack
    X86Packssdw128,  // _mm_packs_epi32 (PACKSSDW)
    X86Packsswb128,  // _mm_packs_epi16 (PACKSSWB)
    X86Packuswb128,  // _mm_packus_epi16 (PACKUSWB)
    X86Punpcklbw128, // _mm_unpacklo_epi8 (PUNPCKLBW)
    X86Punpckhbw128, // _mm_unpackhi_epi8 (PUNPCKHBW)
    X86Punpcklwd128, // _mm_unpacklo_epi16 (PUNPCKLWD)
    X86Punpckhwd128, // _mm_unpackhi_epi16 (PUNPCKHWD)
    // SSE2 set/insert/extract/convert
    X86Set1Epi16,    // _mm_set1_epi16 (splat 16-bit)
    X86Pinsrw128,    // _mm_insert_epi16 (PINSRW)
    X86Pextrw128,    // _mm_extract_epi16 (PEXTRW)
    X86Storeldi128,  // _mm_storel_epi64 (MOVQ store)
    X86Cvtsi128Si32, // _mm_cvtsi128_si32 (MOVD extract)
    X86Cvtsi32Si128, // _mm_cvtsi32_si128 (MOVD insert)
    X86Cvtsi128Si64, // _mm_cvtsi128_si64
    X86Pshuflw128,   // _mm_shufflelo_epi16 (PSHUFLW)
    X86Pshufhw128,   // _mm_shufflehi_epi16 (PSHUFHW)
    // SSE4.1 insert/extract operations
    X86Pinsrd128, // _mm_insert_epi32 (PINSRD)
    X86Pextrd128, // _mm_extract_epi32 (PEXTRD)
    X86Pinsrb128, // _mm_insert_epi8 (PINSRB)
    X86Pextrb128, // _mm_extract_epi8 (PEXTRB)
    X86Pinsrq128, // _mm_insert_epi64 (PINSRQ)
    X86Pextrq128, // _mm_extract_epi64 (PEXTRQ)
    /// __builtin___*_chk: fortification builtins that forward to unchecked libc equivalents
    FortifyChk,
    /// __builtin_va_arg_pack(): used in always_inline fortification wrappers, returns 0
    VaArgPack,
    /// __builtin_thread_pointer(): returns the thread pointer (TLS base address)
    ThreadPointer,
}

impl BuiltinInfo {
    fn simple(libc_name: &str) -> Self {
        Self {
            kind: BuiltinKind::LibcAlias(libc_name.to_string()),
        }
    }

    fn identity() -> Self {
        Self {
            kind: BuiltinKind::Identity,
        }
    }

    fn constant_f64(val: f64) -> Self {
        Self {
            kind: BuiltinKind::ConstantF64(val),
        }
    }

    fn intrinsic(intr: BuiltinIntrinsic) -> Self {
        Self {
            kind: BuiltinKind::Intrinsic(intr),
        }
    }
}

/// Look up a function name and return its builtin info, if it's a known builtin.
pub fn resolve_builtin(name: &str) -> Option<&'static BuiltinInfo> {
    BUILTIN_MAP.get(name)
}

/// Check if a name is a known builtin function.
///
/// This includes both explicitly registered builtins (in BUILTIN_MAP) and
/// atomic/sync builtins that are handled by pattern matching in the IR lowering
/// code (expr_atomics.rs). The atomic builtins must be recognized here so that
/// sema does not emit "implicit declaration" warnings for them.
pub fn is_builtin(name: &str) -> bool {
    if BUILTIN_MAP.contains_key(name) {
        return true;
    }
    // Builtins handled by name in try_lower_builtin_call (before map lookup)
    if matches!(
        name,
        "__builtin_choose_expr" | "__builtin_unreachable" | "__builtin_trap"
    ) {
        return true;
    }
    // Atomic builtins handled by pattern matching in expr_atomics.rs
    is_atomic_builtin(name)
}

/// Check if a name is an atomic/sync builtin handled by the IR lowering code.
/// These are dispatched by name pattern in try_lower_atomic_builtin() and
/// classify_fetch_op()/classify_op_fetch() rather than through the BUILTIN_MAP.
fn is_atomic_builtin(name: &str) -> bool {
    // __atomic_* family (C11-style)
    if name.starts_with("__atomic_") {
        if matches!(
            name,
            "__atomic_fetch_add"
                | "__atomic_fetch_sub"
                | "__atomic_fetch_and"
                | "__atomic_fetch_or"
                | "__atomic_fetch_xor"
                | "__atomic_fetch_nand"
                | "__atomic_add_fetch"
                | "__atomic_sub_fetch"
                | "__atomic_and_fetch"
                | "__atomic_or_fetch"
                | "__atomic_xor_fetch"
                | "__atomic_nand_fetch"
                | "__atomic_exchange_n"
                | "__atomic_exchange"
                | "__atomic_compare_exchange_n"
                | "__atomic_compare_exchange"
                | "__atomic_load_n"
                | "__atomic_load"
                | "__atomic_store_n"
                | "__atomic_store"
                | "__atomic_test_and_set"
                | "__atomic_clear"
                | "__atomic_thread_fence"
                | "__atomic_signal_fence"
                | "__atomic_is_lock_free"
                | "__atomic_always_lock_free"
        ) {
            return true;
        }
        // Also recognize size-suffixed variants: __atomic_load_4, __atomic_store_2, etc.
        return normalize_atomic_size_suffix(name).is_some();
    }
    // __sync_* family (legacy GCC-style)
    // GCC also provides size-suffixed variants (e.g., __sync_fetch_and_add_4,
    // __sync_fetch_and_add_8) which are the actual library entry points.
    // Strip the _N suffix before matching.
    if name.starts_with("__sync_") {
        let base = strip_sync_size_suffix(name);
        return matches!(
            base,
            "__sync_fetch_and_add"
                | "__sync_fetch_and_sub"
                | "__sync_fetch_and_and"
                | "__sync_fetch_and_or"
                | "__sync_fetch_and_xor"
                | "__sync_fetch_and_nand"
                | "__sync_add_and_fetch"
                | "__sync_sub_and_fetch"
                | "__sync_and_and_fetch"
                | "__sync_or_and_fetch"
                | "__sync_xor_and_fetch"
                | "__sync_nand_and_fetch"
                | "__sync_val_compare_and_swap"
                | "__sync_bool_compare_and_swap"
                | "__sync_lock_test_and_set"
                | "__sync_lock_release"
                | "__sync_synchronize"
        );
    }
    false
}

/// Strip size suffix (_1, _2, _4, _8, _16) from GCC __sync_* builtin names.
/// E.g., "__sync_fetch_and_add_8" -> "__sync_fetch_and_add".
/// Returns the name unchanged if no suffix is present.
pub fn strip_sync_size_suffix(name: &str) -> &str {
    if let Some(base) = name
        .strip_suffix("_1")
        .or_else(|| name.strip_suffix("_2"))
        .or_else(|| name.strip_suffix("_4"))
        .or_else(|| name.strip_suffix("_8"))
        .or_else(|| name.strip_suffix("_16"))
    {
        base
    } else {
        name
    }
}

/// Normalize size-suffixed __atomic_* builtins to their canonical form.
/// GCC provides size-suffixed variants (e.g., __atomic_load_4, __atomic_store_2)
/// as libatomic entry points that are also recognized as compiler builtins.
/// These follow direct-value semantics (like __atomic_load_n, __atomic_store_n).
///
/// Returns Some(canonical_name) if a size suffix was stripped, None otherwise.
pub fn normalize_atomic_size_suffix(name: &str) -> Option<&'static str> {
    if !name.starts_with("__atomic_") {
        return None;
    }
    // Try to strip a size suffix
    let base = name
        .strip_suffix("_1")
        .or_else(|| name.strip_suffix("_2"))
        .or_else(|| name.strip_suffix("_4"))
        .or_else(|| name.strip_suffix("_8"))
        .or_else(|| name.strip_suffix("_16"))?;
    // Map to canonical _n variant for ops that have pointer-return vs direct-return distinction
    match base {
        "__atomic_load" => Some("__atomic_load_n"),
        "__atomic_store" => Some("__atomic_store_n"),
        "__atomic_exchange" => Some("__atomic_exchange_n"),
        "__atomic_compare_exchange" => Some("__atomic_compare_exchange_n"),
        // For fetch-op and op-fetch variants, base name is already correct
        "__atomic_fetch_add"
        | "__atomic_fetch_sub"
        | "__atomic_fetch_and"
        | "__atomic_fetch_or"
        | "__atomic_fetch_xor"
        | "__atomic_fetch_nand"
        | "__atomic_add_fetch"
        | "__atomic_sub_fetch"
        | "__atomic_and_fetch"
        | "__atomic_or_fetch"
        | "__atomic_xor_fetch"
        | "__atomic_nand_fetch" => {
            // Return the base name by matching it to a static str
            match base {
                "__atomic_fetch_add" => Some("__atomic_fetch_add"),
                "__atomic_fetch_sub" => Some("__atomic_fetch_sub"),
                "__atomic_fetch_and" => Some("__atomic_fetch_and"),
                "__atomic_fetch_or" => Some("__atomic_fetch_or"),
                "__atomic_fetch_xor" => Some("__atomic_fetch_xor"),
                "__atomic_fetch_nand" => Some("__atomic_fetch_nand"),
                "__atomic_add_fetch" => Some("__atomic_add_fetch"),
                "__atomic_sub_fetch" => Some("__atomic_sub_fetch"),
                "__atomic_and_fetch" => Some("__atomic_and_fetch"),
                "__atomic_or_fetch" => Some("__atomic_or_fetch"),
                "__atomic_xor_fetch" => Some("__atomic_xor_fetch"),
                "__atomic_nand_fetch" => Some("__atomic_nand_fetch"),
                _ => None,
            }
        }
        _ => None,
    }
}
