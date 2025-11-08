#include "shim_internal.h"

#include <cstring>
#include <vector>

namespace {
constexpr std::size_t kMaxCacheableSourceLength = 64 * 1024;
}

namespace pacm_v8 {

static bool ensure_property_path(
    v8::Isolate* isolate,
    v8::Local<v8::Context> ctx,
    std::string_view path,
    v8::Local<v8::Object>& target_out,
    v8::Local<v8::String>& property_out,
    std::string& error_out) {
    if (path.empty()) {
        error_out = "property name was empty";
        return false;
    }

    v8::Local<v8::Object> current = ctx->Global();
    std::size_t start = 0;
    while (start < path.size()) {
        std::size_t dot = path.find('.', start);
        std::string_view segment = dot == std::string_view::npos ? path.substr(start) : path.substr(start, dot - start);
        if (segment.empty()) {
            error_out = "property path contained an empty segment";
            return false;
        }

        std::string segment_str(segment);
        v8::Local<v8::String> key = v8::String::NewFromUtf8(isolate, segment_str.c_str(), v8::NewStringType::kNormal).ToLocalChecked();

        if (dot == std::string_view::npos) {
            target_out = current;
            property_out = key;
            return true;
        }

        v8::Local<v8::Value> next;
        if (!current->Get(ctx, key).ToLocal(&next) || next->IsUndefined() || next->IsNull()) {
            v8::Local<v8::Object> fresh = v8::Object::New(isolate);
            if (!current->Set(ctx, key, fresh).FromMaybe(false)) {
                error_out = "failed to assign intermediate object on property path";
                return false;
            }
            current = fresh;
        } else if (!next->IsObject()) {
            error_out = "property path conflicts with existing non-object value";
            return false;
        } else {
            current = next.As<v8::Object>();
        }

        start = dot + 1;
    }

    error_out = "property name was empty";
    return false;
}

static void dispose_native_callbacks(ContextWrapper* context) {
    for (auto& entry : context->native_callbacks) {
        if (entry.second) {
            ::pacm_v8__host_function_drop(entry.second->function_id);
        }
    }
    context->native_callbacks.clear();
}

static void native_function_trampoline(const v8::FunctionCallbackInfo<v8::Value>& info) {
    v8::Isolate* isolate = info.GetIsolate();
    v8::HandleScope handle_scope(isolate);

    if (info.Data().IsEmpty()) {
        isolate->ThrowException(v8::String::NewFromUtf8(isolate, "host function metadata missing", v8::NewStringType::kNormal).ToLocalChecked());
        return;
    }

    auto external = info.Data().As<v8::External>();
    auto* data = static_cast<NativeCallbackData*>(external->Value());
    if (!data) {
        isolate->ThrowException(v8::String::NewFromUtf8(isolate, "host function metadata missing", v8::NewStringType::kNormal).ToLocalChecked());
        return;
    }

    std::vector<char*> arguments;
    arguments.reserve(info.Length());
    for (int i = 0; i < info.Length(); ++i) {
        arguments.push_back(value_to_utf8(isolate, info[i]));
    }

    const char** argv = arguments.empty() ? nullptr : const_cast<const char**>(arguments.data());
    char* result_ptr = nullptr;
    char* error_ptr = nullptr;

    int status = ::pacm_v8__host_function_invoke(data->function_id, argv, arguments.size(), &result_ptr, &error_ptr);

    for (char* value : arguments) {
        shim_free_string(value);
    }

    if (!status) {
        std::string message = error_ptr ? std::string(error_ptr) : std::string("host function invocation failed");
        if (error_ptr) {
            ::pacm_v8__string_free(error_ptr);
        }
        isolate->ThrowException(v8::String::NewFromUtf8(isolate, message.c_str(), v8::NewStringType::kNormal).ToLocalChecked());
        return;
    }

    if (error_ptr) {
        ::pacm_v8__string_free(error_ptr);
    }

    if (result_ptr) {
        v8::Local<v8::String> result = v8::String::NewFromUtf8(isolate, result_ptr, v8::NewStringType::kNormal).ToLocalChecked();
        ::pacm_v8__string_free(result_ptr);
        info.GetReturnValue().Set(result);
    } else {
        info.GetReturnValue().Set(v8::Undefined(isolate));
    }
}

bool ensure_context(V8ContextHandle handle, ContextWrapper*& out, std::string& error_out) {
    out = unwrap_context(handle);
    if (!out || !out->isolate()) {
        error_out = "invalid V8 context handle";
        return false;
    }
    return true;
}

} // namespace pacm_v8

