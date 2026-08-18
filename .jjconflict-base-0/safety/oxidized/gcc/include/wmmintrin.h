/* CCC compiler bundled wmmintrin.h - AES-NI and CLMUL intrinsics */
#ifndef _WMMINTRIN_H_INCLUDED
#define _WMMINTRIN_H_INCLUDED

#include <emmintrin.h>

/* === AES-NI intrinsics === */

static __inline__ __m128i __attribute__((__always_inline__))
_mm_aesenc_si128(__m128i __V, __m128i __R)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_aesenc128(__V, __R));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_aesenclast_si128(__m128i __V, __m128i __R)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_aesenclast128(__V, __R));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_aesdec_si128(__m128i __V, __m128i __R)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_aesdec128(__V, __R));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_aesdeclast_si128(__m128i __V, __m128i __R)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_aesdeclast128(__V, __R));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_aesimc_si128(__m128i __V)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_aesimc128(__V));
}

/* _mm_aeskeygenassist_si128 requires a compile-time constant imm8 */
#define _mm_aeskeygenassist_si128(V, I) \
    __CCC_M128I_FROM_BUILTIN(__builtin_ia32_aeskeygenassist128((V), (I)))

/* === CLMUL (carry-less multiplication) === */

/* _mm_clmulepi64_si128 requires a compile-time constant imm8 */
#define _mm_clmulepi64_si128(X, Y, I) \
    __CCC_M128I_FROM_BUILTIN(__builtin_ia32_pclmulqdq128((X), (Y), (I)))

#endif /* _WMMINTRIN_H_INCLUDED */
