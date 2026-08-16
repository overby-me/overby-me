/* CCC compiler bundled avx512fintrin.h - AVX-512 Foundation intrinsics */
#ifndef _AVX512FINTRIN_H_INCLUDED
#define _AVX512FINTRIN_H_INCLUDED

#include <avx2intrin.h>

/* AVX-512 512-bit vector types */
typedef struct __attribute__((__aligned__(64))) {
    long long __val[8];
} __m512i;

typedef struct __attribute__((__aligned__(64))) {
    double __val[8];
} __m512d;

typedef struct __attribute__((__aligned__(64))) {
    float __val[16];
} __m512;

/* Unaligned variants */
typedef struct __attribute__((__aligned__(1))) {
    long long __val[8];
} __m512i_u;

/* AVX-512 mask types */
typedef unsigned char __mmask8;
typedef unsigned short __mmask16;

/* === Load / Store === */

static __inline__ __m512i __attribute__((__always_inline__))
_mm512_loadu_si512(void const *__p)
{
    __m512i __r;
    __builtin_memcpy(&__r, __p, sizeof(__r));
    return __r;
}

static __inline__ void __attribute__((__always_inline__))
_mm512_storeu_si512(void *__p, __m512i __a)
{
    __builtin_memcpy(__p, &__a, sizeof(__a));
}

/* === Set === */

static __inline__ __m512i __attribute__((__always_inline__))
_mm512_setzero_si512(void)
{
    return (__m512i){ { 0LL, 0LL, 0LL, 0LL, 0LL, 0LL, 0LL, 0LL } };
}

static __inline__ __m512i __attribute__((__always_inline__))
_mm512_set1_epi64(long long __q)
{
    return (__m512i){ { __q, __q, __q, __q, __q, __q, __q, __q } };
}

/* === Arithmetic === */

static __inline__ __m512i __attribute__((__always_inline__))
_mm512_add_epi64(__m512i __a, __m512i __b)
{
    return (__m512i){ { __a.__val[0] + __b.__val[0],
                        __a.__val[1] + __b.__val[1],
                        __a.__val[2] + __b.__val[2],
                        __a.__val[3] + __b.__val[3],
                        __a.__val[4] + __b.__val[4],
                        __a.__val[5] + __b.__val[5],
                        __a.__val[6] + __b.__val[6],
                        __a.__val[7] + __b.__val[7] } };
}

/* === Population count === */

/* _mm512_popcnt_epi64: population count for each 64-bit element */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_popcnt_epi64(__m512i __a)
{
    __m512i __r;
    for (int __i = 0; __i < 8; __i++) {
        unsigned long long __v = (unsigned long long)__a.__val[__i];
        int __cnt = 0;
        while (__v) {
            __cnt++;
            __v &= __v - 1;
        }
        __r.__val[__i] = __cnt;
    }
    return __r;
}

/* === Reduce === */

/* _mm512_reduce_add_epi64: horizontal sum of all 64-bit elements */
static __inline__ long long __attribute__((__always_inline__))
_mm512_reduce_add_epi64(__m512i __a)
{
    return __a.__val[0] + __a.__val[1] + __a.__val[2] + __a.__val[3]
         + __a.__val[4] + __a.__val[5] + __a.__val[6] + __a.__val[7];
}

/* === Float Load / Store === */

static __inline__ __m512 __attribute__((__always_inline__))
_mm512_loadu_ps(void const *__p)
{
    __m512 __r;
    const float *__fp = (const float *)__p;
    for (int __i = 0; __i < 16; __i++)
        __r.__val[__i] = __fp[__i];
    return __r;
}

static __inline__ void __attribute__((__always_inline__))
_mm512_storeu_ps(void *__p, __m512 __a)
{
    float *__fp = (float *)__p;
    for (int __i = 0; __i < 16; __i++)
        __fp[__i] = __a.__val[__i];
}

/* === Float Set === */

static __inline__ __m512 __attribute__((__always_inline__))
_mm512_setzero_ps(void)
{
    __m512 __r;
    for (int __i = 0; __i < 16; __i++)
        __r.__val[__i] = 0.0f;
    return __r;
}

/* === Float Arithmetic === */

static __inline__ __m512 __attribute__((__always_inline__))
_mm512_add_ps(__m512 __a, __m512 __b)
{
    __m512 __r;
    for (int __i = 0; __i < 16; __i++)
        __r.__val[__i] = __a.__val[__i] + __b.__val[__i];
    return __r;
}

