//! Thin, dependency-free bindings to the Hachimi Edge plugin API (v3).
//!
//! We deliberately resolve every entry point by name instead of relying on the
//! `Vtable` layout, so this plugin keeps working when new symbols are appended
//! to the host API. Anything that fails to resolve is reported once at init and
//! treated as "feature unavailable" rather than a hard crash.

#![allow(non_snake_case)]

use std::ffi::{c_char, c_void};
use std::sync::OnceLock;

/// `hachimi_get_api` as exported by the host.
pub type GetApiFn = extern "C" fn(name: *const c_char) -> *mut c_void;

pub type FnLog = unsafe extern "C" fn(i32, *const c_char, *const c_char);

pub type FnHachimiInstance = unsafe extern "C" fn() -> *const c_void;
pub type FnGetInterceptor = unsafe extern "C" fn(*const c_void) -> *const c_void;
pub type FnHook = unsafe extern "C" fn(*const c_void, *mut c_void, *mut c_void) -> *mut c_void;

pub type FnGetImage = unsafe extern "C" fn(*const c_char) -> *mut c_void;
pub type FnGetClass = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> *mut c_void;
pub type FnGetMethodAddr = unsafe extern "C" fn(*mut c_void, *const c_char, i32) -> *mut c_void;
pub type FnGetField = unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void;
pub type FnGetFieldValue = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void);
pub type FnSingleton = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
pub type FnStringChars = unsafe extern "C" fn(*mut c_void) -> *mut u16;
pub type FnStringLength = unsafe extern "C" fn(*mut c_void) -> i32;
pub type FnGetMainThread = unsafe extern "C" fn() -> *mut c_void;
pub type FnScheduleOnThread = unsafe extern "C" fn(*mut c_void, unsafe extern "C" fn());

pub type GameInitializedCallback = unsafe extern "C" fn(*mut c_void);
pub type PresentCallback = unsafe extern "C" fn(*mut c_void, *mut c_void);
pub type GuiMenuCallback = unsafe extern "C" fn(*mut c_void);
pub type GuiSectionCallback = unsafe extern "C" fn(*mut c_void, *mut c_void);
pub type GuiUiCallback = unsafe extern "C" fn(*mut c_void, *mut c_void);
pub type GuiWindowCallback = unsafe extern "C" fn(*mut c_void, *mut c_void);

pub type FnRegisterOnGameInitialized =
    unsafe extern "C" fn(Option<GameInitializedCallback>, *mut c_void) -> bool;
pub type FnRegisterPresentCallback =
    unsafe extern "C" fn(Option<PresentCallback>, *mut c_void) -> bool;

pub type FnGuiRegisterMenu = unsafe extern "C" fn(
    *const c_char,
    Option<GuiMenuCallback>,
    *mut c_void,
) -> bool;
pub type FnGuiRegisterSection =
    unsafe extern "C" fn(Option<GuiSectionCallback>, *mut c_void) -> bool;
pub type FnGuiNotify = unsafe extern "C" fn(*const c_char) -> bool;
pub type FnGuiNewWindowId = unsafe extern "C" fn() -> i32;
pub type FnGuiShowWindow = unsafe extern "C" fn(
    i32,
    *const c_char,
    Option<GuiWindowCallback>,
    Option<GuiWindowCallback>,
    *mut c_void,
) -> bool;
pub type FnGuiCloseWindow = unsafe extern "C" fn(i32);

pub type FnUiText = unsafe extern "C" fn(*mut c_void, *const c_char) -> bool;
pub type FnUiVoid = unsafe extern "C" fn(*mut c_void) -> bool;
pub type FnUiButton = unsafe extern "C" fn(*mut c_void, *const c_char) -> bool;
pub type FnUiCheckbox = unsafe extern "C" fn(*mut c_void, *const c_char, *mut bool) -> bool;
pub type FnUiColoredLabel = unsafe extern "C" fn(*mut c_void, u8, u8, u8, u8, *const c_char) -> bool;
pub type FnUiGrid = unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    usize,
    f32,
    f32,
    Option<GuiUiCallback>,
    *mut c_void,
) -> bool;

