//! Minimal IL2CPP helpers.
//!
//! Only the pieces a plugin can reach through the Hachimi API are implemented
//! here. Everything is name-resolved and null-checked; nothing panics.

use std::ffi::{c_void, CString};

use crate::api;

pub type Obj = *mut c_void;
pub type Class = *mut c_void;
pub type Field = *mut c_void;

// ---------------------------------------------------------------- layout ----
//
// The plugin API has no array/string length helpers beyond strings, so we read
// the standard IL2CPP object layout directly. This matches Hachimi's own
// `Il2CppArray` / `Il2CppString` definitions (Unity 2020+):
//
//   Il2CppObject { klass: *mut, monitor: *mut }          // 16 bytes
//   Il2CppArray  { obj, bounds: *mut, max_length: usize } // + 16 = 32 bytes
//   data starts at offset 32, elements are object references
//
// Anything that depends on these offsets is confined to this module.

const ARRAY_LENGTH_OFFSET: usize = 24;
const ARRAY_DATA_OFFSET: usize = 32;

pub unsafe fn array_len(array: Obj) -> usize {
    if array.is_null() {
        return 0;
    }
    ((array as *const u8).add(ARRAY_LENGTH_OFFSET) as *const usize).read_unaligned()
}

pub unsafe fn array_get(array: Obj, index: usize) -> Obj {
    if array.is_null() {
        return std::ptr::null_mut();
    }
    let base = (array as *const u8).add(ARRAY_DATA_OFFSET) as *const Obj;
    base.add(index).read_unaligned()
}

// ------------------------------------------------------------- resolver ----

unsafe fn cstr(value: &str) -> Option<CString> {
    CString::new(value).ok()
}

pub unsafe fn assembly_image(name: &str) -> Obj {
    let Some(api) = api::get() else {
        return std::ptr::null_mut();
    };
    let Some(name) = cstr(name) else {
        return std::ptr::null_mut();
    };
    (api.il2cpp_get_assembly_image)(name.as_ptr()) as Obj
}

pub unsafe fn class(image: Obj, namespace: &str, name: &str) -> Class {
    let Some(api) = api::get() else {
        return std::ptr::null_mut();
    };
    let (Some(namespace), Some(name)) = (cstr(namespace), cstr(name)) else {
        return std::ptr::null_mut();
    };
    (api.il2cpp_get_class)(image, namespace.as_ptr(), name.as_ptr())
}

/// Returns the raw method pointer (0 when the method is missing).
pub unsafe fn method_addr(class: Class, name: &str, arg_count: i32) -> usize {
    if class.is_null() {
        return 0;
    }
    let Some(api) = api::get() else {
        return 0;
    };
    let Some(name) = cstr(name) else {
        return 0;
    };
    (api.il2cpp_get_method_addr)(class, name.as_ptr(), arg_count) as usize
}

pub unsafe fn field(class: Class, name: &str) -> Field {
    if class.is_null() {
        return std::ptr::null_mut();
    }
    let Some(api) = api::get() else {
        return std::ptr::null_mut();
    };
    let Some(name) = cstr(name) else {
        return std::ptr::null_mut();
    };
    (api.il2cpp_get_field_from_name)(class, name.as_ptr())
}

pub unsafe fn field_object(object: Obj, field: Field) -> Obj {
    if object.is_null() || field.is_null() {
        return std::ptr::null_mut();
    }
    let Some(api) = api::get() else {
        return std::ptr::null_mut();
    };
    let mut out: Obj = std::ptr::null_mut();
    (api.il2cpp_get_field_value)(object, field, &mut out as *mut Obj as *mut c_void);
    out
}

/// Writes a `System.Boolean` field (one byte in IL2CPP).
pub unsafe fn set_field_bool(object: Obj, field: Field, value: bool) {
    if object.is_null() || field.is_null() {
        return;
    }
    let Some(api) = api::get() else {
        return;
    };
    (api.il2cpp_set_field_value)(object, field, &value as *const bool as *const c_void);
}

pub unsafe fn singleton(class: Class) -> Obj {
    if class.is_null() {
        return std::ptr::null_mut();
    }
    let Some(api) = api::get() else {
        return std::ptr::null_mut();
    };
    (api.il2cpp_get_singleton_like_instance)(class)
}

pub unsafe fn read_string(string: Obj) -> String {
    if string.is_null() {
        return String::new();
    }
    let Some(api) = api::get() else {
        return String::new();
    };
    let length = (api.il2cpp_string_length)(string);
    if length <= 0 {
        return String::new();
    }
    let chars = (api.il2cpp_string_chars)(string);
    if chars.is_null() {
        return String::new();
    }
    let slice = std::slice::from_raw_parts(chars, length as usize);
    String::from_utf16_lossy(slice)
}

// --------------------------------------------------------- invocation ------
//
// IL2CPP method pointers for instance methods take `this` as the first
// argument. These helpers transmute a stored address into a typed call.

pub unsafe fn call_obj0(addr: usize, this: Obj) -> Obj {
    if addr == 0 {
        return std::ptr::null_mut();
    }
    let f: unsafe extern "C" fn(Obj) -> Obj = std::mem::transmute(addr);
    f(this)
}

pub unsafe fn call_i32_0(addr: usize, this: Obj) -> i32 {
    if addr == 0 {
        return 0;
    }
    let f: unsafe extern "C" fn(Obj) -> i32 = std::mem::transmute(addr);
    f(this)
}

pub unsafe fn call_bool_0(addr: usize, this: Obj) -> bool {
    if addr == 0 {
        return false;
    }
    let f: unsafe extern "C" fn(Obj) -> bool = std::mem::transmute(addr);
    f(this)
}

/// Calls `void f(T this, float)` / `void f(float)`-shaped methods.
pub unsafe fn call_void_f32(addr: usize, this: Obj, value: f32) {
    if addr == 0 {
        return;
    }
    let f: unsafe extern "C" fn(Obj, f32) = std::mem::transmute(addr);
    f(this, value)
}

/// Calls `void f(T this, bool)` / `void f(bool)`-shaped methods.
pub unsafe fn call_void_bool(addr: usize, this: Obj, value: bool) {
    if addr == 0 {
        return;
    }
    let f: unsafe extern "C" fn(Obj, bool) = std::mem::transmute(addr);
    f(this, value)
}

pub unsafe fn call_obj1_i32(addr: usize, this: Obj, a: i32) -> Obj {
    if addr == 0 {
        return std::ptr::null_mut();
    }
    let f: unsafe extern "C" fn(Obj, i32) -> Obj = std::mem::transmute(addr);
    f(this, a)
}
