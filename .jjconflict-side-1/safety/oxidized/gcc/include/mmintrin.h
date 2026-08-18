/* CCC compiler bundled mmintrin.h - MMX intrinsics */
#ifndef _MMINTRIN_H_INCLUDED
#define _MMINTRIN_H_INCLUDED

/* MMX intrinsics are only available on x86/x86-64 targets */
#if !defined(__x86_64__) && !defined(__i386__) && !defined(__i686__)
#error "MMX intrinsics (mmintrin.h) require an x86 target"
#endif

/* __m64: 64-bit MMX vector type.
 * Represents 8 bytes / 4 shorts / 2 ints / 1 long long packed value. */
typedef struct __attribute__((__aligned__(8))) {
    long long __val;
} __m64;

/* === Empty (EMMS) === */

/* Signal end of MMX state usage (EMMS). No-op in our scalar implementation
 * since we don't use actual MMX registers. Included for API compatibility. */
static __inline__ void __attribute__((__always_inline__))
_mm_empty(void)
{
    /* no-op in our implementation since we use scalar code */
}

/* === Zero / Set === */

static __inline__ __m64 __attribute__((__always_inline__))
_mm_setzero_si64(void)
{
    __m64 __r;
    __r.__val = 0;
    return __r;
}

static __inline__ __m64 __attribute__((__always_inline__))
_mm_cvtsi32_si64(int __a)
{
    __m64 __r;
    __r.__val = (unsigned int)__a;
    return __r;
}

static __inline__ int __attribute__((__always_inline__))
_mm_cvtsi64_si32(__m64 __a)
{
    return (int)__a.__val;
}

static __inline__ __m64 __attribute__((__always_inline__))
_mm_set_pi32(int __hi, int __lo)
{
    __m64 __r;
    __r.__val = ((long long)(unsigned int)__hi << 32) | (unsigned int)__lo;
    return __r;
}

static __inline__ __m64 __attribute__((__always_inline__))
_mm_set_pi16(short __e3, short __e2, short __e1, short __e0)
{
    __m64 __r;
    __r.__val = ((long long)(unsigned short)__e3 << 48) |
                ((long long)(unsigned short)__e2 << 32) |
                ((long long)(unsigned short)__e1 << 16) |
                ((long long)(unsigned short)__e0);
    return __r;
}

static __inline__ __m64 __attribute__((__always_inline__))
_mm_set_pi8(char __e7, char __e6, char __e5, char __e4,
            char __e3, char __e2, char __e1, char __e0)
{
    __m64 __r;
    __r.__val = ((long long)(unsigned char)__e7 << 56) |
                ((long long)(unsigned char)__e6 << 48) |
                ((long long)(unsigned char)__e5 << 40) |
                ((long long)(unsigned char)__e4 << 32) |
                ((long long)(unsigned char)__e3 << 24) |
                ((long long)(unsigned char)__e2 << 16) |
                ((long long)(unsigned char)__e1 << 8) |
                ((long long)(unsigned char)__e0);
    return __r;
}

static __inline__ __m64 __attribute__((__always_inline__))
_mm_set1_pi32(int __a)
{
    return _mm_set_pi32(__a, __a);
}

static __inline__ __m64 __attribute__((__always_inline__))
_mm_set1_pi16(short __a)
{
    return _mm_set_pi16(__a, __a, __a, __a);
}

static __inline__ __m64 __attribute__((__always_inline__))
_mm_set1_pi8(char __a)
{
    return _mm_set_pi8(__a, __a, __a, __a, __a, __a, __a, __a);
}

static __inline__ __m64 __attribute__((__always_inline__))
_mm_setr_pi32(int __lo, int __hi)
{
    return _mm_set_pi32(__hi, __lo);
}

static __inline__ __m64 __attribute__((__always_inline__))
_mm_setr_pi16(short __e0, short __e1, short __e2, short __e3)
{
    return _mm_set_pi16(__e3, __e2, __e1, __e0);
}

/* === Bitwise === */

static __inline__ __m64 __attribute__((__always_inline__))
_mm_and_si64(__m64 __a, __m64 __b)
{
    __m64 __r;
    __r.__val = __a.__val & __b.__val;
    return __r;
}

