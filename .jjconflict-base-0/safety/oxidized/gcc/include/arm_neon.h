/* CCC compiler bundled arm_neon.h - ARM NEON intrinsics */
#ifndef _ARM_NEON_H_INCLUDED
#define _ARM_NEON_H_INCLUDED

/* ===== Scalar types (ACLE) ===== */
typedef float float32_t;
typedef double float64_t;
typedef unsigned long long poly64_t;
typedef __uint128_t poly128_t;

/* ===== 128-bit vector types (Q registers) ===== */

typedef struct __attribute__((__aligned__(16))) {
    unsigned char __val[16];
} uint8x16_t;

typedef struct __attribute__((__aligned__(16))) {
    signed char __val[16];
} int8x16_t;

typedef struct __attribute__((__aligned__(16))) {
    unsigned short __val[8];
} uint16x8_t;

typedef struct __attribute__((__aligned__(16))) {
    short __val[8];
} int16x8_t;

typedef struct __attribute__((__aligned__(16))) {
    unsigned int __val[4];
} uint32x4_t;

typedef struct __attribute__((__aligned__(16))) {
    int __val[4];
} int32x4_t;

typedef struct __attribute__((__aligned__(16))) {
    unsigned long long __val[2];
} uint64x2_t;

typedef struct __attribute__((__aligned__(16))) {
    long long __val[2];
} int64x2_t;

typedef struct __attribute__((__aligned__(16))) {
    float __val[4];
} float32x4_t;

typedef struct __attribute__((__aligned__(16))) {
    double __val[2];
} float64x2_t;

/* Polynomial 128-bit types */
typedef struct __attribute__((__aligned__(16))) {
    unsigned char __val[16];
} poly8x16_t;

typedef struct __attribute__((__aligned__(16))) {
    unsigned short __val[8];
} poly16x8_t;

typedef struct __attribute__((__aligned__(16))) {
    unsigned long long __val[2];
} poly64x2_t;

/* ===== 64-bit vector types (D registers) ===== */

typedef struct __attribute__((__aligned__(8))) {
    unsigned char __val[8];
} uint8x8_t;

typedef struct __attribute__((__aligned__(8))) {
    signed char __val[8];
} int8x8_t;

typedef struct __attribute__((__aligned__(8))) {
    unsigned short __val[4];
} uint16x4_t;

typedef struct __attribute__((__aligned__(8))) {
    short __val[4];
} int16x4_t;

typedef struct __attribute__((__aligned__(8))) {
    unsigned int __val[2];
} uint32x2_t;

typedef struct __attribute__((__aligned__(8))) {
    int __val[2];
} int32x2_t;

typedef struct __attribute__((__aligned__(8))) {
    unsigned long long __val[1];
} uint64x1_t;

typedef struct __attribute__((__aligned__(8))) {
    long long __val[1];
} int64x1_t;

typedef struct __attribute__((__aligned__(8))) {
    float __val[2];
} float32x2_t;

typedef struct __attribute__((__aligned__(8))) {
    unsigned char __val[8];
} poly8x8_t;

typedef struct __attribute__((__aligned__(8))) {
    unsigned long long __val[1];
} poly64x1_t;

/* ===== Array-of-vectors (structure) types ===== */

/* uint8x8 multi-vector */
typedef struct { uint8x8_t val[2]; } uint8x8x2_t;
typedef struct { uint8x8_t val[3]; } uint8x8x3_t;
typedef struct { uint8x8_t val[4]; } uint8x8x4_t;

/* uint8x16 multi-vector */
typedef struct { uint8x16_t val[2]; } uint8x16x2_t;
typedef struct { uint8x16_t val[3]; } uint8x16x3_t;
typedef struct { uint8x16_t val[4]; } uint8x16x4_t;

/* uint16x4 multi-vector */
typedef struct { uint16x4_t val[2]; } uint16x4x2_t;
typedef struct { uint16x4_t val[4]; } uint16x4x4_t;

/* uint16x8 multi-vector */
typedef struct { uint16x8_t val[2]; } uint16x8x2_t;
typedef struct { uint16x8_t val[4]; } uint16x8x4_t;

/* uint32x2 multi-vector */
typedef struct { uint32x2_t val[2]; } uint32x2x2_t;
typedef struct { uint32x2_t val[4]; } uint32x2x4_t;

/* uint32x4 multi-vector */
typedef struct { uint32x4_t val[2]; } uint32x4x2_t;
typedef struct { uint32x4_t val[4]; } uint32x4x4_t;

/* int8x8 multi-vector */
typedef struct { int8x8_t val[2]; } int8x8x2_t;

/* ================================================================== */
/*                          LOAD INTRINSICS                           */
/* ================================================================== */

/* --- vld1q: load one 128-bit vector --- */

static __inline__ uint8x16_t __attribute__((__always_inline__))
vld1q_u8(const unsigned char *__p)
{
    uint8x16_t __ret;
    __builtin_memcpy(&__ret, __p, 16);
    return __ret;
}

static __inline__ int8x16_t __attribute__((__always_inline__))
vld1q_s8(const signed char *__p)
{
    int8x16_t __ret;
    __builtin_memcpy(&__ret, __p, 16);
    return __ret;
}

static __inline__ uint16x8_t __attribute__((__always_inline__))
vld1q_u16(const unsigned short *__p)
{
    uint16x8_t __ret;
    __builtin_memcpy(&__ret, __p, 16);
    return __ret;
}

static __inline__ uint32x4_t __attribute__((__always_inline__))
vld1q_u32(const unsigned int *__p)
{
    uint32x4_t __ret;
    __builtin_memcpy(&__ret, __p, 16);
    return __ret;
}

static __inline__ uint64x2_t __attribute__((__always_inline__))
vld1q_u64(const unsigned long long *__p)
{
    uint64x2_t __ret;
    __builtin_memcpy(&__ret, __p, 16);
    return __ret;
}

/* --- vld1q_dup: load and broadcast --- */

static __inline__ uint32x4_t __attribute__((__always_inline__))
vld1q_dup_u32(const unsigned int *__p)
{
    uint32x4_t __ret;
    unsigned int __v = *__p;
    __ret.__val[0] = __v;
    __ret.__val[1] = __v;
    __ret.__val[2] = __v;
    __ret.__val[3] = __v;
    return __ret;
}

/* --- vld1q_lane: load one lane --- */

static __inline__ uint32x4_t __attribute__((__always_inline__))
vld1q_lane_u32(const unsigned int *__p, uint32x4_t __v, int __lane)
{
    __v.__val[__lane] = *__p;
    return __v;
}

/* --- vld1q multi-load --- */

static __inline__ uint8x16x4_t __attribute__((__always_inline__))
vld1q_u8_x4(const unsigned char *__p)
{
    uint8x16x4_t __ret;
    __builtin_memcpy(&__ret, __p, 64);
    return __ret;
}

/* --- vld1: load one 64-bit vector --- */

static __inline__ uint8x8_t __attribute__((__always_inline__))
vld1_u8(const unsigned char *__p)
{
    uint8x8_t __ret;
    __builtin_memcpy(&__ret, __p, 8);
    return __ret;
}

/* --- Structure loads (de-interleave) --- */

static __inline__ uint8x16x3_t __attribute__((__always_inline__))
vld3q_u8(const unsigned char *__p)
{
    /* De-interleave 48 bytes into 3 vectors of 16 bytes each */
    uint8x16x3_t __ret;
    for (int __i = 0; __i < 16; __i++) {
        __ret.val[0].__val[__i] = __p[__i * 3 + 0];
        __ret.val[1].__val[__i] = __p[__i * 3 + 1];
        __ret.val[2].__val[__i] = __p[__i * 3 + 2];
    }
    return __ret;
}

static __inline__ uint8x8x3_t __attribute__((__always_inline__))
vld3_dup_u8(const unsigned char *__p)
{
    /* Load 3 bytes and duplicate each into all lanes */
    uint8x8x3_t __ret;
    for (int __i = 0; __i < 8; __i++) {
        __ret.val[0].__val[__i] = __p[0];
        __ret.val[1].__val[__i] = __p[1];
        __ret.val[2].__val[__i] = __p[2];
    }
    return __ret;
}

static __inline__ uint8x8x3_t __attribute__((__always_inline__))
vld3_lane_u8(const unsigned char *__p, uint8x8x3_t __v, int __lane)
{
    __v.val[0].__val[__lane] = __p[0];
    __v.val[1].__val[__lane] = __p[1];
    __v.val[2].__val[__lane] = __p[2];
    return __v;
}

static __inline__ uint32x2x4_t __attribute__((__always_inline__))
vld4_u32(const unsigned int *__p)
{
    /* De-interleave 8 uint32s into 4 vectors of 2 */
    uint32x2x4_t __ret;
    for (int __i = 0; __i < 2; __i++) {
        __ret.val[0].__val[__i] = __p[__i * 4 + 0];
        __ret.val[1].__val[__i] = __p[__i * 4 + 1];
        __ret.val[2].__val[__i] = __p[__i * 4 + 2];
        __ret.val[3].__val[__i] = __p[__i * 4 + 3];
    }
    return __ret;
}

/* ================================================================== */
/*                         STORE INTRINSICS                           */
/* ================================================================== */

static __inline__ void __attribute__((__always_inline__))
vst1q_u8(unsigned char *__p, uint8x16_t __a)
{
    __builtin_memcpy(__p, &__a, 16);
}

static __inline__ void __attribute__((__always_inline__))
vst1q_s8(signed char *__p, int8x16_t __a)
{
    __builtin_memcpy(__p, &__a, 16);
}

static __inline__ void __attribute__((__always_inline__))
vst1q_u16(unsigned short *__p, uint16x8_t __a)
{
    __builtin_memcpy(__p, &__a, 16);
}

static __inline__ void __attribute__((__always_inline__))
vst1q_u32(unsigned int *__p, uint32x4_t __a)
{
    __builtin_memcpy(__p, &__a, 16);
}

static __inline__ void __attribute__((__always_inline__))
vst1q_u64(unsigned long long *__p, uint64x2_t __a)
{
    __builtin_memcpy(__p, &__a, 16);
}

/* --- vst1_lane: store one lane --- */

static __inline__ void __attribute__((__always_inline__))
vst1_lane_u32(unsigned int *__p, uint32x2_t __a, int __lane)
{
    *__p = __a.__val[__lane];
}

/* --- Structure stores (interleave) --- */

static __inline__ void __attribute__((__always_inline__))
vst3_u8(unsigned char *__p, uint8x8x3_t __a)
{
    for (int __i = 0; __i < 8; __i++) {
        __p[__i * 3 + 0] = __a.val[0].__val[__i];
        __p[__i * 3 + 1] = __a.val[1].__val[__i];
        __p[__i * 3 + 2] = __a.val[2].__val[__i];
    }
}

static __inline__ void __attribute__((__always_inline__))
vst4q_u8(unsigned char *__p, uint8x16x4_t __a)
{
    for (int __i = 0; __i < 16; __i++) {
        __p[__i * 4 + 0] = __a.val[0].__val[__i];
        __p[__i * 4 + 1] = __a.val[1].__val[__i];
        __p[__i * 4 + 2] = __a.val[2].__val[__i];
        __p[__i * 4 + 3] = __a.val[3].__val[__i];
    }
}

static __inline__ void __attribute__((__always_inline__))
vst4_lane_u32(unsigned int *__p, uint32x2x4_t __a, int __lane)
{
    __p[0] = __a.val[0].__val[__lane];
    __p[1] = __a.val[1].__val[__lane];
    __p[2] = __a.val[2].__val[__lane];
    __p[3] = __a.val[3].__val[__lane];
}

/* ================================================================== */
/*                       BITWISE OPERATIONS                           */
/* ================================================================== */

static __inline__ uint8x16_t __attribute__((__always_inline__))
veorq_u8(uint8x16_t __a, uint8x16_t __b)
{
    uint8x16_t __ret;
    for (int __i = 0; __i < 16; __i++)
        __ret.__val[__i] = __a.__val[__i] ^ __b.__val[__i];
    return __ret;
}

static __inline__ uint64x2_t __attribute__((__always_inline__))
veorq_u64(uint64x2_t __a, uint64x2_t __b)
{
    uint64x2_t __ret;
    __ret.__val[0] = __a.__val[0] ^ __b.__val[0];
    __ret.__val[1] = __a.__val[1] ^ __b.__val[1];
    return __ret;
}

static __inline__ uint8x16_t __attribute__((__always_inline__))
vandq_u8(uint8x16_t __a, uint8x16_t __b)
{
    uint8x16_t __ret;
    for (int __i = 0; __i < 16; __i++)
        __ret.__val[__i] = __a.__val[__i] & __b.__val[__i];
    return __ret;
}

static __inline__ uint16x8_t __attribute__((__always_inline__))
vandq_u16(uint16x8_t __a, uint16x8_t __b)
{
    uint16x8_t __ret;
    for (int __i = 0; __i < 8; __i++)
        __ret.__val[__i] = __a.__val[__i] & __b.__val[__i];
    return __ret;
}

static __inline__ uint8x16_t __attribute__((__always_inline__))
vorrq_u8(uint8x16_t __a, uint8x16_t __b)
{
    uint8x16_t __ret;
    for (int __i = 0; __i < 16; __i++)
        __ret.__val[__i] = __a.__val[__i] | __b.__val[__i];
    return __ret;
}

static __inline__ uint8x16_t __attribute__((__always_inline__))
vmvnq_u8(uint8x16_t __a)
{
    uint8x16_t __ret;
    for (int __i = 0; __i < 16; __i++)
        __ret.__val[__i] = ~__a.__val[__i];
    return __ret;
}

static __inline__ uint8x16_t __attribute__((__always_inline__))
vbicq_u8(uint8x16_t __a, uint8x16_t __b)
{
    /* Bit clear: a & ~b */
    uint8x16_t __ret;
    for (int __i = 0; __i < 16; __i++)
        __ret.__val[__i] = __a.__val[__i] & ~__b.__val[__i];
    return __ret;
}

/* --- 64-bit bitwise --- */

static __inline__ uint8x8_t __attribute__((__always_inline__))
vbsl_u8(uint8x8_t __sel, uint8x8_t __a, uint8x8_t __b)
{
    /* Bitwise select: (sel & a) | (~sel & b) */
    uint8x8_t __ret;
    for (int __i = 0; __i < 8; __i++)
        __ret.__val[__i] = (__sel.__val[__i] & __a.__val[__i]) |
                           (~__sel.__val[__i] & __b.__val[__i]);
    return __ret;
}

/* ================================================================== */
/*                       SHIFT OPERATIONS                             */
/* ================================================================== */

