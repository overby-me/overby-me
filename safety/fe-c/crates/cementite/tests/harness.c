/* Minimal C harness for Task A4: a genuinely C-compiled translation unit
 * that allocates through libc malloc. When cementite is built with the
 * `interpose` feature, its #[no_mangle] malloc overrides libc's for the
 * whole binary, so this C code's malloc is intercepted and registered —
 * proving interposition applies across the language boundary. */
#include <stddef.h>
#include <stdlib.h>

void *fec_harness_alloc(size_t n) {
  unsigned char *p = (unsigned char *)malloc(n);
  if (p && n > 0) {
    p[0] = 0xAB; /* marker the Rust side checks */
  }
  return p;
}

void fec_harness_free(void *p) { free(p); }
