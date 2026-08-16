/* CCC compiler bundled emmintrin.h - SSE2 intrinsics */
#ifndef _EMMINTRIN_H_INCLUDED
#define _EMMINTRIN_H_INCLUDED

/* SSE2 intrinsics are only available on x86/x86-64 targets */
#if !defined(__x86_64__) && !defined(__i386__) && !defined(__i686__)
#error "SSE2 intrinsics (emmintrin.h) require an x86 target"
#endif

#include <xmmintrin.h>

typedef struct __attribute__((__aligned__(16))) {
    long long __val[2];
} __m128i;

typedef struct __attribute__((__aligned__(1))) {
    long long __val[2];
} __m128i_u;

typedef struct __attribute__((__aligned__(16))) {
    double __val[2];
} __m128d;

typedef struct __attribute__((__aligned__(1))) {
    double __val[2];
} __m128d_u;

/* Internal vector types referenced by GCC system headers (wmmintrin.h, etc.).
 * These enable parsing of system headers that use (__v2di)expr casts.
 * Note: vector_size attribute is parsed but vectors are lowered as aggregates. */
typedef double __v2df __attribute__ ((__vector_size__ (16)));
typedef long long __v2di __attribute__ ((__vector_size__ (16)));
typedef unsigned long long __v2du __attribute__ ((__vector_size__ (16)));
typedef int __v4si __attribute__ ((__vector_size__ (16)));
typedef unsigned int __v4su __attribute__ ((__vector_size__ (16)));
typedef short __v8hi __attribute__ ((__vector_size__ (16)));
typedef unsigned short __v8hu __attribute__ ((__vector_size__ (16)));
typedef char __v16qi __attribute__ ((__vector_size__ (16)));
typedef signed char __v16qs __attribute__ ((__vector_size__ (16)));
typedef unsigned char __v16qu __attribute__ ((__vector_size__ (16)));

/* Helper to convert intrinsic result pointer to __m128i value.
 * Our SSE builtins return a pointer to 16-byte result data.
 * This macro dereferences that pointer to get the __m128i struct value. */
#define __CCC_M128I_FROM_BUILTIN(expr) (*(__m128i *)(expr))

/* === Load / Store === */

