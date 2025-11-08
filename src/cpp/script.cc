#include "shim_internal.h"

namespace pacm_v8 {

bool ensure_script(V8ScriptHandle handle, ScriptWrapper*& out, std::string& error_out) {
    out = unwrap_script(handle);
    if (!out || !out->script) {
        error_out = "invalid V8 script handle";
        return false;
    }
    return true;
}

} // namespace pacm_v8

extern "C" {

V8ScriptHandle shim_compile_script(V8IsolateHandle handle, const char* source, char** error_out) {
    if (error_out) {
        *error_out = nullptr;
    }

    pacm_v8::IsolateWrapper* isolate_wrapper = pacm_v8::unwrap_isolate(handle);
    if (!isolate_wrapper || !isolate_wrapper->isolate) {
        pacm_v8::assign_error(error_out, "invalid isolate handle");
        return nullptr;
    }
    if (!source) {
        pacm_v8::assign_error(error_out, "source was null");
        return nullptr;
    }

    v8::Isolate* isolate = isolate_wrapper->isolate;
    v8::Isolate::Scope isolate_scope(isolate);
    v8::HandleScope handle_scope(isolate);
    v8::Local<v8::Context> context = isolate->GetCurrentContext();
    if (context.IsEmpty()) {
        context = v8::Context::New(isolate);
    }
    v8::Context::Scope context_scope(context);
    v8::TryCatch try_catch(isolate);

    v8::Local<v8::String> src = v8::String::NewFromUtf8(isolate, source, v8::NewStringType::kNormal).ToLocalChecked();
    v8::Local<v8::Script> compiled;
    if (!v8::Script::Compile(context, src).ToLocal(&compiled)) {
        std::string message;
        pacm_v8::capture_exception(isolate, try_catch, message);
        pacm_v8::assign_error(error_out, message);
        return nullptr;
    }

    auto* wrapper = new pacm_v8::ScriptWrapper();
    wrapper->isolate_wrapper = isolate_wrapper;
    wrapper->cache_key.assign(source);
    wrapper->script = std::make_unique<v8::Global<v8::UnboundScript>>(isolate, compiled->GetUnboundScript());
    return reinterpret_cast<V8ScriptHandle>(wrapper);
}

int shim_script_run(V8ScriptHandle script_handle, V8ContextHandle context_handle, char** result_out, char** error_out) {
    if (result_out) {
        *result_out = nullptr;
    }
    if (error_out) {
        *error_out = nullptr;
    }

    pacm_v8::ScriptWrapper* script_wrapper = nullptr;
    std::string error;
    if (!pacm_v8::ensure_script(script_handle, script_wrapper, error)) {
        pacm_v8::assign_error(error_out, error);
        return 0;
    }

    pacm_v8::ContextWrapper* context_wrapper = nullptr;
    if (!pacm_v8::ensure_context(context_handle, context_wrapper, error)) {
        pacm_v8::assign_error(error_out, error);
        return 0;
    }

    v8::Isolate* isolate = context_wrapper->isolate();
    if (!isolate || isolate != script_wrapper->isolate_wrapper->isolate) {
        pacm_v8::assign_error(error_out, "script and context belong to different isolates");
        return 0;
    }

    v8::Isolate::Scope isolate_scope(isolate);
    v8::HandleScope handle_scope(isolate);
    v8::Local<v8::Context> ctx = v8::Local<v8::Context>::New(isolate, *context_wrapper->context);
    v8::Context::Scope context_scope(ctx);
    v8::TryCatch try_catch(isolate);

    v8::Local<v8::UnboundScript> unbound = v8::Local<v8::UnboundScript>::New(isolate, *script_wrapper->script);
    v8::Local<v8::Script> script = unbound->BindToCurrentContext();

    v8::Local<v8::Value> result;
    if (!script->Run(ctx).ToLocal(&result)) {
        std::string message;
        pacm_v8::capture_exception(isolate, try_catch, message);
        pacm_v8::assign_error(error_out, message);
        return 0;
    }

    if (result_out) {
        *result_out = pacm_v8::value_to_utf8(isolate, result);
    }

    if (!script_wrapper->cache_key.empty()) {
        auto persistent = std::make_unique<v8::Global<v8::UnboundScript>>(isolate, unbound);
        context_wrapper->cache.insert_or_assign(script_wrapper->cache_key, std::move(persistent));
    }

    return 1;
}

void shim_script_dispose(V8ScriptHandle handle) {
    pacm_v8::ScriptWrapper* wrapper = pacm_v8::unwrap_script(handle);
    if (!wrapper) {
        return;
    }

    if (wrapper->script) {
        wrapper->script->Reset();
    }

    delete wrapper;
}

} // extern "C"