static __inline__ uint8x16_t __attribute__((__always_inline__))
vshlq_n_u8(uint8x16_t __a, int __n)
{
    uint8x16_t __ret;
    for (int __i = 0; __i < 16; __i++)
        __ret.__val[__i] = (unsigned char)(__a.__val[__i] << __n);
    return __ret;
}

static __inline__ uint8x16_t __attribute__((__always_inline__))
vshrq_n_u8(uint8x16_t __a, int __n)
{
    uint8x16_t __ret;
    for (int __i = 0; __i < 16; __i++)
        __ret.__val[__i] = __a.__val[__i] >> __n;
    return __ret;
}

static __inline__ int8x16_t __attribute__((__always_inline__))
vshrq_n_s8(int8x16_t __a, int __n)
{
    int8x16_t __ret;
    for (int __i = 0; __i < 16; __i++)
        __ret.__val[__i] = __a.__val[__i] >> __n;
    return __ret;
}

/* vshlq_n_u64: shift left uint64x2_t by immediate */
static __inline__ uint64x2_t __attribute__((__always_inline__))
vshlq_n_u64(uint64x2_t __a, int __n)
{
    uint64x2_t __ret;
    __ret.__val[0] = __a.__val[0] << __n;
    __ret.__val[1] = __a.__val[1] << __n;
    return __ret;
}

static __inline__ uint64x2_t __attribute__((__always_inline__))
vshrq_n_u64(uint64x2_t __a, int __n)
{
    uint64x2_t __ret;
    __ret.__val[0] = __a.__val[0] >> __n;
    __ret.__val[1] = __a.__val[1] >> __n;
    return __ret;
}

/* ================================================================== */
/*                    DUPLICATE (BROADCAST)                            */
/* ================================================================== */

static __inline__ uint8x16_t __attribute__((__always_inline__))
vdupq_n_u8(unsigned char __a)
{
    uint8x16_t __ret;
    for (int __i = 0; __i < 16; __i++)
        __ret.__val[__i] = __a;
    return __ret;
}

static __inline__ int8x16_t __attribute__((__always_inline__))
vdupq_n_s8(signed char __a)
{
    int8x16_t __ret;
    for (int __i = 0; __i < 16; __i++)
        __ret.__val[__i] = __a;
    return __ret;
}

static __inline__ uint16x8_t __attribute__((__always_inline__))
vdupq_n_u16(unsigned short __a)
{
    uint16x8_t __ret;
    for (int __i = 0; __i < 8; __i++)
        __ret.__val[__i] = __a;
    return __ret;
}

static __inline__ uint32x4_t __attribute__((__always_inline__))
vdupq_n_u32(unsigned int __a)
{
    uint32x4_t __ret;
    for (int __i = 0; __i < 4; __i++)
        __ret.__val[__i] = __a;
    return __ret;
}

static __inline__ uint8x8_t __attribute__((__always_inline__))
vdup_n_u8(unsigned char __a)
{
    uint8x8_t __ret;
    for (int __i = 0; __i < 8; __i++)
        __ret.__val[__i] = __a;
    return __ret;
}

static __inline__ uint64x1_t __attribute__((__always_inline__))
vmov_n_u64(unsigned long long __a)
{
    uint64x1_t __ret;
    __ret.__val[0] = __a;
    return __ret;
}

/* ================================================================== */
/*                    COMBINE / SPLIT                                  */
/* ================================================================== */

static __inline__ uint8x16_t __attribute__((__always_inline__))
vcombine_u8(uint8x8_t __lo, uint8x8_t __hi)
{
    uint8x16_t __ret;
    __builtin_memcpy(&__ret.__val[0], &__lo.__val[0], 8);
    __builtin_memcpy(&__ret.__val[8], &__hi.__val[0], 8);
    return __ret;
}

static __inline__ int8x16_t __attribute__((__always_inline__))
vcombine_s8(int8x8_t __lo, int8x8_t __hi)
{
    int8x16_t __ret;
    __builtin_memcpy(&__ret.__val[0], &__lo.__val[0], 8);
    __builtin_memcpy(&__ret.__val[8], &__hi.__val[0], 8);
    return __ret;
}

static __inline__ uint64x2_t __attribute__((__always_inline__))
vcombine_u64(uint64x1_t __lo, uint64x1_t __hi)
{
    uint64x2_t __ret;
    __ret.__val[0] = __lo.__val[0];
    __ret.__val[1] = __hi.__val[0];
    return __ret;
}

static __inline__ uint16x8_t __attribute__((__always_inline__))
vcombine_u16(uint16x4_t __lo, uint16x4_t __hi)
{
    uint16x8_t __ret;
    __builtin_memcpy(&__ret.__val[0], &__lo.__val[0], 8);
    __builtin_memcpy(&__ret.__val[4], &__hi.__val[0], 8);
    return __ret;
}

/* --- Get high/low halves --- */

static __inline__ uint8x8_t __attribute__((__always_inline__))
vget_low_u8(uint8x16_t __a)
{
    uint8x8_t __ret;
    __builtin_memcpy(&__ret.__val[0], &__a.__val[0], 8);
    return __ret;
}

static __inline__ uint8x8_t __attribute__((__always_inline__))
vget_high_u8(uint8x16_t __a)
{
    uint8x8_t __ret;
    __builtin_memcpy(&__ret.__val[0], &__a.__val[8], 8);
    return __ret;
}

static __inline__ int8x8_t __attribute__((__always_inline__))
vget_low_s8(int8x16_t __a)
{
    int8x8_t __ret;
    __builtin_memcpy(&__ret.__val[0], &__a.__val[0], 8);
    return __ret;
}

static __inline__ int8x8_t __attribute__((__always_inline__))
vget_high_s8(int8x16_t __a)
{
    int8x8_t __ret;
    __builtin_memcpy(&__ret.__val[0], &__a.__val[8], 8);
    return __ret;
}

static __inline__ uint64x1_t __attribute__((__always_inline__))
vget_low_u64(uint64x2_t __a)
{
    uint64x1_t __ret;
    __ret.__val[0] = __a.__val[0];
    return __ret;
}

static __inline__ uint64x1_t __attribute__((__always_inline__))
vget_high_u64(uint64x2_t __a)
{
    uint64x1_t __ret;
    __ret.__val[0] = __a.__val[1];
    return __ret;
}

static __inline__ poly64x1_t __attribute__((__always_inline__))
vget_low_p64(poly64x2_t __a)
{
    poly64x1_t __ret;
    __ret.__val[0] = __a.__val[0];
    return __ret;
}

/* ================================================================== */
/*                       LANE ACCESS                                   */
/* ================================================================== */

static __inline__ signed char __attribute__((__always_inline__))
vget_lane_s8(int8x8_t __a, int __lane)
{
    return __a.__val[__lane];
}

static __inline__ unsigned char __attribute__((__always_inline__))
vget_lane_u8(uint8x8_t __a, int __lane)
{
    return __a.__val[__lane];
}

static __inline__ unsigned short __attribute__((__always_inline__))
vget_lane_u16(uint16x4_t __a, int __lane)
{
    return __a.__val[__lane];
}

static __inline__ short __attribute__((__always_inline__))
vget_lane_s16(int16x4_t __a, int __lane)
{
    return __a.__val[__lane];
}

static __inline__ unsigned int __attribute__((__always_inline__))
vget_lane_u32(uint32x2_t __a, int __lane)
{
    return __a.__val[__lane];
}

static __inline__ int __attribute__((__always_inline__))
vget_lane_s32(int32x2_t __a, int __lane)
{
    return __a.__val[__lane];
}

static __inline__ float __attribute__((__always_inline__))
vget_lane_f32(float32x2_t __a, int __lane)
{
    return __a.__val[__lane];
}

static __inline__ unsigned char __attribute__((__always_inline__))
vgetq_lane_u8(uint8x16_t __a, int __lane)
{
    return __a.__val[__lane];
}

static __inline__ signed char __attribute__((__always_inline__))
vgetq_lane_s8(int8x16_t __a, int __lane)
{
    return __a.__val[__lane];
}

static __inline__ unsigned short __attribute__((__always_inline__))
vgetq_lane_u16(uint16x8_t __a, int __lane)
{
    return __a.__val[__lane];
}

static __inline__ short __attribute__((__always_inline__))
vgetq_lane_s16(int16x8_t __a, int __lane)
{
    return __a.__val[__lane];
}

static __inline__ int __attribute__((__always_inline__))
vgetq_lane_s32(int32x4_t __a, int __lane)
{
    return __a.__val[__lane];
}

static __inline__ unsigned int __attribute__((__always_inline__))
vgetq_lane_u32(uint32x4_t __a, int __lane)
{
    return __a.__val[__lane];
}

static __inline__ float __attribute__((__always_inline__))
vgetq_lane_f32(float32x4_t __a, int __lane)
{
    return __a.__val[__lane];
}

static __inline__ double __attribute__((__always_inline__))
vgetq_lane_f64(float64x2_t __a, int __lane)
{
    return __a.__val[__lane];
}

/* vset_lane: set a single lane in a vector */
static __inline__ uint8x8_t __attribute__((__always_inline__))
vset_lane_u8(unsigned char __val, uint8x8_t __a, int __lane)
{
    __a.__val[__lane] = __val;
    return __a;
}

static __inline__ uint8x16_t __attribute__((__always_inline__))
vsetq_lane_u8(unsigned char __val, uint8x16_t __a, int __lane)
{
    __a.__val[__lane] = __val;
    return __a;
}

static __inline__ uint16x4_t __attribute__((__always_inline__))
vset_lane_u16(unsigned short __val, uint16x4_t __a, int __lane)
{
    __a.__val[__lane] = __val;
    return __a;
}

static __inline__ uint16x8_t __attribute__((__always_inline__))
vsetq_lane_u16(unsigned short __val, uint16x8_t __a, int __lane)
{
    __a.__val[__lane] = __val;
    return __a;
}

static __inline__ uint32x2_t __attribute__((__always_inline__))
vset_lane_u32(unsigned int __val, uint32x2_t __a, int __lane)
{
    __a.__val[__lane] = __val;
    return __a;
}

static __inline__ uint32x4_t __attribute__((__always_inline__))
vsetq_lane_u32(unsigned int __val, uint32x4_t __a, int __lane)
{
    __a.__val[__lane] = __val;
    return __a;
}

static __inline__ int32x2_t __attribute__((__always_inline__))
vset_lane_s32(int __val, int32x2_t __a, int __lane)
{
    __a.__val[__lane] = __val;
    return __a;
}

static __inline__ int32x4_t __attribute__((__always_inline__))
vsetq_lane_s32(int __val, int32x4_t __a, int __lane)
{
    __a.__val[__lane] = __val;
    return __a;
}

static __inline__ float32x2_t __attribute__((__always_inline__))
vset_lane_f32(float __val, float32x2_t __a, int __lane)
{
    __a.__val[__lane] = __val;
    return __a;
}

static __inline__ float32x4_t __attribute__((__always_inline__))
vsetq_lane_f32(float __val, float32x4_t __a, int __lane)
{
    __a.__val[__lane] = __val;
    return __a;
}

#define vset_lane_u64(__val, __a, __lane) ({ uint64x1_t __r = (__a); __r.__val[(__lane)] = (__val); __r; })
#define vsetq_lane_u64(__val, __a, __lane) ({ uint64x2_t __r = (__a); __r.__val[(__lane)] = (__val); __r; })
#define vset_lane_s64(__val, __a, __lane) ({ int64x1_t __r = (__a); __r.__val[(__lane)] = (__val); __r; })
#define vsetq_lane_s64(__val, __a, __lane) ({ int64x2_t __r = (__a); __r.__val[(__lane)] = (__val); __r; })
#define vsetq_lane_f64(__val, __a, __lane) ({ float64x2_t __r = (__a); __r.__val[(__lane)] = (__val); __r; })

/* ================================================================== */
/*                    ARITHMETIC OPERATIONS                            */
/* ================================================================== */

static __inline__ uint8x16_t __attribute__((__always_inline__))
vaddq_u8(uint8x16_t __a, uint8x16_t __b)
{
    uint8x16_t __ret;
    for (int __i = 0; __i < 16; __i++)
        __ret.__val[__i] = __a.__val[__i] + __b.__val[__i];
    return __ret;
}

static __inline__ uint8x8_t __attribute__((__always_inline__))
vadd_u8(uint8x8_t __a, uint8x8_t __b)
{
    uint8x8_t __ret;
    for (int __i = 0; __i < 8; __i++)
        __ret.__val[__i] = __a.__val[__i] + __b.__val[__i];
    return __ret;
}

/* --- Widening arithmetic --- */

static __inline__ uint16x8_t __attribute__((__always_inline__))
vaddl_u8(uint8x8_t __a, uint8x8_t __b)
{
    /* Widening add: u8 + u8 -> u16 */
    uint16x8_t __ret;
    for (int __i = 0; __i < 8; __i++)
        __ret.__val[__i] = (unsigned short)__a.__val[__i] + (unsigned short)__b.__val[__i];
    return __ret;
}

static __inline__ uint16x8_t __attribute__((__always_inline__))
vabdl_u8(uint8x8_t __a, uint8x8_t __b)
{
    /* Widening absolute difference: |a - b| -> u16 */
    uint16x8_t __ret;
    for (int __i = 0; __i < 8; __i++) {
        int __d = (int)__a.__val[__i] - (int)__b.__val[__i];
        __ret.__val[__i] = (unsigned short)(__d < 0 ? -__d : __d);
    }
    return __ret;
}

static __inline__ uint16x8_t __attribute__((__always_inline__))
vabdq_u16(uint16x8_t __a, uint16x8_t __b)
{
    uint16x8_t __ret;
    for (int __i = 0; __i < 8; __i++) {
        int __d = (int)__a.__val[__i] - (int)__b.__val[__i];
        __ret.__val[__i] = (unsigned short)(__d < 0 ? -__d : __d);
    }
    return __ret;
}

static __inline__ uint8x8_t __attribute__((__always_inline__))
vhadd_u8(uint8x8_t __a, uint8x8_t __b)
{
    /* Halving add: (a + b) >> 1 without overflow */
    uint8x8_t __ret;
    for (int __i = 0; __i < 8; __i++)
        __ret.__val[__i] = ((unsigned short)__a.__val[__i] + (unsigned short)__b.__val[__i]) >> 1;
    return __ret;
}

/* --- Narrowing --- */

static __inline__ uint8x8_t __attribute__((__always_inline__))
vmovn_u16(uint16x8_t __a)
{
    /* Narrow: take low 8 bits of each u16 */
    uint8x8_t __ret;
    for (int __i = 0; __i < 8; __i++)
        __ret.__val[__i] = (unsigned char)__a.__val[__i];
    return __ret;
}

