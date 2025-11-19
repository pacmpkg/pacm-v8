mod error;
mod ffi;
mod native;
mod support;
mod value;

pub use crate::error::{Result, V8Error};
pub use crate::value::JsValue;

// Ensure temporal_capi symbols are linked even though they're only used by V8's C++ code
extern crate temporal_capi;

use std::env;
use std::ffi::CString;
use std::os::raw::c_char;
use std::ptr;

use crate::ffi::{
    V8ContextHandle, V8IsolateHandle, V8ScriptHandle, shim_compile_script,
    shim_context_call_function, shim_context_eval, shim_context_register_host_function,
    shim_context_set_global_number, shim_context_set_global_string, shim_create_context,
    shim_create_isolate, shim_dispose_context, shim_dispose_isolate, shim_script_dispose,
    shim_script_run, shim_v8_initialize,
};
use crate::support::{take_error, take_string};

const NULL_BYTE_MESSAGE: &str = "input contained an interior null byte";

pub struct Isolate {
    handle: V8IsolateHandle,
}

pub struct Context {
    handle: V8ContextHandle,
    isolate: V8IsolateHandle,
    host_functions: Vec<u64>,
}

pub struct Script {
    handle: V8ScriptHandle,
    isolate: V8IsolateHandle,
}

fn resolve_icu_data_path() -> Option<String> {
    if let Ok(path) = env::var("PACM_V8_ICU_DATA_PATH") {
        if !path.is_empty() {
            return Some(path);
        }
    }

    if let Some(path) = option_env!("PACM_V8_ICU_DATA_PATH") {
        if !path.is_empty() {
            return Some(path.to_string());
        }
    }

    None
}

fn initialize_v8(icu_path: Option<&str>) -> Result<()> {
    let icu_cstring = match icu_path {
        Some(path) => Some(CString::new(path).map_err(|_| V8Error::new(NULL_BYTE_MESSAGE))?),
        None => None,
    };

    let icu_ptr = icu_cstring
        .as_ref()
        .map_or(ptr::null(), |value| value.as_ptr());

    let result = unsafe { shim_v8_initialize(icu_ptr) };
    if result == 0 {
        return Err(V8Error::new("failed to initialise V8"));
    }
    Ok(())
}

impl Isolate {
    pub fn new() -> Result<Self> {
        let icu_path = resolve_icu_data_path();
        initialize_v8(icu_path.as_deref())?;

        let handle = unsafe { shim_create_isolate() };
        if handle.is_null() {
            return Err(V8Error::new("failed to create V8 isolate"));
        }

        Ok(Self { handle })
    }

    pub fn raw_handle(&self) -> V8IsolateHandle {
        self.handle
    }

    pub fn create_context(&self) -> Result<Context> {
        if self.handle.is_null() {
            return Err(V8Error::new("isolate was disposed"));
        }

        let handle = unsafe { shim_create_context(self.handle) };
        if handle.is_null() {
            return Err(V8Error::new("failed to create V8 context"));
        }

        Ok(Context {
            handle,
            isolate: self.handle,
            host_functions: Vec::new(),
        })
    }

    pub fn dispose(&mut self) {
        if self.handle.is_null() {
            return;
        }
        unsafe {
            shim_dispose_isolate(self.handle);
        }
        self.handle = ptr::null_mut();
    }
}

impl Drop for Isolate {
    fn drop(&mut self) {
        self.dispose();
    }
}

impl Context {
    pub fn raw_handle(&self) -> V8ContextHandle {
        self.handle
    }

    pub fn isolate_handle(&self) -> V8IsolateHandle {
        self.isolate
    }

    pub fn eval(&self, source: &str) -> Result<JsValue> {
        if self.handle.is_null() {
            return Err(V8Error::new("context was disposed"));
        }

        let c_source = CString::new(source).map_err(|_| V8Error::new(NULL_BYTE_MESSAGE))?;
        let mut result_ptr: *mut c_char = ptr::null_mut();
        let mut error_ptr: *mut c_char = ptr::null_mut();

        let status = unsafe {
            shim_context_eval(
                self.handle,
                c_source.as_ptr(),
                &mut result_ptr,
                &mut error_ptr,
            )
        };

        if status == 0 {
            return Err(unsafe { take_error(error_ptr, "V8 evaluation failed") });
        }

        let value = unsafe { take_string(result_ptr).unwrap_or_default() };
        Ok(JsValue::new(value))
    }

    pub fn set_global_str(&self, name: &str, value: &str) -> Result<()> {
        if self.handle.is_null() {
            return Err(V8Error::new("context was disposed"));
        }
        let c_name = CString::new(name).map_err(|_| V8Error::new(NULL_BYTE_MESSAGE))?;
        let c_value = CString::new(value).map_err(|_| V8Error::new(NULL_BYTE_MESSAGE))?;
        let mut error_ptr: *mut c_char = ptr::null_mut();

        let status = unsafe {
            shim_context_set_global_string(
                self.handle,
                c_name.as_ptr(),
                c_value.as_ptr(),
                &mut error_ptr,
            )
        };

        if status == 0 {
            return Err(unsafe { take_error(error_ptr, "failed to set global string") });
        }

        Ok(())
    }

