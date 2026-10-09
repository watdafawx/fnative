/* fse plugin ABI, version 1.
 *
 * A plugin is a DLL in the plugins folder (FSE_PLUGINS, else "plugins" beside fse.exe) exporting
 *     int fse_plugin_init(const fse_host *host);      returns 0 on success
 * It is called once, before the game starts (keep it quick: start heavy things lazily, on the first call).
 * There it registers its functions with host->register_fn. Lua mods then reach them through the native table:
 *     native.call(plugin, name, input)          on the game thread, returns the output string, or nil, error
 *     native.start(plugin, name, input)         on a worker thread (functions registered FSE_THREADSAFE),
 *     native.poll(id)                           returns "pending" | "done", output | "error", message
 * Input and output are byte strings; JSON is the convention (Lua side: helpers.table_to_json / json_to_table).
 *
 * Any language that can export a C function works: C, C++, Rust, Zig, Go (c-shared), ... */
#ifndef FSE_H
#define FSE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define FSE_ABI 1

/* may be called on a worker thread (native.start); without it only native.call reaches the function */
#define FSE_THREADSAFE 1u

/* A plugin function. `name` is the function name Lua asked for (useful for a "*" handler, registered to catch
 * every name of its plugin). Return 0 and set *out and *out_len to the result, or nonzero and set them to an error
 * message. *out must stay valid until the plugin's next call on the same thread (a thread-local buffer is fine). */
typedef int (*fse_fn)(void *ud, const char *name, const char *in, size_t in_len, const char **out,
                          size_t *out_len);

typedef struct fse_host {
    uint32_t abi;                                   /* FSE_ABI */
    const char *version;                            /* the core's version, "0.2.0" */
    void (*log)(const char *plugin, const char *msg);   /* a line in fse.log */
    void *(*engine_symbol)(const char *pdb_name);   /* address of an engine function by its factorio.pdb name, or NULL */
    /* register `name` (or "*") under `plugin`; returns 0, or nonzero if taken */
    int (*register_fn)(const char *plugin, const char *name, fse_fn fn, void *ud, uint32_t flags);
    /* --- added in core 0.3.0 (same ABI: appended fields; check host->version before relying on them) --- */
    const char *build;                              /* the game build, "GUID-age" of factorio.pdb */
    /* engine struct layouts, read from factorio.pdb for the running build: never hard-code offsets. Only classes
     * and fields listed in a needs file (plugins/<plugin>.needs.json: {"classes": {"Inserter": ["heldStack"]}})
     * are known; anything else, or what this build lacks, returns -1 */
    int64_t (*field_offset)(const char *class_name, const char *field);
    int64_t (*class_size)(const char *class_name);
    /* --- added in core 0.4.0 --- another plugin's function, as Lua's native.call does (from a worker thread call
     * only threadsafe ones). *out stays valid until this thread's next host call. */
    int (*call)(const char *plugin, const char *name, const char *in, size_t in_len, const char **out, size_t *out_len);
} fse_host;

typedef int (*fse_plugin_init_fn)(const fse_host *host);

#ifdef __cplusplus
}
#endif
#endif