/* ================================================================== */
/*                    COMPARISON OPERATIONS                            */
/* ================================================================== */

static __inline__ uint8x16_t __attribute__((__always_inline__))
vceqq_u8(uint8x16_t __a, uint8x16_t __b)
{
    uint8x16_t __ret;
    for (int __i = 0; __i < 16; __i++)
        __ret.__val[__i] = (__a.__val[__i] == __b.__val[__i]) ? 0xFF : 0x00;
    return __ret;
}

static __inline__ uint16x8_t __attribute__((__always_inline__))
vcleq_u16(uint16x8_t __a, uint16x8_t __b)
{
    /* Compare less-than-or-equal unsigned */
    uint16x8_t __ret;
    for (int __i = 0; __i < 8; __i++)
        __ret.__val[__i] = (__a.__val[__i] <= __b.__val[__i]) ? 0xFFFF : 0x0000;
    return __ret;
}

/* ================================================================== */
/*                    EXTRACT / SHUFFLE                                */
/* ================================================================== */

static __inline__ uint8x16_t __attribute__((__always_inline__))
vextq_u8(uint8x16_t __a, uint8x16_t __b, int __n)
{
    /* Extract: concatenate a and b, then extract 16 bytes starting at byte __n */
    uint8x16_t __ret;
    for (int __i = 0; __i < 16; __i++) {
        int __idx = __i + __n;
        __ret.__val[__i] = (__idx < 16) ? __a.__val[__idx] : __b.__val[__idx - 16];
    }
    return __ret;
}

static __inline__ uint8x8_t __attribute__((__always_inline__))
vext_u8(uint8x8_t __a, uint8x8_t __b, int __n)
{
    uint8x8_t __ret;
    for (int __i = 0; __i < 8; __i++) {
        int __idx = __i + __n;
        __ret.__val[__i] = (__idx < 8) ? __a.__val[__idx] : __b.__val[__idx - 8];
    }
    return __ret;
}

/* ================================================================== */
/*                 POLYNOMIAL MULTIPLICATION                           */
/* ================================================================== */

static __inline__ poly8x16_t __attribute__((__always_inline__))
vmulq_p8(poly8x16_t __a, poly8x16_t __b)
{
    /* Polynomial (carry-less) multiply per byte */
    poly8x16_t __ret;
    for (int __i = 0; __i < 16; __i++) {
        unsigned char __r = 0;
        unsigned char __av = __a.__val[__i];
        unsigned char __bv = __b.__val[__i];
        for (int __j = 0; __j < 8; __j++) {
            if (__bv & (1 << __j))
                __r ^= __av << __j;
        }
        __ret.__val[__i] = __r;
    }
    return __ret;
}

/* 64-bit polynomial multiply (for crypto/GHASH).
 * Scalar C implementation of carry-less multiplication - functionally correct
 * but much slower than the hardware PMULL instruction. */
static __inline__ poly128_t __attribute__((__always_inline__))
vmull_p64(poly64_t __a, poly64_t __b)
{
    /* Carry-less multiply of two 64-bit polynomials -> 128-bit result */
    __uint128_t __r = 0;
    for (int __j = 0; __j < 64; __j++) {
        if (__b & (1ULL << __j))
            __r ^= (__uint128_t)__a << __j;
    }
    return (poly128_t)__r;
}

static __inline__ poly128_t __attribute__((__always_inline__))
vmull_high_p64(poly64x2_t __a, poly64x2_t __b)
{
    return vmull_p64(__a.__val[1], __b.__val[1]);
}

/* ================================================================== */
/*                    REVERSE OPERATIONS                               */
/* ================================================================== */

static __inline__ uint16x8_t __attribute__((__always_inline__))
vrev32q_u16(uint16x8_t __a)
{
    /* Reverse 16-bit elements within each 32-bit word */
    uint16x8_t __ret;
    for (int __i = 0; __i < 8; __i += 2) {
        __ret.__val[__i] = __a.__val[__i + 1];
        __ret.__val[__i + 1] = __a.__val[__i];
    }
    return __ret;
}

static __inline__ uint8x16_t __attribute__((__always_inline__))
vrbitq_u8(uint8x16_t __a)
{
    /* Reverse bits within each byte */
    uint8x16_t __ret;
    for (int __i = 0; __i < 16; __i++) {
        unsigned char __v = __a.__val[__i];
        unsigned char __r = 0;
        for (int __j = 0; __j < 8; __j++)
            __r |= ((__v >> __j) & 1) << (7 - __j);
        __ret.__val[__i] = __r;
    }
    return __ret;
}

/* ================================================================== */
/*                   TABLE LOOKUP OPERATIONS                           */
/* ================================================================== */

static __inline__ uint8x16_t __attribute__((__always_inline__))
vqtbl1q_u8(uint8x16_t __a, uint8x16_t __b)
{
    uint8x16_t __ret;
    for (int __i = 0; __i < 16; __i++) {
        unsigned char __idx = __b.__val[__i];
        __ret.__val[__i] = (__idx < 16) ? __a.__val[__idx] : 0;
    }
    return __ret;
}

static __inline__ uint8x16_t __attribute__((__always_inline__))
vqtbx1q_u8(uint8x16_t __def, uint8x16_t __a, uint8x16_t __b)
{
    uint8x16_t __ret;
    for (int __i = 0; __i < 16; __i++) {
        unsigned char __idx = __b.__val[__i];
        __ret.__val[__i] = (__idx < 16) ? __a.__val[__idx] : __def.__val[__i];
    }
    return __ret;
}

static __inline__ uint8x16_t __attribute__((__always_inline__))
vqtbl4q_u8(uint8x16x4_t __a, uint8x16_t __b)
{
    uint8x16_t __ret;
    for (int __i = 0; __i < 16; __i++) {
        unsigned char __idx = __b.__val[__i];
        if (__idx < 16)
            __ret.__val[__i] = __a.val[0].__val[__idx];
        else if (__idx < 32)
            __ret.__val[__i] = __a.val[1].__val[__idx - 16];
        else if (__idx < 48)
            __ret.__val[__i] = __a.val[2].__val[__idx - 32];
        else if (__idx < 64)
            __ret.__val[__i] = __a.val[3].__val[__idx - 48];
        else
            __ret.__val[__i] = 0;
    }
    return __ret;
}

static __inline__ uint8x16_t __attribute__((__always_inline__))
vqtbx4q_u8(uint8x16_t __def, uint8x16x4_t __a, uint8x16_t __b)
{
    uint8x16_t __ret;
    for (int __i = 0; __i < 16; __i++) {
        unsigned char __idx = __b.__val[__i];
        if (__idx < 16)
            __ret.__val[__i] = __a.val[0].__val[__idx];
        else if (__idx < 32)
            __ret.__val[__i] = __a.val[1].__val[__idx - 16];
        else if (__idx < 48)
            __ret.__val[__i] = __a.val[2].__val[__idx - 32];
        else if (__idx < 64)
            __ret.__val[__i] = __a.val[3].__val[__idx - 48];
        else
            __ret.__val[__i] = __def.__val[__i];
    }
    return __ret;
}

/* 64-bit table lookup (AArch32 compat) */

static __inline__ uint8x8_t __attribute__((__always_inline__))
vtbl2_u8(uint8x8x2_t __a, uint8x8_t __b)
{
    uint8x8_t __ret;
    for (int __i = 0; __i < 8; __i++) {
        unsigned char __idx = __b.__val[__i];
        if (__idx < 8)
            __ret.__val[__i] = __a.val[0].__val[__idx];
        else if (__idx < 16)
            __ret.__val[__i] = __a.val[1].__val[__idx - 8];
        else
            __ret.__val[__i] = 0;
    }
    return __ret;
}

static __inline__ uint8x8_t __attribute__((__always_inline__))
vtbx2_u8(uint8x8_t __def, uint8x8x2_t __a, uint8x8_t __b)
{
    uint8x8_t __ret;
    for (int __i = 0; __i < 8; __i++) {
        unsigned char __idx = __b.__val[__i];
        if (__idx < 8)
            __ret.__val[__i] = __a.val[0].__val[__idx];
        else if (__idx < 16)
            __ret.__val[__i] = __a.val[1].__val[__idx - 8];
        else
            __ret.__val[__i] = __def.__val[__i];
    }
    return __ret;
}

/* ================================================================== */
/*                   PAIRWISE / ACROSS-LANE                            */
/* ================================================================== */

static __inline__ int8x8_t __attribute__((__always_inline__))
vpmin_s8(int8x8_t __a, int8x8_t __b)
{
    int8x8_t __ret;
    for (int __i = 0; __i < 4; __i++) {
        signed char __x = __a.__val[2 * __i];
        signed char __y = __a.__val[2 * __i + 1];
        __ret.__val[__i] = (__x < __y) ? __x : __y;
    }
    for (int __i = 0; __i < 4; __i++) {
        signed char __x = __b.__val[2 * __i];
        signed char __y = __b.__val[2 * __i + 1];
        __ret.__val[4 + __i] = (__x < __y) ? __x : __y;
    }
    return __ret;
}

static __inline__ signed char __attribute__((__always_inline__))
vminvq_s8(int8x16_t __a)
{
    signed char __min = __a.__val[0];
    for (int __i = 1; __i < 16; __i++) {
        if (__a.__val[__i] < __min)
            __min = __a.__val[__i];
    }
    return __min;
}

/* vminvq_u32: horizontal minimum across uint32x4_t */
static __inline__ unsigned int __attribute__((__always_inline__))
vminvq_u32(uint32x4_t __a)
{
    unsigned int __min = __a.__val[0];
    for (int __i = 1; __i < 4; __i++) {
        if (__a.__val[__i] < __min)
            __min = __a.__val[__i];
    }
    return __min;
}

/* ================================================================== */
/*                       AES INTRINSICS                                */
/* ================================================================== */
/* ARM Crypto Extension AES intrinsics using inline assembly.          */

static __inline__ uint8x16_t __attribute__((__always_inline__))
vaeseq_u8(uint8x16_t __a, uint8x16_t __b)
{
    uint8x16_t __ret;
    __asm__ __volatile__(
        "ldr q0, [%[a]]\n\t"
        "ldr q1, [%[b]]\n\t"
        "aese v0.16b, v1.16b\n\t"
        "str q0, [%[ret]]\n\t"
        :
        : [a] "r" (&__a), [b] "r" (&__b), [ret] "r" (&__ret)
        : "v0", "v1", "memory"
    );
    return __ret;
}

static __inline__ uint8x16_t __attribute__((__always_inline__))
vaesmcq_u8(uint8x16_t __a)
{
    uint8x16_t __ret;
    __asm__ __volatile__(
        "ldr q0, [%[a]]\n\t"
        "aesmc v0.16b, v0.16b\n\t"
        "str q0, [%[ret]]\n\t"
        :
        : [a] "r" (&__a), [ret] "r" (&__ret)
        : "v0", "memory"
    );
    return __ret;
}

static __inline__ uint8x16_t __attribute__((__always_inline__))
vaesdq_u8(uint8x16_t __a, uint8x16_t __b)
{
    uint8x16_t __ret;
    __asm__ __volatile__(
        "ldr q0, [%[a]]\n\t"
        "ldr q1, [%[b]]\n\t"
        "aesd v0.16b, v1.16b\n\t"
        "str q0, [%[ret]]\n\t"
        :
        : [a] "r" (&__a), [b] "r" (&__b), [ret] "r" (&__ret)
        : "v0", "v1", "memory"
    );
    return __ret;
}

static __inline__ uint8x16_t __attribute__((__always_inline__))
vaesimcq_u8(uint8x16_t __a)
{
    uint8x16_t __ret;
    __asm__ __volatile__(
        "ldr q0, [%[a]]\n\t"
        "aesimc v0.16b, v0.16b\n\t"
        "str q0, [%[ret]]\n\t"
        :
        : [a] "r" (&__a), [ret] "r" (&__ret)
        : "v0", "memory"
    );
    return __ret;
}

/* ================================================================== */
/*                  TYPE REINTERPRET CASTS                              */
/* ================================================================== */