pub type FnGetBaseDir = unsafe extern "C" fn() -> *const c_char;

/// Complete binding table for the subset of the host API this plugin knows
/// about. Kept fully declared (even entries milestone 1 does not call yet) so
/// the surface mirrors `plugin_api::Vtable` and milestone 2 can pick entries up
/// without touching the resolver.
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub struct Api {
    pub log: FnLog,

    pub hachimi_instance: FnHachimiInstance,
    pub hachimi_get_interceptor: FnGetInterceptor,
    pub interceptor_hook: FnHook,

    pub il2cpp_get_assembly_image: FnGetImage,
    pub il2cpp_get_class: FnGetClass,
    pub il2cpp_get_method_addr: FnGetMethodAddr,
    pub il2cpp_get_field_from_name: FnGetField,
    pub il2cpp_get_field_value: FnGetFieldValue,
    pub il2cpp_get_singleton_like_instance: FnSingleton,
    pub il2cpp_string_chars: FnStringChars,
    pub il2cpp_string_length: FnStringLength,
    pub il2cpp_get_main_thread: FnGetMainThread,
    pub il2cpp_schedule_on_thread: FnScheduleOnThread,

    pub hachimi_register_on_game_initialized: FnRegisterOnGameInitialized,
    pub hachimi_register_present_callback: FnRegisterPresentCallback,

    pub gui_register_menu_item: FnGuiRegisterMenu,
    pub gui_register_menu_section: FnGuiRegisterSection,
    pub gui_show_notification: FnGuiNotify,
    pub gui_new_window_id: FnGuiNewWindowId,
    pub gui_show_window: FnGuiShowWindow,
    pub gui_close_window: FnGuiCloseWindow,

    pub gui_ui_heading: FnUiText,
    pub gui_ui_label: FnUiText,
    pub gui_ui_small: FnUiText,
    pub gui_ui_separator: FnUiVoid,
    pub gui_ui_button: FnUiButton,
    pub gui_ui_small_button: FnUiButton,
    pub gui_ui_checkbox: FnUiCheckbox,
    pub gui_ui_colored_label: FnUiColoredLabel,
    pub gui_ui_grid: FnUiGrid,
    pub gui_ui_end_row: FnUiVoid,

    pub hachimi_get_base_dir: FnGetBaseDir,
}

static API: OnceLock<Api> = OnceLock::new();

pub fn set(api: Api) {
    let _ = API.set(api);
}

pub fn get() -> Option<&'static Api> {
    API.get()
}

/// Best-effort logging that works even before the full API is resolved.
pub fn raw_log(get: GetApiFn, level: i32, msg: &str) {
    let ptr = symbol(get, "log");
    if ptr.is_null() {
        return;
    }
    let f: FnLog = unsafe { std::mem::transmute_copy::<*mut c_void, FnLog>(&ptr) };
    let tag = b"honse_pov\0";
    let sanitized = msg.replace('\0', " ");
    let Ok(body) = std::ffi::CString::new(sanitized) else {
        return;
    };
    unsafe { f(level, tag.as_ptr() as *const c_char, body.as_ptr()) };
}

/// Call `get_api("name")` without heap-allocating a C string.
fn symbol(get: GetApiFn, name: &str) -> *mut c_void {
    let bytes = name.as_bytes();
    // Longest name we ask for is comfortably below this.
    let mut buf = [0u8; 64];
    if bytes.len() + 1 > buf.len() {
        return std::ptr::null_mut();
    }
    buf[..bytes.len()].copy_from_slice(bytes);
    // buf[len] is already 0.
    get(buf.as_ptr() as *const c_char)
}