    pub fn set_global_number(&self, name: &str, value: f64) -> Result<()> {
        if self.handle.is_null() {
            return Err(V8Error::new("context was disposed"));
        }

        let c_name = CString::new(name).map_err(|_| V8Error::new(NULL_BYTE_MESSAGE))?;
        let mut error_ptr: *mut c_char = ptr::null_mut();

        let status = unsafe {
            shim_context_set_global_number(self.handle, c_name.as_ptr(), value, &mut error_ptr)
        };

        if status == 0 {
            return Err(unsafe { take_error(error_ptr, "failed to set global number") });
        }

        Ok(())
    }

    pub fn add_function<F>(&mut self, name: &str, func: F) -> Result<()>
    where
        F: Fn(&[JsValue]) -> Result<Option<JsValue>> + Send + Sync + 'static,
    {
        if self.handle.is_null() {
            return Err(V8Error::new("context was disposed"));
        }

        let c_name = CString::new(name).map_err(|_| V8Error::new(NULL_BYTE_MESSAGE))?;
        let function_id = native::register(func);
        let mut error_ptr: *mut c_char = ptr::null_mut();

        let status = unsafe {
            shim_context_register_host_function(
                self.handle,
                c_name.as_ptr(),
                function_id,
                &mut error_ptr,
            )
        };

        if status == 0 {
            native::drop_function(function_id);
            return Err(unsafe { take_error(error_ptr, "failed to register host function") });
        }

        self.host_functions.push(function_id);
        Ok(())
    }

    pub fn call_function(&self, fn_name: &str, args: &[&str]) -> Result<JsValue> {
        if self.handle.is_null() {
            return Err(V8Error::new("context was disposed"));
        }

        let c_name = CString::new(fn_name).map_err(|_| V8Error::new(NULL_BYTE_MESSAGE))?;
        let arg_cstrings: Result<Vec<CString>> = args
            .iter()
            .map(|value| CString::new(*value).map_err(|_| V8Error::new(NULL_BYTE_MESSAGE)))
            .collect();
        let arg_cstrings = arg_cstrings?;
        let arg_ptrs: Vec<*const c_char> =
            arg_cstrings.iter().map(|value| value.as_ptr()).collect();
        let arg_ptr = if arg_ptrs.is_empty() {
            ptr::null()
        } else {
            arg_ptrs.as_ptr()
        };

        let mut result_ptr: *mut c_char = ptr::null_mut();
        let mut error_ptr: *mut c_char = ptr::null_mut();

        let status = unsafe {
            shim_context_call_function(
                self.handle,
                c_name.as_ptr(),
                arg_ptr,
                arg_ptrs.len(),
                &mut result_ptr,
                &mut error_ptr,
            )
        };

        if status == 0 {
            return Err(unsafe { take_error(error_ptr, "failed to call function") });
        }

        let value = unsafe { take_string(result_ptr).unwrap_or_default() };
        Ok(JsValue::new(value))
    }

    pub fn dispose(&mut self) {
        if !self.host_functions.is_empty() {
            native::drop_many(self.host_functions.drain(..));
        }
        if self.handle.is_null() {
            return;
        }
        unsafe {
            shim_dispose_context(self.handle);
        }
        self.handle = ptr::null_mut();
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        self.dispose();
    }
}

impl Script {
    pub fn compile(isolate: &Isolate, source: &str) -> Result<Self> {
        if isolate.handle.is_null() {
            return Err(V8Error::new("isolate was disposed"));
        }

        let c_source = CString::new(source).map_err(|_| V8Error::new(NULL_BYTE_MESSAGE))?;
        let mut error_ptr: *mut c_char = ptr::null_mut();

        let handle =
            unsafe { shim_compile_script(isolate.handle, c_source.as_ptr(), &mut error_ptr) };

        if handle.is_null() {
            return Err(unsafe { take_error(error_ptr, "failed to compile script") });
        }

        Ok(Self {
            handle,
            isolate: isolate.handle,
        })
    }

    pub fn raw_handle(&self) -> V8ScriptHandle {
        self.handle
    }

    pub fn run(&self, context: &Context) -> Result<JsValue> {
        if self.handle.is_null() {
            return Err(V8Error::new("script was disposed"));
        }
        if context.handle.is_null() {
            return Err(V8Error::new("context was disposed"));
        }
        if context.isolate != self.isolate {
            return Err(V8Error::new(
                "script and context belong to different isolates",
            ));
        }

        let mut result_ptr: *mut c_char = ptr::null_mut();
        let mut error_ptr: *mut c_char = ptr::null_mut();

        let status = unsafe {
            shim_script_run(self.handle, context.handle, &mut result_ptr, &mut error_ptr)
        };

        if status == 0 {
            return Err(unsafe { take_error(error_ptr, "failed to run script") });
        }

        let value = unsafe { take_string(result_ptr).unwrap_or_default() };
        Ok(JsValue::new(value))
    }

    pub fn dispose(&mut self) {
        if self.handle.is_null() {
            return;
        }
        unsafe {
            shim_script_dispose(self.handle);
        }
        self.handle = ptr::null_mut();
    }
}

impl Drop for Script {
    fn drop(&mut self) {
        self.dispose();
    }
}