static __inline__ __m64 __attribute__((__always_inline__))
_mm_andnot_si64(__m64 __a, __m64 __b)
{
    __m64 __r;
    __r.__val = (~__a.__val) & __b.__val;
    return __r;
}

static __inline__ __m64 __attribute__((__always_inline__))
_mm_or_si64(__m64 __a, __m64 __b)
{
    __m64 __r;
    __r.__val = __a.__val | __b.__val;
    return __r;
}

static __inline__ __m64 __attribute__((__always_inline__))
_mm_xor_si64(__m64 __a, __m64 __b)
{
    __m64 __r;
    __r.__val = __a.__val ^ __b.__val;
    return __r;
}

/* === Packed Add (8/16/32-bit) === */

static __inline__ __m64 __attribute__((__always_inline__))
_mm_add_pi8(__m64 __a, __m64 __b)
{
    __m64 __r;
    unsigned char *__ra = (unsigned char *)&__a.__val;
    unsigned char *__rb = (unsigned char *)&__b.__val;
    unsigned char __rr[8];
    for (int __i = 0; __i < 8; __i++)
        __rr[__i] = __ra[__i] + __rb[__i];
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

static __inline__ __m64 __attribute__((__always_inline__))
_mm_add_pi16(__m64 __a, __m64 __b)
{
    __m64 __r;
    unsigned short *__ra = (unsigned short *)&__a.__val;
    unsigned short *__rb = (unsigned short *)&__b.__val;
    unsigned short __rr[4];
    for (int __i = 0; __i < 4; __i++)
        __rr[__i] = __ra[__i] + __rb[__i];
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

static __inline__ __m64 __attribute__((__always_inline__))
_mm_add_pi32(__m64 __a, __m64 __b)
{
    __m64 __r;
    unsigned int *__ra = (unsigned int *)&__a.__val;
    unsigned int *__rb = (unsigned int *)&__b.__val;
    unsigned int __rr[2];
    for (int __i = 0; __i < 2; __i++)
        __rr[__i] = __ra[__i] + __rb[__i];
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

/* === Packed Subtract (8/16/32-bit) === */

static __inline__ __m64 __attribute__((__always_inline__))
_mm_sub_pi8(__m64 __a, __m64 __b)
{
    __m64 __r;
    unsigned char *__ra = (unsigned char *)&__a.__val;
    unsigned char *__rb = (unsigned char *)&__b.__val;
    unsigned char __rr[8];
    for (int __i = 0; __i < 8; __i++)
        __rr[__i] = __ra[__i] - __rb[__i];
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

static __inline__ __m64 __attribute__((__always_inline__))
_mm_sub_pi16(__m64 __a, __m64 __b)
{
    __m64 __r;
    unsigned short *__ra = (unsigned short *)&__a.__val;
    unsigned short *__rb = (unsigned short *)&__b.__val;
    unsigned short __rr[4];
    for (int __i = 0; __i < 4; __i++)
        __rr[__i] = __ra[__i] - __rb[__i];
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

static __inline__ __m64 __attribute__((__always_inline__))
_mm_sub_pi32(__m64 __a, __m64 __b)
{
    __m64 __r;
    unsigned int *__ra = (unsigned int *)&__a.__val;
    unsigned int *__rb = (unsigned int *)&__b.__val;
    unsigned int __rr[2];
    for (int __i = 0; __i < 2; __i++)
        __rr[__i] = __ra[__i] - __rb[__i];
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

/* === Packed Saturating Add (8/16-bit signed and unsigned) === */

static __inline__ __m64 __attribute__((__always_inline__))
_mm_adds_pi8(__m64 __a, __m64 __b)
{
    __m64 __r;
    signed char *__ra = (signed char *)&__a.__val;
    signed char *__rb = (signed char *)&__b.__val;
    signed char __rr[8];
    for (int __i = 0; __i < 8; __i++) {
        int __sum = (int)__ra[__i] + (int)__rb[__i];
        if (__sum > 127) __sum = 127;
        if (__sum < -128) __sum = -128;
        __rr[__i] = (signed char)__sum;
    }
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

static __inline__ __m64 __attribute__((__always_inline__))
_mm_adds_pi16(__m64 __a, __m64 __b)
{
    __m64 __r;
    short *__ra = (short *)&__a.__val;
    short *__rb = (short *)&__b.__val;
    short __rr[4];
    for (int __i = 0; __i < 4; __i++) {
        int __sum = (int)__ra[__i] + (int)__rb[__i];
        if (__sum > 32767) __sum = 32767;
        if (__sum < -32768) __sum = -32768;
        __rr[__i] = (short)__sum;
    }
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

static __inline__ __m64 __attribute__((__always_inline__))
_mm_adds_pu8(__m64 __a, __m64 __b)
{
    __m64 __r;
    unsigned char *__ra = (unsigned char *)&__a.__val;
    unsigned char *__rb = (unsigned char *)&__b.__val;
    unsigned char __rr[8];
    for (int __i = 0; __i < 8; __i++) {
        unsigned int __sum = (unsigned int)__ra[__i] + (unsigned int)__rb[__i];
        if (__sum > 255) __sum = 255;
        __rr[__i] = (unsigned char)__sum;
    }
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

static __inline__ __m64 __attribute__((__always_inline__))
_mm_adds_pu16(__m64 __a, __m64 __b)
{
    __m64 __r;
    unsigned short *__ra = (unsigned short *)&__a.__val;
    unsigned short *__rb = (unsigned short *)&__b.__val;
    unsigned short __rr[4];
    for (int __i = 0; __i < 4; __i++) {
        unsigned int __sum = (unsigned int)__ra[__i] + (unsigned int)__rb[__i];
        if (__sum > 65535) __sum = 65535;
        __rr[__i] = (unsigned short)__sum;
    }
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

/* === Packed Saturating Subtract (8/16-bit signed and unsigned) === */

static __inline__ __m64 __attribute__((__always_inline__))
_mm_subs_pi8(__m64 __a, __m64 __b)
{
    __m64 __r;
    signed char *__ra = (signed char *)&__a.__val;
    signed char *__rb = (signed char *)&__b.__val;
    signed char __rr[8];
    for (int __i = 0; __i < 8; __i++) {
        int __diff = (int)__ra[__i] - (int)__rb[__i];
        if (__diff > 127) __diff = 127;
        if (__diff < -128) __diff = -128;
        __rr[__i] = (signed char)__diff;
    }
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

static __inline__ __m64 __attribute__((__always_inline__))
_mm_subs_pi16(__m64 __a, __m64 __b)
{
    __m64 __r;
    short *__ra = (short *)&__a.__val;
    short *__rb = (short *)&__b.__val;
    short __rr[4];
    for (int __i = 0; __i < 4; __i++) {
        int __diff = (int)__ra[__i] - (int)__rb[__i];
        if (__diff > 32767) __diff = 32767;
        if (__diff < -32768) __diff = -32768;
        __rr[__i] = (short)__diff;
    }
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

static __inline__ __m64 __attribute__((__always_inline__))
_mm_subs_pu8(__m64 __a, __m64 __b)
{
    __m64 __r;
    unsigned char *__ra = (unsigned char *)&__a.__val;
    unsigned char *__rb = (unsigned char *)&__b.__val;
    unsigned char __rr[8];
    for (int __i = 0; __i < 8; __i++) {
        int __diff = (int)__ra[__i] - (int)__rb[__i];
        if (__diff < 0) __diff = 0;
        __rr[__i] = (unsigned char)__diff;
    }
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

static __inline__ __m64 __attribute__((__always_inline__))
_mm_subs_pu16(__m64 __a, __m64 __b)
{
    __m64 __r;
    unsigned short *__ra = (unsigned short *)&__a.__val;
    unsigned short *__rb = (unsigned short *)&__b.__val;
    unsigned short __rr[4];
    for (int __i = 0; __i < 4; __i++) {
        int __diff = (int)__ra[__i] - (int)__rb[__i];
        if (__diff < 0) __diff = 0;
        __rr[__i] = (unsigned short)__diff;
    }
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

/* === Packed Multiply === */

/* Multiply packed 16-bit signed integers, return low 16 bits of each 32-bit result */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_mullo_pi16(__m64 __a, __m64 __b)
{
    __m64 __r;
    short *__ra = (short *)&__a.__val;
    short *__rb = (short *)&__b.__val;
    short __rr[4];
    for (int __i = 0; __i < 4; __i++)
        __rr[__i] = (short)((int)__ra[__i] * (int)__rb[__i]);
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

/* Multiply packed 16-bit signed integers, return high 16 bits of each 32-bit result */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_mulhi_pi16(__m64 __a, __m64 __b)
{
    __m64 __r;
    short *__ra = (short *)&__a.__val;
    short *__rb = (short *)&__b.__val;
    short __rr[4];
    for (int __i = 0; __i < 4; __i++)
        __rr[__i] = (short)(((int)__ra[__i] * (int)__rb[__i]) >> 16);
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

/* Multiply and add: (a0*b0 + a1*b1), (a2*b2 + a3*b3) */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_madd_pi16(__m64 __a, __m64 __b)
{
    __m64 __r;
    short *__ra = (short *)&__a.__val;
    short *__rb = (short *)&__b.__val;
    int __rr[2];
    __rr[0] = (int)__ra[0] * (int)__rb[0] + (int)__ra[1] * (int)__rb[1];
    __rr[1] = (int)__ra[2] * (int)__rb[2] + (int)__ra[3] * (int)__rb[3];
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

/* === Shift === */

/* Shift left logical: 16-bit packed */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_slli_pi16(__m64 __a, int __count)
{
    __m64 __r;
    if (__count < 0 || __count > 15) {
        __r.__val = 0;
        return __r;
    }
    unsigned short *__ra = (unsigned short *)&__a.__val;
    unsigned short __rr[4];
    for (int __i = 0; __i < 4; __i++)
        __rr[__i] = __ra[__i] << __count;
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

/* Shift left logical: 32-bit packed */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_slli_pi32(__m64 __a, int __count)
{
    __m64 __r;
    if (__count < 0 || __count > 31) {
        __r.__val = 0;
        return __r;
    }
    unsigned int *__ra = (unsigned int *)&__a.__val;
    unsigned int __rr[2];
    for (int __i = 0; __i < 2; __i++)
        __rr[__i] = __ra[__i] << __count;
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

/* Shift left logical: 64-bit */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_slli_si64(__m64 __a, int __count)
{
    __m64 __r;
    if (__count < 0 || __count > 63) {
        __r.__val = 0;
        return __r;
    }
    __r.__val = (long long)((unsigned long long)__a.__val << __count);
    return __r;
}

/* Shift right logical: 16-bit packed */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_srli_pi16(__m64 __a, int __count)
{
    __m64 __r;
    if (__count < 0 || __count > 15) {
        __r.__val = 0;
        return __r;
    }
    unsigned short *__ra = (unsigned short *)&__a.__val;
    unsigned short __rr[4];
    for (int __i = 0; __i < 4; __i++)
        __rr[__i] = __ra[__i] >> __count;
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

/* Shift right logical: 32-bit packed */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_srli_pi32(__m64 __a, int __count)
{
    __m64 __r;
    if (__count < 0 || __count > 31) {
        __r.__val = 0;
        return __r;
    }
    unsigned int *__ra = (unsigned int *)&__a.__val;
    unsigned int __rr[2];
    for (int __i = 0; __i < 2; __i++)
        __rr[__i] = __ra[__i] >> __count;
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

/* Shift right logical: 64-bit */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_srli_si64(__m64 __a, int __count)
{
    __m64 __r;
    if (__count < 0 || __count > 63) {
        __r.__val = 0;
        return __r;
    }
    __r.__val = (long long)((unsigned long long)__a.__val >> __count);
    return __r;
}

/* Shift right arithmetic: 16-bit packed */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_srai_pi16(__m64 __a, int __count)
{
    __m64 __r;
    if (__count > 15) __count = 15;
    if (__count < 0) __count = 0;
    short *__ra = (short *)&__a.__val;
    short __rr[4];
    for (int __i = 0; __i < 4; __i++)
        __rr[__i] = __ra[__i] >> __count;
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

/* Shift right arithmetic: 32-bit packed */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_srai_pi32(__m64 __a, int __count)
{
    __m64 __r;
    if (__count > 31) __count = 31;
    if (__count < 0) __count = 0;
    int *__ra = (int *)&__a.__val;
    int __rr[2];
    for (int __i = 0; __i < 2; __i++)
        __rr[__i] = __ra[__i] >> __count;
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

/* === Pack === */

/* Pack 32-bit signed integers to 16-bit signed integers with saturation.
 * _mm_packs_pi32(a, b): a contains low 2 words, b contains high 2 words.
 * Result: [sat16(a0), sat16(a1), sat16(b0), sat16(b1)] */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_packs_pi32(__m64 __a, __m64 __b)
{
    __m64 __r;
    int *__ra = (int *)&__a.__val;
    int *__rb = (int *)&__b.__val;
    short __rr[4];
    for (int __i = 0; __i < 2; __i++) {
        int __v = __ra[__i];
        if (__v > 32767) __v = 32767;
        if (__v < -32768) __v = -32768;
        __rr[__i] = (short)__v;
    }
    for (int __i = 0; __i < 2; __i++) {
        int __v = __rb[__i];
        if (__v > 32767) __v = 32767;
        if (__v < -32768) __v = -32768;
        __rr[__i + 2] = (short)__v;
    }
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

/* Pack 16-bit signed integers to 8-bit signed integers with signed saturation.
 * _mm_packs_pi16(a, b): pack a's 4 words and b's 4 words into 8 signed bytes. */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_packs_pi16(__m64 __a, __m64 __b)
{
    __m64 __r;
    short *__ra = (short *)&__a.__val;
    short *__rb = (short *)&__b.__val;
    signed char __rr[8];
    for (int __i = 0; __i < 4; __i++) {
        int __v = __ra[__i];
        if (__v > 127) __v = 127;
        if (__v < -128) __v = -128;
        __rr[__i] = (signed char)__v;
    }
    for (int __i = 0; __i < 4; __i++) {
        int __v = __rb[__i];
        if (__v > 127) __v = 127;
        if (__v < -128) __v = -128;
        __rr[__i + 4] = (signed char)__v;
    }
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

/* Pack 16-bit signed integers to 8-bit unsigned integers with unsigned saturation.
 * _mm_packs_pu16(a, b): pack a's 4 words and b's 4 words into 8 unsigned bytes. */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_packs_pu16(__m64 __a, __m64 __b)
{
    __m64 __r;
    short *__ra = (short *)&__a.__val;
    short *__rb = (short *)&__b.__val;
    unsigned char __rr[8];
    for (int __i = 0; __i < 4; __i++) {
        int __v = __ra[__i];
        if (__v > 255) __v = 255;
        if (__v < 0) __v = 0;
        __rr[__i] = (unsigned char)__v;
    }
    for (int __i = 0; __i < 4; __i++) {
        int __v = __rb[__i];
        if (__v > 255) __v = 255;
        if (__v < 0) __v = 0;
        __rr[__i + 4] = (unsigned char)__v;
    }
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

/* === Compare === */

static __inline__ __m64 __attribute__((__always_inline__))
_mm_cmpeq_pi8(__m64 __a, __m64 __b)
{
    __m64 __r;
    unsigned char *__ra = (unsigned char *)&__a.__val;
    unsigned char *__rb = (unsigned char *)&__b.__val;
    unsigned char __rr[8];
    for (int __i = 0; __i < 8; __i++)
        __rr[__i] = (__ra[__i] == __rb[__i]) ? 0xFF : 0x00;
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

static __inline__ __m64 __attribute__((__always_inline__))
_mm_cmpeq_pi16(__m64 __a, __m64 __b)
{
    __m64 __r;
    unsigned short *__ra = (unsigned short *)&__a.__val;
    unsigned short *__rb = (unsigned short *)&__b.__val;
    unsigned short __rr[4];
    for (int __i = 0; __i < 4; __i++)
        __rr[__i] = (__ra[__i] == __rb[__i]) ? 0xFFFF : 0x0000;
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

static __inline__ __m64 __attribute__((__always_inline__))
_mm_cmpeq_pi32(__m64 __a, __m64 __b)
{
    __m64 __r;
    unsigned int *__ra = (unsigned int *)&__a.__val;
    unsigned int *__rb = (unsigned int *)&__b.__val;
    unsigned int __rr[2];
    for (int __i = 0; __i < 2; __i++)
        __rr[__i] = (__ra[__i] == __rb[__i]) ? 0xFFFFFFFFU : 0x00000000U;
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

static __inline__ __m64 __attribute__((__always_inline__))
_mm_cmpgt_pi8(__m64 __a, __m64 __b)
{
    __m64 __r;
    signed char *__ra = (signed char *)&__a.__val;
    signed char *__rb = (signed char *)&__b.__val;
    unsigned char __rr[8];
    for (int __i = 0; __i < 8; __i++)
        __rr[__i] = (__ra[__i] > __rb[__i]) ? 0xFF : 0x00;
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

static __inline__ __m64 __attribute__((__always_inline__))
_mm_cmpgt_pi16(__m64 __a, __m64 __b)
{
    __m64 __r;
    short *__ra = (short *)&__a.__val;
    short *__rb = (short *)&__b.__val;
    unsigned short __rr[4];
    for (int __i = 0; __i < 4; __i++)
        __rr[__i] = (__ra[__i] > __rb[__i]) ? 0xFFFF : 0x0000;
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

static __inline__ __m64 __attribute__((__always_inline__))
_mm_cmpgt_pi32(__m64 __a, __m64 __b)
{
    __m64 __r;
    int *__ra = (int *)&__a.__val;
    int *__rb = (int *)&__b.__val;
    unsigned int __rr[2];
    for (int __i = 0; __i < 2; __i++)
        __rr[__i] = (__ra[__i] > __rb[__i]) ? 0xFFFFFFFFU : 0x00000000U;
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

/* === Unpack / Interleave === */

/* Interleave low bytes: result = [a0, b0, a1, b1, a2, b2, a3, b3] */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_unpacklo_pi8(__m64 __a, __m64 __b)
{
    __m64 __r;
    unsigned char *__ra = (unsigned char *)&__a.__val;
    unsigned char *__rb = (unsigned char *)&__b.__val;
    unsigned char __rr[8];
    __rr[0] = __ra[0]; __rr[1] = __rb[0];
    __rr[2] = __ra[1]; __rr[3] = __rb[1];
    __rr[4] = __ra[2]; __rr[5] = __rb[2];
    __rr[6] = __ra[3]; __rr[7] = __rb[3];
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

/* Interleave high bytes: result = [a4, b4, a5, b5, a6, b6, a7, b7] */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_unpackhi_pi8(__m64 __a, __m64 __b)
{
    __m64 __r;
    unsigned char *__ra = (unsigned char *)&__a.__val;
    unsigned char *__rb = (unsigned char *)&__b.__val;
    unsigned char __rr[8];
    __rr[0] = __ra[4]; __rr[1] = __rb[4];
    __rr[2] = __ra[5]; __rr[3] = __rb[5];
    __rr[4] = __ra[6]; __rr[5] = __rb[6];
    __rr[6] = __ra[7]; __rr[7] = __rb[7];
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

/* Interleave low words: result = [a0, b0, a1, b1] */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_unpacklo_pi16(__m64 __a, __m64 __b)
{
    __m64 __r;
    unsigned short *__ra = (unsigned short *)&__a.__val;
    unsigned short *__rb = (unsigned short *)&__b.__val;
    unsigned short __rr[4];
    __rr[0] = __ra[0]; __rr[1] = __rb[0];
    __rr[2] = __ra[1]; __rr[3] = __rb[1];
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

/* Interleave high words: result = [a2, b2, a3, b3] */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_unpackhi_pi16(__m64 __a, __m64 __b)
{
    __m64 __r;
    unsigned short *__ra = (unsigned short *)&__a.__val;
    unsigned short *__rb = (unsigned short *)&__b.__val;
    unsigned short __rr[4];
    __rr[0] = __ra[2]; __rr[1] = __rb[2];
    __rr[2] = __ra[3]; __rr[3] = __rb[3];
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

/* Interleave low dwords: result = [a0, b0] */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_unpacklo_pi32(__m64 __a, __m64 __b)
{
    __m64 __r;
    unsigned int *__ra = (unsigned int *)&__a.__val;
    unsigned int *__rb = (unsigned int *)&__b.__val;
    unsigned int __rr[2];
    __rr[0] = __ra[0]; __rr[1] = __rb[0];
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

/* Interleave high dwords: result = [a1, b1] */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_unpackhi_pi32(__m64 __a, __m64 __b)
{
    __m64 __r;
    unsigned int *__ra = (unsigned int *)&__a.__val;
    unsigned int *__rb = (unsigned int *)&__b.__val;
    unsigned int __rr[2];
    __rr[0] = __ra[1]; __rr[1] = __rb[1];
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

/* === 64-bit conversion === */

static __inline__ __m64 __attribute__((__always_inline__))
_mm_cvtsi64_m64(long long __a)
{
    __m64 __r;
    __r.__val = __a;
    return __r;
}

static __inline__ long long __attribute__((__always_inline__))
_mm_cvtm64_si64(__m64 __a)
{
    return __a.__val;
}

/* _m_from_int / _m_to_int aliases */
#define _m_from_int(a) _mm_cvtsi32_si64(a)
#define _m_to_int(a) _mm_cvtsi64_si32(a)
#define _m_from_int64(a) _mm_cvtsi64_m64(a)
#define _m_to_int64(a) _mm_cvtm64_si64(a)

/* _m_ prefix aliases (alternate names) */
#define _m_paddb _mm_add_pi8
#define _m_paddw _mm_add_pi16
#define _m_paddd _mm_add_pi32
#define _m_psubb _mm_sub_pi8
#define _m_psubw _mm_sub_pi16
#define _m_psubd _mm_sub_pi32
#define _m_paddsb _mm_adds_pi8
#define _m_paddsw _mm_adds_pi16
#define _m_paddusb _mm_adds_pu8
#define _m_paddusw _mm_adds_pu16
#define _m_psubsb _mm_subs_pi8
#define _m_psubsw _mm_subs_pi16
#define _m_psubusb _mm_subs_pu8
#define _m_psubusw _mm_subs_pu16
#define _m_pmullw _mm_mullo_pi16
#define _m_pmulhw _mm_mulhi_pi16
#define _m_pmaddwd _mm_madd_pi16
#define _m_pand _mm_and_si64
#define _m_pandn _mm_andnot_si64
#define _m_por _mm_or_si64
#define _m_pxor _mm_xor_si64
#define _m_psllwi _mm_slli_pi16
#define _m_pslldi _mm_slli_pi32
#define _m_psllqi _mm_slli_si64
#define _m_psrlwi _mm_srli_pi16
#define _m_psrldi _mm_srli_pi32
#define _m_psrlqi _mm_srli_si64
#define _m_psrawi _mm_srai_pi16
#define _m_psradi _mm_srai_pi32
#define _m_pcmpeqb _mm_cmpeq_pi8
#define _m_pcmpeqw _mm_cmpeq_pi16
#define _m_pcmpeqd _mm_cmpeq_pi32
#define _m_pcmpgtb _mm_cmpgt_pi8
#define _m_pcmpgtw _mm_cmpgt_pi16
#define _m_pcmpgtd _mm_cmpgt_pi32
#define _m_packsswb _mm_packs_pi16
#define _m_packssdw _mm_packs_pi32
#define _m_packuswb _mm_packs_pu16
#define _m_punpcklbw _mm_unpacklo_pi8
#define _m_punpcklwd _mm_unpacklo_pi16
#define _m_punpckldq _mm_unpacklo_pi32
#define _m_punpckhbw _mm_unpackhi_pi8
#define _m_punpckhwd _mm_unpackhi_pi16
#define _m_punpckhdq _mm_unpackhi_pi32
#define _m_empty _mm_empty

#endif /* _MMINTRIN_H_INCLUDED */