static __inline__ __m512 __attribute__((__always_inline__))
_mm512_mul_ps(__m512 __a, __m512 __b)
{
    __m512 __r;
    for (int __i = 0; __i < 16; __i++)
        __r.__val[__i] = __a.__val[__i] * __b.__val[__i];
    return __r;
}

/* _mm512_fmadd_ps: a*b + c (single-precision, 512-bit) */
static __inline__ __m512 __attribute__((__always_inline__))
_mm512_fmadd_ps(__m512 __a, __m512 __b, __m512 __c)
{
    __m512 __r;
    for (int __i = 0; __i < 16; __i++)
        __r.__val[__i] = __a.__val[__i] * __b.__val[__i] + __c.__val[__i];
    return __r;
}

/* === Float Reduce === */

/* _mm512_reduce_add_ps: horizontal sum of all 16 float elements */
static __inline__ float __attribute__((__always_inline__))
_mm512_reduce_add_ps(__m512 __a)
{
    float __sum = 0.0f;
    for (int __i = 0; __i < 16; __i++)
        __sum += __a.__val[__i];
    return __sum;
}

/* === Integer Arithmetic (32-bit) === */

/* _mm512_add_epi32: add packed 32-bit integers */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_add_epi32(__m512i __a, __m512i __b)
{
    unsigned int *__pa = (unsigned int *)&__a;
    unsigned int *__pb = (unsigned int *)&__b;
    __m512i __r;
    unsigned int *__pr = (unsigned int *)&__r;
    for (int __i = 0; __i < 16; __i++)
        __pr[__i] = __pa[__i] + __pb[__i];
    return __r;
}

/* _mm512_sub_epi32: subtract packed 32-bit integers */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_sub_epi32(__m512i __a, __m512i __b)
{
    unsigned int *__pa = (unsigned int *)&__a;
    unsigned int *__pb = (unsigned int *)&__b;
    __m512i __r;
    unsigned int *__pr = (unsigned int *)&__r;
    for (int __i = 0; __i < 16; __i++)
        __pr[__i] = __pa[__i] - __pb[__i];
    return __r;
}

/* _mm512_mullo_epi32: multiply 32-bit ints, keep low 32 bits */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_mullo_epi32(__m512i __a, __m512i __b)
{
    unsigned int *__pa = (unsigned int *)&__a;
    unsigned int *__pb = (unsigned int *)&__b;
    __m512i __r;
    unsigned int *__pr = (unsigned int *)&__r;
    for (int __i = 0; __i < 16; __i++)
        __pr[__i] = __pa[__i] * __pb[__i];
    return __r;
}

/* === Bitwise === */

/* _mm512_and_si512: bitwise AND */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_and_si512(__m512i __a, __m512i __b)
{
    return (__m512i){ { __a.__val[0] & __b.__val[0],
                        __a.__val[1] & __b.__val[1],
                        __a.__val[2] & __b.__val[2],
                        __a.__val[3] & __b.__val[3],
                        __a.__val[4] & __b.__val[4],
                        __a.__val[5] & __b.__val[5],
                        __a.__val[6] & __b.__val[6],
                        __a.__val[7] & __b.__val[7] } };
}

/* _mm512_or_si512: bitwise OR */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_or_si512(__m512i __a, __m512i __b)
{
    return (__m512i){ { __a.__val[0] | __b.__val[0],
                        __a.__val[1] | __b.__val[1],
                        __a.__val[2] | __b.__val[2],
                        __a.__val[3] | __b.__val[3],
                        __a.__val[4] | __b.__val[4],
                        __a.__val[5] | __b.__val[5],
                        __a.__val[6] | __b.__val[6],
                        __a.__val[7] | __b.__val[7] } };
}

/* _mm512_xor_si512: bitwise XOR */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_xor_si512(__m512i __a, __m512i __b)
{
    return (__m512i){ { __a.__val[0] ^ __b.__val[0],
                        __a.__val[1] ^ __b.__val[1],
                        __a.__val[2] ^ __b.__val[2],
                        __a.__val[3] ^ __b.__val[3],
                        __a.__val[4] ^ __b.__val[4],
                        __a.__val[5] ^ __b.__val[5],
                        __a.__val[6] ^ __b.__val[6],
                        __a.__val[7] ^ __b.__val[7] } };
}