static __inline__ __m128i __attribute__((__always_inline__))
_mm_loadu_si128(__m128i_u const *__p)
{
    return *__p;
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_load_si128(__m128i const *__p)
{
    return *__p;
}

static __inline__ void __attribute__((__always_inline__))
_mm_storeu_si128(__m128i_u *__p, __m128i __b)
{
    *__p = __b;
}

static __inline__ void __attribute__((__always_inline__))
_mm_store_si128(__m128i *__p, __m128i __b)
{
    *__p = __b;
}

/* === Set === */

static __inline__ __m128i __attribute__((__always_inline__))
_mm_set1_epi8(char __b)
{
    unsigned char __ub = (unsigned char)__b;
    long long __q = (long long)__ub;
    __q |= __q << 8;
    __q |= __q << 16;
    __q |= __q << 32;
    return (__m128i){ { __q, __q } };
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_set1_epi32(int __i)
{
    long long __q = (long long)(unsigned int)__i
                  | ((long long)(unsigned int)__i << 32);
    return (__m128i){ { __q, __q } };
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_setzero_si128(void)
{
    return (__m128i){ { 0LL, 0LL } };
}

/* === Compare === */

static __inline__ __m128i __attribute__((__always_inline__))
_mm_cmpeq_epi8(__m128i __a, __m128i __b)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_pcmpeqb128(__a, __b));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_cmpeq_epi32(__m128i __a, __m128i __b)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_pcmpeqd128(__a, __b));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_cmpeq_epi16(__m128i __a, __m128i __b)
{
    /* Compare 8 x 16-bit elements for equality. Returns 0xFFFF for equal, 0x0000 otherwise. */
    unsigned long long __r0 = 0, __r1 = 0;
    unsigned long long __a0 = (unsigned long long)__a.__val[0];
    unsigned long long __b0 = (unsigned long long)__b.__val[0];
    unsigned long long __a1 = (unsigned long long)__a.__val[1];
    unsigned long long __b1 = (unsigned long long)__b.__val[1];
    for (int __i = 0; __i < 4; __i++) {
        unsigned short __va = (unsigned short)(__a0 >> (__i * 16));
        unsigned short __vb = (unsigned short)(__b0 >> (__i * 16));
        if (__va == __vb) __r0 |= (0xFFFFULL << (__i * 16));
    }
    for (int __i = 0; __i < 4; __i++) {
        unsigned short __va = (unsigned short)(__a1 >> (__i * 16));
        unsigned short __vb = (unsigned short)(__b1 >> (__i * 16));
        if (__va == __vb) __r1 |= (0xFFFFULL << (__i * 16));
    }
    return (__m128i){ { (long long)__r0, (long long)__r1 } };
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_cmplt_epi16(__m128i __a, __m128i __b)
{
    /* Returns 0xFFFF for lanes where a < b (signed), 0 otherwise.
     * Equivalent to _mm_cmpgt_epi16(b, a). */
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_pcmpgtw128(__b, __a));
}

/* === Unsigned Saturating Arithmetic === */

static __inline__ __m128i __attribute__((__always_inline__))
_mm_adds_epu8(__m128i __a, __m128i __b)
{
    /* Unsigned saturating add of 16 x 8-bit elements. */
    unsigned char *__pa = (unsigned char *)&__a;
    unsigned char *__pb = (unsigned char *)&__b;
    __m128i __r;
    unsigned char *__pr = (unsigned char *)&__r;
    for (int __i = 0; __i < 16; __i++) {
        unsigned int __s = (unsigned int)__pa[__i] + (unsigned int)__pb[__i];
        __pr[__i] = (unsigned char)(__s > 255 ? 255 : __s);
    }
    return __r;
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_adds_epu16(__m128i __a, __m128i __b)
{
    /* Unsigned saturating add of 8 x 16-bit elements. */
    unsigned short *__pa = (unsigned short *)&__a;
    unsigned short *__pb = (unsigned short *)&__b;
    __m128i __r;
    unsigned short *__pr = (unsigned short *)&__r;
    for (int __i = 0; __i < 8; __i++) {
        unsigned int __s = (unsigned int)__pa[__i] + (unsigned int)__pb[__i];
        __pr[__i] = (unsigned short)(__s > 65535 ? 65535 : __s);
    }
    return __r;
}

/* === Signed Saturating Arithmetic === */

static __inline__ __m128i __attribute__((__always_inline__))
_mm_adds_epi8(__m128i __a, __m128i __b)
{
    /* Signed saturating add of 16 x 8-bit elements. */
    signed char *__pa = (signed char *)&__a;
    signed char *__pb = (signed char *)&__b;
    __m128i __r;
    signed char *__pr = (signed char *)&__r;
    for (int __i = 0; __i < 16; __i++) {
        int __s = (int)__pa[__i] + (int)__pb[__i];
        if (__s > 127) __s = 127;
        if (__s < -128) __s = -128;
        __pr[__i] = (signed char)__s;
    }
    return __r;
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_adds_epi16(__m128i __a, __m128i __b)
{
    /* Signed saturating add of 8 x 16-bit elements. */
    short *__pa = (short *)&__a;
    short *__pb = (short *)&__b;
    __m128i __r;
    short *__pr = (short *)&__r;
    for (int __i = 0; __i < 8; __i++) {
        int __s = (int)__pa[__i] + (int)__pb[__i];
        if (__s > 32767) __s = 32767;
        if (__s < -32768) __s = -32768;
        __pr[__i] = (short)__s;
    }
    return __r;
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_subs_epi16(__m128i __a, __m128i __b)
{
    /* Signed saturating subtract of 8 x 16-bit elements. */
    short *__pa = (short *)&__a;
    short *__pb = (short *)&__b;
    __m128i __r;
    short *__pr = (short *)&__r;
    for (int __i = 0; __i < 8; __i++) {
        int __d = (int)__pa[__i] - (int)__pb[__i];
        if (__d > 32767) __d = 32767;
        if (__d < -32768) __d = -32768;
        __pr[__i] = (short)__d;
    }
    return __r;
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_subs_epu16(__m128i __a, __m128i __b)
{
    /* Unsigned saturating subtract of 8 x 16-bit elements. */
    unsigned short *__pa = (unsigned short *)&__a;
    unsigned short *__pb = (unsigned short *)&__b;
    __m128i __r;
    unsigned short *__pr = (unsigned short *)&__r;
    for (int __i = 0; __i < 8; __i++) {
        int __d = (int)__pa[__i] - (int)__pb[__i];
        __pr[__i] = (unsigned short)(__d < 0 ? 0 : __d);
    }
    return __r;
}

/* === Arithmetic === */

static __inline__ __m128i __attribute__((__always_inline__))
_mm_subs_epu8(__m128i __a, __m128i __b)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_psubusb128(__a, __b));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_subs_epi8(__m128i __a, __m128i __b)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_psubsb128(__a, __b));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_avg_epu8(__m128i __a, __m128i __b)
{
    /* Unsigned byte average with rounding: (a + b + 1) >> 1 for each byte. */
    unsigned char *__pa = (unsigned char *)&__a;
    unsigned char *__pb = (unsigned char *)&__b;
    __m128i __r;
    unsigned char *__pr = (unsigned char *)&__r;
    for (int __i = 0; __i < 16; __i++)
        __pr[__i] = (unsigned char)(((unsigned int)__pa[__i] + (unsigned int)__pb[__i] + 1) >> 1);
    return __r;
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_min_epi16(__m128i __a, __m128i __b)
{
    /* Signed 16-bit minimum for each of 8 lanes. */
    short *__pa = (short *)&__a;
    short *__pb = (short *)&__b;
    __m128i __r;
    short *__pr = (short *)&__r;
    for (int __i = 0; __i < 8; __i++)
        __pr[__i] = __pa[__i] < __pb[__i] ? __pa[__i] : __pb[__i];
    return __r;
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_max_epi16(__m128i __a, __m128i __b)
{
    /* Signed 16-bit maximum for each of 8 lanes. */
    short *__pa = (short *)&__a;
    short *__pb = (short *)&__b;
    __m128i __r;
    short *__pr = (short *)&__r;
    for (int __i = 0; __i < 8; __i++)
        __pr[__i] = __pa[__i] > __pb[__i] ? __pa[__i] : __pb[__i];
    return __r;
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_min_epu8(__m128i __a, __m128i __b)
{
    /* Unsigned byte minimum for each of 16 lanes. */
    unsigned char *__pa = (unsigned char *)&__a;
    unsigned char *__pb = (unsigned char *)&__b;
    __m128i __r;
    unsigned char *__pr = (unsigned char *)&__r;
    for (int __i = 0; __i < 16; __i++)
        __pr[__i] = __pa[__i] < __pb[__i] ? __pa[__i] : __pb[__i];
    return __r;
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_max_epu8(__m128i __a, __m128i __b)
{
    /* Unsigned byte maximum for each of 16 lanes. */
    unsigned char *__pa = (unsigned char *)&__a;
    unsigned char *__pb = (unsigned char *)&__b;
    __m128i __r;
    unsigned char *__pr = (unsigned char *)&__r;
    for (int __i = 0; __i < 16; __i++)
        __pr[__i] = __pa[__i] > __pb[__i] ? __pa[__i] : __pb[__i];
    return __r;
}

/* === Bitwise === */

static __inline__ __m128i __attribute__((__always_inline__))
_mm_or_si128(__m128i __a, __m128i __b)
{
    return (__m128i){ { __a.__val[0] | __b.__val[0],
                        __a.__val[1] | __b.__val[1] } };
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_and_si128(__m128i __a, __m128i __b)
{
    return (__m128i){ { __a.__val[0] & __b.__val[0],
                        __a.__val[1] & __b.__val[1] } };
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_andnot_si128(__m128i __a, __m128i __b)
{
    return (__m128i){ { ~__a.__val[0] & __b.__val[0],
                        ~__a.__val[1] & __b.__val[1] } };
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_xor_si128(__m128i __a, __m128i __b)
{
    return (__m128i){ { __a.__val[0] ^ __b.__val[0],
                        __a.__val[1] ^ __b.__val[1] } };
}

/* === 8-bit Arithmetic === */

static __inline__ __m128i __attribute__((__always_inline__))
_mm_add_epi8(__m128i __a, __m128i __b)
{
    /* Byte-level add using carry-free byte addition trick:
     * For each byte, (a+b) mod 256.  We use a mask to isolate the low
     * bit of each byte pair to propagate carries correctly within bytes
     * but not across byte boundaries. */
    unsigned long long __mask = 0x7f7f7f7f7f7f7f7fULL;
    unsigned long long __a0 = (unsigned long long)__a.__val[0];
    unsigned long long __b0 = (unsigned long long)__b.__val[0];
    unsigned long long __a1 = (unsigned long long)__a.__val[1];
    unsigned long long __b1 = (unsigned long long)__b.__val[1];
    unsigned long long __lo = ((__a0 & __mask) + (__b0 & __mask)) ^ ((__a0 ^ __b0) & ~__mask);
    unsigned long long __hi = ((__a1 & __mask) + (__b1 & __mask)) ^ ((__a1 ^ __b1) & ~__mask);
    return (__m128i){ { (long long)__lo, (long long)__hi } };
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_sub_epi8(__m128i __a, __m128i __b)
{
    unsigned long long __mask = 0x8080808080808080ULL;
    unsigned long long __a0 = (unsigned long long)__a.__val[0];
    unsigned long long __b0 = (unsigned long long)__b.__val[0];
    unsigned long long __a1 = (unsigned long long)__a.__val[1];
    unsigned long long __b1 = (unsigned long long)__b.__val[1];
    unsigned long long __lo = ((__a0 | __mask) - (__b0 & ~__mask)) ^ ((__a0 ^ ~__b0) & __mask);
    unsigned long long __hi = ((__a1 | __mask) - (__b1 & ~__mask)) ^ ((__a1 ^ ~__b1) & __mask);
    return (__m128i){ { (long long)__lo, (long long)__hi } };
}

/* === 16-bit Arithmetic === */

static __inline__ __m128i __attribute__((__always_inline__))
_mm_add_epi16(__m128i __a, __m128i __b)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_paddw128(__a, __b));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_sub_epi16(__m128i __a, __m128i __b)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_psubw128(__a, __b));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_mullo_epi16(__m128i __a, __m128i __b)
{
    /* Multiply 8 x 16-bit signed integers, return low 16 bits of each 32-bit result. */
    unsigned short *__pa = (unsigned short *)&__a;
    unsigned short *__pb = (unsigned short *)&__b;
    __m128i __r;
    unsigned short *__pr = (unsigned short *)&__r;
    for (int __i = 0; __i < 8; __i++)
        __pr[__i] = (unsigned short)((unsigned int)__pa[__i] * (unsigned int)__pb[__i]);
    return __r;
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_mulhi_epi16(__m128i __a, __m128i __b)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_pmulhw128(__a, __b));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_mulhi_epu16(__m128i __a, __m128i __b)
{
    /* Multiply 8 x 16-bit unsigned integers, return high 16 bits of each 32-bit result. */
    unsigned short *__pa = (unsigned short *)&__a;
    unsigned short *__pb = (unsigned short *)&__b;
    __m128i __r;
    unsigned short *__pr = (unsigned short *)&__r;
    for (int __i = 0; __i < 8; __i++)
        __pr[__i] = (unsigned short)(((unsigned int)__pa[__i] * (unsigned int)__pb[__i]) >> 16);
    return __r;
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_madd_epi16(__m128i __a, __m128i __b)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_pmaddwd128(__a, __b));
}

/* === 32-bit Arithmetic === */

static __inline__ __m128i __attribute__((__always_inline__))
_mm_add_epi32(__m128i __a, __m128i __b)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_paddd128(__a, __b));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_sub_epi32(__m128i __a, __m128i __b)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_psubd128(__a, __b));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_cmpgt_epi16(__m128i __a, __m128i __b)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_pcmpgtw128(__a, __b));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_cmpgt_epi8(__m128i __a, __m128i __b)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_pcmpgtb128(__a, __b));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_cmpgt_epi32(__m128i __a, __m128i __b)
{
    /* Compare 4 x 32-bit signed integers: returns 0xFFFFFFFF where a > b, 0 otherwise. */
    int *__pa = (int *)&__a;
    int *__pb = (int *)&__b;
    __m128i __r;
    int *__pr = (int *)&__r;
    for (int __i = 0; __i < 4; __i++)
        __pr[__i] = __pa[__i] > __pb[__i] ? -1 : 0;
    return __r;
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_cmplt_epi8(__m128i __a, __m128i __b)
{
    /* Returns 0xFF for lanes where a < b (signed), 0 otherwise.
     * Equivalent to _mm_cmpgt_epi8(b, a). */
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_pcmpgtb128(__b, __a));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_cmplt_epi32(__m128i __a, __m128i __b)
{
    /* Returns 0xFFFFFFFF for lanes where a < b (signed), 0 otherwise.
     * Equivalent to _mm_cmpgt_epi32(b, a). */
    int *__pa = (int *)&__a;
    int *__pb = (int *)&__b;
    __m128i __r;
    int *__pr = (int *)&__r;
    for (int __i = 0; __i < 4; __i++)
        __pr[__i] = __pa[__i] < __pb[__i] ? -1 : 0;
    return __r;
}

/* _mm_mul_epu32: unsigned 32x32->64 multiply (PMULUDQ)
 * Multiplies the low 32-bit unsigned integers from each 64-bit lane:
 * result[0] = (u32)a[0] * (u32)b[0], result[1] = (u32)a[2] * (u32)b[2] */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_mul_epu32(__m128i __a, __m128i __b)
{
    unsigned long long __a0 = (unsigned long long)(unsigned int)__a.__val[0];
    unsigned long long __b0 = (unsigned long long)(unsigned int)__b.__val[0];
    unsigned long long __a1 = (unsigned long long)(unsigned int)__a.__val[1];
    unsigned long long __b1 = (unsigned long long)(unsigned int)__b.__val[1];
    return (__m128i){ { (long long)(__a0 * __b0), (long long)(__a1 * __b1) } };
}

/* NOTE: _mm_mullo_epi16 is already defined above */

/* === 64-bit Arithmetic === */

static __inline__ __m128i __attribute__((__always_inline__))
_mm_add_epi64(__m128i __a, __m128i __b)
{
    return (__m128i){ { __a.__val[0] + __b.__val[0],
                        __a.__val[1] + __b.__val[1] } };
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_sub_epi64(__m128i __a, __m128i __b)
{
    return (__m128i){ { __a.__val[0] - __b.__val[0],
                        __a.__val[1] - __b.__val[1] } };
}

/* === Pack / Unpack === */

static __inline__ __m128i __attribute__((__always_inline__))
_mm_packs_epi32(__m128i __a, __m128i __b)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_packssdw128(__a, __b));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_packs_epi16(__m128i __a, __m128i __b)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_packsswb128(__a, __b));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_packus_epi16(__m128i __a, __m128i __b)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_packuswb128(__a, __b));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_unpacklo_epi8(__m128i __a, __m128i __b)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_punpcklbw128(__a, __b));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_unpackhi_epi8(__m128i __a, __m128i __b)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_punpckhbw128(__a, __b));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_unpacklo_epi16(__m128i __a, __m128i __b)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_punpcklwd128(__a, __b));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_unpackhi_epi16(__m128i __a, __m128i __b)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_punpckhwd128(__a, __b));
}

