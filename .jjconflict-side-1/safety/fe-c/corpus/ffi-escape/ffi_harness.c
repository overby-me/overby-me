/* Minimal C harness for Task B5 / I9: models SQLite storing a Rust callback
 * plus an opaque pointer and invoking it later (the RUSTSEC-2021-0128 path).
 * fec_register stashes the callback and data; fec_invoke re-enters Rust
 * through the callback with the stored pointer, exactly as SQLite calls a
 * registered scalar function later. The C code itself is not instrumented;
 * only the Rust boundary is checked (trace F8). */
#include <stddef.h>

typedef long (*fec_cb)(void *data);

static fec_cb g_cb = NULL;
static void *g_data = NULL;

void fec_register(fec_cb cb, void *data) {
  g_cb = cb;
  g_data = data;
}

long fec_invoke(void) { return g_cb ? g_cb(g_data) : -1; }
