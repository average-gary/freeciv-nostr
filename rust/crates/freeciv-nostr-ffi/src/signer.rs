//! FFI bindings for the signing subsystem.
//!
//! Wraps `LocalSigner` as an opaque pointer type (`*mut FcnLocalSigner`).
//! Events are passed as JSON strings across the FFI boundary.

use std::os::raw::c_char;

use freeciv_nostr_core::signer::{GameSigner, LocalSigner};
use nostr::prelude::*;

use crate::error::{cstr_to_str, set_last_error, set_last_error_from, string_to_c};
use crate::identity::FcnIdentity;

/// Opaque handle to a local signer.
///
/// Created by `fcn_signer_create()` or `fcn_signer_generate()`.
/// Must be freed with `fcn_signer_free()`.
pub struct FcnLocalSigner {
    inner: LocalSigner,
}

/// Create a local signer from an existing player identity.
///
/// The identity is cloned; the caller retains ownership of `identity`.
///
/// # Safety
///
/// - `identity` must be a valid pointer from `fcn_identity_*` functions.
/// - The returned pointer must be freed with `fcn_signer_free()`.
/// - Returns `NULL` if `identity` is null.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_signer_create(identity: *const FcnIdentity) -> *mut FcnLocalSigner {
    if identity.is_null() {
        set_last_error("null identity pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Caller guarantees identity is a valid FcnIdentity pointer.
    let id = unsafe { &*identity };
    Box::into_raw(Box::new(FcnLocalSigner {
        inner: LocalSigner::new(id.inner.clone()),
    }))
}

/// Create a local signer with a freshly generated keypair.
///
/// # Safety
///
/// The returned pointer must be freed with `fcn_signer_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_signer_generate() -> *mut FcnLocalSigner {
    Box::into_raw(Box::new(FcnLocalSigner {
        inner: LocalSigner::generate(),
    }))
}

/// Get the signer's public key as a hex string (64 chars).
///
/// # Safety
///
/// - `signer` must be a valid pointer from `fcn_signer_*` functions.
/// - The returned string must be freed with `fcn_string_free()`.
/// - Returns `NULL` if `signer` is null.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_signer_public_key_hex(signer: *const FcnLocalSigner) -> *mut c_char {
    if signer.is_null() {
        set_last_error("null signer pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Caller guarantees signer is valid.
    let s = unsafe { &*signer };
    string_to_c(s.inner.public_key().to_hex())
}

/// Sign a Nostr event (passed as JSON), returning the signed event as JSON.
///
/// The input `unsigned_event_json` must be a JSON string representing an
/// unsigned Nostr event with fields: `pubkey`, `created_at`, `kind`,
/// `tags`, `content`. The returned JSON includes the `id` and `sig` fields.
///
/// # Safety
///
/// - `signer` must be a valid pointer from `fcn_signer_*` functions.
/// - `unsigned_event_json` must be a valid null-terminated UTF-8 JSON string.
/// - The returned string must be freed with `fcn_string_free()`.
/// - Returns `NULL` on failure (check `fcn_last_error()`).
#[unsafe(no_mangle)]
pub extern "C" fn fcn_signer_sign_event(
    signer: *const FcnLocalSigner,
    unsigned_event_json: *const c_char,
) -> *mut c_char {
    if signer.is_null() {
        set_last_error("null signer pointer");
        return std::ptr::null_mut();
    }
    let json_str = match cstr_to_str(unsigned_event_json) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };

    // SAFETY: Caller guarantees signer is valid.
    let s = unsafe { &*signer };

    // Parse the unsigned event from JSON
    let unsigned: UnsignedEvent = match serde_json::from_str(json_str) {
        Ok(e) => e,
        Err(e) => {
            set_last_error_from(e);
            return std::ptr::null_mut();
        }
    };

    // Sign it
    let event = match s.inner.sign_event(unsigned) {
        Ok(e) => e,
        Err(e) => {
            set_last_error_from(e);
            return std::ptr::null_mut();
        }
    };

    // Serialize the signed event back to JSON
    match serde_json::to_string(&event) {
        Ok(json) => string_to_c(json),
        Err(e) => {
            set_last_error_from(e);
            std::ptr::null_mut()
        }
    }
}