/* Interleave low 32-bit integers: a0, b0, a1, b1 */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_unpacklo_epi32(__m128i __a, __m128i __b)
{
    unsigned int __a0 = (unsigned int)__a.__val[0];
    unsigned int __a1 = (unsigned int)(__a.__val[0] >> 32);
    unsigned int __b0 = (unsigned int)__b.__val[0];
    unsigned int __b1 = (unsigned int)(__b.__val[0] >> 32);
    long long __lo = (long long)__a0 | ((long long)__b0 << 32);
    long long __hi = (long long)__a1 | ((long long)__b1 << 32);
    return (__m128i){ { __lo, __hi } };
}

/* Interleave high 32-bit integers: a2, b2, a3, b3 */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_unpackhi_epi32(__m128i __a, __m128i __b)
{
    unsigned int __a2 = (unsigned int)__a.__val[1];
    unsigned int __a3 = (unsigned int)(__a.__val[1] >> 32);
    unsigned int __b2 = (unsigned int)__b.__val[1];
    unsigned int __b3 = (unsigned int)(__b.__val[1] >> 32);
    long long __lo = (long long)__a2 | ((long long)__b2 << 32);
    long long __hi = (long long)__a3 | ((long long)__b3 << 32);
    return (__m128i){ { __lo, __hi } };
}

/* Interleave low 64-bit integers: a0, b0 */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_unpacklo_epi64(__m128i __a, __m128i __b)
{
    return (__m128i){ { __a.__val[0], __b.__val[0] } };
}

/* Interleave high 64-bit integers: a1, b1 */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_unpackhi_epi64(__m128i __a, __m128i __b)
{
    return (__m128i){ { __a.__val[1], __b.__val[1] } };
}

/* === Set / Broadcast === */

static __inline__ __m128i __attribute__((__always_inline__))
_mm_set1_epi16(short __w)
{
    unsigned short __uw = (unsigned short)__w;
    long long __q = (long long)__uw | ((long long)__uw << 16)
                  | ((long long)__uw << 32) | ((long long)__uw << 48);
    return (__m128i){ { __q, __q } };
}