static __inline__ int8x16_t __attribute__((__always_inline__))
vreinterpretq_s8_u8(uint8x16_t __a)
{
    int8x16_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

static __inline__ uint8x16_t __attribute__((__always_inline__))
vreinterpretq_u8_s8(int8x16_t __a)
{
    uint8x16_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

static __inline__ uint8x16_t __attribute__((__always_inline__))
vreinterpretq_u8_u16(uint16x8_t __a)
{
    uint8x16_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

static __inline__ uint16x8_t __attribute__((__always_inline__))
vreinterpretq_u16_u8(uint8x16_t __a)
{
    uint16x8_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

static __inline__ uint8x16_t __attribute__((__always_inline__))
vreinterpretq_u8_u32(uint32x4_t __a)
{
    uint8x16_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

static __inline__ uint32x4_t __attribute__((__always_inline__))
vreinterpretq_u32_u8(uint8x16_t __a)
{
    uint32x4_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

static __inline__ uint8x16_t __attribute__((__always_inline__))
vreinterpretq_u8_u64(uint64x2_t __a)
{
    uint8x16_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

static __inline__ uint64x2_t __attribute__((__always_inline__))
vreinterpretq_u64_u8(uint8x16_t __a)
{
    uint64x2_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

static __inline__ uint8x16_t __attribute__((__always_inline__))
vreinterpretq_u8_p8(poly8x16_t __a)
{
    uint8x16_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

static __inline__ poly8x16_t __attribute__((__always_inline__))
vreinterpretq_p8_u8(uint8x16_t __a)
{
    poly8x16_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

static __inline__ poly64x2_t __attribute__((__always_inline__))
vreinterpretq_p64_u8(uint8x16_t __a)
{
    poly64x2_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

static __inline__ uint64x2_t __attribute__((__always_inline__))
vreinterpretq_u64_p64(poly64x2_t __a)
{
    uint64x2_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

static __inline__ uint8x16_t __attribute__((__always_inline__))
vreinterpretq_u8_p128(poly128_t __a)
{
    uint8x16_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

/* ================================================================
 * Additional u32/u64 intrinsics for mbedtls, redis, libsodium
 * ================================================================ */

/* === Create / Duplicate === */

/* vcreate_u32: create uint32x2_t from a raw uint64 value */
static __inline__ uint32x2_t __attribute__((__always_inline__))
vcreate_u32(unsigned long long __a)
{
    uint32x2_t __ret;
    __builtin_memcpy(&__ret, &__a, 8);
    return __ret;
}

/* vcreate_u64: create uint64x1_t from a raw uint64 value */
static __inline__ uint64x1_t __attribute__((__always_inline__))
vcreate_u64(unsigned long long __a)
{
    uint64x1_t __ret;
    __ret.__val[0] = __a;
    return __ret;
}

/* vdup_n_u64: duplicate scalar u64 into single lane of uint64x1_t */
static __inline__ uint64x1_t __attribute__((__always_inline__))
vdup_n_u64(unsigned long long __a)
{
    uint64x1_t __ret;
    __ret.__val[0] = __a;
    return __ret;
}

/* vdup_n_u32: duplicate scalar u32 into both lanes of uint32x2_t */
static __inline__ uint32x2_t __attribute__((__always_inline__))
vdup_n_u32(unsigned int __a)
{
    uint32x2_t __ret;
    __ret.__val[0] = __a;
    __ret.__val[1] = __a;
    return __ret;
}

/* vdupq_n_u64: duplicate scalar u64 into both lanes of uint64x2_t */
static __inline__ uint64x2_t __attribute__((__always_inline__))
vdupq_n_u64(unsigned long long __a)
{
    uint64x2_t __ret;
    __ret.__val[0] = __a;
    __ret.__val[1] = __a;
    return __ret;
}

/* vdupq_n_s32: duplicate scalar s32 into all 4 lanes of int32x4_t */
static __inline__ int32x4_t __attribute__((__always_inline__))
vdupq_n_s32(int __a)
{
    int32x4_t __ret;
    __ret.__val[0] = __a;
    __ret.__val[1] = __a;
    __ret.__val[2] = __a;
    __ret.__val[3] = __a;
    return __ret;
}

/* === Combine (two D-registers -> one Q-register) === */

/* vcombine_u32: combine two uint32x2_t into uint32x4_t */
static __inline__ uint32x4_t __attribute__((__always_inline__))
vcombine_u32(uint32x2_t __lo, uint32x2_t __hi)
{
    uint32x4_t __ret;
    __ret.__val[0] = __lo.__val[0];
    __ret.__val[1] = __lo.__val[1];
    __ret.__val[2] = __hi.__val[0];
    __ret.__val[3] = __hi.__val[1];
    return __ret;
}

/* === Get low/high halves === */

/* vget_low_u32: get low 64-bit half of uint32x4_t */
static __inline__ uint32x2_t __attribute__((__always_inline__))
vget_low_u32(uint32x4_t __a)
{
    uint32x2_t __ret;
    __ret.__val[0] = __a.__val[0];
    __ret.__val[1] = __a.__val[1];
    return __ret;
}

/* vget_high_u32: get high 64-bit half of uint32x4_t */
static __inline__ uint32x2_t __attribute__((__always_inline__))
vget_high_u32(uint32x4_t __a)
{
    uint32x2_t __ret;
    __ret.__val[0] = __a.__val[2];
    __ret.__val[1] = __a.__val[3];
    return __ret;
}

/* === Lane access === */

/* vgetq_lane_u64: extract a single lane from uint64x2_t */
#define vgetq_lane_u64(__a, __lane) ((__a).__val[(__lane)])

/* === Arithmetic: u32 === */

/* vaddq_u32: add uint32x4_t element-wise */
static __inline__ uint32x4_t __attribute__((__always_inline__))
vaddq_u32(uint32x4_t __a, uint32x4_t __b)
{
    uint32x4_t __ret;
    __ret.__val[0] = __a.__val[0] + __b.__val[0];
    __ret.__val[1] = __a.__val[1] + __b.__val[1];
    __ret.__val[2] = __a.__val[2] + __b.__val[2];
    __ret.__val[3] = __a.__val[3] + __b.__val[3];
    return __ret;
}

/* vmulq_u32: multiply uint32x4_t element-wise */
static __inline__ uint32x4_t __attribute__((__always_inline__))
vmulq_u32(uint32x4_t __a, uint32x4_t __b)
{
    uint32x4_t __ret;
    __ret.__val[0] = __a.__val[0] * __b.__val[0];
    __ret.__val[1] = __a.__val[1] * __b.__val[1];
    __ret.__val[2] = __a.__val[2] * __b.__val[2];
    __ret.__val[3] = __a.__val[3] * __b.__val[3];
    return __ret;
}

/* === Arithmetic: u64 === */

/* vaddq_u64: add uint64x2_t element-wise */
static __inline__ uint64x2_t __attribute__((__always_inline__))
vaddq_u64(uint64x2_t __a, uint64x2_t __b)
{
    uint64x2_t __ret;
    __ret.__val[0] = __a.__val[0] + __b.__val[0];
    __ret.__val[1] = __a.__val[1] + __b.__val[1];
    return __ret;
}

/* === Bitwise: u32 === */

/* vandq_u32: bitwise AND uint32x4_t */
static __inline__ uint32x4_t __attribute__((__always_inline__))
vandq_u32(uint32x4_t __a, uint32x4_t __b)
{
    uint32x4_t __ret;
    __ret.__val[0] = __a.__val[0] & __b.__val[0];
    __ret.__val[1] = __a.__val[1] & __b.__val[1];
    __ret.__val[2] = __a.__val[2] & __b.__val[2];
    __ret.__val[3] = __a.__val[3] & __b.__val[3];
    return __ret;
}

/* vorrq_u64: bitwise OR uint64x2_t */
static __inline__ uint64x2_t __attribute__((__always_inline__))
vorrq_u64(uint64x2_t __a, uint64x2_t __b)
{
    uint64x2_t __ret;
    __ret.__val[0] = __a.__val[0] | __b.__val[0];
    __ret.__val[1] = __a.__val[1] | __b.__val[1];
    return __ret;
}

/* vorrq_u32: bitwise OR uint32x4_t */
static __inline__ uint32x4_t __attribute__((__always_inline__))
vorrq_u32(uint32x4_t __a, uint32x4_t __b)
{
    uint32x4_t __ret;
    __ret.__val[0] = __a.__val[0] | __b.__val[0];
    __ret.__val[1] = __a.__val[1] | __b.__val[1];
    __ret.__val[2] = __a.__val[2] | __b.__val[2];
    __ret.__val[3] = __a.__val[3] | __b.__val[3];
    return __ret;
}

/* veorq_u32: bitwise XOR uint32x4_t */
static __inline__ uint32x4_t __attribute__((__always_inline__))
veorq_u32(uint32x4_t __a, uint32x4_t __b)
{
    uint32x4_t __ret;
    __ret.__val[0] = __a.__val[0] ^ __b.__val[0];
    __ret.__val[1] = __a.__val[1] ^ __b.__val[1];
    __ret.__val[2] = __a.__val[2] ^ __b.__val[2];
    __ret.__val[3] = __a.__val[3] ^ __b.__val[3];
    return __ret;
}

/* === Shift: u32 === */

/* vshlq_n_u32: shift left uint32x4_t by immediate */
#define vshlq_n_u32(__a, __n) __extension__ ({ \
    uint32x4_t __r; \
    __r.__val[0] = (__a).__val[0] << (__n); \
    __r.__val[1] = (__a).__val[1] << (__n); \
    __r.__val[2] = (__a).__val[2] << (__n); \
    __r.__val[3] = (__a).__val[3] << (__n); \
    __r; \
})

/* vshrq_n_u32: shift right uint32x4_t by immediate */
#define vshrq_n_u32(__a, __n) __extension__ ({ \
    uint32x4_t __r; \
    __r.__val[0] = (__a).__val[0] >> (__n); \
    __r.__val[1] = (__a).__val[1] >> (__n); \
    __r.__val[2] = (__a).__val[2] >> (__n); \
    __r.__val[3] = (__a).__val[3] >> (__n); \
    __r; \
})

/* vsriq_n_u32: shift right and insert - for each lane,
 * shift __b right by __n, and insert into __a preserving the top __n bits of __a
 * result[i] = (a[i] & ~((1u<<(32-n))-1)) | (b[i] >> n) */
#define vsriq_n_u32(__a, __b, __n) __extension__ ({ \
    uint32x4_t __r; \
    unsigned int __mask = ~((1u << (32 - (__n))) - 1u); \
    __r.__val[0] = ((__a).__val[0] & __mask) | ((__b).__val[0] >> (__n)); \
    __r.__val[1] = ((__a).__val[1] & __mask) | ((__b).__val[1] >> (__n)); \
    __r.__val[2] = ((__a).__val[2] & __mask) | ((__b).__val[2] >> (__n)); \
    __r.__val[3] = ((__a).__val[3] & __mask) | ((__b).__val[3] >> (__n)); \
    __r; \
})

/* vsliq_n_u32: shift left and insert - for each lane,
 * shift __b left by __n, and insert into __a preserving the low __n bits of __a */
#define vsliq_n_u32(__a, __b, __n) __extension__ ({ \
    uint32x4_t __r; \
    unsigned int __mask = (1u << (__n)) - 1u; \
    __r.__val[0] = ((__a).__val[0] & __mask) | ((__b).__val[0] << (__n)); \
    __r.__val[1] = ((__a).__val[1] & __mask) | ((__b).__val[1] << (__n)); \
    __r.__val[2] = ((__a).__val[2] & __mask) | ((__b).__val[2] << (__n)); \
    __r.__val[3] = ((__a).__val[3] & __mask) | ((__b).__val[3] << (__n)); \
    __r; \
})

/* === Compare: u8 === */

/* vmaxq_u8: element-wise maximum of uint8x16_t */
static __inline__ uint8x16_t __attribute__((__always_inline__))
vmaxq_u8(uint8x16_t __a, uint8x16_t __b)
{
    uint8x16_t __ret;
    for (int __i = 0; __i < 16; __i++)
        __ret.__val[__i] = __a.__val[__i] > __b.__val[__i] ? __a.__val[__i] : __b.__val[__i];
    return __ret;
}

/* === Population count === */

/* vcntq_u8: population count per byte */
static __inline__ uint8x16_t __attribute__((__always_inline__))
vcntq_u8(uint8x16_t __a)
{
    uint8x16_t __ret;
    for (int __i = 0; __i < 16; __i++) {
        unsigned char __v = __a.__val[__i];
        unsigned char __c = 0;
        while (__v) { __c += __v & 1; __v >>= 1; }
        __ret.__val[__i] = __c;
    }
    return __ret;
}

/* === Pairwise add long === */

/* vpaddlq_u8: pairwise add adjacent u8 pairs, result as u16 */
static __inline__ uint16x8_t __attribute__((__always_inline__))
vpaddlq_u8(uint8x16_t __a)
{
    uint16x8_t __ret;
    for (int __i = 0; __i < 8; __i++)
        __ret.__val[__i] = (unsigned short)__a.__val[__i * 2] + (unsigned short)__a.__val[__i * 2 + 1];
    return __ret;
}

/* vpaddlq_u16: pairwise add adjacent u16 pairs, result as u32 */
static __inline__ uint32x4_t __attribute__((__always_inline__))
vpaddlq_u16(uint16x8_t __a)
{
    uint32x4_t __ret;
    for (int __i = 0; __i < 4; __i++)
        __ret.__val[__i] = (unsigned int)__a.__val[__i * 2] + (unsigned int)__a.__val[__i * 2 + 1];
    return __ret;
}

/* vpaddlq_u32: pairwise add adjacent u32 pairs, result as u64 */
static __inline__ uint64x2_t __attribute__((__always_inline__))
vpaddlq_u32(uint32x4_t __a)
{
    uint64x2_t __ret;
    __ret.__val[0] = (unsigned long long)__a.__val[0] + (unsigned long long)__a.__val[1];
    __ret.__val[1] = (unsigned long long)__a.__val[2] + (unsigned long long)__a.__val[3];
    return __ret;
}

/* vpadalq_u8: pairwise add and accumulate long u8 -> u16 */
static __inline__ uint16x8_t __attribute__((__always_inline__))
vpadalq_u8(uint16x8_t __acc, uint8x16_t __a)
{
    uint16x8_t __ret;
    for (int __i = 0; __i < 8; __i++)
        __ret.__val[__i] = __acc.__val[__i] + (unsigned short)__a.__val[__i * 2] + (unsigned short)__a.__val[__i * 2 + 1];
    return __ret;
}

/* === Extract / Rotate === */

/* vextq_u64: extract from pair of uint64x2_t */
#define vextq_u64(__a, __b, __n) __extension__ ({ \
    uint64x2_t __r; \
    if ((__n) == 0) { \
        __r = (__a); \
    } else { \
        __r.__val[0] = (__a).__val[1]; \
        __r.__val[1] = (__b).__val[0]; \
    } \
    __r; \
})

/* vextq_u32: extract from pair of uint32x4_t */
#define vextq_u32(__a, __b, __n) __extension__ ({ \
    uint32x4_t __r; \
    unsigned int __tmp[8]; \
    __tmp[0] = (__a).__val[0]; __tmp[1] = (__a).__val[1]; \
    __tmp[2] = (__a).__val[2]; __tmp[3] = (__a).__val[3]; \
    __tmp[4] = (__b).__val[0]; __tmp[5] = (__b).__val[1]; \
    __tmp[6] = (__b).__val[2]; __tmp[7] = (__b).__val[3]; \
    __r.__val[0] = __tmp[(__n)]; __r.__val[1] = __tmp[(__n) + 1]; \
    __r.__val[2] = __tmp[(__n) + 2]; __r.__val[3] = __tmp[(__n) + 3]; \
    __r; \
})

/* === Narrowing === */

/* vmovn_u64: narrow uint64x2_t to uint32x2_t (take low 32 bits of each lane) */
static __inline__ uint32x2_t __attribute__((__always_inline__))
vmovn_u64(uint64x2_t __a)
{
    uint32x2_t __ret;
    __ret.__val[0] = (unsigned int)__a.__val[0];
    __ret.__val[1] = (unsigned int)__a.__val[1];
    return __ret;
}

/* vmovn_u32: narrow uint32x4_t to uint16x4_t (take low 16 bits of each lane) */
static __inline__ uint16x4_t __attribute__((__always_inline__))
vmovn_u32(uint32x4_t __a)
{
    uint16x4_t __ret;
    __ret.__val[0] = (unsigned short)__a.__val[0];
    __ret.__val[1] = (unsigned short)__a.__val[1];
    __ret.__val[2] = (unsigned short)__a.__val[2];
    __ret.__val[3] = (unsigned short)__a.__val[3];
    return __ret;
}

/* vshrn_n_u64: shift right and narrow uint64x2_t to uint32x2_t */
#define vshrn_n_u64(__a, __n) __extension__ ({ \
    uint32x2_t __r; \
    __r.__val[0] = (unsigned int)((__a).__val[0] >> (__n)); \
    __r.__val[1] = (unsigned int)((__a).__val[1] >> (__n)); \
    __r; \
})

