/* CCC compiler bundled xmmintrin.h - SSE intrinsics */
#ifndef _XMMINTRIN_H_INCLUDED
#define _XMMINTRIN_H_INCLUDED

/* SSE intrinsics are only available on x86/x86-64 targets */
#if !defined(__x86_64__) && !defined(__i386__) && !defined(__i686__)
#error "SSE intrinsics (xmmintrin.h) require an x86 target"
#endif

#include <mmintrin.h>

typedef struct __attribute__((__aligned__(16))) {
    float __val[4];
} __m128;

/* Internal vector type referenced by GCC system headers.
 * Note: vector_size attribute is parsed but vectors are lowered as aggregates. */
typedef float __v4sf __attribute__ ((__vector_size__ (16)));

/* _MM_SHUFFLE: build an immediate for _mm_shuffle_ps / _mm_shuffle_epi32.
 * The result encodes four 2-bit lane selectors as (z<<6|y<<4|x<<2|w). */
#define _MM_SHUFFLE(z, y, x, w) (((z) << 6) | ((y) << 4) | ((x) << 2) | (w))

/* === Set / Broadcast === */

static __inline__ __m128 __attribute__((__always_inline__))
_mm_setzero_ps(void)
{
    return (__m128){ { 0.0f, 0.0f, 0.0f, 0.0f } };
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_set1_ps(float __w)
{
    return (__m128){ { __w, __w, __w, __w } };
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_set_ps(float __z, float __y, float __x, float __w)
{
    return (__m128){ { __w, __x, __y, __z } };
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_setr_ps(float __w, float __x, float __y, float __z)
{
    return (__m128){ { __w, __x, __y, __z } };
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_set_ss(float __w)
{
    return (__m128){ { __w, 0.0f, 0.0f, 0.0f } };
}

/* _mm_set_ps1 is a standard alias for _mm_set1_ps */
#define _mm_set_ps1(w) _mm_set1_ps(w)

/* === Load === */

static __inline__ __m128 __attribute__((__always_inline__))
_mm_loadu_ps(const float *__p)
{
    __m128 __r;
    __builtin_memcpy(&__r, __p, 16);
    return __r;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_load_ps(const float *__p)
{
    return *(const __m128 *)__p;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_load_ss(const float *__p)
{
    return (__m128){ { *__p, 0.0f, 0.0f, 0.0f } };
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_load1_ps(const float *__p)
{
    float __v = *__p;
    return (__m128){ { __v, __v, __v, __v } };
}

#define _mm_load_ps1(p) _mm_load1_ps(p)

/* === Store === */

static __inline__ void __attribute__((__always_inline__))
_mm_storeu_ps(float *__p, __m128 __a)
{
    __builtin_memcpy(__p, &__a, 16);
}

static __inline__ void __attribute__((__always_inline__))
_mm_store_ps(float *__p, __m128 __a)
{
    *((__m128 *)__p) = __a;
}

static __inline__ void __attribute__((__always_inline__))
_mm_store_ss(float *__p, __m128 __a)
{
    *__p = __a.__val[0];
}

static __inline__ void __attribute__((__always_inline__))
_mm_store1_ps(float *__p, __m128 __a)
{
    __p[0] = __a.__val[0]; __p[1] = __a.__val[0];
    __p[2] = __a.__val[0]; __p[3] = __a.__val[0];
}

#define _mm_store_ps1(p, a) _mm_store1_ps(p, a)

/* Non-temporal store of 128-bit float vector (MOVNTPS).
 * Implemented as a regular aligned store (non-temporal hint is optimization only). */
static __inline__ void __attribute__((__always_inline__))
_mm_stream_ps(float *__p, __m128 __a)
{
    *((__m128 *)__p) = __a;
}

/* Store the lower 2 floats of __m128 to __m64* memory location. */
static __inline__ void __attribute__((__always_inline__))
_mm_storel_pi(__m64 *__p, __m128 __a)
{
    __builtin_memcpy(__p, &__a, 8);
}

/* Store the upper 2 floats of __m128 to __m64* memory location. */
static __inline__ void __attribute__((__always_inline__))
_mm_storeh_pi(__m64 *__p, __m128 __a)
{
    __builtin_memcpy(__p, (const char *)&__a + 8, 8);
}

/* === Arithmetic === */

static __inline__ __m128 __attribute__((__always_inline__))
_mm_add_ps(__m128 __a, __m128 __b)
{
    return (__m128){ { __a.__val[0] + __b.__val[0], __a.__val[1] + __b.__val[1],
                       __a.__val[2] + __b.__val[2], __a.__val[3] + __b.__val[3] } };
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_sub_ps(__m128 __a, __m128 __b)
{
    return (__m128){ { __a.__val[0] - __b.__val[0], __a.__val[1] - __b.__val[1],
                       __a.__val[2] - __b.__val[2], __a.__val[3] - __b.__val[3] } };
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_mul_ps(__m128 __a, __m128 __b)
{
    return (__m128){ { __a.__val[0] * __b.__val[0], __a.__val[1] * __b.__val[1],
                       __a.__val[2] * __b.__val[2], __a.__val[3] * __b.__val[3] } };
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_div_ps(__m128 __a, __m128 __b)
{
    return (__m128){ { __a.__val[0] / __b.__val[0], __a.__val[1] / __b.__val[1],
                       __a.__val[2] / __b.__val[2], __a.__val[3] / __b.__val[3] } };
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_min_ps(__m128 __a, __m128 __b)
{
    return (__m128){ { __a.__val[0] < __b.__val[0] ? __a.__val[0] : __b.__val[0],
                       __a.__val[1] < __b.__val[1] ? __a.__val[1] : __b.__val[1],
                       __a.__val[2] < __b.__val[2] ? __a.__val[2] : __b.__val[2],
                       __a.__val[3] < __b.__val[3] ? __a.__val[3] : __b.__val[3] } };
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_max_ps(__m128 __a, __m128 __b)
{
    return (__m128){ { __a.__val[0] > __b.__val[0] ? __a.__val[0] : __b.__val[0],
                       __a.__val[1] > __b.__val[1] ? __a.__val[1] : __b.__val[1],
                       __a.__val[2] > __b.__val[2] ? __a.__val[2] : __b.__val[2],
                       __a.__val[3] > __b.__val[3] ? __a.__val[3] : __b.__val[3] } };
}

/* Scalar operations (lowest element only, rest pass through __a) */

static __inline__ __m128 __attribute__((__always_inline__))
_mm_add_ss(__m128 __a, __m128 __b)
{
    __a.__val[0] += __b.__val[0];
    return __a;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_sub_ss(__m128 __a, __m128 __b)
{
    __a.__val[0] -= __b.__val[0];
    return __a;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_mul_ss(__m128 __a, __m128 __b)
{
    __a.__val[0] *= __b.__val[0];
    return __a;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_div_ss(__m128 __a, __m128 __b)
{
    __a.__val[0] /= __b.__val[0];
    return __a;
}

/* === Bitwise (float domain) === */
/* These operate on the bitwise representation of float values,
   using memcpy to type-pun between float and unsigned int. */

static __inline__ __m128 __attribute__((__always_inline__))
_mm_and_ps(__m128 __a, __m128 __b)
{
    unsigned int __ai[4], __bi[4];
    __builtin_memcpy(__ai, &__a, 16);
    __builtin_memcpy(__bi, &__b, 16);
    __ai[0] &= __bi[0]; __ai[1] &= __bi[1];
    __ai[2] &= __bi[2]; __ai[3] &= __bi[3];
    __m128 __r;
    __builtin_memcpy(&__r, __ai, 16);
    return __r;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_andnot_ps(__m128 __a, __m128 __b)
{
    unsigned int __ai[4], __bi[4];
    __builtin_memcpy(__ai, &__a, 16);
    __builtin_memcpy(__bi, &__b, 16);
    __ai[0] = ~__ai[0] & __bi[0]; __ai[1] = ~__ai[1] & __bi[1];
    __ai[2] = ~__ai[2] & __bi[2]; __ai[3] = ~__ai[3] & __bi[3];
    __m128 __r;
    __builtin_memcpy(&__r, __ai, 16);
    return __r;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_or_ps(__m128 __a, __m128 __b)
{
    unsigned int __ai[4], __bi[4];
    __builtin_memcpy(__ai, &__a, 16);
    __builtin_memcpy(__bi, &__b, 16);
    __ai[0] |= __bi[0]; __ai[1] |= __bi[1];
    __ai[2] |= __bi[2]; __ai[3] |= __bi[3];
    __m128 __r;
    __builtin_memcpy(&__r, __ai, 16);
    return __r;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_xor_ps(__m128 __a, __m128 __b)
{
    unsigned int __ai[4], __bi[4];
    __builtin_memcpy(__ai, &__a, 16);
    __builtin_memcpy(__bi, &__b, 16);
    __ai[0] ^= __bi[0]; __ai[1] ^= __bi[1];
    __ai[2] ^= __bi[2]; __ai[3] ^= __bi[3];
    __m128 __r;
    __builtin_memcpy(&__r, __ai, 16);
    return __r;
}

/* === Square root, Reciprocal, Reciprocal square root === */

static __inline__ __m128 __attribute__((__always_inline__))
_mm_sqrt_ps(__m128 __a)
{
    return (__m128){ { __builtin_sqrtf(__a.__val[0]), __builtin_sqrtf(__a.__val[1]),
                       __builtin_sqrtf(__a.__val[2]), __builtin_sqrtf(__a.__val[3]) } };
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_sqrt_ss(__m128 __a)
{
    __a.__val[0] = __builtin_sqrtf(__a.__val[0]);
    return __a;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_rcp_ps(__m128 __a)
{
    return (__m128){ { 1.0f / __a.__val[0], 1.0f / __a.__val[1],
                       1.0f / __a.__val[2], 1.0f / __a.__val[3] } };
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_rcp_ss(__m128 __a)
{
    __a.__val[0] = 1.0f / __a.__val[0];
    return __a;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_rsqrt_ps(__m128 __a)
{
    return (__m128){ { 1.0f / __builtin_sqrtf(__a.__val[0]), 1.0f / __builtin_sqrtf(__a.__val[1]),
                       1.0f / __builtin_sqrtf(__a.__val[2]), 1.0f / __builtin_sqrtf(__a.__val[3]) } };
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_rsqrt_ss(__m128 __a)
{
    __a.__val[0] = 1.0f / __builtin_sqrtf(__a.__val[0]);
    return __a;
}

/* === Comparison (packed) - return all-ones or all-zeros per lane === */

static __inline__ __m128 __attribute__((__always_inline__))
_mm_cmpeq_ps(__m128 __a, __m128 __b)
{
    unsigned int __r[4];
    __r[0] = __a.__val[0] == __b.__val[0] ? 0xFFFFFFFFu : 0;
    __r[1] = __a.__val[1] == __b.__val[1] ? 0xFFFFFFFFu : 0;
    __r[2] = __a.__val[2] == __b.__val[2] ? 0xFFFFFFFFu : 0;
    __r[3] = __a.__val[3] == __b.__val[3] ? 0xFFFFFFFFu : 0;
    __m128 __rv;
    __builtin_memcpy(&__rv, __r, 16);
    return __rv;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_cmplt_ps(__m128 __a, __m128 __b)
{
    unsigned int __r[4];
    __r[0] = __a.__val[0] < __b.__val[0] ? 0xFFFFFFFFu : 0;
    __r[1] = __a.__val[1] < __b.__val[1] ? 0xFFFFFFFFu : 0;
    __r[2] = __a.__val[2] < __b.__val[2] ? 0xFFFFFFFFu : 0;
    __r[3] = __a.__val[3] < __b.__val[3] ? 0xFFFFFFFFu : 0;
    __m128 __rv;
    __builtin_memcpy(&__rv, __r, 16);
    return __rv;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_cmple_ps(__m128 __a, __m128 __b)
{
    unsigned int __r[4];
    __r[0] = __a.__val[0] <= __b.__val[0] ? 0xFFFFFFFFu : 0;
    __r[1] = __a.__val[1] <= __b.__val[1] ? 0xFFFFFFFFu : 0;
    __r[2] = __a.__val[2] <= __b.__val[2] ? 0xFFFFFFFFu : 0;
    __r[3] = __a.__val[3] <= __b.__val[3] ? 0xFFFFFFFFu : 0;
    __m128 __rv;
    __builtin_memcpy(&__rv, __r, 16);
    return __rv;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_cmpgt_ps(__m128 __a, __m128 __b)
{
    return _mm_cmplt_ps(__b, __a);
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_cmpge_ps(__m128 __a, __m128 __b)
{
    return _mm_cmple_ps(__b, __a);
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_cmpneq_ps(__m128 __a, __m128 __b)
{
    unsigned int __r[4];
    __r[0] = __a.__val[0] != __b.__val[0] ? 0xFFFFFFFFu : 0;
    __r[1] = __a.__val[1] != __b.__val[1] ? 0xFFFFFFFFu : 0;
    __r[2] = __a.__val[2] != __b.__val[2] ? 0xFFFFFFFFu : 0;
    __r[3] = __a.__val[3] != __b.__val[3] ? 0xFFFFFFFFu : 0;
    __m128 __rv;
    __builtin_memcpy(&__rv, __r, 16);
    return __rv;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_cmpord_ps(__m128 __a, __m128 __b)
{
    unsigned int __r[4];
    __r[0] = (__a.__val[0] == __a.__val[0] && __b.__val[0] == __b.__val[0]) ? 0xFFFFFFFFu : 0;
    __r[1] = (__a.__val[1] == __a.__val[1] && __b.__val[1] == __b.__val[1]) ? 0xFFFFFFFFu : 0;
    __r[2] = (__a.__val[2] == __a.__val[2] && __b.__val[2] == __b.__val[2]) ? 0xFFFFFFFFu : 0;
    __r[3] = (__a.__val[3] == __a.__val[3] && __b.__val[3] == __b.__val[3]) ? 0xFFFFFFFFu : 0;
    __m128 __rv;
    __builtin_memcpy(&__rv, __r, 16);
    return __rv;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_cmpunord_ps(__m128 __a, __m128 __b)
{
    unsigned int __r[4];
    __r[0] = (__a.__val[0] != __a.__val[0] || __b.__val[0] != __b.__val[0]) ? 0xFFFFFFFFu : 0;
    __r[1] = (__a.__val[1] != __a.__val[1] || __b.__val[1] != __b.__val[1]) ? 0xFFFFFFFFu : 0;
    __r[2] = (__a.__val[2] != __a.__val[2] || __b.__val[2] != __b.__val[2]) ? 0xFFFFFFFFu : 0;
    __r[3] = (__a.__val[3] != __a.__val[3] || __b.__val[3] != __b.__val[3]) ? 0xFFFFFFFFu : 0;
    __m128 __rv;
    __builtin_memcpy(&__rv, __r, 16);
    return __rv;
}

/* Scalar comparison intrinsics (operate on element 0 only, rest pass through __a) */

static __inline__ __m128 __attribute__((__always_inline__))
_mm_cmpeq_ss(__m128 __a, __m128 __b)
{
    unsigned int __u = (__a.__val[0] == __b.__val[0]) ? 0xFFFFFFFFu : 0;
    __builtin_memcpy(&__a.__val[0], &__u, 4);
    return __a;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_cmplt_ss(__m128 __a, __m128 __b)
{
    unsigned int __u = (__a.__val[0] < __b.__val[0]) ? 0xFFFFFFFFu : 0;
    __builtin_memcpy(&__a.__val[0], &__u, 4);
    return __a;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_cmple_ss(__m128 __a, __m128 __b)
{
    unsigned int __u = (__a.__val[0] <= __b.__val[0]) ? 0xFFFFFFFFu : 0;
    __builtin_memcpy(&__a.__val[0], &__u, 4);
    return __a;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_cmpgt_ss(__m128 __a, __m128 __b)
{
    unsigned int __u = (__a.__val[0] > __b.__val[0]) ? 0xFFFFFFFFu : 0;
    __builtin_memcpy(&__a.__val[0], &__u, 4);
    return __a;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_cmpge_ss(__m128 __a, __m128 __b)
{
    unsigned int __u = (__a.__val[0] >= __b.__val[0]) ? 0xFFFFFFFFu : 0;
    __builtin_memcpy(&__a.__val[0], &__u, 4);
    return __a;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_cmpneq_ss(__m128 __a, __m128 __b)
{
    unsigned int __u = (__a.__val[0] != __b.__val[0]) ? 0xFFFFFFFFu : 0;
    __builtin_memcpy(&__a.__val[0], &__u, 4);
    return __a;
}

/* === Integer conversion === */

/* TODO: _mm_cvtss_si32 should use current MXCSR rounding mode (round-to-nearest
   by default), but we use C cast truncation for simplicity. This matches
   _mm_cvttss_si32 behavior. */
static __inline__ int __attribute__((__always_inline__))
_mm_cvtss_si32(__m128 __a)
{
    return (int)__a.__val[0];
}

/* Alias: _mm_cvt_ss2si is standard alias for _mm_cvtss_si32 */
#define _mm_cvt_ss2si(a) _mm_cvtss_si32(a)

static __inline__ int __attribute__((__always_inline__))
_mm_cvttss_si32(__m128 __a)
{
    return (int)__a.__val[0];
}

/* Alias: _mm_cvtt_ss2si */
#define _mm_cvtt_ss2si(a) _mm_cvttss_si32(a)

static __inline__ __m128 __attribute__((__always_inline__))
_mm_cvtsi32_ss(__m128 __a, int __b)
{
    __a.__val[0] = (float)__b;
    return __a;
}

/* Alias: _mm_cvt_si2ss */
#define _mm_cvt_si2ss(a, b) _mm_cvtsi32_ss(a, b)

/* === Shuffle === */

/* _mm_shuffle_ps: shuffle floats from __a and __b using immediate mask.
 * Bits [1:0] select from __a for element 0, [3:2] for element 1,
 * [5:4] select from __b for element 2, [7:6] for element 3. */
#define _mm_shuffle_ps(__a, __b, __imm) __extension__ ({ \
    __m128 __r; \
    __r.__val[0] = (__a).__val[(__imm) & 3]; \
    __r.__val[1] = (__a).__val[((__imm) >> 2) & 3]; \
    __r.__val[2] = (__b).__val[((__imm) >> 4) & 3]; \
    __r.__val[3] = (__b).__val[((__imm) >> 6) & 3]; \
    __r; \
})

/* === Unpack / Interleave === */

static __inline__ __m128 __attribute__((__always_inline__))
_mm_unpacklo_ps(__m128 __a, __m128 __b)
{
    return (__m128){ { __a.__val[0], __b.__val[0], __a.__val[1], __b.__val[1] } };
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_unpackhi_ps(__m128 __a, __m128 __b)
{
    return (__m128){ { __a.__val[2], __b.__val[2], __a.__val[3], __b.__val[3] } };
}

/* === Move === */

static __inline__ __m128 __attribute__((__always_inline__))
_mm_movehl_ps(__m128 __a, __m128 __b)
{
    return (__m128){ { __b.__val[2], __b.__val[3], __a.__val[2], __a.__val[3] } };
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_movelh_ps(__m128 __a, __m128 __b)
{
    return (__m128){ { __a.__val[0], __a.__val[1], __b.__val[0], __b.__val[1] } };
}

static __inline__ float __attribute__((__always_inline__))
_mm_cvtss_f32(__m128 __a)
{
    return __a.__val[0];
}

/* === Compare (packed) - return all-ones or all-zeros per lane === */

static __inline__ int __attribute__((__always_inline__))
_mm_movemask_ps(__m128 __a)
{
    int __r = 0;
    unsigned int __u;
    __builtin_memcpy(&__u, &__a.__val[0], 4); __r |= (__u >> 31);
    __builtin_memcpy(&__u, &__a.__val[1], 4); __r |= ((__u >> 31) << 1);
    __builtin_memcpy(&__u, &__a.__val[2], 4); __r |= ((__u >> 31) << 2);
    __builtin_memcpy(&__u, &__a.__val[3], 4); __r |= ((__u >> 31) << 3);
    return __r;
}

/* === Prefetch === */

/* Prefetch hint constants */
#define _MM_HINT_T0  3
#define _MM_HINT_T1  2
#define _MM_HINT_T2  1
#define _MM_HINT_NTA 0

/* _mm_prefetch: hint to prefetch data into cache.
 * In our implementation this is a no-op since we don't emit prefetch
 * instructions, but it must be defined for source compatibility. */
#define _mm_prefetch(P, I) ((void)(P), (void)(I))

/* === Aligned memory allocation === */

static __inline__ void *__attribute__((__always_inline__))
_mm_malloc(unsigned long __size, unsigned long __align)
{
    void *__ptr;
    if (__align <= sizeof(void *))
        return __builtin_malloc(__size);
    /* Use posix_memalign for aligned allocation */
    if (__size == 0)
        return (void *)0;
    /* Manually align: allocate extra space for alignment and store original pointer */
    void *__raw = __builtin_malloc(__size + __align + sizeof(void *));
    if (!__raw)
        return (void *)0;
    __ptr = (void *)(((unsigned long)((char *)__raw + sizeof(void *) + __align - 1)) & ~(__align - 1));
    ((void **)__ptr)[-1] = __raw;
    return __ptr;
}

static __inline__ void __attribute__((__always_inline__))
_mm_free(void *__ptr)
{
    if (__ptr)
        __builtin_free(((void **)__ptr)[-1]);
}

/* === Fence === */

static __inline__ void __attribute__((__always_inline__))
_mm_sfence(void)
{
    __builtin_ia32_sfence();
}

static __inline__ void __attribute__((__always_inline__))
_mm_pause(void)
{
    __builtin_ia32_pause();
}

/* === MXCSR control/status register === */

/* Read the MXCSR register */
static __inline__ unsigned int __attribute__((__always_inline__))
_mm_getcsr(void)
{
    unsigned int __csr;
    __asm__ __volatile__("stmxcsr %0" : "=m" (__csr));
    return __csr;
}

/* Write the MXCSR register */
static __inline__ void __attribute__((__always_inline__))
_mm_setcsr(unsigned int __csr)
{
    __asm__ __volatile__("ldmxcsr %0" : : "m" (__csr));
}

/* Exception state bits (bits 0-5 of MXCSR) */
#define _MM_EXCEPT_INVALID    0x0001
#define _MM_EXCEPT_DENORM     0x0002
#define _MM_EXCEPT_DIV_ZERO   0x0004
#define _MM_EXCEPT_OVERFLOW   0x0008
#define _MM_EXCEPT_UNDERFLOW  0x0010
#define _MM_EXCEPT_INEXACT    0x0020
#define _MM_EXCEPT_MASK       0x003f

/* Exception mask bits (bits 7-12 of MXCSR) */
#define _MM_MASK_INVALID      0x0080
#define _MM_MASK_DENORM       0x0100
#define _MM_MASK_DIV_ZERO     0x0200
#define _MM_MASK_OVERFLOW     0x0400
#define _MM_MASK_UNDERFLOW    0x0800
#define _MM_MASK_INEXACT      0x1000
#define _MM_MASK_MASK         0x1f80

/* Rounding mode bits (bits 13-14 of MXCSR) */
#define _MM_ROUND_NEAREST     0x0000
#define _MM_ROUND_DOWN        0x2000
#define _MM_ROUND_UP          0x4000
#define _MM_ROUND_TOWARD_ZERO 0x6000
#define _MM_ROUND_MASK        0x6000

/* Flush-to-zero bit (bit 15 of MXCSR) */
#define _MM_FLUSH_ZERO_MASK   0x8000
#define _MM_FLUSH_ZERO_ON     0x8000
#define _MM_FLUSH_ZERO_OFF    0x0000

/* Get/set exception state */
static __inline__ unsigned int __attribute__((__always_inline__))
_MM_GET_EXCEPTION_STATE(void)
{
    return _mm_getcsr() & _MM_EXCEPT_MASK;
}

static __inline__ void __attribute__((__always_inline__))
_MM_SET_EXCEPTION_STATE(unsigned int __mask)
{
    _mm_setcsr((_mm_getcsr() & ~_MM_EXCEPT_MASK) | __mask);
}

/* Get/set exception mask */
static __inline__ unsigned int __attribute__((__always_inline__))
_MM_GET_EXCEPTION_MASK(void)
{
    return _mm_getcsr() & _MM_MASK_MASK;
}

static __inline__ void __attribute__((__always_inline__))
_MM_SET_EXCEPTION_MASK(unsigned int __mask)
{
    _mm_setcsr((_mm_getcsr() & ~_MM_MASK_MASK) | __mask);
}

/* Get/set rounding mode */
static __inline__ unsigned int __attribute__((__always_inline__))
_MM_GET_ROUNDING_MODE(void)
{
    return _mm_getcsr() & _MM_ROUND_MASK;
}

static __inline__ void __attribute__((__always_inline__))
_MM_SET_ROUNDING_MODE(unsigned int __mode)
{
    _mm_setcsr((_mm_getcsr() & ~_MM_ROUND_MASK) | __mode);
}

/* Get/set flush-to-zero mode */
static __inline__ unsigned int __attribute__((__always_inline__))
_MM_GET_FLUSH_ZERO_MODE(void)
{
    return _mm_getcsr() & _MM_FLUSH_ZERO_MASK;
}

static __inline__ void __attribute__((__always_inline__))
_MM_SET_FLUSH_ZERO_MODE(unsigned int __mode)
{
    _mm_setcsr((_mm_getcsr() & ~_MM_FLUSH_ZERO_MASK) | __mode);
}

/* === Scalar min/max === */

static __inline__ __m128 __attribute__((__always_inline__))
_mm_min_ss(__m128 __a, __m128 __b)
{
    __a.__val[0] = __a.__val[0] < __b.__val[0] ? __a.__val[0] : __b.__val[0];
    return __a;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_max_ss(__m128 __a, __m128 __b)
{
    __a.__val[0] = __a.__val[0] > __b.__val[0] ? __a.__val[0] : __b.__val[0];
    return __a;
}

/* === Move === */

static __inline__ __m128 __attribute__((__always_inline__))
_mm_move_ss(__m128 __a, __m128 __b)
{
    __a.__val[0] = __b.__val[0];
    return __a;
}

/* === Negated comparisons (packed) === */

static __inline__ __m128 __attribute__((__always_inline__))
_mm_cmpnlt_ps(__m128 __a, __m128 __b)
{
    unsigned int __r[4];
    __r[0] = !(__a.__val[0] < __b.__val[0]) ? 0xFFFFFFFFu : 0;
    __r[1] = !(__a.__val[1] < __b.__val[1]) ? 0xFFFFFFFFu : 0;
    __r[2] = !(__a.__val[2] < __b.__val[2]) ? 0xFFFFFFFFu : 0;
    __r[3] = !(__a.__val[3] < __b.__val[3]) ? 0xFFFFFFFFu : 0;
    __m128 __rv;
    __builtin_memcpy(&__rv, __r, 16);
    return __rv;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_cmpnle_ps(__m128 __a, __m128 __b)
{
    unsigned int __r[4];
    __r[0] = !(__a.__val[0] <= __b.__val[0]) ? 0xFFFFFFFFu : 0;
    __r[1] = !(__a.__val[1] <= __b.__val[1]) ? 0xFFFFFFFFu : 0;
    __r[2] = !(__a.__val[2] <= __b.__val[2]) ? 0xFFFFFFFFu : 0;
    __r[3] = !(__a.__val[3] <= __b.__val[3]) ? 0xFFFFFFFFu : 0;
    __m128 __rv;
    __builtin_memcpy(&__rv, __r, 16);
    return __rv;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_cmpngt_ps(__m128 __a, __m128 __b)
{
    return _mm_cmpnlt_ps(__b, __a);
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_cmpnge_ps(__m128 __a, __m128 __b)
{
    return _mm_cmpnle_ps(__b, __a);
}

/* === Negated scalar comparisons === */

static __inline__ __m128 __attribute__((__always_inline__))
_mm_cmpnlt_ss(__m128 __a, __m128 __b)
{
    unsigned int __u = !(__a.__val[0] < __b.__val[0]) ? 0xFFFFFFFFu : 0;
    __builtin_memcpy(&__a.__val[0], &__u, 4);
    return __a;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_cmpnle_ss(__m128 __a, __m128 __b)
{
    unsigned int __u = !(__a.__val[0] <= __b.__val[0]) ? 0xFFFFFFFFu : 0;
    __builtin_memcpy(&__a.__val[0], &__u, 4);
    return __a;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_cmpngt_ss(__m128 __a, __m128 __b)
{
    unsigned int __u = !(__a.__val[0] > __b.__val[0]) ? 0xFFFFFFFFu : 0;
    __builtin_memcpy(&__a.__val[0], &__u, 4);
    return __a;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_cmpnge_ss(__m128 __a, __m128 __b)
{
    unsigned int __u = !(__a.__val[0] >= __b.__val[0]) ? 0xFFFFFFFFu : 0;
    __builtin_memcpy(&__a.__val[0], &__u, 4);
    return __a;
}

/* === Scalar ordered/unordered comparisons === */

static __inline__ __m128 __attribute__((__always_inline__))
_mm_cmpord_ss(__m128 __a, __m128 __b)
{
    unsigned int __u = (__a.__val[0] == __a.__val[0] && __b.__val[0] == __b.__val[0]) ? 0xFFFFFFFFu : 0;
    __builtin_memcpy(&__a.__val[0], &__u, 4);
    return __a;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_cmpunord_ss(__m128 __a, __m128 __b)
{
    unsigned int __u = (__a.__val[0] != __a.__val[0] || __b.__val[0] != __b.__val[0]) ? 0xFFFFFFFFu : 0;
    __builtin_memcpy(&__a.__val[0], &__u, 4);
    return __a;
}

/* === Ordered scalar comparison returning int (COMISS) === */

static __inline__ int __attribute__((__always_inline__))
_mm_comieq_ss(__m128 __a, __m128 __b)
{
    return __a.__val[0] == __b.__val[0];
}

static __inline__ int __attribute__((__always_inline__))
_mm_comilt_ss(__m128 __a, __m128 __b)
{
    return __a.__val[0] < __b.__val[0];
}

static __inline__ int __attribute__((__always_inline__))
_mm_comile_ss(__m128 __a, __m128 __b)
{
    return __a.__val[0] <= __b.__val[0];
}

static __inline__ int __attribute__((__always_inline__))
_mm_comigt_ss(__m128 __a, __m128 __b)
{
    return __a.__val[0] > __b.__val[0];
}

static __inline__ int __attribute__((__always_inline__))
_mm_comige_ss(__m128 __a, __m128 __b)
{
    return __a.__val[0] >= __b.__val[0];
}

static __inline__ int __attribute__((__always_inline__))
_mm_comineq_ss(__m128 __a, __m128 __b)
{
    return __a.__val[0] != __b.__val[0];
}

/* === Unordered scalar comparison returning int (UCOMISS) === */

static __inline__ int __attribute__((__always_inline__))
_mm_ucomieq_ss(__m128 __a, __m128 __b)
{
    return __a.__val[0] == __b.__val[0];
}

static __inline__ int __attribute__((__always_inline__))
_mm_ucomilt_ss(__m128 __a, __m128 __b)
{
    return __a.__val[0] < __b.__val[0];
}

static __inline__ int __attribute__((__always_inline__))
_mm_ucomile_ss(__m128 __a, __m128 __b)
{
    return __a.__val[0] <= __b.__val[0];
}

static __inline__ int __attribute__((__always_inline__))
_mm_ucomigt_ss(__m128 __a, __m128 __b)
{
    return __a.__val[0] > __b.__val[0];
}

static __inline__ int __attribute__((__always_inline__))
_mm_ucomige_ss(__m128 __a, __m128 __b)
{
    return __a.__val[0] >= __b.__val[0];
}

static __inline__ int __attribute__((__always_inline__))
_mm_ucomineq_ss(__m128 __a, __m128 __b)
{
    return __a.__val[0] != __b.__val[0];
}

/* === 64-bit integer conversion (SSE) === */

static __inline__ __m128 __attribute__((__always_inline__))
_mm_cvtsi64_ss(__m128 __a, long long __b)
{
    __a.__val[0] = (float)__b;
    return __a;
}

#define _mm_cvtsi64x_ss(a, b) _mm_cvtsi64_ss(a, b)

static __inline__ long long __attribute__((__always_inline__))
_mm_cvtss_si64(__m128 __a)
{
    return (long long)__a.__val[0];
}

#define _mm_cvtss_si64x(a) _mm_cvtss_si64(a)

static __inline__ long long __attribute__((__always_inline__))
_mm_cvttss_si64(__m128 __a)
{
    return (long long)__a.__val[0];
}

#define _mm_cvttss_si64x(a) _mm_cvttss_si64(a)

/* === Load (additional) === */

static __inline__ __m128 __attribute__((__always_inline__))
_mm_loadh_pi(__m128 __a, const __m64 *__p)
{
    __builtin_memcpy((char *)&__a + 8, __p, 8);
    return __a;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_loadl_pi(__m128 __a, const __m64 *__p)
{
    __builtin_memcpy(&__a, __p, 8);
    return __a;
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_loadr_ps(const float *__p)
{
    return (__m128){ { __p[3], __p[2], __p[1], __p[0] } };
}

/* === Store (additional) === */

static __inline__ void __attribute__((__always_inline__))
_mm_storer_ps(float *__p, __m128 __a)
{
    __p[0] = __a.__val[3]; __p[1] = __a.__val[2];
    __p[2] = __a.__val[1]; __p[3] = __a.__val[0];
}

/* === MMX<->float conversion (SSE additions that operate on __m64) === */

/* Convert packed 32-bit integers in __m64 to packed floats (low 2 elements) */
static __inline__ __m128 __attribute__((__always_inline__))
_mm_cvtpi32_ps(__m128 __a, __m64 __b)
{
    int *__bi = (int *)&__b.__val;
    __a.__val[0] = (float)__bi[0];
    __a.__val[1] = (float)__bi[1];
    return __a;
}

#define _mm_cvt_pi2ps(a, b) _mm_cvtpi32_ps(a, b)

/* Convert packed floats (low 2 elements) to packed 32-bit integers in __m64 */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_cvtps_pi32(__m128 __a)
{
    __m64 __r;
    int __rr[2];
    __rr[0] = (int)__a.__val[0];
    __rr[1] = (int)__a.__val[1];
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

#define _mm_cvt_ps2pi(a) _mm_cvtps_pi32(a)

/* Convert packed floats (low 2 elements) to packed 32-bit integers with truncation */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_cvttps_pi32(__m128 __a)
{
    __m64 __r;
    int __rr[2];
    __rr[0] = (int)__a.__val[0];
    __rr[1] = (int)__a.__val[1];
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

#define _mm_cvtt_ps2pi(a) _mm_cvttps_pi32(a)

/* Convert packed 16-bit signed integers to packed floats */
static __inline__ __m128 __attribute__((__always_inline__))
_mm_cvtpi16_ps(__m64 __a)
{
    short *__as = (short *)&__a.__val;
    return (__m128){ { (float)__as[0], (float)__as[1], (float)__as[2], (float)__as[3] } };
}

/* Convert packed 16-bit unsigned integers to packed floats */
static __inline__ __m128 __attribute__((__always_inline__))
_mm_cvtpu16_ps(__m64 __a)
{
    unsigned short *__as = (unsigned short *)&__a.__val;
    return (__m128){ { (float)__as[0], (float)__as[1], (float)__as[2], (float)__as[3] } };
}

/* Convert packed 8-bit signed integers (low 4 bytes) to packed floats */
static __inline__ __m128 __attribute__((__always_inline__))
_mm_cvtpi8_ps(__m64 __a)
{
    signed char *__ab = (signed char *)&__a.__val;
    return (__m128){ { (float)__ab[0], (float)__ab[1], (float)__ab[2], (float)__ab[3] } };
}

/* Convert packed 8-bit unsigned integers (low 4 bytes) to packed floats */
static __inline__ __m128 __attribute__((__always_inline__))
_mm_cvtpu8_ps(__m64 __a)
{
    unsigned char *__ab = (unsigned char *)&__a.__val;
    return (__m128){ { (float)__ab[0], (float)__ab[1], (float)__ab[2], (float)__ab[3] } };
}

/* Convert two __m64 packed 32-bit integers to __m128 packed floats */
static __inline__ __m128 __attribute__((__always_inline__))
_mm_cvtpi32x2_ps(__m64 __a, __m64 __b)
{
    int *__ai = (int *)&__a.__val;
    int *__bi = (int *)&__b.__val;
    return (__m128){ { (float)__ai[0], (float)__ai[1], (float)__bi[0], (float)__bi[1] } };
}

/* Convert packed floats to packed 16-bit signed integers (with saturation) in __m64 */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_cvtps_pi16(__m128 __a)
{
    __m64 __r;
    short __rr[4];
    for (int __i = 0; __i < 4; __i++) {
        int __v = (int)__a.__val[__i];
        if (__v > 32767) __v = 32767;
        if (__v < -32768) __v = -32768;
        __rr[__i] = (short)__v;
    }
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

/* Convert packed floats to packed 8-bit signed integers (low 4 bytes, high 4 zero) */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_cvtps_pi8(__m128 __a)
{
    __m64 __r;
    signed char __rr[8];
    for (int __i = 0; __i < 4; __i++) {
        int __v = (int)__a.__val[__i];
        if (__v > 127) __v = 127;
        if (__v < -128) __v = -128;
        __rr[__i] = (signed char)__v;
    }
    __rr[4] = __rr[5] = __rr[6] = __rr[7] = 0;
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

/* === SSE intrinsics operating on __m64 (SSE additions to MMX) === */

/* Extract a 16-bit integer from __m64 at position N */
#define _mm_extract_pi16(A, N) \
    ((int)(unsigned short)(((unsigned short *)&(A).__val)[(N) & 3]))

#define _m_pextrw(A, N) _mm_extract_pi16(A, N)

/* Insert a 16-bit integer into __m64 at position N */
#define _mm_insert_pi16(A, D, N) __extension__ ({ \
    __m64 __tmp = (A); \
    ((unsigned short *)&__tmp.__val)[(N) & 3] = (unsigned short)(D); \
    __tmp; \
})

#define _m_pinsrw(A, D, N) _mm_insert_pi16(A, D, N)

/* Packed maximum of signed 16-bit integers */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_max_pi16(__m64 __a, __m64 __b)
{
    __m64 __r;
    short *__ra = (short *)&__a.__val;
    short *__rb = (short *)&__b.__val;
    short __rr[4];
    for (int __i = 0; __i < 4; __i++)
        __rr[__i] = __ra[__i] > __rb[__i] ? __ra[__i] : __rb[__i];
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

#define _m_pmaxsw(a, b) _mm_max_pi16(a, b)

/* Packed maximum of unsigned 8-bit integers */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_max_pu8(__m64 __a, __m64 __b)
{
    __m64 __r;
    unsigned char *__ra = (unsigned char *)&__a.__val;
    unsigned char *__rb = (unsigned char *)&__b.__val;
    unsigned char __rr[8];
    for (int __i = 0; __i < 8; __i++)
        __rr[__i] = __ra[__i] > __rb[__i] ? __ra[__i] : __rb[__i];
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

#define _m_pmaxub(a, b) _mm_max_pu8(a, b)

/* Packed minimum of signed 16-bit integers */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_min_pi16(__m64 __a, __m64 __b)
{
    __m64 __r;
    short *__ra = (short *)&__a.__val;
    short *__rb = (short *)&__b.__val;
    short __rr[4];
    for (int __i = 0; __i < 4; __i++)
        __rr[__i] = __ra[__i] < __rb[__i] ? __ra[__i] : __rb[__i];
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

#define _m_pminsw(a, b) _mm_min_pi16(a, b)

/* Packed minimum of unsigned 8-bit integers */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_min_pu8(__m64 __a, __m64 __b)
{
    __m64 __r;
    unsigned char *__ra = (unsigned char *)&__a.__val;
    unsigned char *__rb = (unsigned char *)&__b.__val;
    unsigned char __rr[8];
    for (int __i = 0; __i < 8; __i++)
        __rr[__i] = __ra[__i] < __rb[__i] ? __ra[__i] : __rb[__i];
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

#define _m_pminub(a, b) _mm_min_pu8(a, b)

/* Create mask from most significant bit of each byte in __m64 */
static __inline__ int __attribute__((__always_inline__))
_mm_movemask_pi8(__m64 __a)
{
    int __r = 0;
    unsigned char *__ab = (unsigned char *)&__a.__val;
    for (int __i = 0; __i < 8; __i++)
        __r |= ((__ab[__i] >> 7) & 1) << __i;
    return __r;
}

#define _m_pmovmskb(a) _mm_movemask_pi8(a)

/* Multiply packed unsigned 16-bit integers, return high 16 bits */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_mulhi_pu16(__m64 __a, __m64 __b)
{
    __m64 __r;
    unsigned short *__ra = (unsigned short *)&__a.__val;
    unsigned short *__rb = (unsigned short *)&__b.__val;
    unsigned short __rr[4];
    for (int __i = 0; __i < 4; __i++)
        __rr[__i] = (unsigned short)(((unsigned int)__ra[__i] * (unsigned int)__rb[__i]) >> 16);
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

#define _m_pmulhuw(a, b) _mm_mulhi_pu16(a, b)

/* Shuffle 16-bit integers in __m64 using immediate selector */
#define _mm_shuffle_pi16(A, N) __extension__ ({ \
    __m64 __tmp_a = (A); \
    unsigned short *__src = (unsigned short *)&__tmp_a.__val; \
    __m64 __tmp_r; \
    unsigned short *__dst = (unsigned short *)&__tmp_r.__val; \
    __dst[0] = __src[(N) & 3]; \
    __dst[1] = __src[((N) >> 2) & 3]; \
    __dst[2] = __src[((N) >> 4) & 3]; \
    __dst[3] = __src[((N) >> 6) & 3]; \
    __tmp_r; \
})

#define _m_pshufw(A, N) _mm_shuffle_pi16(A, N)

/* Average of packed unsigned 8-bit integers (rounded) */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_avg_pu8(__m64 __a, __m64 __b)
{
    __m64 __r;
    unsigned char *__ra = (unsigned char *)&__a.__val;
    unsigned char *__rb = (unsigned char *)&__b.__val;
    unsigned char __rr[8];
    for (int __i = 0; __i < 8; __i++)
        __rr[__i] = (unsigned char)(((unsigned int)__ra[__i] + (unsigned int)__rb[__i] + 1) >> 1);
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

#define _m_pavgb(a, b) _mm_avg_pu8(a, b)

/* Average of packed unsigned 16-bit integers (rounded) */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_avg_pu16(__m64 __a, __m64 __b)
{
    __m64 __r;
    unsigned short *__ra = (unsigned short *)&__a.__val;
    unsigned short *__rb = (unsigned short *)&__b.__val;
    unsigned short __rr[4];
    for (int __i = 0; __i < 4; __i++)
        __rr[__i] = (unsigned short)(((unsigned int)__ra[__i] + (unsigned int)__rb[__i] + 1) >> 1);
    __builtin_memcpy(&__r.__val, __rr, 8);
    return __r;
}

#define _m_pavgw(a, b) _mm_avg_pu16(a, b)

/* Sum of absolute differences of packed unsigned 8-bit integers */
static __inline__ __m64 __attribute__((__always_inline__))
_mm_sad_pu8(__m64 __a, __m64 __b)
{
    __m64 __r;
    unsigned char *__ra = (unsigned char *)&__a.__val;
    unsigned char *__rb = (unsigned char *)&__b.__val;
    unsigned int __sum = 0;
    for (int __i = 0; __i < 8; __i++) {
        int __diff = (int)__ra[__i] - (int)__rb[__i];
        __sum += __diff < 0 ? -__diff : __diff;
    }
    __r.__val = __sum;
    return __r;
}

#define _m_psadbw(a, b) _mm_sad_pu8(a, b)

/* Non-temporal store of __m64 */
static __inline__ void __attribute__((__always_inline__))
_mm_stream_pi(__m64 *__p, __m64 __a)
{
    *__p = __a;
}

/* Conditional byte store from __m64 (MASKMOVQ) */
static __inline__ void __attribute__((__always_inline__))
_mm_maskmove_si64(__m64 __a, __m64 __n, char *__p)
{
    unsigned char *__da = (unsigned char *)&__a.__val;
    unsigned char *__dn = (unsigned char *)&__n.__val;
    for (int __i = 0; __i < 8; __i++) {
        if (__dn[__i] & 0x80)
            __p[__i] = __da[__i];
    }
}

#define _m_maskmovq(a, n, p) _mm_maskmove_si64(a, n, p)

/* === Undefined value === */

static __inline__ __m128 __attribute__((__always_inline__))
_mm_undefined_ps(void)
{
    __m128 __r;
    return __r;
}

/* GCC's xmmintrin.h includes emmintrin.h so that code which only includes
 * <xmmintrin.h> still gets access to __m128i and SSE2 intrinsics.
 * Match that behavior here. */
#include <emmintrin.h>

#endif /* _XMMINTRIN_H_INCLUDED */
