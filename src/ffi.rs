use std::os::raw::{c_char, c_double};

pub type V8IsolateHandle = *mut std::ffi::c_void;
pub type V8ContextHandle = *mut std::ffi::c_void;
pub type V8ScriptHandle = *mut std::ffi::c_void;

unsafe extern "C" {
    pub fn shim_v8_initialize(icu_data_path: *const c_char) -> i32;

    pub fn shim_create_isolate() -> V8IsolateHandle;
    pub fn shim_dispose_isolate(isolate: V8IsolateHandle);

    pub fn shim_create_context(isolate: V8IsolateHandle) -> V8ContextHandle;
    pub fn shim_dispose_context(context: V8ContextHandle);

    pub fn shim_context_eval(
        context: V8ContextHandle,
        source: *const c_char,
        result_out: *mut *mut c_char,
        error_out: *mut *mut c_char,
    ) -> i32;

    pub fn shim_context_set_global_string(
        context: V8ContextHandle,
        name: *const c_char,
        value: *const c_char,
        error_out: *mut *mut c_char,
    ) -> i32;

    pub fn shim_context_set_global_number(
        context: V8ContextHandle,
        name: *const c_char,
        value: c_double,
        error_out: *mut *mut c_char,
    ) -> i32;

    pub fn shim_context_register_host_function(
        context: V8ContextHandle,
        name: *const c_char,
        function_id: u64,
        error_out: *mut *mut c_char,
    ) -> i32;

    pub fn shim_context_call_function(
        context: V8ContextHandle,
        fn_name: *const c_char,
        args: *const *const c_char,
        arg_count: usize,
        result_out: *mut *mut c_char,
        error_out: *mut *mut c_char,
    ) -> i32;

    pub fn shim_compile_script(
        isolate: V8IsolateHandle,
        source: *const c_char,
        error_out: *mut *mut c_char,
    ) -> V8ScriptHandle;

    pub fn shim_script_run(
        script: V8ScriptHandle,
        context: V8ContextHandle,
        result_out: *mut *mut c_char,
        error_out: *mut *mut c_char,
    ) -> i32;

    pub fn shim_script_dispose(script: V8ScriptHandle);
    pub fn shim_free_string(ptr: *mut c_char);
}