/* vshrn_n_u32: shift right and narrow uint32x4_t to uint16x4_t */
#define vshrn_n_u32(__a, __n) __extension__ ({ \
    uint16x4_t __r; \
    __r.__val[0] = (unsigned short)((__a).__val[0] >> (__n)); \
    __r.__val[1] = (unsigned short)((__a).__val[1] >> (__n)); \
    __r.__val[2] = (unsigned short)((__a).__val[2] >> (__n)); \
    __r.__val[3] = (unsigned short)((__a).__val[3] >> (__n)); \
    __r; \
})

/* === Widening multiply-accumulate === */

/* vmlal_u32: widening multiply-accumulate u32 -> u64
 * result[i] = acc[i] + (u64)a[i] * (u64)b[i] */
static __inline__ uint64x2_t __attribute__((__always_inline__))
vmlal_u32(uint64x2_t __acc, uint32x2_t __a, uint32x2_t __b)
{
    uint64x2_t __ret;
    __ret.__val[0] = __acc.__val[0] + (unsigned long long)__a.__val[0] * (unsigned long long)__b.__val[0];
    __ret.__val[1] = __acc.__val[1] + (unsigned long long)__a.__val[1] * (unsigned long long)__b.__val[1];
    return __ret;
}

/* vmlal_high_u32: widening multiply-accumulate of high halves */
static __inline__ uint64x2_t __attribute__((__always_inline__))
vmlal_high_u32(uint64x2_t __acc, uint32x4_t __a, uint32x4_t __b)
{
    uint64x2_t __ret;
    __ret.__val[0] = __acc.__val[0] + (unsigned long long)__a.__val[2] * (unsigned long long)__b.__val[2];
    __ret.__val[1] = __acc.__val[1] + (unsigned long long)__a.__val[3] * (unsigned long long)__b.__val[3];
    return __ret;
}

/* vmlal_low_u32: widening multiply-accumulate of low halves */
static __inline__ uint64x2_t __attribute__((__always_inline__))
vmlal_low_u32(uint64x2_t __acc, uint32x4_t __a, uint32x4_t __b)
{
    uint64x2_t __ret;
    __ret.__val[0] = __acc.__val[0] + (unsigned long long)__a.__val[0] * (unsigned long long)__b.__val[0];
    __ret.__val[1] = __acc.__val[1] + (unsigned long long)__a.__val[1] * (unsigned long long)__b.__val[1];
    return __ret;
}

/* === Unzip / De-interleave === */

/* vuzpq_u32: unzip (de-interleave) two uint32x4_t vectors
 * result.val[0] = {a[0], a[2], b[0], b[2]}  (even elements)
 * result.val[1] = {a[1], a[3], b[1], b[3]}  (odd elements) */
static __inline__ uint32x4x2_t __attribute__((__always_inline__))
vuzpq_u32(uint32x4_t __a, uint32x4_t __b)
{
    uint32x4x2_t __ret;
    __ret.val[0].__val[0] = __a.__val[0];
    __ret.val[0].__val[1] = __a.__val[2];
    __ret.val[0].__val[2] = __b.__val[0];
    __ret.val[0].__val[3] = __b.__val[2];
    __ret.val[1].__val[0] = __a.__val[1];
    __ret.val[1].__val[1] = __a.__val[3];
    __ret.val[1].__val[2] = __b.__val[1];
    __ret.val[1].__val[3] = __b.__val[3];
    return __ret;
}

/* === Reinterpret casts: u32 <-> u64 === */

