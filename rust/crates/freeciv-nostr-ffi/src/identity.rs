//! FFI bindings for player identity (key management).
//!
//! Wraps `freeciv_nostr_core::keys::PlayerIdentity` as an opaque pointer type
//! (`*mut FcnIdentity`) for C consumption.

use std::os::raw::c_char;

use freeciv_nostr_core::keys::PlayerIdentity;

use crate::error::{cstr_to_str, set_last_error_from, string_to_c};

/// Opaque handle to a player identity (secp256k1 keypair).
///
/// Created by `fcn_identity_generate()`, `fcn_identity_from_nsec()`, or
/// `fcn_identity_from_hex()`. Must be freed with `fcn_identity_free()`.
pub struct FcnIdentity {
    pub(crate) inner: PlayerIdentity,
}

/// Generate a new random player identity.
///
/// # Safety
///
/// The returned pointer must be freed with `fcn_identity_free()`.
/// Returns `NULL` on failure (check `fcn_last_error()`).
#[unsafe(no_mangle)]
pub extern "C" fn fcn_identity_generate() -> *mut FcnIdentity {
    Box::into_raw(Box::new(FcnIdentity {
        inner: PlayerIdentity::generate(),
    }))
}

/// Create a player identity from a NIP-19 `nsec` bech32 string.
///
/// # Safety
///
/// - `nsec` must be a valid null-terminated UTF-8 string.
/// - The returned pointer must be freed with `fcn_identity_free()`.
/// - Returns `NULL` on failure (check `fcn_last_error()`).
#[unsafe(no_mangle)]
pub extern "C" fn fcn_identity_from_nsec(nsec: *const c_char) -> *mut FcnIdentity {
    let nsec_str = match cstr_to_str(nsec) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    match PlayerIdentity::from_nsec(nsec_str) {
        Ok(identity) => Box::into_raw(Box::new(FcnIdentity { inner: identity })),
        Err(e) => {
            set_last_error_from(e);
            std::ptr::null_mut()
        }
    }
}

/// Create a player identity from a hex-encoded secret key (64 hex chars).
///
/// # Safety
///
/// - `hex` must be a valid null-terminated UTF-8 string.
/// - The returned pointer must be freed with `fcn_identity_free()`.
/// - Returns `NULL` on failure (check `fcn_last_error()`).
#[unsafe(no_mangle)]
pub extern "C" fn fcn_identity_from_hex(hex: *const c_char) -> *mut FcnIdentity {
    let hex_str = match cstr_to_str(hex) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    match PlayerIdentity::from_hex(hex_str) {
        Ok(identity) => Box::into_raw(Box::new(FcnIdentity { inner: identity })),
        Err(e) => {
            set_last_error_from(e);
            std::ptr::null_mut()
        }
    }
}