/* _mm_setr_epi16: set 8 x 16-bit lanes in natural (low-to-high) order */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_setr_epi16(short __w0, short __w1, short __w2, short __w3,
               short __w4, short __w5, short __w6, short __w7)
{
    long long __lo = (long long)(unsigned short)__w0
                   | ((long long)(unsigned short)__w1 << 16)
                   | ((long long)(unsigned short)__w2 << 32)
                   | ((long long)(unsigned short)__w3 << 48);
    long long __hi = (long long)(unsigned short)__w4
                   | ((long long)(unsigned short)__w5 << 16)
                   | ((long long)(unsigned short)__w6 << 32)
                   | ((long long)(unsigned short)__w7 << 48);
    return (__m128i){ { __lo, __hi } };
}

/* _mm_set_epi16: set 8 x 16-bit lanes in reverse (high-to-low) order */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_set_epi16(short __w7, short __w6, short __w5, short __w4,
              short __w3, short __w2, short __w1, short __w0)
{
    return _mm_setr_epi16(__w0, __w1, __w2, __w3, __w4, __w5, __w6, __w7);
}

/* _mm_setr_epi32: set 4 x 32-bit lanes in natural (low-to-high) order */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_setr_epi32(int __i0, int __i1, int __i2, int __i3)
{
    long long __lo = (long long)(unsigned int)__i0
                   | ((long long)(unsigned int)__i1 << 32);
    long long __hi = (long long)(unsigned int)__i2
                   | ((long long)(unsigned int)__i3 << 32);
    return (__m128i){ { __lo, __hi } };
}

/* _mm_set_epi32: set 4 x 32-bit lanes in reverse (high-to-low) order */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_set_epi32(int __i3, int __i2, int __i1, int __i0)
{
    return _mm_setr_epi32(__i0, __i1, __i2, __i3);
}

/* _mm_set_epi8: set 16 x 8-bit lanes in reverse (high-to-low) order */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_set_epi8(char __b15, char __b14, char __b13, char __b12,
             char __b11, char __b10, char __b9, char __b8,
             char __b7, char __b6, char __b5, char __b4,
             char __b3, char __b2, char __b1, char __b0)
{
    long long __lo = (long long)(unsigned char)__b0
                   | ((long long)(unsigned char)__b1 << 8)
                   | ((long long)(unsigned char)__b2 << 16)
                   | ((long long)(unsigned char)__b3 << 24)
                   | ((long long)(unsigned char)__b4 << 32)
                   | ((long long)(unsigned char)__b5 << 40)
                   | ((long long)(unsigned char)__b6 << 48)
                   | ((long long)(unsigned char)__b7 << 56);
    long long __hi = (long long)(unsigned char)__b8
                   | ((long long)(unsigned char)__b9 << 8)
                   | ((long long)(unsigned char)__b10 << 16)
                   | ((long long)(unsigned char)__b11 << 24)
                   | ((long long)(unsigned char)__b12 << 32)
                   | ((long long)(unsigned char)__b13 << 40)
                   | ((long long)(unsigned char)__b14 << 48)
                   | ((long long)(unsigned char)__b15 << 56);
    return (__m128i){ { __lo, __hi } };
}

/* _mm_setr_epi8: set 16 x 8-bit lanes in natural (low-to-high) order */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_setr_epi8(char __b0, char __b1, char __b2, char __b3,
              char __b4, char __b5, char __b6, char __b7,
              char __b8, char __b9, char __b10, char __b11,
              char __b12, char __b13, char __b14, char __b15)
{
    return _mm_set_epi8(__b15, __b14, __b13, __b12,
                        __b11, __b10, __b9, __b8,
                        __b7, __b6, __b5, __b4,
                        __b3, __b2, __b1, __b0);
}

/* _mm_set_epi64x: set two 64-bit integers (high, low) */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_set_epi64x(long long __hi, long long __lo)
{
    return (__m128i){ { __lo, __hi } };
}

/* _mm_set1_epi64x: broadcast 64-bit integer to both lanes */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_set1_epi64x(long long __q)
{
    return (__m128i){ { __q, __q } };
}

/* === Insert / Extract === */

#define _mm_insert_epi16(a, i, imm) \
    __CCC_M128I_FROM_BUILTIN(__builtin_ia32_pinsrw128((a), (i), (imm)))

#define _mm_extract_epi16(a, imm) \
    __builtin_ia32_pextrw128((a), (imm))

/* === Convert / Move === */

static __inline__ int __attribute__((__always_inline__))
_mm_cvtsi128_si32(__m128i __a)
{
    return __builtin_ia32_cvtsi128si32(__a);
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_cvtsi32_si128(int __a)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_cvtsi32si128(__a));
}

static __inline__ long long __attribute__((__always_inline__))
_mm_cvtsi128_si64(__m128i __a)
{
    return __builtin_ia32_cvtsi128si64(__a);
}

#define _mm_cvtsi128_si64x(a) _mm_cvtsi128_si64(a)

/* _mm_cvtsi64_si128: convert 64-bit integer to __m128i (MOVQ, zero upper) */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_cvtsi64_si128(long long __a)
{
    return _mm_set_epi64x(0, __a);
}

#define _mm_cvtsi64x_si128(a) _mm_cvtsi64_si128(a)

/* === Store low 64 bits === */

static __inline__ void __attribute__((__always_inline__))
_mm_storel_epi64(__m128i *__p, __m128i __a)
{
    __builtin_ia32_storeldi128(__p, __a);
}

/* === Shuffle 16-bit === */

#define _mm_shufflelo_epi16(a, imm) \
    __CCC_M128I_FROM_BUILTIN(__builtin_ia32_pshuflw128((a), (imm)))

#define _mm_shufflehi_epi16(a, imm) \
    __CCC_M128I_FROM_BUILTIN(__builtin_ia32_pshufhw128((a), (imm)))

/* === Shift operations === */

/* Bit-level shift left on each 16-bit element (PSLLW) */
#define _mm_slli_epi16(a, count) \
    __CCC_M128I_FROM_BUILTIN(__builtin_ia32_psllwi128((a), (count)))

/* Bit-level shift right logical on each 16-bit element (PSRLW) */
#define _mm_srli_epi16(a, count) \
    __CCC_M128I_FROM_BUILTIN(__builtin_ia32_psrlwi128((a), (count)))

/* Bit-level shift right arithmetic on each 16-bit element (PSRAW) */
#define _mm_srai_epi16(a, count) \
    __CCC_M128I_FROM_BUILTIN(__builtin_ia32_psrawi128((a), (count)))

/* Bit-level shift right arithmetic on each 32-bit element (PSRAD) */
#define _mm_srai_epi32(a, count) \
    __CCC_M128I_FROM_BUILTIN(__builtin_ia32_psradi128((a), (count)))

/* Bit-level shift left on each 32-bit element (PSLLD) */
#define _mm_slli_epi32(a, count) \
    __CCC_M128I_FROM_BUILTIN(__builtin_ia32_pslldi128((a), (count)))

/* Bit-level shift right logical on each 32-bit element (PSRLD) */
#define _mm_srli_epi32(a, count) \
    __CCC_M128I_FROM_BUILTIN(__builtin_ia32_psrldi128((a), (count)))

