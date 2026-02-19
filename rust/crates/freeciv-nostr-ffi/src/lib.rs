//! C FFI bindings for freeciv-nostr Rust crates. Exposes networking and crypto
//! functions to the C game engine.

use std::ffi::CString;
use std::os::raw::c_char;
use std::sync::OnceLock;

static VERSION: OnceLock<CString> = OnceLock::new();

/// Return the freeciv-nostr library version as a C string.
///
/// # Safety
///
/// The returned pointer is valid for the lifetime of the program (static).
#[unsafe(no_mangle)]
pub extern "C" fn fcn_version() -> *const c_char {
    VERSION
        .get_or_init(|| {
            CString::new(env!("CARGO_PKG_VERSION")).expect("version contains null byte")
        })
        .as_ptr()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn version_returns_valid_non_null_pointer() {
        let ptr = fcn_version();
        assert!(!ptr.is_null());
        let cstr = unsafe { CStr::from_ptr(ptr) };
        assert_eq!(cstr.to_str().unwrap(), env!("CARGO_PKG_VERSION"));
    }
}
