#include "shim_internal.h"

#include <stdexcept>

namespace pacm_v8 {

std::unique_ptr<v8::Platform> g_platform;
std::once_flag g_v8_once;

} // namespace pacm_v8

extern "C" {

int shim_v8_initialize(const char* icu_data_path) {
    try {
        std::call_once(pacm_v8::g_v8_once, [icu_data_path]() {
            bool icu_ready = false;
            if (icu_data_path && icu_data_path[0] != '\0') {
                icu_ready = v8::V8::InitializeICU(icu_data_path);
            } else {
                icu_ready = v8::V8::InitializeICUDefaultLocation(nullptr);
            }
            if (!icu_ready) {
                throw std::runtime_error("ICU initialization failed");
            }

            pacm_v8::g_platform = v8::platform::NewDefaultPlatform();
            v8::V8::InitializePlatform(pacm_v8::g_platform.get());
            v8::V8::Initialize();
        });
    } catch (...) {
        return 0;
    }

    return 1;
}

V8IsolateHandle shim_create_isolate() {
    v8::Isolate::CreateParams params;
    params.array_buffer_allocator = v8::ArrayBuffer::Allocator::NewDefaultAllocator();

    v8::Isolate* isolate = v8::Isolate::New(params);
    if (!isolate) {
        delete params.array_buffer_allocator;
        return nullptr;
    }

    auto* wrapper = new pacm_v8::IsolateWrapper();
    wrapper->isolate = isolate;
    wrapper->allocator = params.array_buffer_allocator;
    return reinterpret_cast<V8IsolateHandle>(wrapper);
}

void shim_dispose_isolate(V8IsolateHandle handle) {
    auto* wrapper = pacm_v8::unwrap_isolate(handle);
    if (!wrapper) {
        return;
    }

    if (wrapper->isolate) {
        wrapper->isolate->Dispose();
        wrapper->isolate = nullptr;
    }
    delete wrapper->allocator;
    wrapper->allocator = nullptr;
    delete wrapper;
}

} // extern "C"