/* Byte-level shift left (PSLLDQ): shift __a left by __N bytes, zero-fill */
#define _mm_slli_si128(a, N) \
    __CCC_M128I_FROM_BUILTIN(__builtin_ia32_pslldqi128((a), (N)))

/* Byte-level shift right (PSRLDQ): shift __a right by __N bytes, zero-fill */
#define _mm_srli_si128(a, N) \
    __CCC_M128I_FROM_BUILTIN(__builtin_ia32_psrldqi128((a), (N)))

/* Bit-level shift left on each 64-bit element (PSLLQ) */
#define _mm_slli_epi64(a, count) \
    __CCC_M128I_FROM_BUILTIN(__builtin_ia32_psllqi128((a), (count)))

/* Bit-level shift right on each 64-bit element (PSRLQ) */
#define _mm_srli_epi64(a, count) \
    __CCC_M128I_FROM_BUILTIN(__builtin_ia32_psrlqi128((a), (count)))

/* Shuffle 32-bit integers (PSHUFD) */
#define _mm_shuffle_epi32(a, imm) \
    __CCC_M128I_FROM_BUILTIN(__builtin_ia32_pshufd128((a), (imm)))

/* Load low 64 bits into lower half, zero upper half (MOVQ) */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_loadl_epi64(__m128i const *__p)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_loadldi128(__p));
}

/* === Float/Int Conversion (SSE2) === */

/* Convert packed 32-bit integers to packed single-precision floats */
static __inline__ __m128 __attribute__((__always_inline__))
_mm_cvtepi32_ps(__m128i __a)
{
    int __i0 = (int)__a.__val[0];
    int __i1 = (int)(__a.__val[0] >> 32);
    int __i2 = (int)__a.__val[1];
    int __i3 = (int)(__a.__val[1] >> 32);
    return (__m128){ { (float)__i0, (float)__i1, (float)__i2, (float)__i3 } };
}

/* Convert packed single-precision floats to packed 32-bit integers (round to nearest) */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_cvtps_epi32(__m128 __a)
{
    /* Round to nearest integer. Use (int)(x + copysignf(0.5f, x)) as a
     * portable approximation of round-to-nearest-even. This doesn't perfectly
     * match banker's rounding for .5 cases, but works for typical audio/video usage. */
    int __i0 = (int)(__a.__val[0] >= 0.0f ? __a.__val[0] + 0.5f : __a.__val[0] - 0.5f);
    int __i1 = (int)(__a.__val[1] >= 0.0f ? __a.__val[1] + 0.5f : __a.__val[1] - 0.5f);
    int __i2 = (int)(__a.__val[2] >= 0.0f ? __a.__val[2] + 0.5f : __a.__val[2] - 0.5f);
    int __i3 = (int)(__a.__val[3] >= 0.0f ? __a.__val[3] + 0.5f : __a.__val[3] - 0.5f);
    long long __lo = (long long)(unsigned int)__i0 | ((long long)(unsigned int)__i1 << 32);
    long long __hi = (long long)(unsigned int)__i2 | ((long long)(unsigned int)__i3 << 32);
    return (__m128i){ { __lo, __hi } };
}

/* Convert packed single-precision floats to packed 32-bit integers (truncate) */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_cvttps_epi32(__m128 __a)
{
    int __i0 = (int)__a.__val[0];
    int __i1 = (int)__a.__val[1];
    int __i2 = (int)__a.__val[2];
    int __i3 = (int)__a.__val[3];
    long long __lo = (long long)(unsigned int)__i0 | ((long long)(unsigned int)__i1 << 32);
    long long __hi = (long long)(unsigned int)__i2 | ((long long)(unsigned int)__i3 << 32);
    return (__m128i){ { __lo, __hi } };
}

/* === Miscellaneous === */

static __inline__ int __attribute__((__always_inline__))
_mm_movemask_epi8(__m128i __a)
{
    return __builtin_ia32_pmovmskb128(__a);
}

/* === Streaming / Non-temporal stores === */

static __inline__ void __attribute__((__always_inline__))
_mm_stream_si128(__m128i *__p, __m128i __a)
{
    __builtin_ia32_movntdq(__p, __a);
}

static __inline__ void __attribute__((__always_inline__))
_mm_stream_si64(long long *__p, long long __a)
{
    __builtin_ia32_movnti64(__p, __a);
}

static __inline__ void __attribute__((__always_inline__))
_mm_stream_si32(int *__p, int __a)
{
    __builtin_ia32_movnti(__p, __a);
}

static __inline__ void __attribute__((__always_inline__))
_mm_stream_pd(double *__p, __m128d __a)
{
    __builtin_ia32_movntpd(__p, __a);
}

/* === Type Cast (reinterpret) === */