/// Get the identity from a signer (returns the public key hex).
///
/// This is a convenience function that extracts the underlying identity's
/// public key hex from the signer.
///
/// # Safety
///
/// - `signer` must be a valid pointer from `fcn_signer_*` functions.
/// - The returned string must be freed with `fcn_string_free()`.
/// - Returns `NULL` if `signer` is null.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_signer_identity_public_hex(signer: *const FcnLocalSigner) -> *mut c_char {
    if signer.is_null() {
        set_last_error("null signer pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Caller guarantees signer is valid.
    let s = unsafe { &*signer };
    string_to_c(s.inner.identity().to_public_hex())
}

/// Free a local signer previously returned by an `fcn_signer_*` function.
///
/// # Safety
///
/// `signer` must be a valid pointer from `fcn_signer_create()` or
/// `fcn_signer_generate()`, or `NULL`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_signer_free(signer: *mut FcnLocalSigner) {
    if !signer.is_null() {
        // SAFETY: Caller guarantees this pointer was allocated by us.
        unsafe {
            let _ = Box::from_raw(signer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::fcn_identity_generate;
    use std::ffi::CStr;

    #[test]
    fn signer_generate_and_free() {
        let ptr = fcn_signer_generate();
        assert!(!ptr.is_null());
        fcn_signer_free(ptr);
    }

    #[test]
    fn signer_free_null_is_noop() {
        fcn_signer_free(std::ptr::null_mut());
    }

    #[test]
    fn signer_from_identity() {
        let id_ptr = fcn_identity_generate();
        let signer_ptr = fcn_signer_create(id_ptr);
        assert!(!signer_ptr.is_null());

        // Public keys should match
        let id_hex = crate::identity::fcn_identity_to_public_hex(id_ptr);
        let signer_hex = fcn_signer_public_key_hex(signer_ptr);
        let s1 = unsafe { CStr::from_ptr(id_hex) }.to_str().unwrap();
        let s2 = unsafe { CStr::from_ptr(signer_hex) }.to_str().unwrap();
        assert_eq!(s1, s2);

        crate::fcn_string_free(id_hex);
        crate::fcn_string_free(signer_hex);
        crate::identity::fcn_identity_free(id_ptr);
        fcn_signer_free(signer_ptr);
    }

    #[test]
    fn signer_sign_event_roundtrip() {
        let signer_ptr = fcn_signer_generate();
        let s = unsafe { &*signer_ptr };
        let pubkey = s.inner.public_key();

        // Build an unsigned event
        let unsigned = EventBuilder::new(Kind::Custom(4202), "test payload").build(pubkey);
        let unsigned_json = serde_json::to_string(&unsigned).unwrap();
        let c_json = std::ffi::CString::new(unsigned_json).unwrap();

        let signed_ptr = fcn_signer_sign_event(signer_ptr, c_json.as_ptr());
        assert!(!signed_ptr.is_null(), "sign should succeed");

        let signed_json = unsafe { CStr::from_ptr(signed_ptr) }.to_str().unwrap();
        let event: Event = serde_json::from_str(signed_json).unwrap();
        assert!(event.verify().is_ok());
        assert_eq!(event.pubkey, pubkey);

        crate::fcn_string_free(signed_ptr);
        fcn_signer_free(signer_ptr);
    }

    #[test]
    fn signer_sign_invalid_json_returns_null() {
        let signer_ptr = fcn_signer_generate();
        let bad = std::ffi::CString::new("not json").unwrap();
        let result = fcn_signer_sign_event(signer_ptr, bad.as_ptr());
        assert!(result.is_null());
        assert!(!crate::fcn_last_error().is_null());
        fcn_signer_free(signer_ptr);
    }

    #[test]
    fn null_signer_returns_null() {
        assert!(fcn_signer_create(std::ptr::null()).is_null());
        assert!(fcn_signer_public_key_hex(std::ptr::null()).is_null());
        assert!(fcn_signer_sign_event(std::ptr::null(), std::ptr::null()).is_null());
        assert!(fcn_signer_identity_public_hex(std::ptr::null()).is_null());
    }
}
