use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;
use std::slice;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::error::{Result, V8Error};
use crate::value::JsValue;

type HostCallback = dyn Fn(&[JsValue]) -> Result<Option<JsValue>> + Send + Sync + 'static;

struct HostFunctionEntry {
    callback: Arc<HostCallback>,
}

static REGISTRY: OnceLock<Mutex<HashMap<u64, HostFunctionEntry>>> = OnceLock::new();
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn registry() -> &'static Mutex<HashMap<u64, HostFunctionEntry>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn register<F>(callback: F) -> u64
where
    F: Fn(&[JsValue]) -> Result<Option<JsValue>> + Send + Sync + 'static,
{
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let entry = HostFunctionEntry {
        callback: Arc::from(Box::new(callback) as Box<HostCallback>),
    };
    let mut guard = registry().lock().unwrap();
    guard.insert(id, entry);
    id
}

pub(crate) fn drop_function(id: u64) {
    let mut guard = match REGISTRY.get() {
        Some(lock) => lock.lock().unwrap(),
        None => return,
    };
    guard.remove(&id);
}

pub(crate) fn drop_many(ids: impl IntoIterator<Item = u64>) {
    for id in ids {
        drop_function(id);
    }
}

fn invoke(id: u64, args: &[JsValue]) -> Result<Option<JsValue>> {
    let callback = {
        let guard = registry().lock().unwrap();
        guard
            .get(&id)
            .map(|entry| Arc::clone(&entry.callback))
            .ok_or_else(|| V8Error::new("native function not found"))?
    };
    (callback)(args)
}

unsafe fn convert_args(args: *const *const c_char, count: usize) -> Result<Vec<JsValue>> {
    if args.is_null() || count == 0 {
        return Ok(Vec::new());
    }

    let arg_slice = unsafe { slice::from_raw_parts(args, count) };
    let mut values = Vec::with_capacity(count);

    for &ptr in arg_slice {
        if ptr.is_null() {
            values.push(JsValue::new(String::new()));
            continue;
        }

        let value = unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned();
        values.push(JsValue::new(value));
    }

    Ok(values)
}

unsafe fn set_string(out: *mut *mut c_char, value: Option<String>) -> Result<()> {
    if out.is_null() {
        return Ok(());
    }

    unsafe {
        *out = ptr::null_mut();
    }
    if let Some(value) = value {
        let cstring =
            CString::new(value).map_err(|_| V8Error::new("string contained interior null byte"))?;
        unsafe {
            *out = cstring.into_raw();
        }
    }
    Ok(())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pacm_v8__host_function_invoke(
    id: u64,
    args: *const *const c_char,
    arg_count: usize,
    result_out: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> i32 {
    if !result_out.is_null() {
        unsafe {
            *result_out = ptr::null_mut();
        }
    }
    if !error_out.is_null() {
        unsafe {
            *error_out = ptr::null_mut();
        }
    }

    let args = match unsafe { convert_args(args, arg_count) } {
        Ok(values) => values,
        Err(error) => {
            if !error_out.is_null() {
                let message = CString::new(error.message())
                    .unwrap_or_else(|_| CString::new("failed to convert arguments").unwrap());
                unsafe {
                    *error_out = message.into_raw();
                }
            }
            return 0;
        }
    };

    match invoke(id, &args) {
        Ok(Some(value)) => match unsafe { set_string(result_out, Some(value.into_string())) } {
            Ok(_) => 1,
            Err(error) => {
                if !error_out.is_null() {
                    let message = CString::new(error.message()).unwrap_or_else(|_| {
                        CString::new("host function result contained interior null byte").unwrap()
                    });
                    unsafe {
                        *error_out = message.into_raw();
                    }
                }
                0
            }
        },
        Ok(None) => 1,
        Err(error) => {
            if !error_out.is_null() {
                let message = CString::new(error.message())
                    .unwrap_or_else(|_| CString::new("host function failed").unwrap());
                unsafe {
                    *error_out = message.into_raw();
                }
            }
            0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pacm_v8__host_function_drop(id: u64) {
    drop_function(id);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pacm_v8__string_free(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    let _ = unsafe { CString::from_raw(ptr) };
}
