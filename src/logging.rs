//! Level constants mirror the host (`1=Error .. 5=Trace`). Not every level is
//! used yet; milestone 2 will make heavier use of them.
#![allow(dead_code)]

use std::ffi::CString;

use crate::api;

pub const ERROR: i32 = 1;
pub const WARN: i32 = 2;
pub const INFO: i32 = 3;
pub const DEBUG: i32 = 4;

pub fn log(level: i32, message: &str) {
    let Some(api) = api::get() else {
        return;
    };
    // Log messages are arbitrary strings; NUL bytes would truncate them.
    let sanitized = message.replace('\0', " ");
    let Ok(body) = CString::new(sanitized) else {
        return;
    };
    let tag = b"honse_pov\0";
    unsafe {
        (api.log)(level, tag.as_ptr() as *const _, body.as_ptr());
    }
}

pub fn error(message: &str) {
    log(ERROR, message);
}

pub fn warn(message: &str) {
    log(WARN, message);
}

pub fn info(message: &str) {
    log(INFO, message);
}

pub fn debug(message: &str) {
    log(DEBUG, message);
}