fn resolve_fn<T: Copy>(
    get: GetApiFn,
    name: &str,
    missing: &mut Vec<&'static str>,
    required: bool,
) -> Option<T> {
    let ptr = symbol(get, name);
    if ptr.is_null() {
        if required {
            missing.push(leak(name));
        }
        return None;
    }
    // All of these are plain function pointers on x86_64/aarch64.
    Some(unsafe { std::mem::transmute_copy::<*mut c_void, T>(&ptr) })
}

fn leak(name: &str) -> &'static str {
    Box::leak(name.to_owned().into_boxed_str())
}

/// Resolve the subset of the API we use. Returns `Err(missing_names)` when a
/// required symbol is unavailable (host too old / mismatched build).
pub fn resolve(get: GetApiFn) -> Result<Api, Vec<&'static str>> {
    let mut missing = Vec::new();

    macro_rules! req {
        ($name:literal, $ty:ty) => {
            match resolve_fn::<$ty>(get, $name, &mut missing, true) {
                Some(f) => f,
                None => return Err(missing),
            }
        };
    }

    let api = Api {
        log: req!("log", FnLog),

        hachimi_instance: req!("hachimi_instance", FnHachimiInstance),
        hachimi_get_interceptor: req!("hachimi_get_interceptor", FnGetInterceptor),
        interceptor_hook: req!("interceptor_hook", FnHook),

        il2cpp_get_assembly_image: req!("il2cpp_get_assembly_image", FnGetImage),
        il2cpp_get_class: req!("il2cpp_get_class", FnGetClass),
        il2cpp_get_method_addr: req!("il2cpp_get_method_addr", FnGetMethodAddr),
        il2cpp_get_field_from_name: req!("il2cpp_get_field_from_name", FnGetField),
        il2cpp_get_field_value: req!("il2cpp_get_field_value", FnGetFieldValue),
        il2cpp_get_singleton_like_instance: req!(
            "il2cpp_get_singleton_like_instance",
            FnSingleton
        ),
        il2cpp_string_chars: req!("il2cpp_string_chars", FnStringChars),
        il2cpp_string_length: req!("il2cpp_string_length", FnStringLength),
        il2cpp_get_main_thread: req!("il2cpp_get_main_thread", FnGetMainThread),
        il2cpp_schedule_on_thread: req!("il2cpp_schedule_on_thread", FnScheduleOnThread),

        hachimi_register_on_game_initialized: req!(
            "hachimi_register_on_game_initialized",
            FnRegisterOnGameInitialized
        ),
        hachimi_register_present_callback: req!(
            "hachimi_register_present_callback",
            FnRegisterPresentCallback
        ),

        gui_register_menu_item: req!("gui_register_menu_item", FnGuiRegisterMenu),
        gui_register_menu_section: req!("gui_register_menu_section", FnGuiRegisterSection),
        gui_show_notification: req!("gui_show_notification", FnGuiNotify),
        gui_new_window_id: req!("gui_new_window_id", FnGuiNewWindowId),
        gui_show_window: req!("gui_show_window", FnGuiShowWindow),
        gui_close_window: req!("gui_close_window", FnGuiCloseWindow),

        gui_ui_heading: req!("gui_ui_heading", FnUiText),
        gui_ui_label: req!("gui_ui_label", FnUiText),
        gui_ui_small: req!("gui_ui_small", FnUiText),
        gui_ui_separator: req!("gui_ui_separator", FnUiVoid),
        gui_ui_button: req!("gui_ui_button", FnUiButton),
        gui_ui_small_button: req!("gui_ui_small_button", FnUiButton),
        gui_ui_checkbox: req!("gui_ui_checkbox", FnUiCheckbox),
        gui_ui_colored_label: req!("gui_ui_colored_label", FnUiColoredLabel),
        gui_ui_grid: req!("gui_ui_grid", FnUiGrid),
        gui_ui_end_row: req!("gui_ui_end_row", FnUiVoid),

        hachimi_get_base_dir: req!("hachimi_get_base_dir", FnGetBaseDir),
    };

    Ok(api)
}
