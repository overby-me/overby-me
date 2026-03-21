/* CCC compiler bundled immintrin.h - all x86 SIMD intrinsics */
#ifndef _IMMINTRIN_H_INCLUDED
#define _IMMINTRIN_H_INCLUDED

/* x86 SIMD intrinsics are only available on x86/x86-64 targets */
#if !defined(__x86_64__) && !defined(__i386__) && !defined(__i686__)
#error "x86 SIMD intrinsics (immintrin.h) require an x86 target"
#endif

#include <wmmintrin.h>
#include <smmintrin.h>
#include <shaintrin.h>
#include <bmi2intrin.h>
#include <avxintrin.h>
#include <avx2intrin.h>
#include <fmaintrin.h>
#include <avx512fintrin.h>

/* === RDRAND / RDSEED intrinsics === */

static __inline__ int __attribute__((__always_inline__))
_rdrand16_step(unsigned short *__p)
{
    unsigned char __ok;
    __asm__ __volatile__("rdrand %0; setc %1" : "=r"(*__p), "=qm"(__ok));
    return (int)__ok;
}

static __inline__ int __attribute__((__always_inline__))
_rdrand32_step(unsigned int *__p)
{
    unsigned char __ok;
    __asm__ __volatile__("rdrand %0; setc %1" : "=r"(*__p), "=qm"(__ok));
    return (int)__ok;
}

#ifdef __x86_64__
static __inline__ int __attribute__((__always_inline__))
_rdrand64_step(unsigned long long *__p)
{
    unsigned char __ok;
    __asm__ __volatile__("rdrand %0; setc %1" : "=r"(*__p), "=qm"(__ok));
    return (int)__ok;
}
#endif

static __inline__ int __attribute__((__always_inline__))
_rdseed16_step(unsigned short *__p)
{
    unsigned char __ok;
    __asm__ __volatile__("rdseed %0; setc %1" : "=r"(*__p), "=qm"(__ok));
    return (int)__ok;
}

static __inline__ int __attribute__((__always_inline__))
_rdseed32_step(unsigned int *__p)
{
    unsigned char __ok;
    __asm__ __volatile__("rdseed %0; setc %1" : "=r"(*__p), "=qm"(__ok));
    return (int)__ok;
}

#ifdef __x86_64__
static __inline__ int __attribute__((__always_inline__))
_rdseed64_step(unsigned long long *__p)
{
    unsigned char __ok;
    __asm__ __volatile__("rdseed %0; setc %1" : "=r"(*__p), "=qm"(__ok));
    return (int)__ok;
}
#endif

#endif /* _IMMINTRIN_H_INCLUDED */