static __inline__ __m128i __attribute__((__always_inline__))
_mm_castps_si128(__m128 __a)
{
    __m128i __r;
    __builtin_memcpy(&__r, &__a, sizeof(__r));
    return __r;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_castsi128_ps(__m128i __a)
{
    __m128 __r;
    __builtin_memcpy(&__r, &__a, sizeof(__r));
    return __r;
}

static __inline__ __m128d __attribute__((__always_inline__))
_mm_castps_pd(__m128 __a)
{
    __m128d __r;
    __builtin_memcpy(&__r, &__a, sizeof(__r));
    return __r;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_castpd_ps(__m128d __a)
{
    __m128 __r;
    __builtin_memcpy(&__r, &__a, sizeof(__r));
    return __r;
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_castpd_si128(__m128d __a)
{
    __m128i __r;
    __builtin_memcpy(&__r, &__a, sizeof(__r));
    return __r;
}

static __inline__ __m128d __attribute__((__always_inline__))
_mm_castsi128_pd(__m128i __a)
{
    __m128d __r;
    __builtin_memcpy(&__r, &__a, sizeof(__r));
    return __r;
}

/* ====================================================================
 * SSE2 Double-Precision Floating-Point Intrinsics (__m128d)
 * ==================================================================== */

/* === Double Set / Broadcast === */

/* _mm_set_pd: set two doubles (high, low) - note parameter order */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_set_pd(double __hi, double __lo)
{
    return (__m128d){ { __lo, __hi } };
}

/* _mm_set1_pd: broadcast one double to both lanes */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_set1_pd(double __d)
{
    return (__m128d){ { __d, __d } };
}

#define _mm_set_pd1(d) _mm_set1_pd(d)

/* _mm_set_sd: set low double, zero high */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_set_sd(double __d)
{
    return (__m128d){ { __d, 0.0 } };
}

/* _mm_setr_pd: set in natural order (low, high) */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_setr_pd(double __lo, double __hi)
{
    return (__m128d){ { __lo, __hi } };
}

/* _mm_setzero_pd: zero both lanes */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_setzero_pd(void)
{
    return (__m128d){ { 0.0, 0.0 } };
}

/* _mm_undefined_pd: uninitialized (returns zero for safety) */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_undefined_pd(void)
{
    return (__m128d){ { 0.0, 0.0 } };
}

/* === Double Load === */

/* _mm_load_pd: aligned load of 2 doubles */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_load_pd(double const *__p)
{
    return *(__m128d const *)__p;
}

/* _mm_loadu_pd: unaligned load of 2 doubles */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_loadu_pd(double const *__p)
{
    return *(__m128d_u const *)__p;
}

/* _mm_load_sd: load one double into low lane, zero high */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_load_sd(double const *__p)
{
    return (__m128d){ { *__p, 0.0 } };
}

/* _mm_load1_pd: load one double, broadcast to both lanes */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_load1_pd(double const *__p)
{
    double __d = *__p;
    return (__m128d){ { __d, __d } };
}

#define _mm_load_pd1(p) _mm_load1_pd(p)

/* _mm_loadr_pd: load 2 doubles in reverse order */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_loadr_pd(double const *__p)
{
    return (__m128d){ { __p[1], __p[0] } };
}

/* _mm_loadl_pd: load low double, keep high from __a */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_loadl_pd(__m128d __a, double const *__p)
{
    __a.__val[0] = *__p;
    return __a;
}

/* _mm_loadh_pd: load high double, keep low from __a */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_loadh_pd(__m128d __a, double const *__p)
{
    __a.__val[1] = *__p;
    return __a;
}

/* === Double Store === */

/* _mm_store_pd: aligned store of 2 doubles */
static __inline__ void __attribute__((__always_inline__))
_mm_store_pd(double *__p, __m128d __a)
{
    *(__m128d *)__p = __a;
}

/* _mm_storeu_pd: unaligned store of 2 doubles */
static __inline__ void __attribute__((__always_inline__))
_mm_storeu_pd(double *__p, __m128d __a)
{
    *(__m128d_u *)__p = __a;
}

/* _mm_store_sd: store low double */
static __inline__ void __attribute__((__always_inline__))
_mm_store_sd(double *__p, __m128d __a)
{
    *__p = __a.__val[0];
}

/* _mm_store1_pd: store low double to both positions */
static __inline__ void __attribute__((__always_inline__))
_mm_store1_pd(double *__p, __m128d __a)
{
    __p[0] = __a.__val[0];
    __p[1] = __a.__val[0];
}

#define _mm_store_pd1(p, a) _mm_store1_pd(p, a)

/* _mm_storer_pd: store 2 doubles in reverse order */
static __inline__ void __attribute__((__always_inline__))
_mm_storer_pd(double *__p, __m128d __a)
{
    __p[0] = __a.__val[1];
    __p[1] = __a.__val[0];
}

/* _mm_storel_pd: store low double to memory */
static __inline__ void __attribute__((__always_inline__))
_mm_storel_pd(double *__p, __m128d __a)
{
    *__p = __a.__val[0];
}

/* _mm_storeh_pd: store high double to memory */
static __inline__ void __attribute__((__always_inline__))
_mm_storeh_pd(double *__p, __m128d __a)
{
    *__p = __a.__val[1];
}

/* === Double Arithmetic === */

/* _mm_add_pd: packed double add */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_add_pd(__m128d __a, __m128d __b)
{
    return (__m128d){ { __a.__val[0] + __b.__val[0],
                        __a.__val[1] + __b.__val[1] } };
}

/* _mm_sub_pd: packed double subtract */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_sub_pd(__m128d __a, __m128d __b)
{
    return (__m128d){ { __a.__val[0] - __b.__val[0],
                        __a.__val[1] - __b.__val[1] } };
}

/* _mm_mul_pd: packed double multiply */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_mul_pd(__m128d __a, __m128d __b)
{
    return (__m128d){ { __a.__val[0] * __b.__val[0],
                        __a.__val[1] * __b.__val[1] } };
}

/* _mm_div_pd: packed double divide */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_div_pd(__m128d __a, __m128d __b)
{
    return (__m128d){ { __a.__val[0] / __b.__val[0],
                        __a.__val[1] / __b.__val[1] } };
}

/* _mm_add_sd: scalar double add (low lane only, high unchanged) */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_add_sd(__m128d __a, __m128d __b)
{
    __a.__val[0] += __b.__val[0];
    return __a;
}

/* _mm_sub_sd: scalar double subtract (low lane only) */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_sub_sd(__m128d __a, __m128d __b)
{
    __a.__val[0] -= __b.__val[0];
    return __a;
}

/* _mm_mul_sd: scalar double multiply (low lane only) */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_mul_sd(__m128d __a, __m128d __b)
{
    __a.__val[0] *= __b.__val[0];
    return __a;
}

/* _mm_div_sd: scalar double divide (low lane only) */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_div_sd(__m128d __a, __m128d __b)
{
    __a.__val[0] /= __b.__val[0];
    return __a;
}

/* _mm_sqrt_pd: packed double square root */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_sqrt_pd(__m128d __a)
{
    return (__m128d){ { __builtin_sqrt(__a.__val[0]), __builtin_sqrt(__a.__val[1]) } };
}

/* _mm_sqrt_sd: scalar double square root (low lane only, high from __a) */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_sqrt_sd(__m128d __a, __m128d __b)
{
    return (__m128d){ { __builtin_sqrt(__b.__val[0]), __a.__val[1] } };
}

/* _mm_min_pd: packed double minimum
 * Note: NaN handling may differ from hardware SSE2 (which returns second
 * operand when one input is NaN). This uses C comparison semantics. */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_min_pd(__m128d __a, __m128d __b)
{
    return (__m128d){ { __a.__val[0] < __b.__val[0] ? __a.__val[0] : __b.__val[0],
                        __a.__val[1] < __b.__val[1] ? __a.__val[1] : __b.__val[1] } };
}

/* _mm_max_pd: packed double maximum
 * Note: NaN handling may differ from hardware SSE2. */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_max_pd(__m128d __a, __m128d __b)
{
    return (__m128d){ { __a.__val[0] > __b.__val[0] ? __a.__val[0] : __b.__val[0],
                        __a.__val[1] > __b.__val[1] ? __a.__val[1] : __b.__val[1] } };
}

/* _mm_min_sd: scalar double minimum */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_min_sd(__m128d __a, __m128d __b)
{
    __a.__val[0] = __a.__val[0] < __b.__val[0] ? __a.__val[0] : __b.__val[0];
    return __a;
}

/* _mm_max_sd: scalar double maximum */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_max_sd(__m128d __a, __m128d __b)
{
    __a.__val[0] = __a.__val[0] > __b.__val[0] ? __a.__val[0] : __b.__val[0];
    return __a;
}

/* === Double Bitwise Operations === */

/* _mm_and_pd: bitwise AND */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_and_pd(__m128d __a, __m128d __b)
{
    __m128d __r;
    long long *__ra = (long long *)&__a;
    long long *__rb = (long long *)&__b;
    long long *__rr = (long long *)&__r;
    __rr[0] = __ra[0] & __rb[0];
    __rr[1] = __ra[1] & __rb[1];
    return __r;
}

/* _mm_or_pd: bitwise OR */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_or_pd(__m128d __a, __m128d __b)
{
    __m128d __r;
    long long *__ra = (long long *)&__a;
    long long *__rb = (long long *)&__b;
    long long *__rr = (long long *)&__r;
    __rr[0] = __ra[0] | __rb[0];
    __rr[1] = __ra[1] | __rb[1];
    return __r;
}

/* _mm_xor_pd: bitwise XOR */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_xor_pd(__m128d __a, __m128d __b)
{
    __m128d __r;
    long long *__ra = (long long *)&__a;
    long long *__rb = (long long *)&__b;
    long long *__rr = (long long *)&__r;
    __rr[0] = __ra[0] ^ __rb[0];
    __rr[1] = __ra[1] ^ __rb[1];
    return __r;
}

/* _mm_andnot_pd: bitwise AND NOT (~a & b) */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_andnot_pd(__m128d __a, __m128d __b)
{
    __m128d __r;
    long long *__ra = (long long *)&__a;
    long long *__rb = (long long *)&__b;
    long long *__rr = (long long *)&__r;
    __rr[0] = ~__ra[0] & __rb[0];
    __rr[1] = ~__ra[1] & __rb[1];
    return __r;
}

/* === Double Compare (returns mask: all 1s for true, all 0s for false) === */

static __inline__ __m128d __attribute__((__always_inline__))
_mm_cmpeq_pd(__m128d __a, __m128d __b)
{
    __m128d __r;
    long long *__rr = (long long *)&__r;
    __rr[0] = __a.__val[0] == __b.__val[0] ? ~0LL : 0LL;
    __rr[1] = __a.__val[1] == __b.__val[1] ? ~0LL : 0LL;
    return __r;
}

static __inline__ __m128d __attribute__((__always_inline__))
_mm_cmplt_pd(__m128d __a, __m128d __b)
{
    __m128d __r;
    long long *__rr = (long long *)&__r;
    __rr[0] = __a.__val[0] < __b.__val[0] ? ~0LL : 0LL;
    __rr[1] = __a.__val[1] < __b.__val[1] ? ~0LL : 0LL;
    return __r;
}

static __inline__ __m128d __attribute__((__always_inline__))
_mm_cmple_pd(__m128d __a, __m128d __b)
{
    __m128d __r;
    long long *__rr = (long long *)&__r;
    __rr[0] = __a.__val[0] <= __b.__val[0] ? ~0LL : 0LL;
    __rr[1] = __a.__val[1] <= __b.__val[1] ? ~0LL : 0LL;
    return __r;
}

static __inline__ __m128d __attribute__((__always_inline__))
_mm_cmpgt_pd(__m128d __a, __m128d __b)
{
    return _mm_cmplt_pd(__b, __a);
}

static __inline__ __m128d __attribute__((__always_inline__))
_mm_cmpge_pd(__m128d __a, __m128d __b)
{
    return _mm_cmple_pd(__b, __a);
}

static __inline__ __m128d __attribute__((__always_inline__))
_mm_cmpneq_pd(__m128d __a, __m128d __b)
{
    __m128d __r;
    long long *__rr = (long long *)&__r;
    __rr[0] = __a.__val[0] != __b.__val[0] ? ~0LL : 0LL;
    __rr[1] = __a.__val[1] != __b.__val[1] ? ~0LL : 0LL;
    return __r;
}

static __inline__ __m128d __attribute__((__always_inline__))
_mm_cmpnlt_pd(__m128d __a, __m128d __b)
{
    __m128d __r;
    long long *__rr = (long long *)&__r;
    __rr[0] = !(__a.__val[0] < __b.__val[0]) ? ~0LL : 0LL;
    __rr[1] = !(__a.__val[1] < __b.__val[1]) ? ~0LL : 0LL;
    return __r;
}

static __inline__ __m128d __attribute__((__always_inline__))
_mm_cmpnle_pd(__m128d __a, __m128d __b)
{
    __m128d __r;
    long long *__rr = (long long *)&__r;
    __rr[0] = !(__a.__val[0] <= __b.__val[0]) ? ~0LL : 0LL;
    __rr[1] = !(__a.__val[1] <= __b.__val[1]) ? ~0LL : 0LL;
    return __r;
}

static __inline__ __m128d __attribute__((__always_inline__))
_mm_cmpngt_pd(__m128d __a, __m128d __b)
{
    return _mm_cmpnlt_pd(__b, __a);
}

static __inline__ __m128d __attribute__((__always_inline__))
_mm_cmpnge_pd(__m128d __a, __m128d __b)
{
    return _mm_cmpnle_pd(__b, __a);
}

static __inline__ __m128d __attribute__((__always_inline__))
_mm_cmpord_pd(__m128d __a, __m128d __b)
{
    __m128d __r;
    long long *__rr = (long long *)&__r;
    /* ordered: both operands are not NaN */
    __rr[0] = (__a.__val[0] == __a.__val[0] && __b.__val[0] == __b.__val[0]) ? ~0LL : 0LL;
    __rr[1] = (__a.__val[1] == __a.__val[1] && __b.__val[1] == __b.__val[1]) ? ~0LL : 0LL;
    return __r;
}

static __inline__ __m128d __attribute__((__always_inline__))
_mm_cmpunord_pd(__m128d __a, __m128d __b)
{
    __m128d __r;
    long long *__rr = (long long *)&__r;
    /* unordered: at least one operand is NaN */
    __rr[0] = (__a.__val[0] != __a.__val[0] || __b.__val[0] != __b.__val[0]) ? ~0LL : 0LL;
    __rr[1] = (__a.__val[1] != __a.__val[1] || __b.__val[1] != __b.__val[1]) ? ~0LL : 0LL;
    return __r;
}

/* Scalar double compares (low lane only, high from __a) */

static __inline__ __m128d __attribute__((__always_inline__))
_mm_cmpeq_sd(__m128d __a, __m128d __b)
{
    long long *__ra = (long long *)&__a;
    __ra[0] = __a.__val[0] == __b.__val[0] ? ~0LL : 0LL;
    return __a;
}

static __inline__ __m128d __attribute__((__always_inline__))
_mm_cmplt_sd(__m128d __a, __m128d __b)
{
    long long *__ra = (long long *)&__a;
    __ra[0] = __a.__val[0] < __b.__val[0] ? ~0LL : 0LL;
    return __a;
}

static __inline__ __m128d __attribute__((__always_inline__))
_mm_cmple_sd(__m128d __a, __m128d __b)
{
    long long *__ra = (long long *)&__a;
    __ra[0] = __a.__val[0] <= __b.__val[0] ? ~0LL : 0LL;
    return __a;
}

static __inline__ __m128d __attribute__((__always_inline__))
_mm_cmpgt_sd(__m128d __a, __m128d __b)
{
    long long *__ra = (long long *)&__a;
    __ra[0] = __a.__val[0] > __b.__val[0] ? ~0LL : 0LL;
    return __a;
}

static __inline__ __m128d __attribute__((__always_inline__))
_mm_cmpge_sd(__m128d __a, __m128d __b)
{
    long long *__ra = (long long *)&__a;
    __ra[0] = __a.__val[0] >= __b.__val[0] ? ~0LL : 0LL;
    return __a;
}

/* Scalar compare predicates (return int) */

static __inline__ int __attribute__((__always_inline__))
_mm_comieq_sd(__m128d __a, __m128d __b)
{
    return __a.__val[0] == __b.__val[0];
}

static __inline__ int __attribute__((__always_inline__))
_mm_comilt_sd(__m128d __a, __m128d __b)
{
    return __a.__val[0] < __b.__val[0];
}

static __inline__ int __attribute__((__always_inline__))
_mm_comile_sd(__m128d __a, __m128d __b)
{
    return __a.__val[0] <= __b.__val[0];
}

static __inline__ int __attribute__((__always_inline__))
_mm_comigt_sd(__m128d __a, __m128d __b)
{
    return __a.__val[0] > __b.__val[0];
}

static __inline__ int __attribute__((__always_inline__))
_mm_comige_sd(__m128d __a, __m128d __b)
{
    return __a.__val[0] >= __b.__val[0];
}

static __inline__ int __attribute__((__always_inline__))
_mm_comineq_sd(__m128d __a, __m128d __b)
{
    return __a.__val[0] != __b.__val[0];
}

#define _mm_ucomieq_sd(a, b) _mm_comieq_sd(a, b)
#define _mm_ucomilt_sd(a, b) _mm_comilt_sd(a, b)
#define _mm_ucomile_sd(a, b) _mm_comile_sd(a, b)
#define _mm_ucomigt_sd(a, b) _mm_comigt_sd(a, b)
#define _mm_ucomige_sd(a, b) _mm_comige_sd(a, b)
#define _mm_ucomineq_sd(a, b) _mm_comineq_sd(a, b)

/* === Double Shuffle / Unpack === */

/* _mm_unpacklo_pd: interleave low doubles: {a[0], b[0]} */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_unpacklo_pd(__m128d __a, __m128d __b)
{
    return (__m128d){ { __a.__val[0], __b.__val[0] } };
}

/* _mm_unpackhi_pd: interleave high doubles: {a[1], b[1]} */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_unpackhi_pd(__m128d __a, __m128d __b)
{
    return (__m128d){ { __a.__val[1], __b.__val[1] } };
}

/* _mm_shuffle_pd: shuffle two doubles based on immediate mask */
#define _mm_shuffle_pd(__a, __b, __imm) \
    ((__m128d){ { (__a).__val[(__imm) & 1], (__b).__val[((__imm) >> 1) & 1] } })

/* _mm_move_sd: move low double from __b, keep high from __a */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_move_sd(__m128d __a, __m128d __b)
{
    return (__m128d){ { __b.__val[0], __a.__val[1] } };
}

/* _mm_movemask_pd: extract sign bits of two doubles */
static __inline__ int __attribute__((__always_inline__))
_mm_movemask_pd(__m128d __a)
{
    long long *__p = (long long *)&__a;
    return ((__p[0] >> 63) & 1) | (((__p[1] >> 63) & 1) << 1);
}

/* === Double Conversion === */

/* _mm_cvtsd_f64: extract low double as scalar */
static __inline__ double __attribute__((__always_inline__))
_mm_cvtsd_f64(__m128d __a)
{
    return __a.__val[0];
}

/* _mm_cvtsd_si32: convert low double to int (round to nearest) */
static __inline__ int __attribute__((__always_inline__))
_mm_cvtsd_si32(__m128d __a)
{
    double __d = __a.__val[0];
    return (int)(__d >= 0.0 ? __d + 0.5 : __d - 0.5);
}

/* _mm_cvttsd_si32: convert low double to int (truncate) */
static __inline__ int __attribute__((__always_inline__))
_mm_cvttsd_si32(__m128d __a)
{
    return (int)__a.__val[0];
}

/* _mm_cvtsd_si64: convert low double to long long (round to nearest) */
static __inline__ long long __attribute__((__always_inline__))
_mm_cvtsd_si64(__m128d __a)
{
    double __d = __a.__val[0];
    return (long long)(__d >= 0.0 ? __d + 0.5 : __d - 0.5);
}

#define _mm_cvtsd_si64x(a) _mm_cvtsd_si64(a)

/* _mm_cvttsd_si64: convert low double to long long (truncate) */
static __inline__ long long __attribute__((__always_inline__))
_mm_cvttsd_si64(__m128d __a)
{
    return (long long)__a.__val[0];
}

#define _mm_cvttsd_si64x(a) _mm_cvttsd_si64(a)

/* _mm_cvtsi32_sd: convert int to double, set low lane, keep high from __a */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_cvtsi32_sd(__m128d __a, int __b)
{
    __a.__val[0] = (double)__b;
    return __a;
}

/* _mm_cvtsi64_sd: convert long long to double, set low lane */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_cvtsi64_sd(__m128d __a, long long __b)
{
    __a.__val[0] = (double)__b;
    return __a;
}

#define _mm_cvtsi64x_sd(a, b) _mm_cvtsi64_sd(a, b)

/* _mm_cvtpd_ps: convert 2 doubles to 2 floats (low half of __m128) */
static __inline__ __m128 __attribute__((__always_inline__))
_mm_cvtpd_ps(__m128d __a)
{
    return (__m128){ { (float)__a.__val[0], (float)__a.__val[1], 0.0f, 0.0f } };
}

/* _mm_cvtps_pd: convert 2 floats (low half of __m128) to 2 doubles */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_cvtps_pd(__m128 __a)
{
    return (__m128d){ { (double)__a.__val[0], (double)__a.__val[1] } };
}

/* _mm_cvtsd_ss: convert low double to float, put in low lane of __a */
static __inline__ __m128 __attribute__((__always_inline__))
_mm_cvtsd_ss(__m128 __a, __m128d __b)
{
    __a.__val[0] = (float)__b.__val[0];
    return __a;
}

/* _mm_cvtss_sd: convert low float to double, put in low lane of __a */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_cvtss_sd(__m128d __a, __m128 __b)
{
    __a.__val[0] = (double)__b.__val[0];
    return __a;
}

/* _mm_cvtpd_epi32: convert 2 doubles to 2 packed 32-bit integers (round to nearest) */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_cvtpd_epi32(__m128d __a)
{
    int __i0 = (int)(__a.__val[0] >= 0.0 ? __a.__val[0] + 0.5 : __a.__val[0] - 0.5);
    int __i1 = (int)(__a.__val[1] >= 0.0 ? __a.__val[1] + 0.5 : __a.__val[1] - 0.5);
    long long __lo = (long long)(unsigned int)__i0 | ((long long)(unsigned int)__i1 << 32);
    return (__m128i){ { __lo, 0LL } };
}

/* _mm_cvttpd_epi32: convert 2 doubles to 2 packed 32-bit integers (truncate) */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_cvttpd_epi32(__m128d __a)
{
    int __i0 = (int)__a.__val[0];
    int __i1 = (int)__a.__val[1];
    long long __lo = (long long)(unsigned int)__i0 | ((long long)(unsigned int)__i1 << 32);
    return (__m128i){ { __lo, 0LL } };
}

/* _mm_cvtepi32_pd: convert 2 packed 32-bit integers (low half) to 2 doubles */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_cvtepi32_pd(__m128i __a)
{
    int __i0 = (int)__a.__val[0];
    int __i1 = (int)(__a.__val[0] >> 32);
    return (__m128d){ { (double)__i0, (double)__i1 } };
}

/* === Fence / Cache === */

static __inline__ void __attribute__((__always_inline__))
_mm_lfence(void)
{
    __builtin_ia32_lfence();
}

static __inline__ void __attribute__((__always_inline__))
_mm_mfence(void)
{
    __builtin_ia32_mfence();
}

static __inline__ void __attribute__((__always_inline__))
_mm_clflush(void const *__p)
{
    __builtin_ia32_clflush(__p);
}

#endif /* _EMMINTRIN_H_INCLUDED */