/// Get the public key as a NIP-19 `npub` bech32 string.
///
/// # Safety
///
/// - `identity` must be a valid pointer from `fcn_identity_generate()` or similar.
/// - The returned string must be freed with `fcn_string_free()`.
/// - Returns `NULL` if `identity` is null.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_identity_to_npub(identity: *const FcnIdentity) -> *mut c_char {
    if identity.is_null() {
        crate::error::set_last_error("null identity pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Caller guarantees identity is a valid FcnIdentity pointer.
    let id = unsafe { &*identity };
    string_to_c(id.inner.to_npub())
}

/// Get the secret key as a NIP-19 `nsec` bech32 string.
///
/// # Safety
///
/// - `identity` must be a valid pointer from `fcn_identity_generate()` or similar.
/// - The returned string must be freed with `fcn_string_free()`.
/// - Returns `NULL` if `identity` is null.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_identity_to_nsec(identity: *const FcnIdentity) -> *mut c_char {
    if identity.is_null() {
        crate::error::set_last_error("null identity pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Caller guarantees identity is a valid FcnIdentity pointer.
    let id = unsafe { &*identity };
    string_to_c(id.inner.to_nsec())
}

/// Get the public key as a hex string (64 chars).
///
/// # Safety
///
/// - `identity` must be a valid pointer from `fcn_identity_generate()` or similar.
/// - The returned string must be freed with `fcn_string_free()`.
/// - Returns `NULL` if `identity` is null.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_identity_to_public_hex(identity: *const FcnIdentity) -> *mut c_char {
    if identity.is_null() {
        crate::error::set_last_error("null identity pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Caller guarantees identity is a valid FcnIdentity pointer.
    let id = unsafe { &*identity };
    string_to_c(id.inner.to_public_hex())
}

/// Free a player identity previously returned by an `fcn_identity_*` function.
///
/// # Safety
///
/// `identity` must be a valid pointer from `fcn_identity_generate()`,
/// `fcn_identity_from_nsec()`, or `fcn_identity_from_hex()`, or `NULL`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_identity_free(identity: *mut FcnIdentity) {
    if !identity.is_null() {
        // SAFETY: Caller guarantees this pointer was allocated by us.
        unsafe {
            let _ = Box::from_raw(identity);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn generate_and_free() {
        let ptr = fcn_identity_generate();
        assert!(!ptr.is_null());
        fcn_identity_free(ptr);
    }

    #[test]
    fn free_null_is_noop() {
        fcn_identity_free(std::ptr::null_mut());
    }

    #[test]
    fn roundtrip_nsec() {
        let ptr = fcn_identity_generate();
        assert!(!ptr.is_null());

        let nsec_ptr = fcn_identity_to_nsec(ptr);
        assert!(!nsec_ptr.is_null());
        let nsec = unsafe { CStr::from_ptr(nsec_ptr) }.to_str().unwrap();
        assert!(nsec.starts_with("nsec1"));

        // Recreate from nsec
        let ptr2 = fcn_identity_from_nsec(nsec_ptr);
        assert!(!ptr2.is_null());

        // Public keys should match
        let hex1 = fcn_identity_to_public_hex(ptr);
        let hex2 = fcn_identity_to_public_hex(ptr2);
        let s1 = unsafe { CStr::from_ptr(hex1) }.to_str().unwrap();
        let s2 = unsafe { CStr::from_ptr(hex2) }.to_str().unwrap();
        assert_eq!(s1, s2);

        crate::error::fcn_string_free(hex1);
        crate::error::fcn_string_free(hex2);
        crate::error::fcn_string_free(nsec_ptr);
        fcn_identity_free(ptr);
        fcn_identity_free(ptr2);
    }

    #[test]
    fn roundtrip_hex() {
        let ptr = fcn_identity_generate();
        let hex_ptr = fcn_identity_to_public_hex(ptr);
        assert!(!hex_ptr.is_null());
        let hex = unsafe { CStr::from_ptr(hex_ptr) }.to_str().unwrap();
        assert_eq!(hex.len(), 64);

        crate::error::fcn_string_free(hex_ptr);
        fcn_identity_free(ptr);
    }

    #[test]
    fn npub_encoding() {
        let ptr = fcn_identity_generate();
        let npub_ptr = fcn_identity_to_npub(ptr);
        assert!(!npub_ptr.is_null());
        let npub = unsafe { CStr::from_ptr(npub_ptr) }.to_str().unwrap();
        assert!(npub.starts_with("npub1"));

        crate::error::fcn_string_free(npub_ptr);
        fcn_identity_free(ptr);
    }

    #[test]
    fn invalid_nsec_returns_null() {
        let bad = std::ffi::CString::new("not_a_valid_nsec").unwrap();
        let ptr = fcn_identity_from_nsec(bad.as_ptr());
        assert!(ptr.is_null());
        let err = crate::fcn_last_error();
        assert!(!err.is_null());
    }

    #[test]
    fn invalid_hex_returns_null() {
        let bad = std::ffi::CString::new("zzz").unwrap();
        let ptr = fcn_identity_from_hex(bad.as_ptr());
        assert!(ptr.is_null());
        let err = crate::fcn_last_error();
        assert!(!err.is_null());
    }

    #[test]
    fn null_identity_returns_null() {
        assert!(fcn_identity_to_npub(std::ptr::null()).is_null());
        assert!(fcn_identity_to_nsec(std::ptr::null()).is_null());
        assert!(fcn_identity_to_public_hex(std::ptr::null()).is_null());
    }
}
