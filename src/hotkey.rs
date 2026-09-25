//! Global hotkey for the picker window.
//!
//! # Why Win32 polling
//!
//! Hachimi's plugin API exposes no key events, and Hachimi Edge has no WndProc
//! plugin hook, so key state is read straight from `GetAsyncKeyState` on the present
//! callback — the same approach honse-tracker uses. Polling is global (it reports
//! keys regardless of focus), so it is gated on the foreground window belonging to
//! *this* process; otherwise the chord would fire while you type in another
//! application.
//!
//! # Why Ctrl or Alt is required
//!
//! Taken from honse-tracker's reasoning, because it is the same problem:
//!
//! - a bare key, or Shift alone, types characters — unusable as a global chord;
//! - **Ctrl+Alt is AltGr on Windows.** On non-US layouts AltGr is how everyday
//!   characters are typed (on a Spanish keyboard `AltGr+2` is `@`), so a Ctrl+Alt
//!   chord fires while the player is typing perfectly ordinary text.
//!
//! Ctrl+Shift produces no characters on any layout, so it is the default base.
//! The chord is not refused without one, only warned about, since someone may
//! deliberately want a bare key.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::{api, logging};

const VK_SHIFT: u32 = 0x10;
const VK_CONTROL: u32 = 0x11;
const VK_MENU: u32 = 0x12;

/// A modifier set plus a primary virtual-key code.
/// `vk == 0` means unbound and never matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub vk: u32,
}

impl Chord {
    pub const NONE: Self = Self {
        ctrl: false,
        shift: false,
        alt: false,
        vk: 0,
    };

    /// Ctrl+Shift+P: "P" for POV, on a modifier base that cannot type.
    pub const DEFAULT: Self = Self {
        ctrl: true,
        shift: true,
        alt: false,
        vk: b'P' as u32,
    };

    pub const fn is_bound(self) -> bool {
        self.vk != 0
    }

    /// Whether the chord is safe to use globally. See the module docs.
    pub const fn is_typeable(self) -> bool {
        !self.ctrl && !self.alt
    }

    /// Parses `"ctrl+shift+p"`, or `"none"` to unbind.
    ///
    /// Returns `None` for anything unrecognised, so a typo in the config leaves the
    /// previous binding alone instead of silently unbinding.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.eq_ignore_ascii_case("none") || text.is_empty() {
            return Some(Self::NONE);
        }

        let mut chord = Self::NONE;
        let mut key_seen = false;

        for token in text.split('+') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }

            match token.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => chord.ctrl = true,
                "shift" => chord.shift = true,
                "alt" | "menu" => chord.alt = true,
                name => {
                    if key_seen {
                        return None; // two primary keys
                    }
                    chord.vk = key_code(name)?;
                    key_seen = true;
                }
            }
        }

        if !chord.is_bound() {
            return None;
        }
        Some(chord)
    }

    pub fn describe(self) -> String {
        if !self.is_bound() {
            return "none".to_owned();
        }
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("ctrl".to_owned());
        }
        if self.shift {
            parts.push("shift".to_owned());
        }
        if self.alt {
            parts.push("alt".to_owned());
        }
        parts.push(format!("vk{:#04x}", self.vk));
        parts.join("+")
    }
}

fn key_code(name: &str) -> Option<u32> {
    let bytes = name.as_bytes();
    if bytes.len() == 1 {
        let c = bytes[0].to_ascii_uppercase();
        // VK_A..VK_Z and VK_0..VK_9 share their ASCII values.
        if c.is_ascii_uppercase() || c.is_ascii_digit() {
            return Some(c as u32);
        }
    }

    if let Some(digits) = name.strip_prefix('f') {
        if let Ok(n) = digits.parse::<u32>() {
            if (1..=12).contains(&n) {
                return Some(0x70 + n - 1); // VK_F1 = 0x70
            }
        }
    }

    match name {
        "space" => Some(0x20),
        "tab" => Some(0x09),
        "insert" => Some(0x2D),
        "delete" | "del" => Some(0x2E),
        "home" => Some(0x24),
        "end" => Some(0x23),
        "pageup" => Some(0x21),
        "pagedown" => Some(0x22),
        "backquote" => Some(0xC0),
        "minus" => Some(0xBD),
        "equals" => Some(0xBB),
        "comma" => Some(0xBC),
        "period" => Some(0xBE),
        "slash" => Some(0xBF),
        "semicolon" => Some(0xBA),
        "quote" => Some(0xDE),
        "bracketleft" => Some(0xDB),
        "bracketright" => Some(0xDD),
        "backslash" => Some(0xDC),
        _ => None,
    }
}

