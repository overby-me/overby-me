/* CCC compiler bundled bmi2intrin.h - BMI2 intrinsics */
#ifndef _BMI2INTRIN_H_INCLUDED
#define _BMI2INTRIN_H_INCLUDED

/* _bzhi_u32: zero high bits starting from specified bit position */
static __inline__ unsigned int __attribute__((__always_inline__))
_bzhi_u32(unsigned int __src, unsigned int __index)
{
    __index &= 0xFF;
    if (__index >= 32)
        return __src;
    return __src & ((1U << __index) - 1U);
}

#ifdef __x86_64__
/* _bzhi_u64: zero high bits starting from specified bit position (64-bit) */
static __inline__ unsigned long long __attribute__((__always_inline__))
_bzhi_u64(unsigned long long __src, unsigned long long __index)
{
    __index &= 0xFF;
    if (__index >= 64)
        return __src;
    return __src & ((1ULL << __index) - 1ULL);
}
#endif

/* _pdep_u32: parallel bit deposit */
static __inline__ unsigned int __attribute__((__always_inline__))
_pdep_u32(unsigned int __src, unsigned int __mask)
{
    unsigned int __result = 0;
    unsigned int __k = 0;
    for (unsigned int __i = 0; __i < 32; __i++) {
        if (__mask & (1U << __i)) {
            if (__src & (1U << __k))
                __result |= (1U << __i);
            __k++;
        }
    }
    return __result;
}

/* _pext_u32: parallel bit extract */
static __inline__ unsigned int __attribute__((__always_inline__))
_pext_u32(unsigned int __src, unsigned int __mask)
{
    unsigned int __result = 0;
    unsigned int __k = 0;
    for (unsigned int __i = 0; __i < 32; __i++) {
        if (__mask & (1U << __i)) {
            if (__src & (1U << __i))
                __result |= (1U << __k);
            __k++;
        }
    }
    return __result;
}

#ifdef __x86_64__
/* _pdep_u64: parallel bit deposit (64-bit) */
static __inline__ unsigned long long __attribute__((__always_inline__))
_pdep_u64(unsigned long long __src, unsigned long long __mask)
{
    unsigned long long __result = 0;
    unsigned long long __k = 0;
    for (unsigned long long __i = 0; __i < 64; __i++) {
        if (__mask & (1ULL << __i)) {
            if (__src & (1ULL << __k))
                __result |= (1ULL << __i);
            __k++;
        }
    }
    return __result;
}

/* _pext_u64: parallel bit extract (64-bit) */
static __inline__ unsigned long long __attribute__((__always_inline__))
_pext_u64(unsigned long long __src, unsigned long long __mask)
{
    unsigned long long __result = 0;
    unsigned long long __k = 0;
    for (unsigned long long __i = 0; __i < 64; __i++) {
        if (__mask & (1ULL << __i)) {
            if (__src & (1ULL << __i))
                __result |= (1ULL << __k);
            __k++;
        }
    }
    return __result;
}
#endif

#endif /* _BMI2INTRIN_H_INCLUDED */
