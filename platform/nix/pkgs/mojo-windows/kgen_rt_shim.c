/*
 * kgen_rt_shim.c — Minimal KGEN CompilerRT shim for Windows (MinGW-w64)
 *
 * Provides the KGEN_CompilerRT_* symbols that Mojo-compiled Windows object
 * files reference.  The official Mojo SDK ships these as libKGENCompilerRT-
 * Shared.so (Linux) but no Windows equivalent exists.  This shim implements
 * the subset the compiler actually emits calls to, with enough fidelity
 * for single-threaded desktop GUI applications (mojo-gui).
 *
 * Compiled to a static library with MinGW-w64:
 *   x86_64-w64-mingw32-gcc -c -O2 -o kgen_rt_shim.o kgen_rt_shim.c
 *   x86_64-w64-mingw32-ar rcs libkgen_rt.a kgen_rt_shim.o
 *
 * Linked into a Mojo Windows executable:
 *   x86_64-w64-mingw32-gcc app.obj -o app.exe -L. -lkgen_rt
 *
 * SPDX-License-Identifier: MIT
 */

#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>

/* Windows-specific headers */
#ifdef _WIN32
#  include <windows.h>
#  include <malloc.h>   /* _aligned_malloc / _aligned_free */
#else
/* Allow building on Linux for testing (never shipped) */
#  include <signal.h>
#endif

/* ══════════════════════════════════════════════════════════════════════════
 * Forward declarations & types
 * ══════════════════════════════════════════════════════════════════════════ */

/* Opaque async runtime handle */
typedef struct kgen_async_runtime kgen_async_runtime_t;

/* Opaque spin-waiter handle */
typedef struct kgen_spin_waiter kgen_spin_waiter_t;

/* Opaque async chain handle */
typedef struct kgen_async_chain kgen_async_chain_t;

/* {ptr, i64} string slice — Mojo's LLVM IR declares global-name arguments as
 * `{ ptr, i64 }` by value.  On x86_64-pc-windows-gnu the LLVM backend
 * flattens this into TWO register arguments (RCX=ptr, RDX=len) rather than
 * passing a hidden pointer as the Windows x64 ABI would for a 16-byte C
 * struct.  Therefore we accept the two fields as separate parameters in the
 * C functions below instead of using a struct type. */

/* Global registry entry */
typedef struct kgen_global_entry {
    struct kgen_global_entry *next;
    char                     *name;       /* heap-allocated copy */
    int64_t                   name_len;
    void                     *value;      /* pointer returned by init_fn */
    void                    (*destroy_fn)(void *);
} kgen_global_entry_t;

/* ══════════════════════════════════════════════════════════════════════════
 * Internal state (file-scoped)
 * ══════════════════════════════════════════════════════════════════════════ */

/* Stored argc/argv */
static int     g_argc = 0;
static char  **g_argv = NULL;

/* Dummy async runtime singleton (single-threaded) */
static struct kgen_async_runtime {
    int64_t parallelism;
} g_runtime_storage = { 1 };

static kgen_async_runtime_t *g_current_runtime = NULL;

/* Global registry (singly-linked list — sufficient for typical programs
 * that register <20 globals) */
static kgen_global_entry_t *g_globals_head = NULL;

/* Monotonic operation counter */
static volatile int64_t g_next_op_id = 0;

/* ══════════════════════════════════════════════════════════════════════════
 * Aligned memory allocation
 * ══════════════════════════════════════════════════════════════════════════ */

__attribute__((visibility("default")))
void *KGEN_CompilerRT_AlignedAlloc(int64_t alignment, int64_t size) {
    if (size == 0) return NULL;
    if (alignment < (int64_t)sizeof(void *))
        alignment = (int64_t)sizeof(void *);
#ifdef _WIN32
    return _aligned_malloc((size_t)size, (size_t)alignment);
#else
    void *p = NULL;
    if (posix_memalign(&p, (size_t)alignment, (size_t)size) != 0)
        return NULL;
    return p;
#endif
}

__attribute__((visibility("default")))
void KGEN_CompilerRT_AlignedFree(void *ptr) {
    if (!ptr) return;
#ifdef _WIN32
    _aligned_free(ptr);
#else
    free(ptr);
#endif
}

/* ══════════════════════════════════════════════════════════════════════════
 * Async runtime (single-threaded stub)
 *
 * The async runtime in the real KGEN library manages a thread pool for
 * parallel Mojo tasks.  mojo-gui desktop apps are single-threaded (the
 * Blitz event loop runs on the main thread), so a no-op runtime suffices.
 * ══════════════════════════════════════════════════════════════════════════ */

