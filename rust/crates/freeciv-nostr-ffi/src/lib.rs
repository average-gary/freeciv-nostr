//! C FFI bindings for freeciv-nostr Rust crates. Exposes networking and crypto
//! functions to the C game engine.

use std::ffi::CStr;
use std::os::raw::c_char;

/// Return the freeciv-nostr library version as a C string.
///
/// # Safety
///
/// The returned pointer is valid for the lifetime of the program (static).
#[unsafe(no_mangle)]
pub extern "C" fn fcn_version() -> *const c_char {
    // SAFETY: The byte literal includes a trailing NUL and contains no
    // interior NUL bytes, so `from_bytes_with_nul_unchecked` is sound.
    const VERSION: &CStr = unsafe { CStr::from_bytes_with_nul_unchecked(b"0.1.0\0") };
    VERSION.as_ptr()
}
