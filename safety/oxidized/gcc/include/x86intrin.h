/* CCC compiler bundled x86intrin.h - x86 intrinsics umbrella header */
#ifndef _X86INTRIN_H_INCLUDED
#define _X86INTRIN_H_INCLUDED

/* x86 intrinsics are only available on x86/x86-64 targets */
#if !defined(__x86_64__) && !defined(__i386__) && !defined(__i686__)
#error "x86 intrinsics (x86intrin.h) require an x86 target"
#endif

/* Include all SIMD intrinsics */
#include <immintrin.h>

/* rdtsc - Read Time-Stamp Counter */
static __inline__ unsigned long long
__attribute__((__always_inline__))
__rdtsc(void)
{
    unsigned int __lo, __hi;
    __asm__ __volatile__("rdtsc" : "=a"(__lo), "=d"(__hi));
    return ((unsigned long long)__hi << 32) | __lo;
}

/* rdtscp - Read Time-Stamp Counter and Processor ID */
static __inline__ unsigned long long
__attribute__((__always_inline__))
__rdtscp(unsigned int *__aux)
{
    unsigned int __lo, __hi;
    __asm__ __volatile__("rdtscp" : "=a"(__lo), "=d"(__hi), "=c"(*__aux));
    return ((unsigned long long)__hi << 32) | __lo;
}

/* Compatibility aliases */
#define _rdtsc()      __rdtsc()
#define _rdtscp(a)    __rdtscp(a)

/* Bit-scan intrinsics */
static __inline__ int
__attribute__((__always_inline__))
__bsfd(int __a)
{
    int __r;
    __asm__("bsfl %1, %0" : "=r"(__r) : "rm"(__a));
    return __r;
}

static __inline__ int
__attribute__((__always_inline__))
__bsrd(int __a)
{
    int __r;
    __asm__("bsrl %1, %0" : "=r"(__r) : "rm"(__a));
    return __r;
}

/* Byte-swap intrinsics */
static __inline__ int
__attribute__((__always_inline__))
__bswapd(int __a)
{
    return __builtin_bswap32(__a);
}

#ifdef __x86_64__
static __inline__ long long
__attribute__((__always_inline__))
__bswapq(long long __a)
{
    return __builtin_bswap64(__a);
}

static __inline__ int
__attribute__((__always_inline__))
__bsfq(long long __a)
{
    long long __r;
    __asm__("bsfq %1, %0" : "=r"(__r) : "rm"(__a));
    return (int)__r;
}

static __inline__ int
__attribute__((__always_inline__))
__bsrq(long long __a)
{
    long long __r;
    __asm__("bsrq %1, %0" : "=r"(__r) : "rm"(__a));
    return (int)__r;
}
#endif /* __x86_64__ */

/* Pause instruction - hint for spin-wait loops */
static __inline__ void
__attribute__((__always_inline__))
__pause(void)
{
    __asm__ __volatile__("pause");
}

/* Rotation intrinsics */
static __inline__ unsigned char
__attribute__((__always_inline__))
__rolb(unsigned char __a, int __n)
{
    return (unsigned char)((__a << (__n & 7)) | (__a >> (8 - (__n & 7))));
}

static __inline__ unsigned short
__attribute__((__always_inline__))
__rolw(unsigned short __a, int __n)
{
    return (unsigned short)((__a << (__n & 15)) | (__a >> (16 - (__n & 15))));
}

static __inline__ unsigned int
__attribute__((__always_inline__))
__rold(unsigned int __a, int __n)
{
    return (__a << (__n & 31)) | (__a >> (32 - (__n & 31)));
}

static __inline__ unsigned char
__attribute__((__always_inline__))
__rorb(unsigned char __a, int __n)
{
    return (unsigned char)((__a >> (__n & 7)) | (__a << (8 - (__n & 7))));
}

static __inline__ unsigned short
__attribute__((__always_inline__))
__rorw(unsigned short __a, int __n)
{
    return (unsigned short)((__a >> (__n & 15)) | (__a << (16 - (__n & 15))));
}

static __inline__ unsigned int
__attribute__((__always_inline__))
__rord(unsigned int __a, int __n)
{
    return (__a >> (__n & 31)) | (__a << (32 - (__n & 31)));
}

#ifdef __x86_64__
static __inline__ unsigned long long
__attribute__((__always_inline__))
__rolq(unsigned long long __a, int __n)
{
    return (__a << (__n & 63)) | (__a >> (64 - (__n & 63)));
}

static __inline__ unsigned long long
__attribute__((__always_inline__))
__rorq(unsigned long long __a, int __n)
{
    return (__a >> (__n & 63)) | (__a << (64 - (__n & 63)));
}
#endif /* __x86_64__ */

#endif /* _X86INTRIN_H_INCLUDED */