/* _mm512_andnot_si512: bitwise AND-NOT (~a & b) */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_andnot_si512(__m512i __a, __m512i __b)
{
    return (__m512i){ { ~__a.__val[0] & __b.__val[0],
                        ~__a.__val[1] & __b.__val[1],
                        ~__a.__val[2] & __b.__val[2],
                        ~__a.__val[3] & __b.__val[3],
                        ~__a.__val[4] & __b.__val[4],
                        ~__a.__val[5] & __b.__val[5],
                        ~__a.__val[6] & __b.__val[6],
                        ~__a.__val[7] & __b.__val[7] } };
}

/* === Set (additional) === */

/* _mm512_set1_epi32: broadcast 32-bit integer */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_set1_epi32(int __i)
{
    long long __q = (long long)(unsigned int)__i
                  | ((long long)(unsigned int)__i << 32);
    return (__m512i){ { __q, __q, __q, __q, __q, __q, __q, __q } };
}

/* _mm512_set1_epi8: broadcast 8-bit integer */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_set1_epi8(char __b)
{
    unsigned char __ub = (unsigned char)__b;
    long long __q = (long long)__ub;
    __q |= __q << 8;
    __q |= __q << 16;
    __q |= __q << 32;
    return (__m512i){ { __q, __q, __q, __q, __q, __q, __q, __q } };
}

/* === Extract === */

/* _mm512_extracti64x4_epi64: extract 256-bit lane from 512-bit register */
static __inline__ __m256i __attribute__((__always_inline__))
_mm512_extracti64x4_epi64(__m512i __a, int __imm)
{
    if (__imm & 1)
        return (__m256i){ { __a.__val[4], __a.__val[5], __a.__val[6], __a.__val[7] } };
    else
        return (__m256i){ { __a.__val[0], __a.__val[1], __a.__val[2], __a.__val[3] } };
}

/* _mm512_extracti32x4_epi32: extract 128-bit lane from 512-bit register */
static __inline__ __m128i __attribute__((__always_inline__))
_mm512_extracti32x4_epi32(__m512i __a, int __imm)
{
    int __lane = __imm & 3;
    return (__m128i){ { __a.__val[__lane * 2], __a.__val[__lane * 2 + 1] } };
}

/* === Conversion / Sign Extension (512-bit) === */

/* _mm512_cvtepi8_epi16: sign-extend 32 packed 8-bit to 32 packed 16-bit (AVX-512BW) */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_cvtepi8_epi16(__m256i __a)
{
    signed char *__pa = (signed char *)&__a;
    __m512i __r;
    short *__pr = (short *)&__r;
    for (int __i = 0; __i < 32; __i++)
        __pr[__i] = (short)__pa[__i];
    return __r;
}

/* _mm512_cvtepi8_epi32: sign-extend 16 packed 8-bit to 16 packed 32-bit */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_cvtepi8_epi32(__m128i __a)
{
    signed char *__pa = (signed char *)&__a;
    __m512i __r;
    int *__pr = (int *)&__r;
    for (int __i = 0; __i < 16; __i++)
        __pr[__i] = (int)__pa[__i];
    return __r;
}

/* _mm512_cvtepi16_epi32: sign-extend 16 packed 16-bit to 16 packed 32-bit */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_cvtepi16_epi32(__m256i __a)
{
    short *__pa = (short *)&__a;
    __m512i __r;
    int *__pr = (int *)&__r;
    for (int __i = 0; __i < 16; __i++)
        __pr[__i] = (int)__pa[__i];
    return __r;
}

/* === Multiply-Add (512-bit) === */

/* _mm512_madd_epi16: multiply signed 16-bit, hadd adjacent pairs -> 32-bit (AVX-512BW) */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_madd_epi16(__m512i __a, __m512i __b)
{
    short *__pa = (short *)&__a;
    short *__pb = (short *)&__b;
    __m512i __r;
    int *__pr = (int *)&__r;
    for (int __i = 0; __i < 16; __i++)
        __pr[__i] = (int)__pa[__i * 2] * (int)__pb[__i * 2]
                   + (int)__pa[__i * 2 + 1] * (int)__pb[__i * 2 + 1];
    return __r;
}

/* === Reduce (32-bit) === */

