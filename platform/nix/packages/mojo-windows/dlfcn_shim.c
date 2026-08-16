/*
 * dlfcn_shim.c — POSIX dlfcn compatibility shim for Windows (MinGW-w64)
 *
 * Mojo's sys.ffi._DLHandle uses dlopen/dlsym/dlerror/dlclose internally.
 * These POSIX functions don't exist on Windows. This shim maps them to
 * the Windows equivalents:
 *
 *   dlopen  → LoadLibraryA / LoadLibraryW
 *   dlsym   → GetProcAddress
 *   dlclose → FreeLibrary
 *   dlerror → FormatMessage + GetLastError
 *
 * Compiled into libkgen_rt.a alongside kgen_rt_shim.o:
 *   x86_64-w64-mingw32-gcc -c -O2 -o dlfcn_shim.o dlfcn_shim.c
 *   x86_64-w64-mingw32-ar rcs libkgen_rt.a kgen_rt_shim.o dlfcn_shim.o
 *
 * SPDX-License-Identifier: MIT
 */

#ifdef _WIN32

#include <windows.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>

/* ══════════════════════════════════════════════════════════════════════════
 * Thread-local error message buffer
 * ══════════════════════════════════════════════════════════════════════════ */

/* dlerror() must return a human-readable string describing the last error.
 * We use a thread-local buffer so concurrent threads don't clobber each
 * other's messages. */
#define DLFCN_ERRBUF_SIZE 512
static __thread char dlfcn_errbuf[DLFCN_ERRBUF_SIZE];
static __thread int  dlfcn_has_error = 0;

static void dlfcn_set_error(void) {
    DWORD err = GetLastError();
    if (err == 0) {
        dlfcn_errbuf[0] = '\0';
        dlfcn_has_error = 0;
        return;
    }

    DWORD len = FormatMessageA(
        FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
        NULL,
        err,
        MAKELANGID(LANG_NEUTRAL, SUBLANG_DEFAULT),
        dlfcn_errbuf,
        DLFCN_ERRBUF_SIZE - 1,
        NULL
    );

    if (len == 0) {
        /* FormatMessage itself failed — fall back to numeric code */
        snprintf(dlfcn_errbuf, DLFCN_ERRBUF_SIZE, "Unknown error 0x%08lx", (unsigned long)err);
    } else {
        /* Strip trailing \r\n that FormatMessage appends */
        while (len > 0 && (dlfcn_errbuf[len - 1] == '\r' || dlfcn_errbuf[len - 1] == '\n')) {
            dlfcn_errbuf[--len] = '\0';
        }
    }

    dlfcn_has_error = 1;
}

/* ══════════════════════════════════════════════════════════════════════════
 * dlopen — load a shared library
 * ══════════════════════════════════════════════════════════════════════════
 *
 * POSIX flags (RTLD_LAZY, RTLD_NOW, RTLD_GLOBAL, RTLD_LOCAL) are accepted
 * but ignored — Windows always resolves symbols eagerly on LoadLibrary and
 * doesn't distinguish global vs local symbol namespaces.
 *
 * If filename is NULL, returns a handle to the main executable (matching
 * POSIX behavior where dlopen(NULL, ...) returns a handle for global
 * symbol lookup).
 */
void *dlopen(const char *filename, int flags) {
    (void)flags;

    HMODULE handle;

    if (filename == NULL) {
        /* Return handle to the calling process (main executable) */
        handle = GetModuleHandleA(NULL);
    } else {
        /* Try loading the library.
         * SetErrorMode suppresses the "DLL not found" system dialog box. */
        UINT old_mode = SetErrorMode(SEM_FAILCRITICALERRORS);
        handle = LoadLibraryA(filename);
        SetErrorMode(old_mode);
    }

    if (handle == NULL) {
        dlfcn_set_error();
        return NULL;
    }

    dlfcn_has_error = 0;
    return (void *)handle;
}

/* ══════════════════════════════════════════════════════════════════════════
 * dlsym — look up a symbol in a shared library
 * ══════════════════════════════════════════════════════════════════════════ */
void *dlsym(void *handle, const char *symbol) {
    if (handle == NULL) {
        SetLastError(ERROR_INVALID_HANDLE);
        dlfcn_set_error();
        return NULL;
    }

    /* Clear any prior error */
    SetLastError(0);

    FARPROC addr = GetProcAddress((HMODULE)handle, symbol);

    if (addr == NULL) {
        dlfcn_set_error();
        return NULL;
    }

    dlfcn_has_error = 0;
    return (void *)(uintptr_t)addr;
}

/* ══════════════════════════════════════════════════════════════════════════
 * dlclose — unload a shared library
 * ══════════════════════════════════════════════════════════════════════════ */
int dlclose(void *handle) {
    if (handle == NULL) {
        SetLastError(ERROR_INVALID_HANDLE);
        dlfcn_set_error();
        return -1;
    }

    BOOL ok = FreeLibrary((HMODULE)handle);
    if (!ok) {
        dlfcn_set_error();
        return -1;
    }

    dlfcn_has_error = 0;
    return 0;
}

/* ══════════════════════════════════════════════════════════════════════════
 * dlerror — return a human-readable error string
 * ══════════════════════════════════════════════════════════════════════════
 *
 * Per POSIX, dlerror() returns NULL if no error has occurred since the
 * last call to dlerror(), and clears the error state on each call.
 */
char *dlerror(void) {
    if (!dlfcn_has_error) {
        return NULL;
    }

    dlfcn_has_error = 0;
    return dlfcn_errbuf;
}

#endif /* _WIN32 */