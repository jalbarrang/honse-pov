//! Race state tracking.
//!
//! Milestone 1 only *reads* race state so the UI can list the runners. Nothing
//! here touches the camera yet; milestone 2 will consume `selected_index()` and
//! drive an actual POV camera.
//!
//! The refresh runs from hooks on the race view's `UpdateView(float)`, which is
//! invoked on the Unity main thread once per frame while a race is playing.
//!
//! There is one implementation per race-view class, because each overrides the
//! virtual method separately:
//!
//! * `RaceViewReplay`      - normal races (single mode, daily, team stadium, ...)
//! * `RaceViewStoryReplay` - story/episode races
//! * `ReceViewLegendReplay` inherits `RaceViewReplay.UpdateView`, so it is
//!   covered by the first hook without an extra target.
//!
//! `RaceViewBase.UpdateView` / `LateUpdateView` are deliberately *not* used:
//! Hachimi Edge itself hooks `RaceViewBase.LateUpdateView`, and sharing a
//! MinHook target with the host is the fragile case we want to avoid.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use crate::{api, il2cpp, logging};

/// How often the runner list is rebuilt, in milliseconds.
const REFRESH_INTERVAL_MS: u64 = 100;

#[derive(Clone, Debug, Default)]
pub struct Runner {
    /// Index used by the race manager / model controllers (`horseIndex`).
    pub index: i32,
    pub gate_no: i32,
    /// 0-based popularity rank as stored by the game, so `+1` is the human rank.
    pub popularity: i32,
    /// Whether this is the player's own runner.
    ///
    /// Derived from `RaceHorseManagerBase.GetPlayerHorseIndex()` rather than
    /// `HorseData.get_IsUser()`, which on this build reports true for the last
    /// three entries of a nine runner field.
    pub is_player: bool,
    /// Raw `HorseData.get_IsUser()` value, kept for diagnosis only.
    pub raw_is_user: bool,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub active: bool,
    pub player_index: i32,
    pub runners: Vec<Runner>,
}

impl Snapshot {
    pub const fn new() -> Self {
        Self {
            active: false,
            player_index: -1,
            runners: Vec::new(),
        }
    }
}

static SNAPSHOT: Mutex<Snapshot> = Mutex::new(Snapshot::new());
static SELECTED: AtomicI32 = AtomicI32::new(-1);
static LAST_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Game classes / fields / methods resolved once at game-init time.
#[derive(Clone, Copy)]
struct Classes {
    race_manager: usize,
    horse_manager_field: usize,
    get_horse_race_infos: usize,
    get_player_horse_index: usize,
    horse_info_get_horse_data: usize,
    horse_data_get_name: usize,
    horse_data_get_gate_no: usize,
    horse_data_get_popularity: usize,
    horse_data_get_is_user: usize,
}

static CLASSES: OnceLock<Classes> = OnceLock::new();
static INSTALLED: AtomicBool = AtomicBool::new(false);
static INSTALL_FAILED: AtomicBool = AtomicBool::new(false);

static CLOCK: OnceLock<Instant> = OnceLock::new();
static LAST_REFRESH_MS: AtomicU64 = AtomicU64::new(0);

// ------------------------------------------------------------------- api ----

pub fn classes_ready() -> bool {
    CLASSES.get().is_some()
}

/// True when `install()` bailed out (missing class/method, hook failure).
pub fn install_failed() -> bool {
    INSTALL_FAILED.load(Ordering::Relaxed)
}

