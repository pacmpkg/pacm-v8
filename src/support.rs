use std::ffi::CStr;
use std::os::raw::c_char;

use crate::error::V8Error;
use crate::ffi;

pub(crate) unsafe fn take_string(ptr: *mut c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let string = unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    unsafe {
        ffi::shim_free_string(ptr);
    }
    Some(string)
}

pub(crate) unsafe fn take_error(ptr: *mut c_char, fallback: &str) -> V8Error {
    let message = unsafe { take_string(ptr) }.unwrap_or_else(|| fallback.to_string());
    V8Error::new(message)
}
