//! Plugin GUI: a runner picker rendered both in the Hachimi menu and in a
//! standalone plugin window.
//!
//! All rendering reads from the cached snapshot only. GUI callbacks run on the
//! render thread, so no IL2CPP calls happen here.

use std::ffi::{c_void, CString};
use std::sync::atomic::{AtomicI32, Ordering};

use crate::{api, pov, race, race::Runner};

const WINDOW_TITLE: &str = "Race POV (experimental)";
const GRID_ID: &str = "honse_pov_runner_grid";

static WINDOW_ID: AtomicI32 = AtomicI32::new(-1);

/// Slice descriptor handed to the synchronous grid callback. A `&[T]` is a fat
/// pointer and cannot be cast to `*mut c_void`, so we pass this instead.
#[repr(C)]
struct RunnerSlice {
    ptr: *const Runner,
    len: usize,
}

// ------------------------------------------------------------- text utils ---

unsafe fn ui_text(f: api::FnUiText, ui: *mut c_void, text: &str) {
    if let Ok(text) = CString::new(text) {
        let _ = f(ui, text.as_ptr());
    }
}

unsafe fn ui_colored(f: api::FnUiColoredLabel, ui: *mut c_void, text: &str, rgba: [u8; 4]) {
    if let Ok(text) = CString::new(text) {
        let _ = f(ui, rgba[0], rgba[1], rgba[2], rgba[3], text.as_ptr());
    }
}

unsafe fn ui_void(f: api::FnUiVoid, ui: *mut c_void) {
    let _ = f(ui);
}

unsafe fn ui_button(f: api::FnUiButton, ui: *mut c_void, text: &str) -> bool {
    match CString::new(text) {
        Ok(text) => f(ui, text.as_ptr()),
        Err(_) => false,
    }
}

unsafe fn ui_checkbox(f: api::FnUiCheckbox, ui: *mut c_void, text: &str, value: &mut bool) -> bool {
    match CString::new(text) {
        Ok(text) => f(ui, text.as_ptr(), value),
        Err(_) => false,
    }
}

// ------------------------------------------------------------------ window --

fn window_id(api: &api::Api) -> i32 {
    let existing = WINDOW_ID.load(Ordering::Relaxed);
    if existing >= 0 {
        return existing;
    }
    let id = unsafe { (api.gui_new_window_id)() };
    if id >= 0 {
        WINDOW_ID.store(id, Ordering::Relaxed);
    }
    id
}

pub fn open_window() {
    let Some(api) = api::get() else {
        return;
    };

    let id = window_id(api);
    if id < 0 {
        return;
    }
    let Ok(title) = CString::new(WINDOW_TITLE) else {
        return;
    };

    // The host replaces an existing window with the same id, so this is safe to
    // call repeatedly.
    unsafe {
        let _ = (api.gui_show_window)(
            id,
            title.as_ptr(),
            Some(window_contents),
            Some(window_bottom),
            std::ptr::null_mut(),
        );
    }
}

unsafe extern "C" fn window_contents(ui: *mut c_void, _userdata: *mut c_void) {
    render(ui, true);
}

unsafe extern "C" fn window_bottom(ui: *mut c_void, _userdata: *mut c_void) {
    let Some(api) = api::get() else {
        return;
    };

    ui_void(api.gui_ui_separator, ui);

    if ui_button(api.gui_ui_small_button, ui, "Clear selection") {
        race::set_selected_index(-1);
    }

    if ui_button(api.gui_ui_small_button, ui, "Close") {
        let id = WINDOW_ID.load(Ordering::Relaxed);
        if id >= 0 {
            (api.gui_close_window)(id);
        }
        WINDOW_ID.store(-1, Ordering::Relaxed);
    }
}

// ------------------------------------------------------------ menu section --

unsafe extern "C" fn menu_section(ui: *mut c_void, _userdata: *mut c_void) {
    render(ui, false);
}

unsafe extern "C" fn menu_item_open_window(_userdata: *mut c_void) {
    open_window();
}

// --------------------------------------------------------------- renderer ---

