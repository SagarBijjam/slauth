#![deny(warnings)]

//! # Slauth
//!
//! Auth utils for MFA algorithms

/// Module for hotp algorithms
pub mod oath;

#[cfg(feature = "u2f")]
pub mod u2f;

#[cfg(feature = "webauthn-server")]
pub mod webauthn;

#[cfg(feature = "native-bindings")]
pub mod strings {
    use std::{
        ffi::{CStr, CString},
        os::raw::c_char,
    };

    pub unsafe fn c_char_to_string_checked(cchar: *const c_char) -> Option<String> {
        let c_str = CStr::from_ptr(cchar);
        match c_str.to_str() {
            Ok(string) => Some(string.to_string()),
            Err(_) => None,
        }
    }

    pub unsafe fn c_char_to_string(cchar: *const c_char) -> String {
        let c_str = CStr::from_ptr(cchar);
        let r_str = match c_str.to_str() {
            Ok(string) => string,
            Err(_) => "",
        };
        r_str.to_string()
    }

    pub fn string_to_c_char(r_string: String) -> *mut c_char {
        CString::new(r_string)
            .expect("Converting a string into a c_char should not fail")
            .into_raw()
    }

    pub unsafe fn mut_c_char_to_string(cchar: *mut c_char) -> String {
        let c_string = if cchar.is_null() {
            CString::from_vec_unchecked(vec![])
        } else {
            CString::from_raw(cchar)
        };
        let c_str = c_string.as_c_str();
        let r_str = match c_str.to_str() {
            Err(_) => "",
            Ok(string) => string,
        };
        r_str.to_string()
    }
}
