#include "shim_internal.h"

#include <cstdlib>
#include <cstring>

namespace pacm_v8 {

char* copy_string(const std::string& value) {
    return copy_string(value.data(), value.size());
}

char* copy_string(const char* data, std::size_t length) {
    if (!data) {
        length = 0;
    }
    char* buffer = static_cast<char*>(std::malloc(length + 1));
    if (!buffer) {
        return nullptr;
    }
    if (length > 0 && data) {
        std::memcpy(buffer, data, length);
    }
    buffer[length] = '\0';
    return buffer;
}

char* value_to_utf8(v8::Isolate* isolate, v8::Local<v8::Value> value) {
    if (!isolate || value.IsEmpty()) {
        return copy_string("");
    }
    v8::String::Utf8Value utf8(isolate, value);
    if (*utf8) {
        return copy_string(*utf8, static_cast<std::size_t>(utf8.length()));
    }
    return copy_string("");
}

void assign_error(char** error_out, const std::string& message) {
    if (!error_out) {
        return;
    }
    *error_out = copy_string(message);
}

bool capture_exception(v8::Isolate* isolate, v8::TryCatch& try_catch, std::string& message_out) {
    if (!isolate) {
        message_out = "unknown V8 exception";
        return false;
    }

    if (try_catch.HasCaught()) {
        v8::HandleScope scope(isolate);
        v8::String::Utf8Value exception(isolate, try_catch.Exception());
        if (*exception) {
            message_out.assign(*exception, exception.length());
        } else {
            message_out = "unknown V8 exception";
        }

        v8::Local<v8::Message> message = try_catch.Message();
        if (!message.IsEmpty()) {
            v8::String::Utf8Value detailed(isolate, message->Get());
            if (*detailed) {
                message_out.append("\n");
                message_out.append(*detailed, detailed.length());
            }
        }
        return true;
    }

    message_out = "V8 execution failed";
    return false;
}

bool ensure_isolate(V8IsolateHandle handle, IsolateWrapper*& out, std::string& error_out) {
    out = unwrap_isolate(handle);
    if (!out || !out->isolate) {
        error_out = "invalid isolate handle";
        return false;
    }
    return true;
}

} // namespace pacm_v8

extern "C" {

void shim_free_string(char* value) {
    if (value) {
        std::free(value);
    }
}

char* shim_eval(V8ContextHandle handle, const char* source) {
    char* result = nullptr;
    char* error = nullptr;
    if (shim_context_eval(handle, source, &result, &error)) {
        return result;
    }
    return error;
}

} // extern "C"
