/* The smallest fnative plugin, in C: shows the ABI (include/fnative.h).
 * Build: clang-cl /LD /O2 /I ..\..\include hello.c /Fe:hello.dll   (test/build_plugins.py does it) */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <windows.h>
#include "fnative.h"

static const fnative_host *host;

/* one output buffer per thread: valid until this thread's next call, as the ABI asks */
static __declspec(thread) char buf[4096];

static int out_str(const char *s, const char **out, size_t *out_len) {
    *out = s;
    *out_len = strlen(s);
    return 0;
}

/* hello.echo(input) -> input */
static int echo(void *ud, const char *name, const char *in, size_t len, const char **out, size_t *out_len) {
    (void)ud; (void)name;
    *out = in;            /* (the input stays alive for the whole call, and the host copies the result at once) */
    *out_len = len;
    return 0;
}

/* hello.reverse(input) -> the bytes backwards */
static int reverse(void *ud, const char *name, const char *in, size_t len, const char **out, size_t *out_len) {
    (void)ud; (void)name;
    if (len > sizeof buf) return out_str("input longer than 4096 bytes", out, out_len), 1;
    for (size_t i = 0; i < len; i++) buf[i] = in[len - 1 - i];
    *out = buf;
    *out_len = len;
    return 0;
}

/* hello.slow(ms) -> "slept <ms> ms on thread <id>": for native.start / native.poll */
static int slow(void *ud, const char *name, const char *in, size_t len, const char **out, size_t *out_len) {
    (void)ud; (void)name;
    char num[32] = {0};
    memcpy(num, in, len < 31 ? len : 31);
    DWORD ms = (DWORD)atoi(num);
    Sleep(ms);
    snprintf(buf, sizeof buf, "slept %lu ms on thread %lu", ms, GetCurrentThreadId());
    return out_str(buf, out, out_len);
}

__declspec(dllexport) int fnative_plugin_init(const fnative_host *h) {
    if (h->abi != FNATIVE_ABI) return 1;
    host = h;
    h->register_fn("hello", "echo", echo, NULL, FNATIVE_THREADSAFE);
    h->register_fn("hello", "reverse", reverse, NULL, FNATIVE_THREADSAFE);
    h->register_fn("hello", "slow", slow, NULL, FNATIVE_THREADSAFE);
    h->log("hello", "hello from C");
    return 0;
}
