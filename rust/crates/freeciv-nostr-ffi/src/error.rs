//! Thread-local error handling for the FFI layer.
//!
//! Follows the SQLite pattern: when an FFI function fails, it stores a
//! human-readable error string in thread-local storage. The C caller
//! retrieves it with `fcn_last_error()`.

use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

/// Store an error message for the current thread.
///
/// Called internally by FFI functions before returning NULL or an error code.
pub(crate) fn set_last_error(msg: &str) {
    let c = CString::new(msg).unwrap_or_else(|_| {
        CString::new("error message contained null byte").expect("static string")
    });
    LAST_ERROR.with(|cell| {
        *cell.borrow_mut() = Some(c);
    });
}

/// Convenience: set last error from any Display-able type.
pub(crate) fn set_last_error_from<E: std::fmt::Display>(err: E) {
    set_last_error(&err.to_string());
}

/// Helper to convert a `*const c_char` to `&str`, setting an error on failure.
///
/// Returns `None` if the pointer is null or the string is not valid UTF-8.
pub(crate) fn cstr_to_str<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        set_last_error("null string pointer");
        return None;
    }
    // SAFETY: Caller guarantees ptr is valid for the duration of the call.
    let cstr = unsafe { CStr::from_ptr(ptr) };
    match cstr.to_str() {
        Ok(s) => Some(s),
        Err(e) => {
            set_last_error_from(e);
            None
        }
    }
}

/// Helper to convert a Rust `String` to an owned `*mut c_char`.
///
/// The caller must free the returned pointer with `fcn_string_free()`.
/// Returns `NULL` if the string contains a null byte (and sets last error).
pub(crate) fn string_to_c(s: String) -> *mut c_char {
    match CString::new(s) {
        Ok(c) => c.into_raw(),
        Err(e) => {
            set_last_error_from(e);
            std::ptr::null_mut()
        }
    }
}

// ---- Public FFI functions ----

/// Retrieve the last error message for the current thread.
///
/// Returns a pointer to a static string, or `NULL` if no error has occurred.
/// The returned pointer is valid until the next FFI call on the same thread.
///
/// # Safety
///
/// The returned pointer must NOT be freed by the caller. It points to
/// thread-local storage and is invalidated by the next FFI call.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_last_error() -> *const c_char {
    LAST_ERROR.with(|cell| {
        let borrow = cell.borrow();
        match borrow.as_ref() {
            Some(c) => c.as_ptr(),
            None => std::ptr::null(),
        }
    })
}

/// Free a string previously returned by an `fcn_*` function.
///
/// # Safety
///
/// `ptr` must be a pointer previously returned by an `fcn_*` function that
/// returns `*mut c_char`, or `NULL` (which is a no-op).
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn fcn_string_free(ptr: *mut c_char) {
    if !ptr.is_null() {
        // SAFETY: Caller guarantees ptr was allocated by CString::into_raw().
        unsafe {
            let _ = CString::from_raw(ptr);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn last_error_is_null_initially() {
        // Clear any prior state
        LAST_ERROR.with(|cell| *cell.borrow_mut() = None);
        let ptr = fcn_last_error();
        assert!(ptr.is_null());
    }

    #[test]
    fn set_and_get_error() {
        set_last_error("something went wrong");
        let ptr = fcn_last_error();
        assert!(!ptr.is_null());
        let msg = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        assert_eq!(msg, "something went wrong");
    }

    #[test]
    fn string_free_null_is_noop() {
        fcn_string_free(std::ptr::null_mut()); // should not crash
    }

    #[test]
    fn string_free_frees_valid_string() {
        let s = CString::new("hello").unwrap();
        let ptr = s.into_raw();
        fcn_string_free(ptr); // should not crash or leak
    }

    #[test]
    fn string_to_c_returns_valid_string() {
        let ptr = string_to_c("hello world".to_string());
        assert!(!ptr.is_null());
        let msg = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        assert_eq!(msg, "hello world");
        fcn_string_free(ptr);
    }

    #[test]
    fn cstr_to_str_null_sets_error() {
        LAST_ERROR.with(|cell| *cell.borrow_mut() = None);
        assert!(cstr_to_str(std::ptr::null()).is_none());
        let ptr = fcn_last_error();
        assert!(!ptr.is_null());
    }
}
