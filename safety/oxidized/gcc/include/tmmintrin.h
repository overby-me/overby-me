/* CCC compiler bundled tmmintrin.h - SSSE3 intrinsics */
#ifndef _TMMINTRIN_H_INCLUDED
#define _TMMINTRIN_H_INCLUDED

#include <pmmintrin.h>

/* _mm_abs_epi16: absolute value of signed 16-bit integers (PABSW) */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_abs_epi16(__m128i __a)
{
    short *__pa = (short *)&__a;
    __m128i __r;
    short *__pr = (short *)&__r;
    for (int __i = 0; __i < 8; __i++)
        __pr[__i] = __pa[__i] < 0 ? (short)-__pa[__i] : __pa[__i];
    return __r;
}

/* _mm_abs_epi8: absolute value of signed 8-bit integers (PABSB) */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_abs_epi8(__m128i __a)
{
    signed char *__pa = (signed char *)&__a;
    __m128i __r;
    unsigned char *__pr = (unsigned char *)&__r;
    for (int __i = 0; __i < 16; __i++)
        __pr[__i] = (unsigned char)(__pa[__i] < 0 ? -__pa[__i] : __pa[__i]);
    return __r;
}

/* _mm_abs_epi32: absolute value of signed 32-bit integers (PABSD) */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_abs_epi32(__m128i __a)
{
    int *__pa = (int *)&__a;
    __m128i __r;
    int *__pr = (int *)&__r;
    for (int __i = 0; __i < 4; __i++)
        __pr[__i] = __pa[__i] < 0 ? -__pa[__i] : __pa[__i];
    return __r;
}

/* _mm_maddubs_epi16: multiply unsigned 8-bit * signed 8-bit, horizontally add
 * adjacent pairs to produce 8 x 16-bit results with saturation (PMADDUBSW).
 * __a is treated as unsigned, __b as signed. */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_maddubs_epi16(__m128i __a, __m128i __b)
{
    unsigned char *__pa = (unsigned char *)&__a;
    signed char *__pb = (signed char *)&__b;
    __m128i __r;
    short *__pr = (short *)&__r;
    for (int __i = 0; __i < 8; __i++) {
        int __s = (int)__pa[__i * 2] * (int)__pb[__i * 2]
                + (int)__pa[__i * 2 + 1] * (int)__pb[__i * 2 + 1];
        /* Saturate to [-32768, 32767] */
        if (__s > 32767) __s = 32767;
        if (__s < -32768) __s = -32768;
        __pr[__i] = (short)__s;
    }
    return __r;
}

/* _mm_shuffle_epi8: shuffle bytes according to control mask (PSHUFB) */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_shuffle_epi8(__m128i __a, __m128i __b)
{
    unsigned char *__pa = (unsigned char *)&__a;
    unsigned char *__pb = (unsigned char *)&__b;
    __m128i __r;
    unsigned char *__pr = (unsigned char *)&__r;
    for (int __i = 0; __i < 16; __i++) {
        if (__pb[__i] & 0x80)
            __pr[__i] = 0;
        else
            __pr[__i] = __pa[__pb[__i] & 0x0F];
    }
    return __r;
}

/* _mm_alignr_epi8: concatenate __a (high) and __b (low) into a 32-byte
 * intermediate, shift right by __n bytes, return the low 16 bytes (PALIGNR) */