extern "C" {

V8ContextHandle shim_create_context(V8IsolateHandle handle) {
    pacm_v8::IsolateWrapper* isolate_wrapper = pacm_v8::unwrap_isolate(handle);
    if (!isolate_wrapper || !isolate_wrapper->isolate) {
        return nullptr;
    }

    v8::Isolate* isolate = isolate_wrapper->isolate;
    v8::Isolate::Scope isolate_scope(isolate);
    v8::HandleScope handle_scope(isolate);
    v8::Local<v8::Context> local_context = v8::Context::New(isolate);
    auto persistent = std::make_unique<v8::Global<v8::Context>>(isolate, local_context);

    auto* wrapper = new pacm_v8::ContextWrapper();
    wrapper->isolate_wrapper = isolate_wrapper;
    wrapper->context = std::move(persistent);
    return reinterpret_cast<V8ContextHandle>(wrapper);
}

void shim_dispose_context(V8ContextHandle handle) {
    pacm_v8::ContextWrapper* context = pacm_v8::unwrap_context(handle);
    if (!context) {
        return;
    }

    for (auto& entry : context->cache) {
        if (entry.second) {
            entry.second->Reset();
        }
    }
    context->cache.clear();

    pacm_v8::dispose_native_callbacks(context);

    if (context->context) {
        context->context->Reset();
    }

    delete context;
}

int shim_context_eval(V8ContextHandle handle, const char* source, char** result_out, char** error_out) {
    if (result_out) {
        *result_out = nullptr;
    }
    if (error_out) {
        *error_out = nullptr;
    }

    pacm_v8::ContextWrapper* context = nullptr;
    std::string error;
    if (!pacm_v8::ensure_context(handle, context, error)) {
        pacm_v8::assign_error(error_out, error);
        return 0;
    }
    if (!source) {
        pacm_v8::assign_error(error_out, "source was null");
        return 0;
    }

    v8::Isolate* isolate = context->isolate();
    v8::Isolate::Scope isolate_scope(isolate);
    v8::HandleScope handle_scope(isolate);
    v8::Local<v8::Context> ctx = v8::Local<v8::Context>::New(isolate, *context->context);
    v8::Context::Scope context_scope(ctx);

    v8::TryCatch try_catch(isolate);

    v8::Local<v8::Script> script;
    auto cached = context->cache.find(source);
    if (cached != context->cache.end()) {
        v8::Local<v8::UnboundScript> unbound = v8::Local<v8::UnboundScript>::New(isolate, *cached->second);
        script = unbound->BindToCurrentContext();
    } else {
        v8::Local<v8::String> src = v8::String::NewFromUtf8(isolate, source, v8::NewStringType::kNormal).ToLocalChecked();
        if (!v8::Script::Compile(ctx, src).ToLocal(&script)) {
            std::string message;
            pacm_v8::capture_exception(isolate, try_catch, message);
            pacm_v8::assign_error(error_out, message);
            return 0;
        }

        const std::size_t len = std::strlen(source);
        if (len <= kMaxCacheableSourceLength) {
            v8::Local<v8::UnboundScript> unbound = script->GetUnboundScript();
            auto persistent = std::make_unique<v8::Global<v8::UnboundScript>>(isolate, unbound);
            context->cache.emplace(std::string{source, len}, std::move(persistent));
        }
    }

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

    return 1;
}

int shim_context_set_global_string(V8ContextHandle handle, const char* name, const char* value, char** error_out) {
    if (error_out) {
        *error_out = nullptr;
    }

    pacm_v8::ContextWrapper* context = nullptr;
    std::string error;
    if (!pacm_v8::ensure_context(handle, context, error)) {
        pacm_v8::assign_error(error_out, error);
        return 0;
    }

    if (!name) {
        pacm_v8::assign_error(error_out, "property name was null");
        return 0;
    }

    v8::Isolate* isolate = context->isolate();
    v8::Isolate::Scope isolate_scope(isolate);
    v8::HandleScope handle_scope(isolate);
    v8::Local<v8::Context> ctx = v8::Local<v8::Context>::New(isolate, *context->context);
    v8::Context::Scope context_scope(ctx);
    v8::TryCatch try_catch(isolate);

    v8::Local<v8::Object> target;
    v8::Local<v8::String> key;
    if (!pacm_v8::ensure_property_path(isolate, ctx, name, target, key, error)) {
        pacm_v8::assign_error(error_out, error);
        return 0;
    }
    v8::Local<v8::Value> js_value = v8::String::NewFromUtf8(isolate, value ? value : "", v8::NewStringType::kNormal).ToLocalChecked();

    if (!target->Set(ctx, key, js_value).FromMaybe(false)) {
        std::string message;
        pacm_v8::capture_exception(isolate, try_catch, message);
        pacm_v8::assign_error(error_out, message);
        return 0;
    }

    return 1;
}

int shim_context_set_global_number(V8ContextHandle handle, const char* name, double value, char** error_out) {
    if (error_out) {
        *error_out = nullptr;
    }

    pacm_v8::ContextWrapper* context = nullptr;
    std::string error;
    if (!pacm_v8::ensure_context(handle, context, error)) {
        pacm_v8::assign_error(error_out, error);
        return 0;
    }
    if (!name) {
        pacm_v8::assign_error(error_out, "property name was null");
        return 0;
    }

    v8::Isolate* isolate = context->isolate();
    v8::Isolate::Scope isolate_scope(isolate);
    v8::HandleScope handle_scope(isolate);
    v8::Local<v8::Context> ctx = v8::Local<v8::Context>::New(isolate, *context->context);
    v8::Context::Scope context_scope(ctx);
    v8::TryCatch try_catch(isolate);

    v8::Local<v8::Object> target;
    v8::Local<v8::String> key;
    if (!pacm_v8::ensure_property_path(isolate, ctx, name, target, key, error)) {
        pacm_v8::assign_error(error_out, error);
        return 0;
    }
    v8::Local<v8::Number> js_value = v8::Number::New(isolate, value);

    if (!target->Set(ctx, key, js_value).FromMaybe(false)) {
        std::string message;
        pacm_v8::capture_exception(isolate, try_catch, message);
        pacm_v8::assign_error(error_out, message);
        return 0;
    }

    return 1;
}

int shim_context_register_host_function(V8ContextHandle handle, const char* name, uint64_t function_id, char** error_out) {
    if (error_out) {
        *error_out = nullptr;
    }

    pacm_v8::ContextWrapper* context = nullptr;
    std::string error;
    if (!pacm_v8::ensure_context(handle, context, error)) {
        pacm_v8::assign_error(error_out, error);
        return 0;
    }

    if (!name) {
        pacm_v8::assign_error(error_out, "function name was null");
        return 0;
    }

    v8::Isolate* isolate = context->isolate();
    v8::Isolate::Scope isolate_scope(isolate);
    v8::HandleScope handle_scope(isolate);
    v8::Local<v8::Context> ctx = v8::Local<v8::Context>::New(isolate, *context->context);
    v8::Context::Scope context_scope(ctx);
    v8::TryCatch try_catch(isolate);

    v8::Local<v8::Object> target;
    v8::Local<v8::String> key;
    if (!pacm_v8::ensure_property_path(isolate, ctx, name, target, key, error)) {
        pacm_v8::assign_error(error_out, error);
        return 0;
    }

    auto data = std::make_unique<pacm_v8::NativeCallbackData>();
    data->function_id = function_id;

    v8::Local<v8::External> metadata = v8::External::New(isolate, data.get());
    v8::Local<v8::FunctionTemplate> tpl = v8::FunctionTemplate::New(isolate, pacm_v8::native_function_trampoline, metadata);
    v8::Local<v8::Function> function;
    if (!tpl->GetFunction(ctx).ToLocal(&function)) {
        std::string message;
        pacm_v8::capture_exception(isolate, try_catch, message);
        pacm_v8::assign_error(error_out, message);
        return 0;
    }

    function->SetName(key);

    if (!target->Set(ctx, key, function).FromMaybe(false)) {
        std::string message;
        pacm_v8::capture_exception(isolate, try_catch, message);
        pacm_v8::assign_error(error_out, message);
        return 0;
    }

    std::string path_key(name);
    auto existing = context->native_callbacks.find(path_key);
    if (existing != context->native_callbacks.end()) {
        if (existing->second) {
            ::pacm_v8__host_function_drop(existing->second->function_id);
        }
        context->native_callbacks.erase(existing);
    }

    context->native_callbacks.emplace(std::move(path_key), std::move(data));

    return 1;
}

int shim_context_call_function(V8ContextHandle handle, const char* fn_name, const char** args, std::size_t arg_count, char** result_out, char** error_out) {
    if (result_out) {
        *result_out = nullptr;
    }
    if (error_out) {
        *error_out = nullptr;
    }

    pacm_v8::ContextWrapper* context = nullptr;
    std::string error;
    if (!pacm_v8::ensure_context(handle, context, error)) {
        pacm_v8::assign_error(error_out, error);
        return 0;
    }
    if (!fn_name) {
        pacm_v8::assign_error(error_out, "function name was null");
        return 0;
    }

    v8::Isolate* isolate = context->isolate();
    v8::Isolate::Scope isolate_scope(isolate);
    v8::HandleScope handle_scope(isolate);
    v8::Local<v8::Context> ctx = v8::Local<v8::Context>::New(isolate, *context->context);
    v8::Context::Scope context_scope(ctx);
    v8::TryCatch try_catch(isolate);

    v8::Local<v8::Object> global = ctx->Global();
    v8::Local<v8::String> key = v8::String::NewFromUtf8(isolate, fn_name, v8::NewStringType::kNormal).ToLocalChecked();
    v8::Local<v8::Value> maybe_function;
    if (!global->Get(ctx, key).ToLocal(&maybe_function) || !maybe_function->IsFunction()) {
        pacm_v8::assign_error(error_out, "global function not found");
        return 0;
    }

    v8::Local<v8::Function> function = maybe_function.As<v8::Function>();

    std::vector<v8::Local<v8::Value>> js_args;
    js_args.reserve(arg_count);
    for (std::size_t i = 0; i < arg_count; ++i) {
        const char* arg = args ? args[i] : nullptr;
        if (!arg) {
            js_args.push_back(v8::Undefined(isolate));
            continue;
        }
        js_args.push_back(v8::String::NewFromUtf8(isolate, arg, v8::NewStringType::kNormal).ToLocalChecked());
    }

    v8::Local<v8::Value> result;
    if (!function->Call(ctx, global, static_cast<int>(js_args.size()), js_args.data()).ToLocal(&result)) {
        std::string message;
        pacm_v8::capture_exception(isolate, try_catch, message);
        pacm_v8::assign_error(error_out, message);
        return 0;
    }

    if (result_out) {
        *result_out = pacm_v8::value_to_utf8(isolate, result);
    }

    return 1;
}

} // extern "C"
