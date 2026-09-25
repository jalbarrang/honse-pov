//! honse_pov - a Hachimi Edge plugin for experimenting with a race POV camera.
//!
//! # Milestone 1
//!
//! * Resolve the race classes through the Hachimi plugin API.
//! * Hook the race view `UpdateView(float)` methods to get a main-thread tick.
//! * Build a snapshot of the current race's runners.
//! * Expose a runner picker in the Hachimi menu and in a standalone window.
//!
//! The camera is intentionally **not** touched yet. `race::selected_index()` is
//! the single input milestone 2 will consume to drive a POV camera.
//!
//! # Why there is no `hachimi_register_on_game_initialized` here
//!
//! Hachimi Edge v0.31.2 (`Hachimi::on_hooking_finished`) dispatches the
//! game-initialized callbacks *before* it runs plugin init:
//!
//! ```text
//! GameSystem::on_game_initialized();   // fires plugin_init_callbacks
//! ...
//! for plugin in self.plugins { plugin.init(); }   // a plugin can only register here
//! ```
//!
//! The second dispatch site (`GameSystem::InitializeGame_MoveNext`) is only
//! installed when `ui_scale != 1.0`, so with the default config the callback
//! never fires at all. Plugins must therefore do their setup directly in the
//! entry point.
//!
//! That is safe: plugin init runs right after `il2cpp::hook::init()`, i.e. after
//! the IL2CPP runtime is up and `umamusume.dll` is loaded. Hachimi itself
//! resolves and hooks the same race classes one phase earlier.

mod api;
mod hotkey;
mod il2cpp;
mod logging;
mod math;
mod pov;
mod race;
mod ui;

use std::ffi::c_void;

use api::GetApiFn;

/// The plugin API version this plugin is written against.
const REQUIRED_API_VERSION: i32 = 3;

#[repr(i32)]
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum InitResult {
    Error = 0,
    Ok = 1,
}

/// Best-effort retry, only registered when the direct install attempt failed.
/// On hosts affected by the ordering issue above this may never run.
unsafe extern "C" fn on_game_initialized(_userdata: *mut c_void) {
    if race::classes_ready() {
        return;
    }
    logging::info("game-initialized callback fired; retrying race tracking install");
    if !race::install() {
        logging::error("race tracking retry failed - the runner list will stay empty");
    }
}

/// Hachimi Edge plugin entry point (API v3: name-resolved symbols).
///
/// Runs during `DllMain` -> `load_libraries`, after Hachimi's own il2cpp hooks
/// are installed, so the game's managed assemblies are already available.
#[no_mangle]
pub extern "C" fn hachimi_init_v3(get_api: GetApiFn, version: i32) -> InitResult {
    if version < REQUIRED_API_VERSION {
        api::raw_log(
            get_api,
            logging::ERROR,
            &format!(
                "honse_pov requires plugin API v{} or newer, host reports v{}",
                REQUIRED_API_VERSION, version
            ),
        );
        return InitResult::Error;
    }

    match api::resolve(get_api) {
        Ok(resolved) => api::set(resolved),
        Err(missing) => {
            api::raw_log(
                get_api,
                logging::ERROR,
                &format!(
                    "honse_pov: host is missing required API symbols: {}",
                    missing.join(", ")
                ),
            );
            return InitResult::Error;
        }
    }

    logging::info(&format!("honse_pov 0.1.0 loaded (host plugin API v{})", version));

    ui::register();

    if race::install() {
        logging::info("race tracking ready");
    } else {
        // Keep a retry around in case a future host initializes plugins earlier.
        logging::warn(
            "race tracking install failed at plugin init; registering a retry for the \
             game-initialized callback (which this host build may never dispatch)",
        );

        let Some(api) = api::get() else {
            return InitResult::Error;
        };
        let registered = unsafe {
            (api.hachimi_register_on_game_initialized)(Some(on_game_initialized), std::ptr::null_mut())
        };
        if !registered {
            logging::error("could not register the retry callback either");
        }
    }

    // Milestone 2 camera. Independent of the race tracking above; failures are
    // surfaced in the picker UI instead of aborting the plugin.
    pov::install();
    hotkey::install();

    InitResult::Ok
}