#define _mm_alignr_epi8(__a, __b, __n) __extension__ ({    \
    __m128i __r_alignr;                                     \
    unsigned char *__pa_alignr = (unsigned char *)&(__a);    \
    unsigned char *__pb_alignr = (unsigned char *)&(__b);    \
    unsigned char *__pr_alignr = (unsigned char *)&__r_alignr; \
    unsigned char __tmp_alignr[32];                         \
    for (int __i = 0; __i < 16; __i++)                     \
        __tmp_alignr[__i] = __pb_alignr[__i];              \
    for (int __i = 0; __i < 16; __i++)                     \
        __tmp_alignr[16 + __i] = __pa_alignr[__i];         \
    if ((__n) >= 32) {                                      \
        for (int __i = 0; __i < 16; __i++)                 \
            __pr_alignr[__i] = 0;                           \
    } else {                                                \
        for (int __i = 0; __i < 16; __i++)                 \
            __pr_alignr[__i] = ((__n) + __i < 32) ?        \
                __tmp_alignr[(__n) + __i] : 0;              \
    }                                                       \
    __r_alignr;                                             \
})

/* _mm_hadd_epi16: horizontal add adjacent pairs of 16-bit integers (PHADDW) */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_hadd_epi16(__m128i __a, __m128i __b)
{
    short *__pa = (short *)&__a;
    short *__pb = (short *)&__b;
    __m128i __r;
    short *__pr = (short *)&__r;
    /* From __a: add adjacent pairs */
    __pr[0] = (short)(__pa[0] + __pa[1]);
    __pr[1] = (short)(__pa[2] + __pa[3]);
    __pr[2] = (short)(__pa[4] + __pa[5]);
    __pr[3] = (short)(__pa[6] + __pa[7]);
    /* From __b: add adjacent pairs */
    __pr[4] = (short)(__pb[0] + __pb[1]);
    __pr[5] = (short)(__pb[2] + __pb[3]);
    __pr[6] = (short)(__pb[4] + __pb[5]);
    __pr[7] = (short)(__pb[6] + __pb[7]);
    return __r;
}

/* _mm_hadd_epi32: horizontal add adjacent pairs of 32-bit integers (PHADDD) */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_hadd_epi32(__m128i __a, __m128i __b)
{
    int *__pa = (int *)&__a;
    int *__pb = (int *)&__b;
    __m128i __r;
    int *__pr = (int *)&__r;
    __pr[0] = __pa[0] + __pa[1];
    __pr[1] = __pa[2] + __pa[3];
    __pr[2] = __pb[0] + __pb[1];
    __pr[3] = __pb[2] + __pb[3];
    return __r;
}

/* _mm_hsub_epi16: horizontal subtract adjacent pairs of 16-bit integers (PHSUBW) */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_hsub_epi16(__m128i __a, __m128i __b)
{
    short *__pa = (short *)&__a;
    short *__pb = (short *)&__b;
    __m128i __r;
    short *__pr = (short *)&__r;
    __pr[0] = (short)(__pa[0] - __pa[1]);
    __pr[1] = (short)(__pa[2] - __pa[3]);
    __pr[2] = (short)(__pa[4] - __pa[5]);
    __pr[3] = (short)(__pa[6] - __pa[7]);
    __pr[4] = (short)(__pb[0] - __pb[1]);
    __pr[5] = (short)(__pb[2] - __pb[3]);
    __pr[6] = (short)(__pb[4] - __pb[5]);
    __pr[7] = (short)(__pb[6] - __pb[7]);
    return __r;
}

/* _mm_hsub_epi32: horizontal subtract adjacent pairs of 32-bit integers (PHSUBD) */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_hsub_epi32(__m128i __a, __m128i __b)
{
    int *__pa = (int *)&__a;
    int *__pb = (int *)&__b;
    __m128i __r;
    int *__pr = (int *)&__r;
    __pr[0] = __pa[0] - __pa[1];
    __pr[1] = __pa[2] - __pa[3];
    __pr[2] = __pb[0] - __pb[1];
    __pr[3] = __pb[2] - __pb[3];
    return __r;
}