/* vreinterpretq_u32_u64: reinterpret uint64x2_t as uint32x4_t */
static __inline__ uint32x4_t __attribute__((__always_inline__))
vreinterpretq_u32_u64(uint64x2_t __a)
{
    uint32x4_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

/* vreinterpretq_u64_u32: reinterpret uint32x4_t as uint64x2_t */
static __inline__ uint64x2_t __attribute__((__always_inline__))
vreinterpretq_u64_u32(uint32x4_t __a)
{
    uint64x2_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

/* vreinterpretq_s32_u32: reinterpret uint32x4_t as int32x4_t */
static __inline__ int32x4_t __attribute__((__always_inline__))
vreinterpretq_s32_u32(uint32x4_t __a)
{
    int32x4_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

/* vreinterpretq_u32_s32: reinterpret int32x4_t as uint32x4_t */
static __inline__ uint32x4_t __attribute__((__always_inline__))
vreinterpretq_u32_s32(int32x4_t __a)
{
    uint32x4_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

/* vreinterpretq_u16_u32: reinterpret uint32x4_t as uint16x8_t */
static __inline__ uint16x8_t __attribute__((__always_inline__))
vreinterpretq_u16_u32(uint32x4_t __a)
{
    uint16x8_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

/* vreinterpretq_u32_u16: reinterpret uint16x8_t as uint32x4_t */
static __inline__ uint32x4_t __attribute__((__always_inline__))
vreinterpretq_u32_u16(uint16x8_t __a)
{
    uint32x4_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

/* === SHA-256 crypto intrinsics (software implementation) === */
/* Used by mbedtls SHA-256 hardware acceleration. */

/* vsha256su0q_u32: SHA-256 schedule update 0 */
static __inline__ uint32x4_t __attribute__((__always_inline__))
vsha256su0q_u32(uint32x4_t __w0_3, uint32x4_t __w4_7)
{
    uint32x4_t __ret;
    for (int __i = 0; __i < 4; __i++) {
        unsigned int __w = (__i < 3) ? __w0_3.__val[__i + 1] : __w4_7.__val[0];
        /* sigma0: ROTR(7) ^ ROTR(18) ^ SHR(3) */
        unsigned int __s0 = ((__w >> 7) | (__w << 25)) ^ ((__w >> 18) | (__w << 14)) ^ (__w >> 3);
        __ret.__val[__i] = __w0_3.__val[__i] + __s0;
    }
    return __ret;
}

/* vsha256su1q_u32: SHA-256 schedule update 1 */
static __inline__ uint32x4_t __attribute__((__always_inline__))
vsha256su1q_u32(uint32x4_t __tw0_3, uint32x4_t __w8_11, uint32x4_t __w12_15)
{
    uint32x4_t __ret;
    unsigned int __wm2[4];
    __wm2[0] = __w12_15.__val[2];
    __wm2[1] = __w12_15.__val[3];
    __wm2[2] = __tw0_3.__val[0];
    __wm2[3] = __tw0_3.__val[1];
    for (int __i = 0; __i < 4; __i++) {
        unsigned int __w = __wm2[__i];
        /* sigma1: ROTR(17) ^ ROTR(19) ^ SHR(10) */
        unsigned int __s1 = ((__w >> 17) | (__w << 15)) ^ ((__w >> 19) | (__w << 13)) ^ (__w >> 10);
        unsigned int __w9;
        if (__i < 2) __w9 = __w8_11.__val[__i + 2];
        else __w9 = __w12_15.__val[__i - 2];
        __ret.__val[__i] = __tw0_3.__val[__i] + __s1 + __w9;
    }
    return __ret;
}

/* vsha256hq_u32: SHA-256 hash update (part 1) */
static __inline__ uint32x4_t __attribute__((__always_inline__))
vsha256hq_u32(uint32x4_t __hash_abcd, uint32x4_t __hash_efgh, uint32x4_t __wk)
{
    unsigned int __a = __hash_abcd.__val[0], __b = __hash_abcd.__val[1];
    unsigned int __c = __hash_abcd.__val[2], __d = __hash_abcd.__val[3];
    unsigned int __e = __hash_efgh.__val[0], __f = __hash_efgh.__val[1];
    unsigned int __g = __hash_efgh.__val[2], __h = __hash_efgh.__val[3];
    for (int __i = 0; __i < 4; __i++) {
        unsigned int __S1 = ((__e >> 6) | (__e << 26)) ^ ((__e >> 11) | (__e << 21)) ^ ((__e >> 25) | (__e << 7));
        unsigned int __ch = (__e & __f) ^ (~__e & __g);
        unsigned int __temp1 = __h + __S1 + __ch + __wk.__val[__i];
        unsigned int __S0 = ((__a >> 2) | (__a << 30)) ^ ((__a >> 13) | (__a << 19)) ^ ((__a >> 22) | (__a << 10));
        unsigned int __maj = (__a & __b) ^ (__a & __c) ^ (__b & __c);
        unsigned int __temp2 = __S0 + __maj;
        __h = __g; __g = __f; __f = __e; __e = __d + __temp1;
        __d = __c; __c = __b; __b = __a; __a = __temp1 + __temp2;
    }
    uint32x4_t __ret;
    __ret.__val[0] = __a; __ret.__val[1] = __b;
    __ret.__val[2] = __c; __ret.__val[3] = __d;
    return __ret;
}

/* vsha256h2q_u32: SHA-256 hash update (part 2) */
static __inline__ uint32x4_t __attribute__((__always_inline__))
vsha256h2q_u32(uint32x4_t __hash_efgh, uint32x4_t __hash_abcd, uint32x4_t __wk)
{
    unsigned int __a = __hash_abcd.__val[0], __b = __hash_abcd.__val[1];
    unsigned int __c = __hash_abcd.__val[2], __d = __hash_abcd.__val[3];
    unsigned int __e = __hash_efgh.__val[0], __f = __hash_efgh.__val[1];
    unsigned int __g = __hash_efgh.__val[2], __h = __hash_efgh.__val[3];
    for (int __i = 0; __i < 4; __i++) {
        unsigned int __S1 = ((__e >> 6) | (__e << 26)) ^ ((__e >> 11) | (__e << 21)) ^ ((__e >> 25) | (__e << 7));
        unsigned int __ch = (__e & __f) ^ (~__e & __g);
        unsigned int __temp1 = __h + __S1 + __ch + __wk.__val[__i];
        unsigned int __S0 = ((__a >> 2) | (__a << 30)) ^ ((__a >> 13) | (__a << 19)) ^ ((__a >> 22) | (__a << 10));
        unsigned int __maj = (__a & __b) ^ (__a & __c) ^ (__b & __c);
        unsigned int __temp2 = __S0 + __maj;
        __h = __g; __g = __f; __f = __e; __e = __d + __temp1;
        __d = __c; __c = __b; __b = __a; __a = __temp1 + __temp2;
    }
    uint32x4_t __ret;
    __ret.__val[0] = __e; __ret.__val[1] = __f;
    __ret.__val[2] = __g; __ret.__val[3] = __h;
    return __ret;
}

/* === Load/Store u64 (64-bit / D-register) === */

/* vld1_u64: load 1 x u64 (64-bit) */
static __inline__ uint64x1_t __attribute__((__always_inline__))
vld1_u64(const unsigned long long *__p)
{
    uint64x1_t __ret;
    __builtin_memcpy(&__ret, __p, 8);
    return __ret;
}

/* === Load/Store u32 (64-bit / D-register) === */

/* vld1_u32: load 2 x u32 (64-bit) */
static __inline__ uint32x2_t __attribute__((__always_inline__))
vld1_u32(unsigned int const *__p)
{
    uint32x2_t __ret;
    __builtin_memcpy(&__ret, __p, 8);
    return __ret;
}

/* vst1_u32: store 2 x u32 (64-bit) */
static __inline__ void __attribute__((__always_inline__))
vst1_u32(unsigned int *__p, uint32x2_t __a)
{
    __builtin_memcpy(__p, &__a, 8);
}

/* === Store 8x8 (64-bit / D-register) === */

/* vst1_u8: store 8 x u8 (64-bit) */
static __inline__ void __attribute__((__always_inline__))
vst1_u8(unsigned char *__p, uint8x8_t __a)
{
    __builtin_memcpy(__p, &__a, 8);
}

/* === Reinterpret casts === */

/* vreinterpret_u64_u8: reinterpret uint8x8_t as uint64x1_t (no code) */
static __inline__ uint64x1_t __attribute__((__always_inline__))
vreinterpret_u64_u8(uint8x8_t __a)
{
    uint64x1_t __ret;
    __builtin_memcpy(&__ret, &__a, 8);
    return __ret;
}

/* === Get lane === */

/* vget_lane_u64: extract lane from uint64x1_t */
static __inline__ unsigned long long __attribute__((__always_inline__))
vget_lane_u64(uint64x1_t __a, int __lane)
{
    (void)__lane;
    return __a.__val[0];
}

/* === Narrowing shifts === */

/* vshrn_n_u16: shift right narrow (u16 -> u8) */
static __inline__ uint8x8_t __attribute__((__always_inline__))
vshrn_n_u16(uint16x8_t __a, int __n)
{
    uint8x8_t __ret;
    for (int __i = 0; __i < 8; __i++) {
        __ret.__val[__i] = (unsigned char)(__a.__val[__i] >> __n);
    }
    return __ret;
}

/* === Shift and insert === */

/* vsli_n_u8: shift left and insert (64-bit, per-byte) */
static __inline__ uint8x8_t __attribute__((__always_inline__))
vsli_n_u8(uint8x8_t __a, uint8x8_t __b, int __n)
{
    uint8x8_t __ret;
    unsigned char __mask = (unsigned char)((0xFF << __n) & 0xFF);
    for (int __i = 0; __i < 8; __i++) {
        __ret.__val[__i] = (__a.__val[__i] & ~__mask) | ((unsigned char)(__b.__val[__i] << __n) & __mask);
    }
    return __ret;
}

/* vsriq_n_u8: shift right and insert (128-bit, per-byte) */
static __inline__ uint8x16_t __attribute__((__always_inline__))
vsriq_n_u8(uint8x16_t __a, uint8x16_t __b, int __n)
{
    uint8x16_t __ret;
    unsigned char __mask = (unsigned char)(0xFF >> __n);
    for (int __i = 0; __i < 16; __i++) {
        __ret.__val[__i] = (__a.__val[__i] & ~__mask) | ((__b.__val[__i] >> __n) & __mask);
    }
    return __ret;
}

/* === Multi-element structure loads === */

/* vld2q_u16: load 2-element interleaved u16 (128-bit x 2) */
static __inline__ uint16x8x2_t __attribute__((__always_inline__))
vld2q_u16(unsigned short const *__p)
{
    uint16x8x2_t __ret;
    for (int __i = 0; __i < 8; __i++) {
        __ret.val[0].__val[__i] = __p[__i * 2];
        __ret.val[1].__val[__i] = __p[__i * 2 + 1];
    }
    return __ret;
}

/* vld4q_u8: load 4-element interleaved u8 (128-bit x 4) */
static __inline__ uint8x16x4_t __attribute__((__always_inline__))
vld4q_u8(unsigned char const *__p)
{
    uint8x16x4_t __ret;
    for (int __i = 0; __i < 16; __i++) {
        __ret.val[0].__val[__i] = __p[__i * 4];
        __ret.val[1].__val[__i] = __p[__i * 4 + 1];
        __ret.val[2].__val[__i] = __p[__i * 4 + 2];
        __ret.val[3].__val[__i] = __p[__i * 4 + 3];
    }
    return __ret;
}

/* ================================================================== */
/*          REINTERPRET CASTS: signed <-> unsigned                     */
/* ================================================================== */

/* vreinterpretq_u8_s16: reinterpret int16x8_t as uint8x16_t */
static __inline__ uint8x16_t __attribute__((__always_inline__))
vreinterpretq_u8_s16(int16x8_t __a)
{
    uint8x16_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

/* vreinterpretq_s16_u8: reinterpret uint8x16_t as int16x8_t */
static __inline__ int16x8_t __attribute__((__always_inline__))
vreinterpretq_s16_u8(uint8x16_t __a)
{
    int16x8_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

/* vreinterpretq_u8_s32: reinterpret int32x4_t as uint8x16_t */
static __inline__ uint8x16_t __attribute__((__always_inline__))
vreinterpretq_u8_s32(int32x4_t __a)
{
    uint8x16_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

/* ================================================================== */
/*          DUPLICATE (BROADCAST): signed types                        */
/* ================================================================== */

/* vdupq_n_s16: broadcast signed 16-bit value into all 8 lanes */
static __inline__ int16x8_t __attribute__((__always_inline__))
vdupq_n_s16(short __a)
{
    int16x8_t __ret;
    for (int __i = 0; __i < 8; __i++)
        __ret.__val[__i] = __a;
    return __ret;
}

/* ================================================================== */
/*           HORIZONTAL REDUCTION OPERATIONS                           */
/* ================================================================== */

/* vmaxvq_u8: horizontal max of all uint8x16_t lanes -> scalar uint8_t */
static __inline__ unsigned char __attribute__((__always_inline__))
vmaxvq_u8(uint8x16_t __a)
{
    unsigned char __ret = __a.__val[0];
    for (int __i = 1; __i < 16; __i++)
        if (__a.__val[__i] > __ret) __ret = __a.__val[__i];
    return __ret;
}

/* vminvq_u8: horizontal min of all uint8x16_t lanes -> scalar uint8_t */
static __inline__ unsigned char __attribute__((__always_inline__))
vminvq_u8(uint8x16_t __a)
{
    unsigned char __ret = __a.__val[0];
    for (int __i = 1; __i < 16; __i++)
        if (__a.__val[__i] < __ret) __ret = __a.__val[__i];
    return __ret;
}

/* vmaxvq_u32: horizontal max of all uint32x4_t lanes -> scalar uint32_t */
static __inline__ unsigned int __attribute__((__always_inline__))
vmaxvq_u32(uint32x4_t __a)
{
    unsigned int __ret = __a.__val[0];
    for (int __i = 1; __i < 4; __i++)
        if (__a.__val[__i] > __ret) __ret = __a.__val[__i];
    return __ret;
}

/* vmaxvq_s16: horizontal max across int16x8_t */
static __inline__ short __attribute__((__always_inline__))
vmaxvq_s16(int16x8_t __a)
{
    short __max = __a.__val[0];
    for (int __i = 1; __i < 8; __i++) {
        if (__a.__val[__i] > __max)
            __max = __a.__val[__i];
    }
    return __max;
}

/* vmaxvq_u16: horizontal max across uint16x8_t */
static __inline__ unsigned short __attribute__((__always_inline__))
vmaxvq_u16(uint16x8_t __a)
{
    unsigned short __max = __a.__val[0];
    for (int __i = 1; __i < 8; __i++) {
        if (__a.__val[__i] > __max)
            __max = __a.__val[__i];
    }
    return __max;
}

/* ================================================================== */
/*           SATURATING ARITHMETIC                                     */
/* ================================================================== */

/* vqsubq_u8: saturating subtract uint8x16_t (clamp at 0) */
static __inline__ uint8x16_t __attribute__((__always_inline__))
vqsubq_u8(uint8x16_t __a, uint8x16_t __b)
{
    uint8x16_t __ret;
    for (int __i = 0; __i < 16; __i++)
        __ret.__val[__i] = __a.__val[__i] > __b.__val[__i] ? __a.__val[__i] - __b.__val[__i] : 0;
    return __ret;
}

/* vqaddq_u8: saturating add uint8x16_t (clamp at 255) */
static __inline__ uint8x16_t __attribute__((__always_inline__))
vqaddq_u8(uint8x16_t __a, uint8x16_t __b)
{
    uint8x16_t __ret;
    for (int __i = 0; __i < 16; __i++) {
        unsigned int __sum = (unsigned int)__a.__val[__i] + (unsigned int)__b.__val[__i];
        __ret.__val[__i] = __sum > 255 ? 255 : (unsigned char)__sum;
    }
    return __ret;
}

/* ================================================================== */
/*           ADDITIONAL COMPARISON OPERATIONS                          */
/* ================================================================== */

/* vceqq_u32: element-wise equality compare uint32x4_t */
static __inline__ uint32x4_t __attribute__((__always_inline__))
vceqq_u32(uint32x4_t __a, uint32x4_t __b)
{
    uint32x4_t __ret;
    for (int __i = 0; __i < 4; __i++)
        __ret.__val[__i] = (__a.__val[__i] == __b.__val[__i]) ? 0xFFFFFFFF : 0x00000000;
    return __ret;
}

/* vceqq_u16: element-wise equality compare uint16x8_t */
static __inline__ uint16x8_t __attribute__((__always_inline__))
vceqq_u16(uint16x8_t __a, uint16x8_t __b)
{
    uint16x8_t __ret;
    for (int __i = 0; __i < 8; __i++)
        __ret.__val[__i] = (__a.__val[__i] == __b.__val[__i]) ? 0xFFFF : 0x0000;
    return __ret;
}

/* vqaddq_s16: saturating add int16x8_t */
static __inline__ int16x8_t __attribute__((__always_inline__))
vqaddq_s16(int16x8_t __a, int16x8_t __b)
{
    int16x8_t __ret;
    for (int __i = 0; __i < 8; __i++) {
        int __sum = (int)__a.__val[__i] + (int)__b.__val[__i];
        if (__sum > 32767) __sum = 32767;
        else if (__sum < -32768) __sum = -32768;
        __ret.__val[__i] = (short)__sum;
    }
    return __ret;
}

/* vqsubq_u16: saturating subtract uint16x8_t */
static __inline__ uint16x8_t __attribute__((__always_inline__))
vqsubq_u16(uint16x8_t __a, uint16x8_t __b)
{
    uint16x8_t __ret;
    for (int __i = 0; __i < 8; __i++) {
        int __diff = (int)__a.__val[__i] - (int)__b.__val[__i];
        __ret.__val[__i] = __diff < 0 ? 0 : (unsigned short)__diff;
    }
    return __ret;
}

/* vcgtq_s16: compare greater-than int16x8_t -> uint16x8_t mask */
static __inline__ uint16x8_t __attribute__((__always_inline__))
vcgtq_s16(int16x8_t __a, int16x8_t __b)
{
    uint16x8_t __ret;
    for (int __i = 0; __i < 8; __i++)
        __ret.__val[__i] = (__a.__val[__i] > __b.__val[__i]) ? 0xFFFF : 0x0000;
    return __ret;
}

/* vmaxq_s16: element-wise maximum of int16x8_t */
static __inline__ int16x8_t __attribute__((__always_inline__))
vmaxq_s16(int16x8_t __a, int16x8_t __b)
{
    int16x8_t __ret;
    for (int __i = 0; __i < 8; __i++)
        __ret.__val[__i] = __a.__val[__i] > __b.__val[__i] ? __a.__val[__i] : __b.__val[__i];
    return __ret;
}

/* ===== Float32 NEON intrinsics ===== */

/* vld1q_f32: load 4 floats into float32x4_t */
static __inline__ float32x4_t __attribute__((__always_inline__))
vld1q_f32(const float *__p)
{
    float32x4_t __ret;
    __ret.__val[0] = __p[0];
    __ret.__val[1] = __p[1];
    __ret.__val[2] = __p[2];
    __ret.__val[3] = __p[3];
    return __ret;
}

/* vst1q_f32: store float32x4_t to memory */
static __inline__ void __attribute__((__always_inline__))
vst1q_f32(float *__p, float32x4_t __a)
{
    __p[0] = __a.__val[0];
    __p[1] = __a.__val[1];
    __p[2] = __a.__val[2];
    __p[3] = __a.__val[3];
}

/* vst1_f32: store float32x2_t (64-bit) to memory */
static __inline__ void __attribute__((__always_inline__))
vst1_f32(float *__p, float32x2_t __a)
{
    __p[0] = __a.__val[0];
    __p[1] = __a.__val[1];
}

/* vaddq_f32: add float32x4_t element-wise */
static __inline__ float32x4_t __attribute__((__always_inline__))
vaddq_f32(float32x4_t __a, float32x4_t __b)
{
    float32x4_t __ret;
    __ret.__val[0] = __a.__val[0] + __b.__val[0];
    __ret.__val[1] = __a.__val[1] + __b.__val[1];
    __ret.__val[2] = __a.__val[2] + __b.__val[2];
    __ret.__val[3] = __a.__val[3] + __b.__val[3];
    return __ret;
}

/* vsubq_f32: subtract float32x4_t element-wise */
static __inline__ float32x4_t __attribute__((__always_inline__))
vsubq_f32(float32x4_t __a, float32x4_t __b)
{
    float32x4_t __ret;
    __ret.__val[0] = __a.__val[0] - __b.__val[0];
    __ret.__val[1] = __a.__val[1] - __b.__val[1];
    __ret.__val[2] = __a.__val[2] - __b.__val[2];
    __ret.__val[3] = __a.__val[3] - __b.__val[3];
    return __ret;
}

/* vmulq_f32: multiply float32x4_t element-wise */
static __inline__ float32x4_t __attribute__((__always_inline__))
vmulq_f32(float32x4_t __a, float32x4_t __b)
{
    float32x4_t __ret;
    __ret.__val[0] = __a.__val[0] * __b.__val[0];
    __ret.__val[1] = __a.__val[1] * __b.__val[1];
    __ret.__val[2] = __a.__val[2] * __b.__val[2];
    __ret.__val[3] = __a.__val[3] * __b.__val[3];
    return __ret;
}

/* vmovq_n_f32: broadcast a float to all lanes of float32x4_t */
static __inline__ float32x4_t __attribute__((__always_inline__))
vmovq_n_f32(float __a)
{
    float32x4_t __ret;
    __ret.__val[0] = __a;
    __ret.__val[1] = __a;
    __ret.__val[2] = __a;
    __ret.__val[3] = __a;
    return __ret;
}

/* vget_low_f32: get low 64-bit half of float32x4_t as float32x2_t */
static __inline__ float32x2_t __attribute__((__always_inline__))
vget_low_f32(float32x4_t __a)
{
    float32x2_t __ret;
    __ret.__val[0] = __a.__val[0];
    __ret.__val[1] = __a.__val[1];
    return __ret;
}

/* vget_high_f32: get high 64-bit half of float32x4_t as float32x2_t */
static __inline__ float32x2_t __attribute__((__always_inline__))
vget_high_f32(float32x4_t __a)
{
    float32x2_t __ret;
    __ret.__val[0] = __a.__val[2];
    __ret.__val[1] = __a.__val[3];
    return __ret;
}

/* vcombine_f32: combine two float32x2_t into float32x4_t */
static __inline__ float32x4_t __attribute__((__always_inline__))
vcombine_f32(float32x2_t __lo, float32x2_t __hi)
{
    float32x4_t __ret;
    __ret.__val[0] = __lo.__val[0];
    __ret.__val[1] = __lo.__val[1];
    __ret.__val[2] = __hi.__val[0];
    __ret.__val[3] = __hi.__val[1];
    return __ret;
}

/* vrev64q_f32: reverse floats within each 64-bit lane */
static __inline__ float32x4_t __attribute__((__always_inline__))
vrev64q_f32(float32x4_t __a)
{
    float32x4_t __ret;
    __ret.__val[0] = __a.__val[1];
    __ret.__val[1] = __a.__val[0];
    __ret.__val[2] = __a.__val[3];
    __ret.__val[3] = __a.__val[2];
    return __ret;
}

/* vcltq_f32: compare less-than, returns uint32x4_t mask */
static __inline__ uint32x4_t __attribute__((__always_inline__))
vcltq_f32(float32x4_t __a, float32x4_t __b)
{
    uint32x4_t __ret;
    __ret.__val[0] = __a.__val[0] < __b.__val[0] ? 0xFFFFFFFFu : 0;
    __ret.__val[1] = __a.__val[1] < __b.__val[1] ? 0xFFFFFFFFu : 0;
    __ret.__val[2] = __a.__val[2] < __b.__val[2] ? 0xFFFFFFFFu : 0;
    __ret.__val[3] = __a.__val[3] < __b.__val[3] ? 0xFFFFFFFFu : 0;
    return __ret;
}

/* vcvtq_s32_f32: convert float32x4_t to int32x4_t (round toward zero) */
static __inline__ int32x4_t __attribute__((__always_inline__))
vcvtq_s32_f32(float32x4_t __a)
{
    int32x4_t __ret;
    __ret.__val[0] = (int)__a.__val[0];
    __ret.__val[1] = (int)__a.__val[1];
    __ret.__val[2] = (int)__a.__val[2];
    __ret.__val[3] = (int)__a.__val[3];
    return __ret;
}

/* vqaddq_s32: saturating add int32x4_t */
static __inline__ int32x4_t __attribute__((__always_inline__))
vqaddq_s32(int32x4_t __a, int32x4_t __b)
{
    int32x4_t __ret;
    for (int __i = 0; __i < 4; __i++) {
        long long __sum = (long long)__a.__val[__i] + (long long)__b.__val[__i];
        if (__sum > 2147483647LL) __sum = 2147483647LL;
        if (__sum < -2147483648LL) __sum = -2147483648LL;
        __ret.__val[__i] = (int)__sum;
    }
    return __ret;
}

/* vqmovn_s32: saturating narrow int32x4_t to int16x4_t */
static __inline__ int16x4_t __attribute__((__always_inline__))
vqmovn_s32(int32x4_t __a)
{
    int16x4_t __ret;
    for (int __i = 0; __i < 4; __i++) {
        int __v = __a.__val[__i];
        if (__v > 32767) __v = 32767;
        if (__v < -32768) __v = -32768;
        __ret.__val[__i] = (short)__v;
    }
    return __ret;
}

/* vst1_lane_s16: store one lane from int16x4_t */
static __inline__ void __attribute__((__always_inline__))
vst1_lane_s16(short *__p, int16x4_t __a, int __lane)
{
    *__p = __a.__val[__lane];
}

/* ================================================================== */
/*              u16 ARITHMETIC / COMPARE / BITWISE                    */
/* ================================================================== */

/* vaddq_u16: element-wise add uint16x8_t */
static __inline__ uint16x8_t __attribute__((__always_inline__))
vaddq_u16(uint16x8_t __a, uint16x8_t __b)
{
    uint16x8_t __ret;
    for (int __i = 0; __i < 8; __i++)
        __ret.__val[__i] = __a.__val[__i] + __b.__val[__i];
    return __ret;
}

/* vaddq_s16: element-wise add int16x8_t */
static __inline__ int16x8_t __attribute__((__always_inline__))
vaddq_s16(int16x8_t __a, int16x8_t __b)
{
    int16x8_t __ret;
    for (int __i = 0; __i < 8; __i++)
        __ret.__val[__i] = __a.__val[__i] + __b.__val[__i];
    return __ret;
}

/* vsubq_u16: element-wise subtract uint16x8_t */
static __inline__ uint16x8_t __attribute__((__always_inline__))
vsubq_u16(uint16x8_t __a, uint16x8_t __b)
{
    uint16x8_t __ret;
    for (int __i = 0; __i < 8; __i++)
        __ret.__val[__i] = __a.__val[__i] - __b.__val[__i];
    return __ret;
}

/* vsubq_s16: element-wise subtract int16x8_t */
static __inline__ int16x8_t __attribute__((__always_inline__))
vsubq_s16(int16x8_t __a, int16x8_t __b)
{
    int16x8_t __ret;
    for (int __i = 0; __i < 8; __i++)
        __ret.__val[__i] = __a.__val[__i] - __b.__val[__i];
    return __ret;
}

/* vsubq_u32: element-wise subtract uint32x4_t */
static __inline__ uint32x4_t __attribute__((__always_inline__))
vsubq_u32(uint32x4_t __a, uint32x4_t __b)
{
    uint32x4_t __ret;
    for (int __i = 0; __i < 4; __i++)
        __ret.__val[__i] = __a.__val[__i] - __b.__val[__i];
    return __ret;
}

/* vmaxq_u16: element-wise unsigned max uint16x8_t */
static __inline__ uint16x8_t __attribute__((__always_inline__))
vmaxq_u16(uint16x8_t __a, uint16x8_t __b)
{
    uint16x8_t __ret;
    for (int __i = 0; __i < 8; __i++)
        __ret.__val[__i] = __a.__val[__i] > __b.__val[__i] ? __a.__val[__i] : __b.__val[__i];
    return __ret;
}

/* vminq_u16: element-wise unsigned min uint16x8_t */
static __inline__ uint16x8_t __attribute__((__always_inline__))
vminq_u16(uint16x8_t __a, uint16x8_t __b)
{
    uint16x8_t __ret;
    for (int __i = 0; __i < 8; __i++)
        __ret.__val[__i] = __a.__val[__i] < __b.__val[__i] ? __a.__val[__i] : __b.__val[__i];
    return __ret;
}

/* vminq_s16: element-wise signed min int16x8_t */
static __inline__ int16x8_t __attribute__((__always_inline__))
vminq_s16(int16x8_t __a, int16x8_t __b)
{
    int16x8_t __ret;
    for (int __i = 0; __i < 8; __i++)
        __ret.__val[__i] = __a.__val[__i] < __b.__val[__i] ? __a.__val[__i] : __b.__val[__i];
    return __ret;
}

/* vmulq_u16: element-wise multiply uint16x8_t */
static __inline__ uint16x8_t __attribute__((__always_inline__))
vmulq_u16(uint16x8_t __a, uint16x8_t __b)
{
    uint16x8_t __ret;
    for (int __i = 0; __i < 8; __i++)
        __ret.__val[__i] = __a.__val[__i] * __b.__val[__i];
    return __ret;
}

/* ================================================================== */
/*              u16 COMPARE INTRINSICS                                */
/* ================================================================== */

/* vcgtzq_s16: compare greater than zero, int16x8_t -> uint16x8_t */
static __inline__ uint16x8_t __attribute__((__always_inline__))
vcgtzq_s16(int16x8_t __a)
{
    uint16x8_t __ret;
    for (int __i = 0; __i < 8; __i++)
        __ret.__val[__i] = (__a.__val[__i] > 0) ? 0xFFFF : 0x0000;
    return __ret;
}

/* vcgtzq_s32: compare greater than zero, int32x4_t -> uint32x4_t */
static __inline__ uint32x4_t __attribute__((__always_inline__))
vcgtzq_s32(int32x4_t __a)
{
    uint32x4_t __ret;
    for (int __i = 0; __i < 4; __i++)
        __ret.__val[__i] = (__a.__val[__i] > 0) ? 0xFFFFFFFF : 0x00000000;
    return __ret;
}

/* vcltzq_s16: compare less than zero, int16x8_t -> uint16x8_t */
static __inline__ uint16x8_t __attribute__((__always_inline__))
vcltzq_s16(int16x8_t __a)
{
    uint16x8_t __ret;
    for (int __i = 0; __i < 8; __i++)
        __ret.__val[__i] = (__a.__val[__i] < 0) ? 0xFFFF : 0x0000;
    return __ret;
}

/* vtstq_u16: test bits, uint16x8_t */
static __inline__ uint16x8_t __attribute__((__always_inline__))
vtstq_u16(uint16x8_t __a, uint16x8_t __b)
{
    uint16x8_t __ret;
    for (int __i = 0; __i < 8; __i++)
        __ret.__val[__i] = (__a.__val[__i] & __b.__val[__i]) ? 0xFFFF : 0x0000;
    return __ret;
}

/* vtstq_u32: test bits, uint32x4_t */
static __inline__ uint32x4_t __attribute__((__always_inline__))
vtstq_u32(uint32x4_t __a, uint32x4_t __b)
{
    uint32x4_t __ret;
    for (int __i = 0; __i < 4; __i++)
        __ret.__val[__i] = (__a.__val[__i] & __b.__val[__i]) ? 0xFFFFFFFF : 0x00000000;
    return __ret;
}

/* ================================================================== */
/*              BITWISE SELECT (vbslq)                                */
/* ================================================================== */

/* vbslq_u8: bitwise select, uint8x16_t */
static __inline__ uint8x16_t __attribute__((__always_inline__))
vbslq_u8(uint8x16_t __sel, uint8x16_t __a, uint8x16_t __b)
{
    uint8x16_t __ret;
    for (int __i = 0; __i < 16; __i++)
        __ret.__val[__i] = (__sel.__val[__i] & __a.__val[__i]) |
                           (~__sel.__val[__i] & __b.__val[__i]);
    return __ret;
}

/* vbslq_u16: bitwise select, uint16x8_t */
static __inline__ uint16x8_t __attribute__((__always_inline__))
vbslq_u16(uint16x8_t __sel, uint16x8_t __a, uint16x8_t __b)
{
    uint16x8_t __ret;
    for (int __i = 0; __i < 8; __i++)
        __ret.__val[__i] = (__sel.__val[__i] & __a.__val[__i]) |
                           (~__sel.__val[__i] & __b.__val[__i]);
    return __ret;
}

/* vbslq_u32: bitwise select, uint32x4_t */
static __inline__ uint32x4_t __attribute__((__always_inline__))
vbslq_u32(uint32x4_t __sel, uint32x4_t __a, uint32x4_t __b)
{
    uint32x4_t __ret;
    for (int __i = 0; __i < 4; __i++)
        __ret.__val[__i] = (__sel.__val[__i] & __a.__val[__i]) |
                           (~__sel.__val[__i] & __b.__val[__i]);
    return __ret;
}

/* vbslq_u64: bitwise select, uint64x2_t */
static __inline__ uint64x2_t __attribute__((__always_inline__))
vbslq_u64(uint64x2_t __sel, uint64x2_t __a, uint64x2_t __b)
{
    uint64x2_t __ret;
    for (int __i = 0; __i < 2; __i++)
        __ret.__val[__i] = (__sel.__val[__i] & __a.__val[__i]) |
                           (~__sel.__val[__i] & __b.__val[__i]);
    return __ret;
}

/* vbslq_s8: bitwise select, int8x16_t (using u8 bitwise ops) */
static __inline__ int8x16_t __attribute__((__always_inline__))
vbslq_s8(uint8x16_t __sel, int8x16_t __a, int8x16_t __b)
{
    int8x16_t __ret;
    for (int __i = 0; __i < 16; __i++)
        __ret.__val[__i] = (__sel.__val[__i] & (unsigned char)__a.__val[__i]) |
                           (~__sel.__val[__i] & (unsigned char)__b.__val[__i]);
    return __ret;
}

/* vbslq_s16: bitwise select, int16x8_t (using u16 bitwise ops) */
static __inline__ int16x8_t __attribute__((__always_inline__))
vbslq_s16(uint16x8_t __sel, int16x8_t __a, int16x8_t __b)
{
    int16x8_t __ret;
    for (int __i = 0; __i < 8; __i++)
        __ret.__val[__i] = (__sel.__val[__i] & (unsigned short)__a.__val[__i]) |
                           (~__sel.__val[__i] & (unsigned short)__b.__val[__i]);
    return __ret;
}

/* vbslq_s32: bitwise select, int32x4_t (using u32 bitwise ops) */
static __inline__ int32x4_t __attribute__((__always_inline__))
vbslq_s32(uint32x4_t __sel, int32x4_t __a, int32x4_t __b)
{
    int32x4_t __ret;
    for (int __i = 0; __i < 4; __i++)
        __ret.__val[__i] = (__sel.__val[__i] & (unsigned int)__a.__val[__i]) |
                           (~__sel.__val[__i] & (unsigned int)__b.__val[__i]);
    return __ret;
}

/* ================================================================== */
/*              EXTRACT (vextq_u16)                                   */
/* ================================================================== */

/* vextq_u16: extract from pair of uint16x8_t */
#define vextq_u16(__a, __b, __n) __extension__ ({ \
    uint16x8_t __r; \
    unsigned short __tmp[16]; \
    __tmp[0] = (__a).__val[0]; __tmp[1] = (__a).__val[1]; \
    __tmp[2] = (__a).__val[2]; __tmp[3] = (__a).__val[3]; \
    __tmp[4] = (__a).__val[4]; __tmp[5] = (__a).__val[5]; \
    __tmp[6] = (__a).__val[6]; __tmp[7] = (__a).__val[7]; \
    __tmp[8] = (__b).__val[0]; __tmp[9] = (__b).__val[1]; \
    __tmp[10] = (__b).__val[2]; __tmp[11] = (__b).__val[3]; \
    __tmp[12] = (__b).__val[4]; __tmp[13] = (__b).__val[5]; \
    __tmp[14] = (__b).__val[6]; __tmp[15] = (__b).__val[7]; \
    __r.__val[0] = __tmp[(__n)]; __r.__val[1] = __tmp[(__n) + 1]; \
    __r.__val[2] = __tmp[(__n) + 2]; __r.__val[3] = __tmp[(__n) + 3]; \
    __r.__val[4] = __tmp[(__n) + 4]; __r.__val[5] = __tmp[(__n) + 5]; \
    __r.__val[6] = __tmp[(__n) + 6]; __r.__val[7] = __tmp[(__n) + 7]; \
    __r; \
})

/* vextq_s16: extract from pair of int16x8_t */
#define vextq_s16(__a, __b, __n) __extension__ ({ \
    int16x8_t __r; \
    short __tmp[16]; \
    __tmp[0] = (__a).__val[0]; __tmp[1] = (__a).__val[1]; \
    __tmp[2] = (__a).__val[2]; __tmp[3] = (__a).__val[3]; \
    __tmp[4] = (__a).__val[4]; __tmp[5] = (__a).__val[5]; \
    __tmp[6] = (__a).__val[6]; __tmp[7] = (__a).__val[7]; \
    __tmp[8] = (__b).__val[0]; __tmp[9] = (__b).__val[1]; \
    __tmp[10] = (__b).__val[2]; __tmp[11] = (__b).__val[3]; \
    __tmp[12] = (__b).__val[4]; __tmp[13] = (__b).__val[5]; \
    __tmp[14] = (__b).__val[6]; __tmp[15] = (__b).__val[7]; \
    __r.__val[0] = __tmp[(__n)]; __r.__val[1] = __tmp[(__n) + 1]; \
    __r.__val[2] = __tmp[(__n) + 2]; __r.__val[3] = __tmp[(__n) + 3]; \
    __r.__val[4] = __tmp[(__n) + 4]; __r.__val[5] = __tmp[(__n) + 5]; \
    __r.__val[6] = __tmp[(__n) + 6]; __r.__val[7] = __tmp[(__n) + 7]; \
    __r; \
})

/* ================================================================== */
/*              TABLE LOOKUP (vqtbl2q)                                */
/* ================================================================== */

/* vqtbl2q_u8: table lookup across 2 registers (32 bytes) */
static __inline__ uint8x16_t __attribute__((__always_inline__))
vqtbl2q_u8(uint8x16x2_t __a, uint8x16_t __b)
{
    uint8x16_t __ret;
    for (int __i = 0; __i < 16; __i++) {
        unsigned char __idx = __b.__val[__i];
        if (__idx < 16)
            __ret.__val[__i] = __a.val[0].__val[__idx];
        else if (__idx < 32)
            __ret.__val[__i] = __a.val[1].__val[__idx - 16];
        else
            __ret.__val[__i] = 0;
    }
    return __ret;
}

/* vqtbx2q_u8: table lookup extend across 2 registers */
static __inline__ uint8x16_t __attribute__((__always_inline__))
vqtbx2q_u8(uint8x16_t __def, uint8x16x2_t __a, uint8x16_t __b)
{
    uint8x16_t __ret;
    for (int __i = 0; __i < 16; __i++) {
        unsigned char __idx = __b.__val[__i];
        if (__idx < 16)
            __ret.__val[__i] = __a.val[0].__val[__idx];
        else if (__idx < 32)
            __ret.__val[__i] = __a.val[1].__val[__idx - 16];
        else
            __ret.__val[__i] = __def.__val[__i];
    }
    return __ret;
}

/* vqtbl3q_u8: table lookup across 3 registers (48 bytes) */
static __inline__ uint8x16_t __attribute__((__always_inline__))
vqtbl3q_u8(uint8x16x3_t __a, uint8x16_t __b)
{
    uint8x16_t __ret;
    for (int __i = 0; __i < 16; __i++) {
        unsigned char __idx = __b.__val[__i];
        if (__idx < 16)
            __ret.__val[__i] = __a.val[0].__val[__idx];
        else if (__idx < 32)
            __ret.__val[__i] = __a.val[1].__val[__idx - 16];
        else if (__idx < 48)
            __ret.__val[__i] = __a.val[2].__val[__idx - 32];
        else
            __ret.__val[__i] = 0;
    }
    return __ret;
}

/* ================================================================== */
/*              UNZIP / DE-INTERLEAVE (AArch64 vuzp1/vuzp2)          */
/* ================================================================== */

/* vuzp1q_u16: unzip even elements from two uint16x8_t
 * result = {a[0], a[2], a[4], a[6], b[0], b[2], b[4], b[6]} */
static __inline__ uint16x8_t __attribute__((__always_inline__))
vuzp1q_u16(uint16x8_t __a, uint16x8_t __b)
{
    uint16x8_t __ret;
    __ret.__val[0] = __a.__val[0];
    __ret.__val[1] = __a.__val[2];
    __ret.__val[2] = __a.__val[4];
    __ret.__val[3] = __a.__val[6];
    __ret.__val[4] = __b.__val[0];
    __ret.__val[5] = __b.__val[2];
    __ret.__val[6] = __b.__val[4];
    __ret.__val[7] = __b.__val[6];
    return __ret;
}

/* vuzp2q_u16: unzip odd elements from two uint16x8_t
 * result = {a[1], a[3], a[5], a[7], b[1], b[3], b[5], b[7]} */
static __inline__ uint16x8_t __attribute__((__always_inline__))
vuzp2q_u16(uint16x8_t __a, uint16x8_t __b)
{
    uint16x8_t __ret;
    __ret.__val[0] = __a.__val[1];
    __ret.__val[1] = __a.__val[3];
    __ret.__val[2] = __a.__val[5];
    __ret.__val[3] = __a.__val[7];
    __ret.__val[4] = __b.__val[1];
    __ret.__val[5] = __b.__val[3];
    __ret.__val[6] = __b.__val[5];
    __ret.__val[7] = __b.__val[7];
    return __ret;
}

/* vuzp1q_u8: unzip even elements from two uint8x16_t */
static __inline__ uint8x16_t __attribute__((__always_inline__))
vuzp1q_u8(uint8x16_t __a, uint8x16_t __b)
{
    uint8x16_t __ret;
    for (int __i = 0; __i < 8; __i++) {
        __ret.__val[__i] = __a.__val[__i * 2];
        __ret.__val[__i + 8] = __b.__val[__i * 2];
    }
    return __ret;
}

/* vuzp2q_u8: unzip odd elements from two uint8x16_t */
static __inline__ uint8x16_t __attribute__((__always_inline__))
vuzp2q_u8(uint8x16_t __a, uint8x16_t __b)
{
    uint8x16_t __ret;
    for (int __i = 0; __i < 8; __i++) {
        __ret.__val[__i] = __a.__val[__i * 2 + 1];
        __ret.__val[__i + 8] = __b.__val[__i * 2 + 1];
    }
    return __ret;
}

/* vuzp1q_u32: unzip even elements from two uint32x4_t */
static __inline__ uint32x4_t __attribute__((__always_inline__))
vuzp1q_u32(uint32x4_t __a, uint32x4_t __b)
{
    uint32x4_t __ret;
    __ret.__val[0] = __a.__val[0];
    __ret.__val[1] = __a.__val[2];
    __ret.__val[2] = __b.__val[0];
    __ret.__val[3] = __b.__val[2];
    return __ret;
}

/* vuzp2q_u32: unzip odd elements from two uint32x4_t */
static __inline__ uint32x4_t __attribute__((__always_inline__))
vuzp2q_u32(uint32x4_t __a, uint32x4_t __b)
{
    uint32x4_t __ret;
    __ret.__val[0] = __a.__val[1];
    __ret.__val[1] = __a.__val[3];
    __ret.__val[2] = __b.__val[1];
    __ret.__val[3] = __b.__val[3];
    return __ret;
}

/* vzip1q_u16: zip/interleave low halves of two uint16x8_t
 * result = {a[0], b[0], a[1], b[1], a[2], b[2], a[3], b[3]} */
static __inline__ uint16x8_t __attribute__((__always_inline__))
vzip1q_u16(uint16x8_t __a, uint16x8_t __b)
{
    uint16x8_t __ret;
    __ret.__val[0] = __a.__val[0]; __ret.__val[1] = __b.__val[0];
    __ret.__val[2] = __a.__val[1]; __ret.__val[3] = __b.__val[1];
    __ret.__val[4] = __a.__val[2]; __ret.__val[5] = __b.__val[2];
    __ret.__val[6] = __a.__val[3]; __ret.__val[7] = __b.__val[3];
    return __ret;
}

/* vzip2q_u16: zip/interleave high halves of two uint16x8_t
 * result = {a[4], b[4], a[5], b[5], a[6], b[6], a[7], b[7]} */
static __inline__ uint16x8_t __attribute__((__always_inline__))
vzip2q_u16(uint16x8_t __a, uint16x8_t __b)
{
    uint16x8_t __ret;
    __ret.__val[0] = __a.__val[4]; __ret.__val[1] = __b.__val[4];
    __ret.__val[2] = __a.__val[5]; __ret.__val[3] = __b.__val[5];
    __ret.__val[4] = __a.__val[6]; __ret.__val[5] = __b.__val[6];
    __ret.__val[6] = __a.__val[7]; __ret.__val[7] = __b.__val[7];
    return __ret;
}

/* vzip1q_u32: zip/interleave low halves of two uint32x4_t */
static __inline__ uint32x4_t __attribute__((__always_inline__))
vzip1q_u32(uint32x4_t __a, uint32x4_t __b)
{
    uint32x4_t __ret;
    __ret.__val[0] = __a.__val[0]; __ret.__val[1] = __b.__val[0];
    __ret.__val[2] = __a.__val[1]; __ret.__val[3] = __b.__val[1];
    return __ret;
}

/* vzip2q_u32: zip/interleave high halves of two uint32x4_t */
static __inline__ uint32x4_t __attribute__((__always_inline__))
vzip2q_u32(uint32x4_t __a, uint32x4_t __b)
{
    uint32x4_t __ret;
    __ret.__val[0] = __a.__val[2]; __ret.__val[1] = __b.__val[2];
    __ret.__val[2] = __a.__val[3]; __ret.__val[3] = __b.__val[3];
    return __ret;
}

/* ================================================================== */
/*              ADDITIONAL REINTERPRET CASTS                          */
/* ================================================================== */

/* vreinterpretq_s16_u16: reinterpret uint16x8_t as int16x8_t */
static __inline__ int16x8_t __attribute__((__always_inline__))
vreinterpretq_s16_u16(uint16x8_t __a)
{
    int16x8_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

/* vreinterpretq_u16_s16: reinterpret int16x8_t as uint16x8_t */
static __inline__ uint16x8_t __attribute__((__always_inline__))
vreinterpretq_u16_s16(int16x8_t __a)
{
    uint16x8_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

/* vreinterpretq_u64_u16: reinterpret uint16x8_t as uint64x2_t */
static __inline__ uint64x2_t __attribute__((__always_inline__))
vreinterpretq_u64_u16(uint16x8_t __a)
{
    uint64x2_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

/* vreinterpretq_u16_u64: reinterpret uint64x2_t as uint16x8_t */
static __inline__ uint16x8_t __attribute__((__always_inline__))
vreinterpretq_u16_u64(uint64x2_t __a)
{
    uint16x8_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

/* vreinterpretq_s32_s16: reinterpret int16x8_t as int32x4_t */
static __inline__ int32x4_t __attribute__((__always_inline__))
vreinterpretq_s32_s16(int16x8_t __a)
{
    int32x4_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

/* vreinterpretq_s16_s32: reinterpret int32x4_t as int16x8_t */
static __inline__ int16x8_t __attribute__((__always_inline__))
vreinterpretq_s16_s32(int32x4_t __a)
{
    int16x8_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

/* vreinterpretq_u16_s8: reinterpret int8x16_t as uint16x8_t */
static __inline__ uint16x8_t __attribute__((__always_inline__))
vreinterpretq_u16_s8(int8x16_t __a)
{
    uint16x8_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

/* vreinterpretq_s8_s16: reinterpret int16x8_t as int8x16_t */
static __inline__ int8x16_t __attribute__((__always_inline__))
vreinterpretq_s8_s16(int16x8_t __a)
{
    int8x16_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

/* vreinterpretq_s64_u64: reinterpret uint64x2_t as int64x2_t */
static __inline__ int64x2_t __attribute__((__always_inline__))
vreinterpretq_s64_u64(uint64x2_t __a)
{
    int64x2_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

/* vreinterpretq_u64_s64: reinterpret int64x2_t as uint64x2_t */
static __inline__ uint64x2_t __attribute__((__always_inline__))
vreinterpretq_u64_s64(int64x2_t __a)
{
    uint64x2_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

/* vreinterpretq_s8_u16: reinterpret uint16x8_t as int8x16_t */
static __inline__ int8x16_t __attribute__((__always_inline__))
vreinterpretq_s8_u16(uint16x8_t __a)
{
    int8x16_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

/* vreinterpretq_f32_u32: reinterpret uint32x4_t as float32x4_t */
static __inline__ float32x4_t __attribute__((__always_inline__))
vreinterpretq_f32_u32(uint32x4_t __a)
{
    float32x4_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

/* vreinterpretq_u32_f32: reinterpret float32x4_t as uint32x4_t */
static __inline__ uint32x4_t __attribute__((__always_inline__))
vreinterpretq_u32_f32(float32x4_t __a)
{
    uint32x4_t __ret;
    __builtin_memcpy(&__ret, &__a, 16);
    return __ret;
}

/* ================================================================== */
/*              ADDITIONAL SHIFT INTRINSICS                           */
/* ================================================================== */

/* vshlq_n_u16: shift left by immediate, uint16x8_t */
#define vshlq_n_u16(__a, __n) __extension__ ({ \
    uint16x8_t __r; \
    for (int __i = 0; __i < 8; __i++) \
        __r.__val[__i] = (__a).__val[__i] << (__n); \
    __r; \
})

/* vshrq_n_u16: shift right by immediate, uint16x8_t */
#define vshrq_n_u16(__a, __n) __extension__ ({ \
    uint16x8_t __r; \
    for (int __i = 0; __i < 8; __i++) \
        __r.__val[__i] = (__a).__val[__i] >> (__n); \
    __r; \
})

/* vshlq_n_s16: shift left by immediate, int16x8_t */
#define vshlq_n_s16(__a, __n) __extension__ ({ \
    int16x8_t __r; \
    for (int __i = 0; __i < 8; __i++) \
        __r.__val[__i] = (__a).__val[__i] << (__n); \
    __r; \
})

/* vshrq_n_s16: shift right by immediate, int16x8_t */
#define vshrq_n_s16(__a, __n) __extension__ ({ \
    int16x8_t __r; \
    for (int __i = 0; __i < 8; __i++) \
        __r.__val[__i] = (__a).__val[__i] >> (__n); \
    __r; \
})

/* ================================================================== */
/*         ADDITIONAL WIDENING / NARROWING INTRINSICS                 */
/* ================================================================== */

/* vmovl_u16: widen uint16x4_t to uint32x4_t */
static __inline__ uint32x4_t __attribute__((__always_inline__))
vmovl_u16(uint16x4_t __a)
{
    uint32x4_t __ret;
    __ret.__val[0] = __a.__val[0];
    __ret.__val[1] = __a.__val[1];
    __ret.__val[2] = __a.__val[2];
    __ret.__val[3] = __a.__val[3];
    return __ret;
}

/* vmovl_s16: widen int16x4_t to int32x4_t */
static __inline__ int32x4_t __attribute__((__always_inline__))
vmovl_s16(int16x4_t __a)
{
    int32x4_t __ret;
    __ret.__val[0] = __a.__val[0];
    __ret.__val[1] = __a.__val[1];
    __ret.__val[2] = __a.__val[2];
    __ret.__val[3] = __a.__val[3];
    return __ret;
}

/* vmovl_high_u16: widen high half of uint16x8_t to uint32x4_t */
static __inline__ uint32x4_t __attribute__((__always_inline__))
vmovl_high_u16(uint16x8_t __a)
{
    uint32x4_t __ret;
    __ret.__val[0] = __a.__val[4];
    __ret.__val[1] = __a.__val[5];
    __ret.__val[2] = __a.__val[6];
    __ret.__val[3] = __a.__val[7];
    return __ret;
}

/* vmovl_high_u8: widen high half of uint8x16_t to uint16x8_t */
static __inline__ uint16x8_t __attribute__((__always_inline__))
vmovl_high_u8(uint8x16_t __a)
{
    uint16x8_t __ret;
    for (int __i = 0; __i < 8; __i++)
        __ret.__val[__i] = __a.__val[__i + 8];
    return __ret;
}

/* ================================================================== */
/*              LANE / HALF / COMBINE INTRINSICS (s16)                */
/* ================================================================== */

/* vsetq_lane_s16: set one lane in int16x8_t */
static __inline__ int16x8_t __attribute__((__always_inline__))
vsetq_lane_s16(short __val, int16x8_t __a, int __lane)
{
    __a.__val[__lane] = __val;
    return __a;
}

/* vget_low_u16: get low half of uint16x8_t as uint16x4_t */
static __inline__ uint16x4_t __attribute__((__always_inline__))
vget_low_u16(uint16x8_t __a)
{
    uint16x4_t __ret;
    __ret.__val[0] = __a.__val[0];
    __ret.__val[1] = __a.__val[1];
    __ret.__val[2] = __a.__val[2];
    __ret.__val[3] = __a.__val[3];
    return __ret;
}

/* vget_high_u16: get high half of uint16x8_t as uint16x4_t */
static __inline__ uint16x4_t __attribute__((__always_inline__))
vget_high_u16(uint16x8_t __a)
{
    uint16x4_t __ret;
    __ret.__val[0] = __a.__val[4];
    __ret.__val[1] = __a.__val[5];
    __ret.__val[2] = __a.__val[6];
    __ret.__val[3] = __a.__val[7];
    return __ret;
}


#endif /* _ARM_NEON_H_INCLUDED */
