#pragma once
#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef void* V8IsolateHandle;
typedef void* V8ContextHandle;
typedef void* V8ScriptHandle;

// einmalige Initialisierung. Optionaler Pfad zu icudtl.dat (UTF-8 kodiert).
int shim_v8_initialize(const char* icu_data_path);

// Isolate erzeugen / zerstören
V8IsolateHandle shim_create_isolate();
void shim_dispose_isolate(V8IsolateHandle isolate);

// Context erzeugen / zerstören
V8ContextHandle shim_create_context(V8IsolateHandle isolate);
void shim_dispose_context(V8ContextHandle ctx);

// Context helpers
int shim_context_eval(V8ContextHandle ctx, const char* source, char** result_out, char** error_out);
int shim_context_set_global_string(V8ContextHandle ctx, const char* name, const char* value, char** error_out);
int shim_context_set_global_number(V8ContextHandle ctx, const char* name, double value, char** error_out);
int shim_context_register_host_function(V8ContextHandle ctx, const char* name, uint64_t function_id, char** error_out);
int shim_context_call_function(
	V8ContextHandle ctx,
	const char* fn_name,
	const char** args,
	size_t arg_count,
	char** result_out,
	char** error_out
);

// Script helpers
V8ScriptHandle shim_compile_script(V8IsolateHandle isolate, const char* source, char** error_out);
int shim_script_run(V8ScriptHandle script, V8ContextHandle ctx, char** result_out, char** error_out);
void shim_script_dispose(V8ScriptHandle script);

// Legacy eval helper for backwards compatibility
char* shim_eval(V8ContextHandle ctx, const char* source);
void shim_free_string(char* s);

#ifdef __cplusplus
}
#endif