fn render(ui: *mut c_void, in_window: bool) {
    let Some(api) = api::get() else {
        return;
    };

    let snapshot = race::snapshot();

    unsafe {
        // The window already shows this in its title bar; only the menu section
        // needs a heading.
        if !in_window {
            ui_text(api.gui_ui_heading, ui, WINDOW_TITLE);
            ui_void(api.gui_ui_separator, ui);
        }

        if !race::classes_ready() {
            let message = if race::install_failed() {
                "Race tracking failed to install. Check the Hachimi log for the missing class/method."
            } else {
                "Waiting for game initialization (classes not resolved yet)."
            };
            ui_text(api.gui_ui_label, ui, message);
            return;
        }

        if !in_window && ui_button(api.gui_ui_button, ui, "Open runner picker window") {
            open_window();
        }

        // Deselect has to be reachable from the menu too, not just the window's
        // bottom bar: with POV on, the selection is what drives the camera, so
        // clearing it is how you hand control back without leaving POV enabled on
        // some arbitrary runner.
        if race::selected_index() >= 0
            && ui_button(api.gui_ui_small_button, ui, "Deselect runner")
        {
            race::set_selected_index(-1);
        }

        // --- milestone 2: POV toggle ---
        let mut pov_enabled = pov::enabled();
        if ui_checkbox(
            api.gui_ui_checkbox,
            ui,
            "Enable POV (camera follows the selected runner)",
            &mut pov_enabled,
        ) {
            pov::set_enabled(pov_enabled);
        }

        let status = pov::status();
        if pov::is_available() {
            ui_text(api.gui_ui_small, ui, &status);
        } else {
            ui_colored(api.gui_ui_colored_label, ui, &status, [255, 140, 140, 255]);
        }

        if pov::enabled() && race::selected_index() < 0 {
            ui_colored(
                api.gui_ui_colored_label,
                ui,
                "Select a runner below to aim the camera.",
                [255, 200, 120, 255],
            );
        }

        ui_void(api.gui_ui_separator, ui);

        if !snapshot.active {
            ui_text(api.gui_ui_label, ui, "No active race detected.");
            ui_text(
                api.gui_ui_small,
                ui,
                "Start a race; the runner list appears once the race is running.",
            );
            return;
        }

        let selected = race::selected_index();
        let selected_label = match snapshot.runners.iter().find(|r| r.index == selected) {
            Some(runner) => format!(
                "Selected: gate {} - {} (#{})",
                runner.gate_no, runner.name, runner.index
            ),
            None => "Selected: none".to_owned(),
        };        ui_colored(api.gui_ui_colored_label, ui, &selected_label, [120, 220, 160, 255]);
        ui_text(
            api.gui_ui_small,
            ui,
            &format!(
                "{} runners | player horse index: {}",
                snapshot.runners.len(),
                snapshot.player_index
            ),
        );
        ui_void(api.gui_ui_separator, ui);

        render_runner_grid(api, ui, &snapshot.runners);

        if !in_window {
            ui_void(api.gui_ui_separator, ui);
            ui_text(
                api.gui_ui_small,
                ui,
                "POV is forced while enabled; turn it off to hand the camera back to the game.",
            );
        }
    }
}

unsafe fn render_runner_grid(api: &api::Api, ui: *mut c_void, runners: &[Runner]) {
    if runners.is_empty() {
        ui_text(api.gui_ui_label, ui, "Race is active but no runners were reported.");
        return;
    }

    let Ok(id) = CString::new(GRID_ID) else {
        return;
    };

    let slice = RunnerSlice {
        ptr: runners.as_ptr(),
        len: runners.len(),
    };

    // `gui_ui_grid` invokes the callback synchronously, so `slice` is valid for
    // the whole call.
    let _ = (api.gui_ui_grid)(
        ui,
        id.as_ptr(),
        5,
        6.0,
        2.0,
        Some(grid_rows),
        &slice as *const RunnerSlice as *mut c_void,
    );

    ui_text(api.gui_ui_small, ui, "select | gate | name | popularity | tag");
}

unsafe extern "C" fn grid_rows(ui: *mut c_void, userdata: *mut c_void) {
    let Some(api) = api::get() else {
        return;
    };
    if userdata.is_null() {
        return;
    }

    // SAFETY: `render_runner_grid` passes a `RunnerSlice` pointing at a slice
    // that outlives this synchronous call.
    let slice = &*(userdata as *const RunnerSlice);
    if slice.ptr.is_null() {
        return;
    }
    let runners = std::slice::from_raw_parts(slice.ptr, slice.len);
    let selected = race::selected_index();

    for runner in runners {
        let is_selected = runner.index == selected;
        let label = if is_selected { "* selected" } else { "select" };

        if ui_button(api.gui_ui_small_button, ui, label) {
            race::set_selected_index(runner.index);
        }

        ui_text(api.gui_ui_label, ui, &format!("#{}", runner.gate_no));
        ui_text(api.gui_ui_label, ui, &runner.name);
        // The game stores popularity 0-based, so +1 is the displayed rank (1 = favourite).
        ui_text(api.gui_ui_label, ui, &format!("{}", runner.popularity + 1));
        ui_text(
            api.gui_ui_label,
            ui,
            if runner.is_player { "YOU" } else { "" },
        );

        ui_void(api.gui_ui_end_row, ui);
    }
}

// ------------------------------------------------------------------ setup ---

pub fn register() {
    let Some(api) = api::get() else {
        return;
    };

    unsafe {
        let _ = (api.gui_register_menu_section)(Some(menu_section), std::ptr::null_mut());

        if let Ok(label) = CString::new("Open Race POV window") {
            let _ = (api.gui_register_menu_item)(
                label.as_ptr(),
                Some(menu_item_open_window),
                std::ptr::null_mut(),
            );
        }
    }
}