pub fn snapshot() -> Snapshot {
    match SNAPSHOT.lock() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

pub fn selected_index() -> i32 {
    SELECTED.load(Ordering::Relaxed)
}

pub fn set_selected_index(index: i32) {
    let previous = SELECTED.swap(index, Ordering::Relaxed);
    if previous != index {
        logging::info(&format!("selected runner index -> {}", index));
    }
}

// ---------------------------------------------------------------- resolve ---

/// Resolves classes and installs the tick hooks. Idempotent: safe to call from
/// plugin init and again from a retry callback.
pub fn install() -> bool {
    if INSTALLED.load(Ordering::Acquire) {
        return true;
    }

    let installed = install_inner();
    if installed {
        INSTALL_FAILED.store(false, Ordering::Relaxed);
        INSTALLED.store(true, Ordering::Release);
    } else {
        INSTALL_FAILED.store(true, Ordering::Relaxed);
    }
    installed
}

fn install_inner() -> bool {
    // SAFETY: this runs during plugin init, after the host has brought up the IL2CPP runtime.
    // Every `il2cpp::` call here is a lookup that returns null on failure and is checked before use.
    unsafe {
        let image = il2cpp::assembly_image("umamusume.dll");
        if image.is_null() {
            logging::error("could not resolve assembly image 'umamusume.dll'");
            return false;
        }

        let race_manager = il2cpp::class(image, "Gallop", "RaceManager");
        let horse_manager = il2cpp::class(image, "Gallop", "RaceHorseManagerBase");
        let horse_info = il2cpp::class(image, "Gallop", "HorseRaceInfo");
        let horse_data = il2cpp::class(image, "Gallop", "HorseData");
        let race_view_replay = il2cpp::class(image, "Gallop", "RaceViewReplay");
        // Story races only. Missing this class is not fatal.
        let race_view_story = il2cpp::class(image, "Gallop", "RaceViewStoryReplay");

        for (name, class) in [
            ("RaceManager", race_manager),
            ("RaceHorseManagerBase", horse_manager),
            ("HorseRaceInfo", horse_info),
            ("HorseData", horse_data),
            ("RaceViewReplay", race_view_replay),
        ] {
            if class.is_null() {
                logging::error(&format!("could not resolve class Gallop.{}", name));
                return false;
            }
        }

        if race_view_story.is_null() {
            logging::warn("Gallop.RaceViewStoryReplay not found; story races will not populate the list");
        }

        let horse_manager_field = il2cpp::field(race_manager, "_horseManager");
        if horse_manager_field.is_null() {
            logging::error("could not resolve RaceManager._horseManager");
            return false;
        }

        let classes = Classes {
            race_manager: race_manager as usize,
            horse_manager_field: horse_manager_field as usize,
            get_horse_race_infos: il2cpp::method_addr(horse_manager, "GetHorseRaceInfos", 0),
            get_player_horse_index: il2cpp::method_addr(horse_manager, "GetPlayerHorseIndex", 0),
            horse_info_get_horse_data: il2cpp::method_addr(horse_info, "get_HorseData", 0),
            horse_data_get_name: il2cpp::method_addr(horse_data, "get_charaName", 0),
            horse_data_get_gate_no: il2cpp::method_addr(horse_data, "get_GateNo", 0),
            horse_data_get_popularity: il2cpp::method_addr(horse_data, "get_Popularity", 0),
            horse_data_get_is_user: il2cpp::method_addr(horse_data, "get_IsUser", 0),
        };

        for (name, addr) in [
            ("RaceHorseManagerBase.GetHorseRaceInfos", classes.get_horse_race_infos),
            (
                "RaceHorseManagerBase.GetPlayerHorseIndex",
                classes.get_player_horse_index,
            ),
            ("HorseRaceInfo.get_HorseData", classes.horse_info_get_horse_data),
            ("HorseData.get_charaName", classes.horse_data_get_name),
            ("HorseData.get_GateNo", classes.horse_data_get_gate_no),
            ("HorseData.get_Popularity", classes.horse_data_get_popularity),
            ("HorseData.get_IsUser", classes.horse_data_get_is_user),
        ] {
            if addr == 0 {
                logging::error(&format!("could not resolve method {}", name));
                return false;
            }
        }

        // Install the ticks first: `classes_ready()` gates the UI, so we must
        // not advertise readiness before the hooks that populate the snapshot
        // are in place.
        let primary = install_tick(
            race_view_replay,
            update_view_replay as *mut c_void,
            &UPDATE_VIEW_REPLAY_TRAMPOLINE,
            "Gallop.RaceViewReplay.UpdateView",
        );

        if !race_view_story.is_null() {
            install_tick(
                race_view_story,
                update_view_story_replay as *mut c_void,
                &UPDATE_VIEW_STORY_REPLAY_TRAMPOLINE,
                "Gallop.RaceViewStoryReplay.UpdateView",
            );
        }

        if !primary {
            logging::error("failed to hook any race view tick; runner list will stay empty");
            return false;
        }

        let _ = CLASSES.set(classes);
    }

    logging::info("race tracking installed");
    true
}

/// Hooks `<race_view>.UpdateView(float)` purely to get a main-thread tick.
/// The original is always called; we only read state.
unsafe fn install_tick(
    race_view: il2cpp::Class,
    detour: *mut c_void,
    trampoline_slot: &AtomicUsize,
    label: &str,
) -> bool {
    let Some(api) = api::get() else {
        return false;
    };

    let target = il2cpp::method_addr(race_view, "UpdateView", 1);
    if target == 0 {
        logging::error(&format!("could not resolve {}.UpdateView(float)", label));
        return false;
    }

    let hachimi = (api.hachimi_instance)();
    if hachimi.is_null() {
        logging::error("hachimi_instance() returned null");
        return false;
    }
    let interceptor = (api.hachimi_get_interceptor)(hachimi);
    if interceptor.is_null() {
        logging::error("hachimi_get_interceptor() returned null");
        return false;
    }

    let trampoline = (api.interceptor_hook)(interceptor, target as *mut c_void, detour);
    if trampoline.is_null() {
        logging::error(&format!("interceptor_hook failed for {}", label));
        return false;
    }

    trampoline_slot.store(trampoline as usize, Ordering::Release);
    logging::info(&format!("hooked {}", label));
    true
}

type UpdateViewFn = unsafe extern "C" fn(this: *mut c_void, delta_time: f32);

/// Declares a tick detour with its own trampoline slot. Both race-view classes
/// need their own detour because `interceptor_hook` maps one detour address to
/// one target.
macro_rules! tick_detour {
    ($name:ident, $slot:ident) => {
        static $slot: AtomicUsize = AtomicUsize::new(0);

        unsafe extern "C" fn $name(this: *mut c_void, delta_time: f32) {
            if should_refresh() {
                refresh();
            }

            // Milestone 2: recompute the POV pose for the selected runner. Cheap
            // no-op while the toggle is off.
            crate::pov::on_race_tick(this);

            let trampoline = $slot.load(Ordering::Acquire);
            if trampoline != 0 {
                let original: UpdateViewFn = std::mem::transmute(trampoline);
                original(this, delta_time);
            }
        }
    };
}

tick_detour!(update_view_replay, UPDATE_VIEW_REPLAY_TRAMPOLINE);
tick_detour!(update_view_story_replay, UPDATE_VIEW_STORY_REPLAY_TRAMPOLINE);

fn should_refresh() -> bool {
    let elapsed = CLOCK.get_or_init(Instant::now).elapsed().as_millis() as u64;
    let last = LAST_REFRESH_MS.load(Ordering::Relaxed);
    if elapsed.saturating_sub(last) >= REFRESH_INTERVAL_MS {
        LAST_REFRESH_MS.store(elapsed, Ordering::Relaxed);
        true
    } else {
        false
    }
}

// ---------------------------------------------------------------- refresh ---

/// Runs on the Unity main thread. Never panics and never unwinds.
fn refresh() {
    let Some(classes) = CLASSES.get().copied() else {
        return;
    };

    // SAFETY: `classes` comes from `CLASSES`, which is only published once `install_inner` has
    // resolved every class and method. Those pointers are valid for the process lifetime, and
    // `refresh` is driven from the Unity main thread.
    let snapshot = unsafe { build_snapshot(classes) };

    // Dump the field once per race so the index/gate/popularity/player mapping is
    // verifiable straight from the log rather than inferred from the UI.
    let was_active = LAST_ACTIVE.swap(snapshot.active, Ordering::Relaxed);
    if snapshot.active && !was_active {
        let field = snapshot
            .runners
            .iter()
            .map(|r| {
                format!(
                    "{}|gate{}|pop{}(+1={})|{}{}",
                    r.index,
                    r.gate_no,
                    r.popularity,
                    r.popularity + 1,
                    if r.is_player { "PLAYER" } else { "-" },
                    if r.raw_is_user && !r.is_player { "(rawUser)" } else { "" }
                )
            })
            .collect::<Vec<_>>()
            .join("  ");
        logging::info(&format!(
            "POV field ({} runners, player index {}): {}",
            snapshot.runners.len(),
            snapshot.player_index,
            field
        ));
    }

    match SNAPSHOT.lock() {
        Ok(mut guard) => *guard = snapshot,
        Err(poisoned) => *poisoned.into_inner() = snapshot,
    }
}

unsafe fn build_snapshot(classes: Classes) -> Snapshot {
    let race_manager = il2cpp::singleton(classes.race_manager as il2cpp::Class);
    if race_manager.is_null() {
        return Snapshot::new();
    }

    let horse_manager = il2cpp::field_object(race_manager, classes.horse_manager_field as il2cpp::Field);
    if horse_manager.is_null() {
        return Snapshot::new();
    }

    let infos = il2cpp::call_obj0(classes.get_horse_race_infos, horse_manager);
    if infos.is_null() {
        return Snapshot::new();
    }

    let count = il2cpp::array_len(infos);
    if count == 0 {
        return Snapshot::new();
    }

    let player_index = il2cpp::call_i32_0(classes.get_player_horse_index, horse_manager);

    let mut runners = Vec::with_capacity(count);
    for index in 0..count {
        let info = il2cpp::array_get(infos, index);
        if info.is_null() {
            continue;
        }

        let horse_data = il2cpp::call_obj0(classes.horse_info_get_horse_data, info);
        if horse_data.is_null() {
            continue;
        }

        runners.push(Runner {
            index: index as i32,
            gate_no: il2cpp::call_i32_0(classes.horse_data_get_gate_no, horse_data),
            popularity: il2cpp::call_i32_0(classes.horse_data_get_popularity, horse_data),
            is_player: index as i32 == player_index,
            raw_is_user: il2cpp::call_bool_0(classes.horse_data_get_is_user, horse_data),
            name: il2cpp::read_string(il2cpp::call_obj0(classes.horse_data_get_name, horse_data)),
        });
    }

    Snapshot {
        active: !runners.is_empty(),
        player_index,
        runners,
    }
}
