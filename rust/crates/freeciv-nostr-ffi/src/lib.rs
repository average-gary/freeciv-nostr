//! C FFI bindings for freeciv-nostr Rust crates.
//!
//! Exposes identity, signing, verification, and chain management to the C
//! game engine through opaque pointer types and JSON-based event marshalling.
//!
//! # Error Handling
//!
//! Functions that can fail return `NULL` (for pointer-returning functions) or
//! a negative error code (for int-returning functions). In both cases, a
//! human-readable error message is available via `fcn_last_error()`.
//!
//! # Memory Ownership
//!
//! - Opaque pointers (`*mut FcnIdentity`, etc.) are owned by the caller and
//!   must be freed with the corresponding `fcn_*_free()` function.
//! - Returned `*mut c_char` strings are owned by the caller and must be freed
//!   with `fcn_string_free()`.
//! - String parameters (`*const c_char`) are borrowed for the duration of the
//!   call; the caller retains ownership.

mod chain;
mod error;
mod identity;
mod signer;
mod util;
mod verifier;

use std::ffi::CString;
use std::os::raw::c_char;
use std::sync::OnceLock;

// Re-export the error infrastructure at crate root for the version function.
pub use error::{fcn_last_error, fcn_string_free};

static VERSION: OnceLock<CString> = OnceLock::new();

/// Return the freeciv-nostr library version as a C string.
///
/// # Safety
///
/// The returned pointer is valid for the lifetime of the program (static).
/// The caller must NOT free this pointer.
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
