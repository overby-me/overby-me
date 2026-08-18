/* CCC compiler bundled fmaintrin.h - FMA3 intrinsics */
#ifndef _FMAINTRIN_H_INCLUDED
#define _FMAINTRIN_H_INCLUDED

#include <avxintrin.h>

/* === 128-bit FMA === */

/* _mm_fmadd_ps: a*b + c (single-precision, 128-bit) */
static __inline__ __m128 __attribute__((__always_inline__))
_mm_fmadd_ps(__m128 __a, __m128 __b, __m128 __c)
{
    __m128 __r;
    for (int __i = 0; __i < 4; __i++)
        __r.__val[__i] = __a.__val[__i] * __b.__val[__i] + __c.__val[__i];
    return __r;
}

/* _mm_fmadd_pd: a*b + c (double-precision, 128-bit) */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_fmadd_pd(__m128d __a, __m128d __b, __m128d __c)
{
    double *__pa = (double *)&__a;
    double *__pb = (double *)&__b;
    double *__pc = (double *)&__c;
    __m128d __r;
    double *__pr = (double *)&__r;
    __pr[0] = __pa[0] * __pb[0] + __pc[0];
    __pr[1] = __pa[1] * __pb[1] + __pc[1];
    return __r;
}

/* === 256-bit FMA === */

/* _mm256_fmadd_ps: a*b + c (single-precision, 256-bit) */
static __inline__ __m256 __attribute__((__always_inline__))
_mm256_fmadd_ps(__m256 __a, __m256 __b, __m256 __c)
{
    __m256 __r;
    for (int __i = 0; __i < 8; __i++)
        __r.__val[__i] = __a.__val[__i] * __b.__val[__i] + __c.__val[__i];
    return __r;
}

/* _mm256_fmadd_pd: a*b + c (double-precision, 256-bit) */
static __inline__ __m256d __attribute__((__always_inline__))
_mm256_fmadd_pd(__m256d __a, __m256d __b, __m256d __c)
{
    __m256d __r;
    for (int __i = 0; __i < 4; __i++)
        __r.__val[__i] = __a.__val[__i] * __b.__val[__i] + __c.__val[__i];
    return __r;
}

/* _mm256_fmsub_ps: a*b - c (single-precision, 256-bit) */
static __inline__ __m256 __attribute__((__always_inline__))
_mm256_fmsub_ps(__m256 __a, __m256 __b, __m256 __c)
{
    __m256 __r;
    for (int __i = 0; __i < 8; __i++)
        __r.__val[__i] = __a.__val[__i] * __b.__val[__i] - __c.__val[__i];
    return __r;
}

/* _mm256_fnmadd_ps: -(a*b) + c (single-precision, 256-bit) */
static __inline__ __m256 __attribute__((__always_inline__))
_mm256_fnmadd_ps(__m256 __a, __m256 __b, __m256 __c)
{
    __m256 __r;
    for (int __i = 0; __i < 8; __i++)
        __r.__val[__i] = -(__a.__val[__i] * __b.__val[__i]) + __c.__val[__i];
    return __r;
}

#endif /* _FMAINTRIN_H_INCLUDED */
