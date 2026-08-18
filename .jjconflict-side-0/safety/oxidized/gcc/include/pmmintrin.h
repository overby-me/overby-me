/* CCC compiler bundled pmmintrin.h - SSE3 intrinsics */
#ifndef _PMMINTRIN_H_INCLUDED
#define _PMMINTRIN_H_INCLUDED

/* SSE3 intrinsics are only available on x86/x86-64 targets */
#if !defined(__x86_64__) && !defined(__i386__) && !defined(__i686__)
#error "SSE3 intrinsics (pmmintrin.h) require an x86 target"
#endif

#include <emmintrin.h>

/* _mm_hadd_ps: horizontal add packed single-precision (HADDPS)
 * Result: { a[0]+a[1], a[2]+a[3], b[0]+b[1], b[2]+b[3] } */
static __inline__ __m128 __attribute__((__always_inline__))
_mm_hadd_ps(__m128 __a, __m128 __b)
{
    return (__m128){ { __a.__val[0] + __a.__val[1],
                       __a.__val[2] + __a.__val[3],
                       __b.__val[0] + __b.__val[1],
                       __b.__val[2] + __b.__val[3] } };
}

/* _mm_hadd_pd: horizontal add packed double-precision (HADDPD)
 * Result: { a[0]+a[1], b[0]+b[1] } */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_hadd_pd(__m128d __a, __m128d __b)
{
    double *__pa = (double *)&__a;
    double *__pb = (double *)&__b;
    __m128d __r;
    double *__pr = (double *)&__r;
    __pr[0] = __pa[0] + __pa[1];
    __pr[1] = __pb[0] + __pb[1];
    return __r;
}

/* _mm_hsub_ps: horizontal subtract packed single-precision (HSUBPS) */
static __inline__ __m128 __attribute__((__always_inline__))
_mm_hsub_ps(__m128 __a, __m128 __b)
{
    return (__m128){ { __a.__val[0] - __a.__val[1],
                       __a.__val[2] - __a.__val[3],
                       __b.__val[0] - __b.__val[1],
                       __b.__val[2] - __b.__val[3] } };
}

/* _mm_movehdup_ps: duplicate odd-indexed single-precision elements (MOVSHDUP) */
static __inline__ __m128 __attribute__((__always_inline__))
_mm_movehdup_ps(__m128 __a)
{
    return (__m128){ { __a.__val[1], __a.__val[1], __a.__val[3], __a.__val[3] } };
}

/* _mm_moveldup_ps: duplicate even-indexed single-precision elements (MOVSLDUP) */
static __inline__ __m128 __attribute__((__always_inline__))
_mm_moveldup_ps(__m128 __a)
{
    return (__m128){ { __a.__val[0], __a.__val[0], __a.__val[2], __a.__val[2] } };
}

#endif /* _PMMINTRIN_H_INCLUDED */