__attribute__((visibility("default")))
void *KGEN_CompilerRT_AsyncRT_CreateRuntime(int64_t parallelism) {
    g_runtime_storage.parallelism = parallelism > 0 ? parallelism : 1;
    g_current_runtime = &g_runtime_storage;
    return (void *)g_current_runtime;
}

__attribute__((visibility("default")))
void KGEN_CompilerRT_AsyncRT_DestroyRuntime(void *rt) {
    (void)rt;
    g_current_runtime = NULL;
}

__attribute__((visibility("default")))
void *KGEN_CompilerRT_AsyncRT_GetCurrentRuntime(void) {
    return (void *)g_current_runtime;
}

__attribute__((visibility("default")))
int64_t KGEN_CompilerRT_AsyncRT_ParallelismLevel(void) {
    return g_runtime_storage.parallelism;
}

/* Execute a task synchronously (single-threaded fallback).
 * Signature: void Execute(runtime, fn_ptr, context, ...) — the exact
 * calling convention is opaque; we call fn_ptr(context) which is correct
 * for the single-task case the compiler emits. */
__attribute__((visibility("default")))
void KGEN_CompilerRT_AsyncRT_Execute(void *runtime,
                                     void (*fn)(void *),
                                     void *context) {
    (void)runtime;
    if (fn) fn(context);
}

/* Chain (continuation) stubs — single-threaded: execute inline */

__attribute__((visibility("default")))
void KGEN_CompilerRT_AsyncRT_InitializeChain(void *chain) {
    (void)chain;
}

__attribute__((visibility("default")))
void KGEN_CompilerRT_AsyncRT_DestroyChain(void *chain) {
    (void)chain;
}

__attribute__((visibility("default")))
void KGEN_CompilerRT_AsyncRT_Complete(void *chain) {
    (void)chain;
}

__attribute__((visibility("default")))
void KGEN_CompilerRT_AsyncRT_AndThen(void *chain,
                                     void (*fn)(void *),
                                     void *context) {
    (void)chain;
    if (fn) fn(context);
}

__attribute__((visibility("default")))
void KGEN_CompilerRT_AsyncRT_Wait(void *chain) {
    (void)chain;
}

__attribute__((visibility("default")))
int64_t KGEN_CompilerRT_AsyncRT_Wait_Timeout(void *chain,
                                              int64_t timeout_ns) {
    (void)chain;
    (void)timeout_ns;
    return 0; /* success */
}

__attribute__((visibility("default")))
void *KGEN_CompilerRT_AsyncRT_CreateAsyncs_Error(void) {
    return NULL;
}

/* Spin-waiter stubs */

__attribute__((visibility("default")))
void KGEN_CompilerRT_AsyncRT_InitializeSpinWaiter(void *waiter) {
    (void)waiter;
}

__attribute__((visibility("default")))
void KGEN_CompilerRT_AsyncRT_DestroySpinWaiter(void *waiter) {
    (void)waiter;
}

__attribute__((visibility("default")))
void KGEN_CompilerRT_AsyncRT_SpinWaiter_Wait(void *waiter) {
    (void)waiter;
#ifdef _WIN32
    SwitchToThread();
#endif
}

/* ══════════════════════════════════════════════════════════════════════════
 * Global variable registry
 *
 * Mojo uses a name-indexed registry of lazily-initialised global values.
 * GetOrCreateGlobal(name, init_fn, destroy_fn) returns a pointer to the
 * value.  If the name hasn't been registered yet, it calls init_fn to
 * allocate / initialise the value and records destroy_fn for cleanup.
 *
 * The init_fn signature (from LLVM IR) is:  void *init_fn(void)
 * The destroy_fn signature is:              void  destroy_fn(void *)
 * ══════════════════════════════════════════════════════════════════════════ */

/* Helper: compare a name (ptr+len) with a registry entry */
static int name_matches(const kgen_global_entry_t *entry,
                        const char *ptr, int64_t len) {
    if (entry->name_len != len) return 0;
    return memcmp(entry->name, ptr, (size_t)len) == 0;
}

/* Helper: allocate and insert a new global entry */
static kgen_global_entry_t *insert_entry(const char *name_ptr,
                                         int64_t name_len,
                                         void *value,
                                         void (*destroy_fn)(void *)) {
    kgen_global_entry_t *e =
        (kgen_global_entry_t *)malloc(sizeof(kgen_global_entry_t));
    if (!e) {
        fprintf(stderr, "KGEN_CompilerRT: out of memory registering global\n");
        abort();
    }
    e->name = (char *)malloc((size_t)name_len);
    if (!e->name) {
        fprintf(stderr, "KGEN_CompilerRT: out of memory registering global\n");
        abort();
    }
    memcpy(e->name, name_ptr, (size_t)name_len);
    e->name_len   = name_len;
    e->value      = value;
    e->destroy_fn = destroy_fn;
    e->next       = g_globals_head;
    g_globals_head = e;
    return e;
}

