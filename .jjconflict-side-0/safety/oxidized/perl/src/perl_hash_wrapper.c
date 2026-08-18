#include <stdint.h>
#include <string.h>

typedef uint8_t U8;
typedef uint16_t U16;
typedef uint32_t U32;
typedef uint64_t U64;
typedef size_t STRLEN;
#define PERL_STATIC_INLINE static inline
#define SBOX32_STATIC_INLINE static inline
#define PERL_SEEN_HV_FUNC_H_
#define U8TO64_LE(p) ((U64)((p)[0]) | ((U64)((p)[1])<<8) | ((U64)((p)[2])<<16) | ((U64)((p)[3])<<24) | ((U64)((p)[4])<<32) | ((U64)((p)[5])<<40) | ((U64)((p)[6])<<48) | ((U64)((p)[7])<<56))
#define U8TO32_LE(p) ((U32)((p)[0]) | ((U32)((p)[1])<<8) | ((U32)((p)[2])<<16) | ((U32)((p)[3])<<24))
#define U8TO16_LE(p) ((U16)((p)[0]) | ((U16)((p)[1])<<8))
#define ROTL64(x,n) (((x) << (n)) | ((x) >> (64 - (n))))
#define ROTL32(x,n) (((x) << (n)) | ((x) >> (32 - (n))))
#define ROTR32(x,n) (((x) >> (n)) | ((x) << (32 - (n))))
#define U64_CONST(x) ((U64)x##ULL)
#define U32_CONST(x) ((U32)x##U)
#define UINT64_C(x) ((U64)x##ULL)
#define UINT32_C(x) ((U32)x##U)
#define STMT_START do
#define STMT_END while(0)
#define SBOX32_MAX_LEN 24
#define SBOX32_CHURN_ROUNDS 128
#define CAN64BITHASH 1
#define SBOX32_STATE_BYTES ((1 + (256 * SBOX32_MAX_LEN)) * sizeof(U32))

#include "perl_siphash.h"
#include "sbox32_hash.h"

/* Compute perl hash for a key with seed=0. Uses SBOX32 for short
 * keys (<=24 bytes), SipHash13 for longer. */
uint32_t perl_hash_seed0_ffi(const uint8_t *key, size_t keylen) {
    static U8 sip_state[32];
    static U8 sbox_state[SBOX32_STATE_BYTES + 32];
    static int initialized = 0;
    if (!initialized) {
        U8 zero[16] = {0};
        S_perl_siphash_seed_state(zero, sip_state);
        sbox32_seed_state128(zero, sbox_state);
        initialized = 1;
    }
    if (keylen <= SBOX32_MAX_LEN) {
        return sbox32_hash_with_state(sbox_state, key, keylen);
    } else {
        return S_perl_hash_siphash_1_3_with_state(sip_state, key, keylen);
    }
}