static CHORD: Mutex<Chord> = Mutex::new(Chord::DEFAULT);
static WAS_DOWN: AtomicBool = AtomicBool::new(false);
static INSTALLED: AtomicBool = AtomicBool::new(false);

pub fn set_chord(chord: Chord) {
    let mut guard = match CHORD.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    if *guard == chord {
        return;
    }

    if chord.is_bound() && chord.is_typeable() {
        logging::warn(&format!(
            "POV: hotkey {} has no Ctrl or Alt, so it will also fire while typing",
            chord.describe()
        ));
    }

    logging::info(&format!("POV: window hotkey = {}", chord.describe()));
    *guard = chord;
}

/// The chord currently bound, for logging.
pub fn current() -> Chord {
    chord()
}

fn chord() -> Chord {
    match CHORD.lock() {
        Ok(guard) => *guard,
        Err(poisoned) => *poisoned.into_inner(),
    }
}

/// Registers the per-frame poll. Needs the host's present callback, which only
/// exists on Windows and only when the Hachimi GUI is enabled.
pub fn install() {
    if INSTALLED.swap(true, Ordering::Relaxed) {
        return;
    }

    let Some(api) = api::get() else {
        return;
    };

    let registered = unsafe {
        (api.hachimi_register_present_callback)(Some(poll), std::ptr::null_mut())
    };
    if registered {
        logging::info(&format!(
            "POV: window hotkey polling at {}",
            chord().describe()
        ));
    } else {
        logging::warn("POV: present callback unavailable; the window hotkey will not work");
    }
}

unsafe extern "C" fn poll(_swapchain: *mut c_void, _userdata: *mut c_void) {
    if !foreground_is_ours() {
        // Reset the edge state so a chord held while unfocused does not fire the
        // moment the game regains focus.
        WAS_DOWN.store(false, Ordering::Relaxed);
        return;
    }

    let chord = chord();
    if !chord.is_bound() {
        return;
    }

    let down = chord_is_down(chord);
    let was_down = WAS_DOWN.swap(down, Ordering::Relaxed);
    if down && !was_down {
        crate::ui::toggle_window();
    }
}

fn chord_is_down(chord: Chord) -> bool {
    if !key_down(chord.vk) {
        return false;
    }
    // Modifiers must match exactly, so Ctrl+Shift+P does not also fire on
    // Ctrl+Shift+Alt+P.
    key_down(VK_CONTROL) == chord.ctrl
        && key_down(VK_SHIFT) == chord.shift
        && key_down(VK_MENU) == chord.alt
}

#[cfg(target_os = "windows")]
mod win {
    use std::ffi::c_void;

    #[link(name = "user32")]
    extern "system" {
        pub fn GetAsyncKeyState(v_key: i32) -> i16;
        pub fn GetForegroundWindow() -> *mut c_void;
        pub fn GetWindowThreadProcessId(hwnd: *mut c_void, process_id: *mut u32) -> u32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        pub fn GetCurrentProcessId() -> u32;
    }
}

fn key_down(vk: u32) -> bool {
    #[cfg(target_os = "windows")]
    unsafe {
        // High bit set means the key is currently down.
        (win::GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = vk;
        false
    }
}

/// Whether the foreground window belongs to this process. The game draws its own
/// overlay, so while Hachimi's menu is open the game window is still the foreground
/// one and chords keep working.
fn foreground_is_ours() -> bool {
    #[cfg(target_os = "windows")]
    unsafe {
        let hwnd = win::GetForegroundWindow();
        if hwnd.is_null() {
            return false;
        }
        let mut pid = 0u32;
        win::GetWindowThreadProcessId(hwnd, &mut pid);
        pid != 0 && pid == win::GetCurrentProcessId()
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}