__attribute__((visibility("default")))
void *KGEN_CompilerRT_GetOrCreateGlobal(const char *name_ptr,
                                        int64_t name_len,
                                        void *(*init_fn)(void),
                                        void (*destroy_fn)(void *)) {
    /* Search existing globals */
    kgen_global_entry_t *entry = g_globals_head;
    while (entry) {
        if (name_matches(entry, name_ptr, name_len))
            return entry->value;
        entry = entry->next;
    }

    /* Not found — initialise */
    void *value = init_fn ? init_fn() : NULL;

    /* Register */
    insert_entry(name_ptr, name_len, value, destroy_fn);
    return value;
}

__attribute__((visibility("default")))
void *KGEN_CompilerRT_GetOrCreateGlobalIndexed(const char *name_ptr,
                                               int64_t name_len,
                                               int64_t index,
                                               void *(*init_fn)(void),
                                               void (*destroy_fn)(void *)) {
    /* Append index to name for a unique key */
    char buf[512];
    int64_t base_len = name_len < 480 ? name_len : 480;
    memcpy(buf, name_ptr, (size_t)base_len);
    int extra = snprintf(buf + base_len, sizeof(buf) - (size_t)base_len,
                         ":%lld", (long long)index);

    return KGEN_CompilerRT_GetOrCreateGlobal(buf, base_len + extra,
                                             init_fn, destroy_fn);
}

__attribute__((visibility("default")))
void *KGEN_CompilerRT_GetGlobalOrNull(const char *name_ptr,
                                      int64_t name_len) {
    kgen_global_entry_t *entry = g_globals_head;
    while (entry) {
        if (name_matches(entry, name_ptr, name_len))
            return entry->value;
        entry = entry->next;
    }
    return NULL;
}

__attribute__((visibility("default")))
void KGEN_CompilerRT_InsertGlobal(const char *name_ptr,
                                  int64_t name_len,
                                  void *value,
                                  void (*destroy_fn)(void *)) {
    insert_entry(name_ptr, name_len, value, destroy_fn);
}

__attribute__((visibility("default")))
void KGEN_CompilerRT_DestroyGlobals(void) {
    kgen_global_entry_t *entry = g_globals_head;
    while (entry) {
        kgen_global_entry_t *next = entry->next;
        if (entry->destroy_fn && entry->value)
            entry->destroy_fn(entry->value);
        free(entry->name);
        free(entry);
        entry = next;
    }
    g_globals_head = NULL;
}

/* ══════════════════════════════════════════════════════════════════════════
 * argc/argv storage
 * ══════════════════════════════════════════════════════════════════════════ */

__attribute__((visibility("default")))
void KGEN_CompilerRT_SetArgV(int32_t argc, char **argv) {
    g_argc = (int)argc;
    g_argv = argv;
}

__attribute__((visibility("default")))
void *KGEN_CompilerRT_GetArgV(void) {
    /* Returns a pointer to {argc, argv} — Mojo reads these via the
     * pointer.  We return a pointer to g_argc which is followed by
     * g_argv in memory (they are adjacent statics). */
    static struct { int32_t argc; char **argv; } argpack;
    argpack.argc = (int32_t)g_argc;
    argpack.argv = g_argv;
    return &argpack;
}

/* ══════════════════════════════════════════════════════════════════════════
 * Stack trace & fault handler
 * ══════════════════════════════════════════════════════════════════════════ */

#ifdef _WIN32
static LONG WINAPI fault_handler(EXCEPTION_POINTERS *info) {
    DWORD code = info->ExceptionRecord->ExceptionCode;
    if (code == EXCEPTION_ACCESS_VIOLATION ||
        code == EXCEPTION_STACK_OVERFLOW   ||
        code == EXCEPTION_INT_DIVIDE_BY_ZERO) {
        fprintf(stderr, "\nFatal error: unhandled exception 0x%08lx at %p\n",
                (unsigned long)code,
                info->ExceptionRecord->ExceptionAddress);
        fflush(stderr);
    }
    return EXCEPTION_CONTINUE_SEARCH;
}
#endif

__attribute__((visibility("default")))
void KGEN_CompilerRT_PrintStackTraceOnFault(void) {
#ifdef _WIN32
    SetUnhandledExceptionFilter(fault_handler);
#else
    /* POSIX: install signal handlers for common faults */
    signal(SIGSEGV, SIG_DFL);
    signal(SIGABRT, SIG_DFL);
#endif
}

