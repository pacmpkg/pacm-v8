#pragma once

#include "shim.h"
#include "v8.h"
#include "libplatform/libplatform.h"

#include <cstddef>
#include <cstdint>
#include <functional>
#include <memory>
#include <mutex>
#include <string>
#include <string_view>
#include <unordered_map>
#include <chrono>

namespace pacm_v8 {

struct ScriptCacheHash {
    using is_transparent = void;
    std::size_t operator()(std::string_view value) const noexcept {
        return std::hash<std::string_view>{}(value);
    }
};

struct ScriptCacheEq {
    using is_transparent = void;
    bool operator()(std::string_view lhs, std::string_view rhs) const noexcept {
        return lhs == rhs;
    }
};

struct IsolateWrapper {
    v8::Isolate* isolate;
    v8::ArrayBuffer::Allocator* allocator;
};

struct NativeCallbackData;

struct ContextWrapper {
    IsolateWrapper* isolate_wrapper;
    std::unique_ptr<v8::Global<v8::Context>> context;
    std::unordered_map<std::string, std::unique_ptr<v8::Global<v8::UnboundScript>>, ScriptCacheHash, ScriptCacheEq> cache;
    std::unordered_map<std::string, std::unique_ptr<NativeCallbackData>, ScriptCacheHash, ScriptCacheEq> native_callbacks;

    v8::Isolate* isolate() const { return isolate_wrapper ? isolate_wrapper->isolate : nullptr; }
};

struct ScriptWrapper {
    IsolateWrapper* isolate_wrapper;
    std::unique_ptr<v8::Global<v8::UnboundScript>> script;
    std::string cache_key;
};

struct NativeCallbackData {
    uint64_t function_id;
};

inline IsolateWrapper* unwrap_isolate(V8IsolateHandle handle) {
    return reinterpret_cast<IsolateWrapper*>(handle);
}

inline ContextWrapper* unwrap_context(V8ContextHandle handle) {
    return reinterpret_cast<ContextWrapper*>(handle);
}

inline ScriptWrapper* unwrap_script(V8ScriptHandle handle) {
    return reinterpret_cast<ScriptWrapper*>(handle);
}

char* copy_string(const std::string& value);
char* copy_string(const char* data, std::size_t length);
char* value_to_utf8(v8::Isolate* isolate, v8::Local<v8::Value> value);
void assign_error(char** error_out, const std::string& message);
bool capture_exception(v8::Isolate* isolate, v8::TryCatch& try_catch, std::string& message_out);

bool ensure_isolate(V8IsolateHandle handle, IsolateWrapper*& out, std::string& error_out);
bool ensure_context(V8ContextHandle handle, ContextWrapper*& out, std::string& error_out);
bool ensure_script(V8ScriptHandle handle, ScriptWrapper*& out, std::string& error_out);

extern std::unique_ptr<v8::Platform> g_platform;
extern std::once_flag g_v8_once;

} // namespace pacm_v8

extern "C" int pacm_v8__host_function_invoke(uint64_t function_id, const char** args, std::size_t arg_count, char** result_out, char** error_out);
extern "C" void pacm_v8__host_function_drop(uint64_t function_id);
extern "C" void pacm_v8__string_free(char* value);