/* _mm_sign_epi8: negate/zero/keep bytes based on sign of __b (PSIGNB) */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_sign_epi8(__m128i __a, __m128i __b)
{
    signed char *__pa = (signed char *)&__a;
    signed char *__pb = (signed char *)&__b;
    __m128i __r;
    signed char *__pr = (signed char *)&__r;
    for (int __i = 0; __i < 16; __i++) {
        if (__pb[__i] < 0) __pr[__i] = (signed char)-__pa[__i];
        else if (__pb[__i] == 0) __pr[__i] = 0;
        else __pr[__i] = __pa[__i];
    }
    return __r;
}

/* _mm_sign_epi16: negate/zero/keep 16-bit ints based on sign (PSIGNW) */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_sign_epi16(__m128i __a, __m128i __b)
{
    short *__pa = (short *)&__a;
    short *__pb = (short *)&__b;
    __m128i __r;
    short *__pr = (short *)&__r;
    for (int __i = 0; __i < 8; __i++) {
        if (__pb[__i] < 0) __pr[__i] = (short)-__pa[__i];
        else if (__pb[__i] == 0) __pr[__i] = 0;
        else __pr[__i] = __pa[__i];
    }
    return __r;
}

/* _mm_sign_epi32: negate/zero/keep 32-bit ints based on sign (PSIGND) */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_sign_epi32(__m128i __a, __m128i __b)
{
    int *__pa = (int *)&__a;
    int *__pb = (int *)&__b;
    __m128i __r;
    int *__pr = (int *)&__r;
    for (int __i = 0; __i < 4; __i++) {
        if (__pb[__i] < 0) __pr[__i] = -__pa[__i];
        else if (__pb[__i] == 0) __pr[__i] = 0;
        else __pr[__i] = __pa[__i];
    }
    return __r;
}

/* _mm_mulhrs_epi16: multiply high with round and scale (PMULHRSW) */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_mulhrs_epi16(__m128i __a, __m128i __b)
{
    short *__pa = (short *)&__a;
    short *__pb = (short *)&__b;
    __m128i __r;
    short *__pr = (short *)&__r;
    for (int __i = 0; __i < 8; __i++) {
        int __t = ((int)__pa[__i] * (int)__pb[__i] + 0x4000) >> 15;
        __pr[__i] = (short)__t;
    }
    return __r;
}

/* _mm_hadds_epi16: horizontal add with saturation (PHADDSW) */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_hadds_epi16(__m128i __a, __m128i __b)
{
    short *__pa = (short *)&__a;
    short *__pb = (short *)&__b;
    __m128i __r;
    short *__pr = (short *)&__r;
    for (int __i = 0; __i < 4; __i++) {
        int __s = (int)__pa[__i * 2] + (int)__pa[__i * 2 + 1];
        if (__s > 32767) __s = 32767; if (__s < -32768) __s = -32768;
        __pr[__i] = (short)__s;
    }
    for (int __i = 0; __i < 4; __i++) {
        int __s = (int)__pb[__i * 2] + (int)__pb[__i * 2 + 1];
        if (__s > 32767) __s = 32767; if (__s < -32768) __s = -32768;
        __pr[4 + __i] = (short)__s;
    }
    return __r;
}

/* _mm_hsubs_epi16: horizontal subtract with saturation (PHSUBSW) */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_hsubs_epi16(__m128i __a, __m128i __b)
{
    short *__pa = (short *)&__a;
    short *__pb = (short *)&__b;
    __m128i __r;
    short *__pr = (short *)&__r;
    for (int __i = 0; __i < 4; __i++) {
        int __s = (int)__pa[__i * 2] - (int)__pa[__i * 2 + 1];
        if (__s > 32767) __s = 32767; if (__s < -32768) __s = -32768;
        __pr[__i] = (short)__s;
    }
    for (int __i = 0; __i < 4; __i++) {
        int __s = (int)__pb[__i * 2] - (int)__pb[__i * 2 + 1];
        if (__s > 32767) __s = 32767; if (__s < -32768) __s = -32768;
        __pr[4 + __i] = (short)__s;
    }
    return __r;
}

#endif /* _TMMINTRIN_H_INCLUDED */