__attribute__((visibility("default")))
int64_t KGEN_CompilerRT_GetStackTrace(void *buf, int64_t max_frames) {
    (void)buf;
    (void)max_frames;
    /* Stack trace capture is best-effort.  Return 0 frames. */
    return 0;
}

/* ══════════════════════════════════════════════════════════════════════════
 * CPU topology (used by parallelism heuristics)
 * ══════════════════════════════════════════════════════════════════════════ */

__attribute__((visibility("default")))
int64_t KGEN_CompilerRT_NumPhysicalCores(void) {
#ifdef _WIN32
    SYSTEM_INFO si;
    GetSystemInfo(&si);
    return (int64_t)si.dwNumberOfProcessors;
#else
    return 1;
#endif
}

__attribute__((visibility("default")))
int64_t KGEN_CompilerRT_NumLogicalCores(void) {
#ifdef _WIN32
    SYSTEM_INFO si;
    GetSystemInfo(&si);
    return (int64_t)si.dwNumberOfProcessors;
#else
    return 1;
#endif
}

__attribute__((visibility("default")))
int64_t KGEN_CompilerRT_NumPerformanceCores(void) {
    return KGEN_CompilerRT_NumPhysicalCores();
}

/* ══════════════════════════════════════════════════════════════════════════
 * fprintf shim (Mojo calls this for runtime error messages)
 * ══════════════════════════════════════════════════════════════════════════ */

__attribute__((visibility("default")))
int KGEN_CompilerRT_fprintf(void *stream, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int ret = vfprintf(stream ? (FILE *)stream : stderr, fmt, ap);
    va_end(ap);
    return ret;
}

/* ══════════════════════════════════════════════════════════════════════════
 * Monotonic operation ID (used by Mojo's Dict/Set for ordering)
 * ══════════════════════════════════════════════════════════════════════════ */

__attribute__((visibility("default")))
int64_t KGEN_CompilerRT_GetNextOpId(void) {
    return g_next_op_id++;
}

/* ══════════════════════════════════════════════════════════════════════════
 * Python integration (stub — Mojo on Windows desktop doesn't use Python)
 * ══════════════════════════════════════════════════════════════════════════ */

__attribute__((visibility("default")))
void KGEN_CompilerRT_Python_SetPythonPath(const char *path) {
    (void)path;
}

/* ══════════════════════════════════════════════════════════════════════════
 * ASAN integration (stub — not used in release builds)
 * ══════════════════════════════════════════════════════════════════════════ */

__attribute__((visibility("default")))
void KGEN_CompilerRT_SetAsanAllocators(void *alloc_fn, void *free_fn) {
    (void)alloc_fn;
    (void)free_fn;
}

/* ══════════════════════════════════════════════════════════════════════════
 * Time-trace profiler (stubs — no profiling in cross-compiled builds)
 * ══════════════════════════════════════════════════════════════════════════ */

__attribute__((visibility("default")))
void KGEN_CompilerRT_TimeTraceProfilerBegin(const char *name) {
    (void)name;
}

__attribute__((visibility("default")))
void KGEN_CompilerRT_TimeTraceProfilerBeginDetail(const char *name,
                                                  const char *detail) {
    (void)name;
    (void)detail;
}

__attribute__((visibility("default")))
void KGEN_CompilerRT_TimeTraceProfilerBeginTask(const char *name,
                                                int64_t task_id) {
    (void)name;
    (void)task_id;
}

__attribute__((visibility("default")))
void KGEN_CompilerRT_TimeTraceProfilerEnd(void) {
    /* no-op */
}

__attribute__((visibility("default")))
int64_t KGEN_CompilerRT_TimeTraceProfilerGetCurrentId(void) {
    return 0;
}

__attribute__((visibility("default")))
void KGEN_CompilerRT_TimeTraceProfilerSetCurrentId(int64_t id) {
    (void)id;
}

/* ══════════════════════════════════════════════════════════════════════════
 * Tracy profiler integration (stubs)
 * ══════════════════════════════════════════════════════════════════════════ */

__attribute__((visibility("default")))
int KGEN_CompilerRT_TracyIsEnabled(void) {
    return 0;
}

__attribute__((visibility("default")))
void KGEN_CompilerRT_TracyZoneBegin(const char *name,
                                    const char *function,
                                    const char *file,
                                    int32_t line) {
    (void)name;
    (void)function;
    (void)file;
    (void)line;
}

__attribute__((visibility("default")))
void KGEN_CompilerRT_TracyZoneEnd(void) {
    /* no-op */
}