/* _mm512_reduce_add_epi32: horizontal sum of all 32-bit elements */
static __inline__ int __attribute__((__always_inline__))
_mm512_reduce_add_epi32(__m512i __a)
{
    int *__p = (int *)&__a;
    int __sum = 0;
    for (int __i = 0; __i < 16; __i++)
        __sum += __p[__i];
    return __sum;
}

/* === Subtract (64-bit) === */

/* _mm512_sub_epi64: subtract packed 64-bit integers */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_sub_epi64(__m512i __a, __m512i __b)
{
    return (__m512i){ { __a.__val[0] - __b.__val[0],
                        __a.__val[1] - __b.__val[1],
                        __a.__val[2] - __b.__val[2],
                        __a.__val[3] - __b.__val[3],
                        __a.__val[4] - __b.__val[4],
                        __a.__val[5] - __b.__val[5],
                        __a.__val[6] - __b.__val[6],
                        __a.__val[7] - __b.__val[7] } };
}

/* === Shift === */

/* _mm512_slli_epi32: shift 32-bit integers left */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_slli_epi32(__m512i __a, unsigned int __count)
{
    if (__count > 31) return _mm512_setzero_si512();
    unsigned int *__pa = (unsigned int *)&__a;
    __m512i __r;
    unsigned int *__pr = (unsigned int *)&__r;
    for (int __i = 0; __i < 16; __i++)
        __pr[__i] = __pa[__i] << __count;
    return __r;
}

/* _mm512_srli_epi32: shift 32-bit integers right */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_srli_epi32(__m512i __a, unsigned int __count)
{
    if (__count > 31) return _mm512_setzero_si512();
    unsigned int *__pa = (unsigned int *)&__a;
    __m512i __r;
    unsigned int *__pr = (unsigned int *)&__r;
    for (int __i = 0; __i < 16; __i++)
        __pr[__i] = __pa[__i] >> __count;
    return __r;
}

/* _mm512_slli_epi64: shift 64-bit integers left */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_slli_epi64(__m512i __a, unsigned int __count)
{
    if (__count > 63) return _mm512_setzero_si512();
    unsigned long long *__pa = (unsigned long long *)&__a;
    __m512i __r;
    unsigned long long *__pr = (unsigned long long *)&__r;
    for (int __i = 0; __i < 8; __i++)
        __pr[__i] = __pa[__i] << __count;
    return __r;
}

/* _mm512_srli_epi64: shift 64-bit integers right */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_srli_epi64(__m512i __a, unsigned int __count)
{
    if (__count > 63) return _mm512_setzero_si512();
    unsigned long long *__pa = (unsigned long long *)&__a;
    __m512i __r;
    unsigned long long *__pr = (unsigned long long *)&__r;
    for (int __i = 0; __i < 8; __i++)
        __pr[__i] = __pa[__i] >> __count;
    return __r;
}

/* === Compare === */

/* _mm512_cmpeq_epi32_mask: compare 32-bit ints for equality, return mask */
static __inline__ __mmask16 __attribute__((__always_inline__))
_mm512_cmpeq_epi32_mask(__m512i __a, __m512i __b)
{
    unsigned int *__pa = (unsigned int *)&__a;
    unsigned int *__pb = (unsigned int *)&__b;
    __mmask16 __mask = 0;
    for (int __i = 0; __i < 16; __i++)
        if (__pa[__i] == __pb[__i])
            __mask |= (1u << __i);
    return __mask;
}

/* === Insert === */

/* _mm512_inserti64x4: insert 256-bit lane into 512-bit register */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_inserti64x4(__m512i __a, __m256i __b, int __imm)
{
    __m512i __r = __a;
    if (__imm & 1) {
        __r.__val[4] = __b.__val[0]; __r.__val[5] = __b.__val[1];
        __r.__val[6] = __b.__val[2]; __r.__val[7] = __b.__val[3];
    } else {
        __r.__val[0] = __b.__val[0]; __r.__val[1] = __b.__val[1];
        __r.__val[2] = __b.__val[2]; __r.__val[3] = __b.__val[3];
    }
    return __r;
}

/* === Broadcast === */

/* _mm512_broadcastsi128_si512: broadcast 128-bit to all 4 lanes */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_broadcastsi128_si512(__m128i __a)
{
    return (__m512i){ { __a.__val[0], __a.__val[1],
                        __a.__val[0], __a.__val[1],
                        __a.__val[0], __a.__val[1],
                        __a.__val[0], __a.__val[1] } };
}

#endif /* _AVX512FINTRIN_H_INCLUDED */
