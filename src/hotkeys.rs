//! Wires the shared `honse-hotkeys` crate into this plugin's frame tick.
//!
//! The crate owns chord parsing, edge triggering and the foreground gate, and is
//! deliberately independent of any plugin-API binding. This module is the adapter:
//! it owns the registry, registers the host's present callback as the frame tick,
//! and applies the configured chord.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use honse_hotkeys::{Chord, Hotkeys};

use crate::{api, logging};

/// Ctrl+Shift+P: "P" for POV, on a modifier base that cannot type.
/// Overridden by `window_hotkey` in `honse_pov.ini`.
const DEFAULT_CHORD: Chord = Chord {
    mods: honse_hotkeys::Mods::CTRL,
    vk: b'P' as u32,
};

static HOTKEYS: Mutex<Hotkeys> = Mutex::new(Hotkeys::new());
static HANDLE: AtomicU64 = AtomicU64::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);

fn with<R>(f: impl FnOnce(&mut Hotkeys) -> R) -> R {
    let mut guard = match HOTKEYS.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    f(&mut guard)
}

pub fn install() {
    if INSTALLED.swap(true, Ordering::Relaxed) {
        return;
    }

    set_chord(DEFAULT_CHORD);

    let Some(api) = api::get() else {
        return;
    };

    // The present callback is the only per-frame tick a plugin can get, and it
    // exists only on Windows and only when the Hachimi GUI is enabled.
    // SAFETY: `frame_tick` is a valid `extern "C"` callback and the context pointer is unused by
    // this plugin, so the host accepts a null one.
    let registered = unsafe { (api.hachimi_register_present_callback)(Some(frame_tick), std::ptr::null_mut()) };
    if !registered {
        logging::warn("POV: present callback unavailable; the window hotkey will not work");
    }
}

/// Rebinds the toggle. An unbound chord (`Chord::NONE`) leaves the binding in place
/// but never matching, which is how `window_hotkey = none` disables it.
pub fn set_chord(chord: Chord) {
    let previous = current();
    if previous == chord {
        return;
    }

    if chord.is_bound() && chord.is_typeable() {
        logging::warn(&format!(
            "POV: hotkey {} has no Ctrl or Alt, so it will also fire while typing",
            chord.describe()
        ));
    }

    with(|hotkeys| {
        let handle = HANDLE.load(Ordering::Relaxed);
        if handle != 0 && hotkeys.set_chord(handle, chord) {
            return;
        }
        if let Ok(new_handle) = hotkeys.register(chord, toggle, std::ptr::null_mut()) {
            HANDLE.store(new_handle, Ordering::Relaxed);
        }
    });

    logging::info(&format!("POV: window hotkey = {}", chord.describe()));
}

pub fn current() -> Chord {
    let handle = HANDLE.load(Ordering::Relaxed);
    if handle == 0 {
        return Chord::NONE;
    }
    with(|hotkeys| hotkeys.chord_of(handle)).unwrap_or(Chord::NONE)
}

/// Called by the host on the render thread, once per present.
unsafe extern "C" fn frame_tick(_swapchain: *mut c_void, _userdata: *mut c_void) {
    with(|hotkeys| hotkeys.poll());
}

extern "C" fn toggle(_userdata: *mut c_void) {
    crate::ui::toggle_window();
}
