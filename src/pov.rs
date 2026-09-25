//! Milestone 2: the actual POV camera.
//!
//! # Design
//!
//! Activation is an explicit toggle plus a selected runner (see `ENABLED` /
//! `race::selected_index`). Selection alone never touches the camera, and
//! toggling off stops all writes, so the game's camera logic resumes on the next
//! frame untouched.
//!
//! While enabled the plugin does four things:
//!
//! 1. Places the camera at the selected runner's eye midpoint, facing forward.
//! 2. Hides that runner's `M_Hair` / `M_Face` child objects so the camera is not
//!    looking at the inside of their head.
//! 3. Sets the camera FOV to `POV_FOV`.
//! 4. Suppresses cut-ins via the game's own `EventCamera._unPlayable` flag.
//!
//! Look control is intentionally absent: The plugin API exposes no key or mouse
//! events, so free-look would need `UnityEngine.Input` polling through il2cpp.
//!
//! # Why `CourseCameraController.OnPreCull`
//!
//! Hachimi Edge already owns the obvious targets, and MinHook (v0.9.0 as
//! vendored by the `minhook` crate) enforces **one hook per target** —
//! `MH_CreateHook` returns `MH_ERROR_ALREADY_CREATED`, there is no chaining.
//! Occupied targets therefore cannot be used at all:
//!
//! * `Transform.set_position_Injected` / `set_rotation_Injected` /
//!   `Internal_LookAt_Injected` — the mechanism Hachimi's own free camera uses
//! * `RaceViewBase.LateUpdateView`
//! * `RaceCameraManager.AlterLateUpdate` / `ChangeCameraMode` / `PlayEventCamera`
//! * `RaceHorseManagerBase.Init` / `Release`
//! * `GameSystem.Update` / `LateUpdate`
//!
//! Because Hachimi's free camera works by substituting the position passed into
//! `Transform.set_position_Injected` *during* `RaceCameraManager.AlterLateUpdate`,
//! the game's final camera write must happen inside that call. Any hook that runs
//! strictly later therefore wins. `OnPreCull` is guaranteed later than every
//! `LateUpdate`, and Hachimi does not touch it.
//!
//! # Scope of the override
//!
//! Only applied while the race camera is the active one
//! (`CurrentCamera == MainCamera`), so gate-in and the goal camera still work.
//! Blocking camera changes would require hooking the occupied
//! `ChangeCameraMode` / `PlayEventCamera`, which is why cut-ins are suppressed
//! through `EventCamera._unPlayable` instead.

use std::ffi::{c_void, CString};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use crate::{
    api, il2cpp, logging,
    math::{Quat, Vec3},
};

/// `Gallop.CoursePrefabParam.CharaAttachTransform` enum values.
///
/// The shipped method is `GetPrefabAttachTransform(CharaAttachTransform type)`
/// — a single enum argument. Hachimi Edge resolves it with `args_count = 2`
/// (a `part`/`name` pair), so its own address comes back NULL and its race
/// first-person path silently never runs. The enum ordering is:
///
/// ```text
/// Root=0 M_Face=1 Waist=2 Spine=3 Chest=4 Neck=5 Head=6
/// Eye_L=7 Eye_R=8 Toe_L=9 Toe_R=10 Nose=11
/// ```
const ATTACH_EYE_LEFT: i32 = 7;
const ATTACH_EYE_RIGHT: i32 = 8;

/// Which `CharaAttachTransform` the camera rides on.
///
/// `Eyes` averages the two eye transforms (a head-mounted view, must hide the
/// self model at wide FOV). `Chest` reproduces the look the game's own first
/// person cam uses: sitting inside the torso, which hides it by backface culling
/// while leaving the arms and hands visible in front.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttachPoint {
    Eyes,
    Head,
    Neck,
    Chest,
}

impl AttachPoint {
    const fn value(self) -> i32 {
        match self {
            Self::Chest => 4,
            Self::Neck => 5,
            Self::Head => 6,
            Self::Eyes => ATTACH_EYE_LEFT,
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text.to_ascii_lowercase().as_str() {
            "eyes" | "eye" => Some(Self::Eyes),
            "head" => Some(Self::Head),
            "neck" => Some(Self::Neck),
            "chest" | "body" => Some(Self::Chest),
            _ => None,
        }
    }
}

/// How `RaceViewBase.Culling` is treated while POV is on.
///
/// The game culls each model by distance from *its* camera and pauses the hair and
/// cloth springs of anything it culls (`ModelController.SetVisible` →
/// `CySpringController.IsSkipUpdatePre`). That pass runs in the LateUpdate phase,
/// before we move the camera in `OnPreCull`, so its idea of "on screen" is the
/// game's camera rather than our POV — which is why runners beside the one you are
/// watching freeze mid-gallop while everything else keeps moving.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CullingMode {
    /// Leave the game's culling alone.
    Default,
    /// Keep the call, but pass `CullingType.None` so nothing is culled.
    None,
    /// Drop the call entirely.
    Skip,
}

impl CullingMode {
    fn parse(text: &str) -> Option<Self> {
        match text.to_ascii_lowercase().as_str() {
            "default" | "game" => Some(Self::Default),
            "none" => Some(Self::None),
            "skip" => Some(Self::Skip),
            _ => None,
        }
    }
}

/// Field of view, offsets and stabiliser settings.
///
/// Defaults come from Trainers' Legend G (`liveFirstPersonOffset`
/// `(0, 0.075, 0.015)`, also Hachimi Edge's `live_first_person_offset` default)
/// and are overridable at runtime through `honse_pov.ini` in the Hachimi folder,
/// which is reloaded once a second so tuning does not need a rebuild.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Tuning {
    /// Field of view in degrees, **horizontal** (the Quake convention).
    /// Unity's `Camera.fieldOfView` is vertical, so this is converted using the
    /// camera's aspect ratio. 90 horizontal is ~59 vertical at 16:9.
    fov: f32,
    /// Which attach point the camera rides on.
    attach: AttachPoint,
    /// Metres along the attach point's local forward axis (+Z).
    forward: f32,
    /// Metres straight up, world space. Negative moves down.
    up: f32,
    /// Max rotation change per frame, radians. `<= 0` tracks the head rigidly.
    max_rot_step: f32,
    /// Near clip plane while POV is active. `<= 0` leaves the game's value alone.
    ///
    /// Required because the game derives its near clip from the camera's FOV: our
    /// persistent FOV override is read back on the next frame and inflates the
    /// clip plane, which is what makes runners metres ahead get cut away.
    near_clip: f32,
    /// Far clip plane while POV is active. `<= 0` leaves the game's value alone.
    ///
    /// The game computes its far clip from the camera transform
    /// (`RaceCameraManager.CalcFarClipPlane`), so it shrinks at some track
    /// positions and clips the skydome, which reads as the sky being eaten. Same
    /// value Hachimi Edge forces for its own free camera.
    far_clip: f32,
    /// Hide the `M_Face` / `M_Hair` meshes.
    hide_head: bool,
    /// How the game's per-model culling pass is treated while POV is on.
    culling: CullingMode,
    /// Also hide the rest of the runner's own model (`M_Body` and friends).
    ///
    /// Needed for the eye mount: the camera sits at the face and the shoulders
    /// are only ~20 cm below, so at a wide FOV the runner's own torso fills the
    /// bottom of the frame. From *inside* the chest the torso culls itself, so
    /// this can stay off for the chest mount and leave the arms visible.
    hide_body: bool,
    suppress_cutins: bool,
}

impl Tuning {
    const fn defaults() -> Self {
        Self {
            fov: 90.0,
            attach: AttachPoint::Eyes,
            forward: 0.015,
            up: 0.075,
            max_rot_step: 0.01,
            near_clip: 0.05,
            far_clip: 2500.0,
            hide_head: true,
            culling: CullingMode::None,
            hide_body: true,
            suppress_cutins: true,
        }
    }
}

const CONFIG_FILE: &str = "honse_pov.ini";

const DEFAULT_CONFIG: &str = "\
# honse_pov tuning. Reloaded automatically every second, no restart needed.\n\
# Distances are metres in the attach point's local frame: z = forward, y = up.\n\
#\n\
# fov           HORIZONTAL field of view in degrees, Quake convention.\n\
#               Unity's fieldOfView is vertical, so this is converted with the\n\
#               camera aspect: 90 horizontal is ~59 vertical at 16:9.\n\
# attach        which bone the camera rides: eyes | head | neck | chest\n\
# forward       nudge along the attach point's forward axis\n\
# up            nudge straight up (negative moves down, e.g. into the chest)\n\
# max_rot_step  max rotation change per frame in radians; 0 = rigid head tracking\n\
# near_clip     near clip plane while POV is on; 0 = leave the game's value alone\n\
# far_clip      far clip plane while POV is on; 0 = leave the game's value alone.\n\
#               The game recomputes this from the camera transform, and it\n\
#               shrinks at some track positions, clipping the skydome.\n\
# hide_head     hide the M_Face / M_Hair meshes (1/0)\n\
# hide_body     also hide the runner's own body meshes (1/0). Only needed for the\n\
#               eye mount; from inside the chest the torso culls itself.\n\
# culling       what the game's per-model culling pass does while POV is on:\n\
#                 default = leave it alone\n\
#                 none    = keep the call but never cull (fixes frozen hair/cloth)\n\
#                 skip    = drop the call\n\
# suppress_cutins  drop skill cut-ins at the source while POV is on (1/0)\n\
#\n\
# Chest cam, matching the game's own first person view (arms/hands visible):\n\
#   attach = chest\n\
#   hide_body = 0\n\
#   up = 0\n\
fov = 90\n\
attach = eyes\n\
forward = 0.015\n\
up = 0.075\n\
max_rot_step = 0.01\n\
near_clip = 0.05\n\
far_clip = 2500\n\
hide_head = 1\n\
hide_body = 1\n\
culling = none\n\
suppress_cutins = 1\n";

static TUNING: Mutex<Tuning> = Mutex::new(Tuning::defaults());
static LAST_TUNING_RELOAD_MS: AtomicU64 = AtomicU64::new(0);
static CONFIG_WRITTEN: AtomicBool = AtomicBool::new(false);

fn tuning() -> Tuning {
    match TUNING.lock() {
        Ok(guard) => *guard,
        Err(poisoned) => *poisoned.into_inner(),
    }
}

/// Re-reads `honse_pov.ini` at most once a second, writing a commented default
/// file the first time so there is something to edit.
fn reload_tuning() {
    let now = elapsed_ms();
    if now.saturating_sub(LAST_TUNING_RELOAD_MS.load(Ordering::Relaxed)) < 1000 {
        return;
    }
    LAST_TUNING_RELOAD_MS.store(now, Ordering::Relaxed);

    let Some(api) = api::get() else {
        return;
    };

    let base = unsafe { (api.hachimi_get_base_dir)() };
    if base.is_null() {
        return;
    }
    let base = unsafe { std::ffi::CStr::from_ptr(base) };
    let path = std::path::Path::new(&*base.to_string_lossy()).join(CONFIG_FILE);

    if !path.exists() {
        if !CONFIG_WRITTEN.swap(true, Ordering::Relaxed) {
            match std::fs::write(&path, DEFAULT_CONFIG) {
                Ok(()) => logging::info(&format!("POV: wrote default config to {}", path.display())),
                Err(e) => logging::warn(&format!("POV: could not write {}: {e}", path.display())),
            }
        }
        return;
    }

    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };

    let mut next = Tuning::defaults();
    for line in text.lines() {
        let line = line.split(['#', ';']).next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        let flag = |v: &str| matches!(v, "1" | "true" | "yes" | "on");

        match key {
            "fov" => {
                if let Ok(v) = value.parse() {
                    next.fov = v;
                }
            }
            "forward" => {
                if let Ok(v) = value.parse() {
                    next.forward = v;
                }
            }
            "up" => {
                if let Ok(v) = value.parse() {
                    next.up = v;
                }
            }
            "max_rot_step" => {
                if let Ok(v) = value.parse() {
                    next.max_rot_step = v;
                }
            }
            "near_clip" => {
                if let Ok(v) = value.parse() {
                    next.near_clip = v;
                }
            }
            "far_clip" => {
                if let Ok(v) = value.parse() {
                    next.far_clip = v;
                }
            }
            "attach" => {
                if let Some(v) = AttachPoint::parse(value) {
                    next.attach = v;
                }
            }
            "hide_head" => next.hide_head = flag(value),
            "culling" => {
                if let Some(v) = CullingMode::parse(value) {
                    next.culling = v;
                }
            }
            "hide_body" => next.hide_body = flag(value),
            "suppress_cutins" => next.suppress_cutins = flag(value),
            _ => {}
        }
    }

    let mut guard = match TUNING.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if *guard != next {
        logging::info(&format!(
            "POV tuning: fov={}(h) attach={:?} forward={} up={} max_rot_step={} near_clip={} far_clip={} hide_head={} hide_body={} culling={:?} suppress_cutins={}",
            next.fov, next.attach, next.forward, next.up, next.max_rot_step, next.near_clip,
            next.far_clip, next.hide_head, next.hide_body, next.culling, next.suppress_cutins
        ));
        *guard = next;
    }
}

/// Child objects of the model owner that block an eye-level view.
const HEAD_PARTS: [&str; 2] = ["M_Hair", "M_Face"];

/// Everything that makes up the runner's own visible model. Hiding all of it is
/// what gives a "no self model" first person view; the head list alone leaves the
/// torso and shoulders in frame.
const BODY_PARTS: [&str; 7] = [
    "M_Body", "M_Cheek", "M_Face", "M_Hair", "M_Mayu", "Eyes", "M_Tail",
];

const LOG_INTERVAL_MS: u64 = 1000;

#[derive(Clone, Copy, Debug)]
pub struct Pose {
    pub pos: Vec3,
    pub rot: Quat,
}

static ENABLED: AtomicBool = AtomicBool::new(false);
static POSE: Mutex<Option<Pose>> = Mutex::new(None);
static ON_PRE_CULL_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static CLOCK: OnceLock<Instant> = OnceLock::new();
static LAST_POSE_LOG_MS: AtomicU64 = AtomicU64::new(0);
static LAST_APPLY_LOG_MS: AtomicU64 = AtomicU64::new(0);
static LAST_CUTIN_LOG_MS: AtomicU64 = AtomicU64::new(0);
static LAST_CULLING_LOG_MS: AtomicU64 = AtomicU64::new(0);

/// Tracks whether we have actually flipped `EventCamera._unPlayable`, so the flag
/// is only touched on transitions and can always be restored.
static CUTINS_SUPPRESSED: AtomicBool = AtomicBool::new(false);

/// `GameObject` pointers we deactivated, and the runner they belong to.
static HIDDEN_HEADS: Mutex<Vec<usize>> = Mutex::new(Vec::new());
static HIDDEN_FOR: AtomicI32 = AtomicI32::new(-1);

/// Previous frame's applied orientation, for the stabiliser. `None` means "snap
/// to the target", which is what we want on the first frame and whenever the
/// runner or POV state changes.
static LAST_ROT: Mutex<Option<Quat>> = Mutex::new(None);

/// Set once by `install()` so the UI can explain why POV is unavailable.
static UNAVAILABLE_REASON: Mutex<Option<String>> = Mutex::new(None);

#[derive(Clone, Copy)]
struct PovClasses {
    /// `CourseCameraController._cameraManager` — the manager belonging to the
    /// instance whose `OnPreCull` we hooked.
    course_manager_field: usize,
    get_main_camera: usize,
    get_current_camera: usize,
    /// `UnityEngine.Component::get_transform()` is an icall, not a managed method.
    component_get_transform: usize,
    camera_set_fov: usize,
    camera_get_near_clip: usize,
    camera_set_near_clip: usize,
    camera_get_far_clip: usize,
    camera_set_far_clip: usize,
    camera_get_aspect: usize,
    get_model_controller: usize,
    get_prefab_attach_transform: usize,
    transform_get_position: usize,
    transform_get_rotation: usize,
    transform_set_position: usize,
    transform_set_rotation: usize,

    // Cut-in suppression.
    get_event_camera: usize,
    event_camera_unplayable_field: usize,

    // Head-part hiding.
    model_get_owner: usize,
    gameobject_get_transform: usize,
    transform_get_child_count: usize,
    transform_get_child: usize,
    component_get_gameobject: usize,
    object_get_name: usize,
    gameobject_set_active: usize,
    object_is_native_alive: usize,
}

static CLASSES: OnceLock<PovClasses> = OnceLock::new();

type GetVec3Fn = unsafe extern "C" fn(*mut c_void, *mut Vec3);
type SetVec3Fn = unsafe extern "C" fn(*mut c_void, *mut Vec3);
type GetQuatFn = unsafe extern "C" fn(*mut c_void, *mut Quat);
type SetQuatFn = unsafe extern "C" fn(*mut c_void, *mut Quat);
type GetTransformFn = unsafe extern "C" fn(*mut c_void) -> *mut c_void;

// ------------------------------------------------------------- public api ---

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn set_enabled(value: bool) {
    if ENABLED.swap(value, Ordering::Relaxed) != value {
        logging::info(if value {
            "POV enabled"
        } else {
            "POV disabled; camera returned to the game"
        });
        if !value {
            clear_pose();
        }
    }
}

pub fn is_available() -> bool {
    CLASSES.get().is_some()
}

/// Human-readable state for the picker UI.
pub fn status() -> String {
    if let Some(reason) = UNAVAILABLE_REASON.lock().ok().and_then(|g| g.clone()) {
        return format!("POV unavailable: {reason}");
    }
    if !is_available() {
        return "POV unavailable: not resolved yet".to_owned();
    }
    if !enabled() {
        return "POV off".to_owned();
    }
    match POSE.lock().ok().and_then(|g| *g) {
        Some(pose) => format!(
            "POV on - eye ({:.2}, {:.2}, {:.2}), fov {:.0}",
            pose.pos.x, pose.pos.y, pose.pos.z, tuning().fov
        ),
        None => "POV on - waiting for a selected runner with a loaded model".to_owned(),
    }
}

fn clear_pose() {
    match POSE.lock() {
        Ok(mut guard) => *guard = None,
        Err(poisoned) => *poisoned.into_inner() = None,
    }
    reset_rotation_stabilizer();
}

fn reset_rotation_stabilizer() {
    match LAST_ROT.lock() {
        Ok(mut guard) => *guard = None,
        Err(poisoned) => *poisoned.into_inner() = None,
    }
}

/// Clamps the per-frame rotation change so the view follows the head smoothly
/// instead of inheriting every animated bob.
fn stabilize_rotation(target: Quat, max_step: f32) -> Quat {
    if !(max_step > 0.0) {
        return target;
    }

    let mut guard = match LAST_ROT.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    let applied = match *guard {
        Some(previous) => {
            let gap = Quat::angle_to(previous, target);
            if gap <= max_step {
                target
            } else {
                Quat::slerp(previous, target, max_step / gap)
            }
        }
        None => target,
    };

    *guard = Some(applied);
    applied
}

fn set_unavailable(reason: String) {
    logging::error(&format!("POV unavailable: {reason}"));
    if let Ok(mut guard) = UNAVAILABLE_REASON.lock() {
        *guard = Some(reason);
    }
}

fn elapsed_ms() -> u64 {
    CLOCK.get_or_init(Instant::now).elapsed().as_millis() as u64
}

fn should_log(slot: &AtomicU64) -> bool {
    let now = elapsed_ms();
    let last = slot.load(Ordering::Relaxed);
    if now.saturating_sub(last) >= LOG_INTERVAL_MS {
        slot.store(now, Ordering::Relaxed);
        true
    } else {
        false
    }
}

// ---------------------------------------------------------------- install ---

pub fn install() {
    unsafe {
        let core_image = il2cpp::assembly_image("UnityEngine.CoreModule.dll");
        let game_image = il2cpp::assembly_image("umamusume.dll");
        if core_image.is_null() || game_image.is_null() {
            set_unavailable("could not resolve engine/game assemblies".to_owned());
            return;
        }

        let race_manager = il2cpp::class(game_image, "Gallop", "RaceCameraManager");
        let race_view_base = il2cpp::class(game_image, "Gallop", "RaceViewBase");
        let model_controller = il2cpp::class(game_image, "Gallop", "RaceModelController");
        let base_model_controller = il2cpp::class(game_image, "Gallop", "ModelController");
        let event_camera = il2cpp::class(game_image, "Gallop", "EventCamera");
        let course_camera_controller =
            il2cpp::class(game_image, "Gallop", "CourseCameraController");

        let camera_class = il2cpp::class(core_image, "UnityEngine", "Camera");
        let gameobject_class = il2cpp::class(core_image, "UnityEngine", "GameObject");
        let transform_class = il2cpp::class(core_image, "UnityEngine", "Transform");
        let object_class = il2cpp::class(core_image, "UnityEngine", "Object");

        for (name, class) in [
            ("Gallop.RaceCameraManager", race_manager),
            ("Gallop.RaceViewBase", race_view_base),
            ("Gallop.RaceModelController", model_controller),
            ("Gallop.ModelController", base_model_controller),
            ("Gallop.EventCamera", event_camera),
            ("Gallop.CourseCameraController", course_camera_controller),
            ("UnityEngine.Camera", camera_class),
            ("UnityEngine.GameObject", gameobject_class),
            ("UnityEngine.Transform", transform_class),
            ("UnityEngine.Object", object_class),
        ] {
            if class.is_null() {
                set_unavailable(format!("could not resolve class {name}"));
                return;
            }
        }

        let Some(api) = api::get() else {
            set_unavailable("plugin API not initialised".to_owned());
            return;
        };

        // Unity 2020+ exposes these as icalls; resolve by name exactly as Hachimi does.
        let icall = |name: &str| -> usize {
            match CString::new(name) {
                Ok(name) => (api.il2cpp_resolve_icall)(name.as_ptr()) as usize,
                Err(_) => 0,
            }
        };

        let classes = PovClasses {
            course_manager_field: il2cpp::field(course_camera_controller, "_cameraManager") as usize,
            get_main_camera: il2cpp::method_addr(race_manager, "get_MainCamera", 0),
            get_current_camera: il2cpp::method_addr(race_manager, "get_CurrentCamera", 0),
            component_get_transform: icall("UnityEngine.Component::get_transform()"),
            camera_set_fov: il2cpp::method_addr(camera_class, "set_fieldOfView", 1),
            camera_get_near_clip: icall("UnityEngine.Camera::get_nearClipPlane()"),
            camera_set_near_clip: icall("UnityEngine.Camera::set_nearClipPlane(System.Single)"),
            camera_get_far_clip: icall("UnityEngine.Camera::get_farClipPlane()"),
            camera_set_far_clip: icall("UnityEngine.Camera::set_farClipPlane(System.Single)"),
            camera_get_aspect: il2cpp::method_addr(camera_class, "get_aspect", 0),
            get_model_controller: il2cpp::method_addr(race_view_base, "GetModelController", 1),
            get_prefab_attach_transform: il2cpp::method_addr(
                model_controller,
                "GetPrefabAttachTransform",
                1,
            ),
            transform_get_position: icall("UnityEngine.Transform::get_position_Injected(UnityEngine.Vector3&)"),
            transform_get_rotation: icall("UnityEngine.Transform::get_rotation_Injected(UnityEngine.Quaternion&)"),
            transform_set_position: icall("UnityEngine.Transform::set_position_Injected(UnityEngine.Vector3&)"),
            transform_set_rotation: icall("UnityEngine.Transform::set_rotation_Injected(UnityEngine.Quaternion&)"),
            get_event_camera: il2cpp::method_addr(race_manager, "get_EventCamera", 0),
            event_camera_unplayable_field: il2cpp::field(event_camera, "_unPlayable") as usize,
            model_get_owner: il2cpp::method_addr(base_model_controller, "get_OwnerObject", 0),
            gameobject_get_transform: il2cpp::method_addr(gameobject_class, "get_transform", 0),
            transform_get_child_count: il2cpp::method_addr(transform_class, "get_childCount", 0),
            transform_get_child: il2cpp::method_addr(transform_class, "GetChild", 1),
            component_get_gameobject: icall("UnityEngine.Component::get_gameObject()"),
            object_get_name: il2cpp::method_addr(object_class, "get_name", 0),
            gameobject_set_active: icall("UnityEngine.GameObject::SetActive(System.Boolean)"),
            object_is_native_alive: il2cpp::method_addr(object_class, "IsNativeObjectAlive", 1),
        };

        for (name, addr) in [
            ("CourseCameraController._cameraManager", classes.course_manager_field),
            ("RaceCameraManager.get_MainCamera", classes.get_main_camera),
            ("RaceCameraManager.get_CurrentCamera", classes.get_current_camera),
            ("Component::get_transform", classes.component_get_transform),
            ("Camera.set_fieldOfView", classes.camera_set_fov),
            ("Camera::get_nearClipPlane", classes.camera_get_near_clip),
            ("Camera::set_nearClipPlane", classes.camera_set_near_clip),
            ("Camera::get_farClipPlane", classes.camera_get_far_clip),
            ("Camera::set_farClipPlane", classes.camera_set_far_clip),
            ("Camera.get_aspect", classes.camera_get_aspect),
            ("RaceViewBase.GetModelController", classes.get_model_controller),
            ("RaceModelController.GetPrefabAttachTransform", classes.get_prefab_attach_transform),
            ("Transform::get_position_Injected", classes.transform_get_position),
            ("Transform::get_rotation_Injected", classes.transform_get_rotation),
            ("Transform::set_position_Injected", classes.transform_set_position),
            ("Transform::set_rotation_Injected", classes.transform_set_rotation),
            ("RaceCameraManager.get_EventCamera", classes.get_event_camera),
            ("EventCamera._unPlayable", classes.event_camera_unplayable_field),
            ("ModelController.get_OwnerObject", classes.model_get_owner),
            ("GameObject.get_transform", classes.gameobject_get_transform),
            ("Transform.get_childCount", classes.transform_get_child_count),
            ("Transform.GetChild", classes.transform_get_child),
            ("Component::get_gameObject", classes.component_get_gameobject),
            ("Object.get_name", classes.object_get_name),
            ("GameObject::SetActive", classes.gameobject_set_active),
            ("Object.IsNativeObjectAlive", classes.object_is_native_alive),
        ] {
            if addr == 0 {
                set_unavailable(format!("could not resolve {name}"));
                return;
            }
        }

        // Hook the post-LateUpdate point. Deliberately not a target Hachimi owns.
        let target = il2cpp::method_addr(course_camera_controller, "OnPreCull", 0);
        if target == 0 {
            set_unavailable("could not resolve CourseCameraController.OnPreCull".to_owned());
            return;
        }

        let hachimi = (api.hachimi_instance)();
        let interceptor = (api.hachimi_get_interceptor)(hachimi);
        if interceptor.is_null() {
            set_unavailable("hachimi_get_interceptor returned null".to_owned());
            return;
        }

        let trampoline = (api.interceptor_hook)(
            interceptor,
            target as *mut c_void,
            course_on_pre_cull as *mut c_void,
        );
        if trampoline.is_null() {
            set_unavailable("interceptor_hook failed for CourseCameraController.OnPreCull".to_owned());
            return;
        }

        ON_PRE_CULL_TRAMPOLINE.store(trampoline as usize, Ordering::Release);
        let _ = CLASSES.set(classes);
    }

    install_cutin_hooks();
    install_culling_hook();

    logging::info("POV installed (hooked Gallop.CourseCameraController.OnPreCull)");
}

// -------------------------------------------------------- cut-in blocking ---
//
// `EventCamera._unPlayable` only stops cut-ins that route through the event
// camera, and `AddCutInInfo` never fires in practice: the reserve is built once at
// race load by `CreatePlayReserved`, before the race runs. So the gate that
// actually matters is `FindReserve`, the query the race makes before entering the
// cut-in state — answering false is equivalent to having an empty reserve list,
// and every other piece of race code runs exactly as it always did.
//
// `AddCutInInfo` is still hooked (both overloads, which share a name and are told
// apart by arity) to catch anything added incrementally, and to avoid the asset
// load that queuing would trigger.

static ADD_CUT_IN_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static ADD_CUT_IN_NAMED_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static FIND_RESERVE_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// Trailing `MethodInfo*` is part of the IL2CPP calling convention and is
/// forwarded rather than dropped: passing it on is correct whether or not the
/// compiled method reads it, and reconstructing one would not be.
type AddCutInFn = unsafe extern "C" fn(*mut c_void, *mut c_void, i32, i32, f32, *mut c_void);
type AddCutInNamedFn =
    unsafe extern "C" fn(*mut c_void, *mut c_void, i32, i32, *mut c_void, f32, *mut c_void);
type FindReserveFn = unsafe extern "C" fn(*mut c_void, i32, *mut c_void) -> bool;

fn suppress_cutins() -> bool {
    enabled() && tuning().suppress_cutins
}

/// `RaceManager.CutInCategory`: 0 Null, 1 Eye, 2 Unique, 3 UniqueRare.
fn should_drop_cutin(category: i32, skill_id: i32) -> bool {
    if !suppress_cutins() {
        return false;
    }
    if should_log(&LAST_CUTIN_LOG_MS) {
        logging::info(&format!(
            "POV: dropped cut-in (category {category}, skill {skill_id})"
        ));
    }
    true
}

/// The gate the race actually consults. Returning false means "no cut-in queued".
unsafe extern "C" fn find_reserve(
    this: *mut c_void,
    horse_index: i32,
    method: *mut c_void,
) -> bool {
    if suppress_cutins() {
        if should_log(&LAST_CUTIN_LOG_MS) {
            logging::info(&format!("POV: cut-in suppressed for horse {horse_index}"));
        }
        return false;
    }

    let trampoline = FIND_RESERVE_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline == 0 {
        // Unreachable: the hook only exists once the trampoline is stored. Refuse
        // the cut-in rather than claim one exists.
        return false;
    }
    let original: FindReserveFn = std::mem::transmute(trampoline);
    original(this, horse_index, method)
}

unsafe extern "C" fn add_cut_in_info(
    this: *mut c_void,
    horse: *mut c_void,
    category: i32,
    skill_id: i32,
    time: f32,
    method: *mut c_void,
) {
    if should_drop_cutin(category, skill_id) {
        return;
    }
    let trampoline = ADD_CUT_IN_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline != 0 {
        let original: AddCutInFn = std::mem::transmute(trampoline);
        original(this, horse, category, skill_id, time, method);
    }
}

unsafe extern "C" fn add_cut_in_info_named(
    this: *mut c_void,
    horse: *mut c_void,
    category: i32,
    skill_id: i32,
    cut_in_name: *mut c_void,
    time: f32,
    method: *mut c_void,
) {
    if should_drop_cutin(category, skill_id) {
        return;
    }
    let trampoline = ADD_CUT_IN_NAMED_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline != 0 {
        let original: AddCutInNamedFn = std::mem::transmute(trampoline);
        original(this, horse, category, skill_id, cut_in_name, time, method);
    }
}

fn install_cutin_hooks() {
    unsafe {
        let Some(api) = api::get() else {
            return;
        };
        let image = il2cpp::assembly_image("umamusume.dll");
        if image.is_null() {
            return;
        }
        let class = il2cpp::class(image, "Gallop", "RaceSkillCutInReserveCreator");
        if class.is_null() {
            logging::warn("POV: RaceSkillCutInReserveCreator not found; unique skill cut-ins will still play");
            return;
        }

        let hachimi = (api.hachimi_instance)();
        let interceptor = (api.hachimi_get_interceptor)(hachimi);
        if interceptor.is_null() {
            return;
        }

        for (arity, detour, slot, label) in [
            (
                4,
                add_cut_in_info as *mut c_void,
                &ADD_CUT_IN_TRAMPOLINE,
                "AddCutInInfo/4",
            ),
            (
                5,
                add_cut_in_info_named as *mut c_void,
                &ADD_CUT_IN_NAMED_TRAMPOLINE,
                "AddCutInInfo/5",
            ),
        ] {
            let target = il2cpp::method_addr(class, "AddCutInInfo", arity);
            if target == 0 {
                logging::warn(&format!("POV: {label} not found; cut-ins may still play"));
                continue;
            }

            let trampoline = (api.interceptor_hook)(interceptor, target as *mut c_void, detour);
            if trampoline.is_null() {
                logging::warn(&format!("POV: hook failed for {label}"));
                continue;
            }

            slot.store(trampoline as usize, Ordering::Release);
            logging::info(&format!("POV: hooked RaceSkillCutInReserveCreator.{label}"));
        }

        // The one that actually matters: the race asks this before entering the
        // cut-in state, so it covers reserves built at load time too.
        let target = il2cpp::method_addr(class, "FindReserve", 1);
        if target == 0 {
            logging::warn("POV: FindReserve not found; unique skill cut-ins will still play");
            return;
        }
        let trampoline =
            (api.interceptor_hook)(interceptor, target as *mut c_void, find_reserve as *mut c_void);
        if trampoline.is_null() {
            logging::warn("POV: hook failed for FindReserve");
            return;
        }
        FIND_RESERVE_TRAMPOLINE.store(trampoline as usize, Ordering::Release);
        logging::info("POV: hooked RaceSkillCutInReserveCreator.FindReserve");
    }
}

// ------------------------------------------------------- head-part hiding ---

/// Hides the selected runner's hair and face meshes by setting those two child
/// GameObjects inactive.
///
/// This is a reversible *visibility* toggle: the objects are not deleted or
/// modified, the bone hierarchy and skinning are untouched, and the runner keeps
/// animating normally. It only stops the face/hair meshes from being drawn, which
/// is what makes an eye-level camera usable. Mirrors what Hachimi Edge's free
/// camera does for race first-person.
unsafe fn hide_head_parts(classes: PovClasses, model: il2cpp::Obj, hide_body: bool) {
    let owner = il2cpp::call_obj0(classes.model_get_owner, model);
    if owner.is_null() {
        logging::warn("POV: model has no owner object; cannot hide head parts");
        return;
    }

    let root = il2cpp::call_obj0(classes.gameobject_get_transform, owner);
    if root.is_null() {
        logging::warn("POV: owner object has no transform");
        return;
    }

    let count = il2cpp::call_i32_0(classes.transform_get_child_count, root);
    if count <= 0 {
        logging::warn("POV: owner transform has no children; cannot hide head parts");
        return;
    }

    let mut hidden = Vec::new();
    let mut names = Vec::new();
    for i in 0..count {
        let child = il2cpp::call_obj1_i32(classes.transform_get_child, root, i);
        if child.is_null() {
            continue;
        }

        let game_object = il2cpp::call_obj0(classes.component_get_gameobject, child);
        if game_object.is_null() {
            continue;
        }

        let name = il2cpp::read_string(il2cpp::call_obj0(classes.object_get_name, game_object));
        if names.len() < 24 {
            names.push(name.clone());
        }

        if HEAD_PARTS.contains(&name.as_str()) || (hide_body && BODY_PARTS.contains(&name.as_str())) {
            il2cpp::call_void_bool(classes.gameobject_set_active, game_object, false);
            hidden.push(game_object as usize);
        }
    }

    // One line per runner change. If nothing matched, the child names tell us
    // what this model build actually calls them.
    logging::info(&format!(
        "POV: owner children ({count}) = [{}] -> hid {}",
        names.join(", "),
        hidden.len()
    ));

    match HIDDEN_HEADS.lock() {
        Ok(mut guard) => *guard = hidden,
        Err(poisoned) => *poisoned.into_inner() = hidden,
    }
}

/// Re-shows anything `hide_head_parts` hid. Safe to call repeatedly: the
/// native-alive check guards against objects destroyed by a scene change.
unsafe fn restore_head_parts(classes: PovClasses) {
    let hidden = match HIDDEN_HEADS.lock() {
        Ok(mut guard) => std::mem::take(&mut *guard),
        Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
    };

    for ptr in hidden {
        let game_object = ptr as il2cpp::Obj;
        if game_object.is_null() || !il2cpp::call_bool_0(classes.object_is_native_alive, game_object) {
            continue;
        }
        il2cpp::call_void_bool(classes.gameobject_set_active, game_object, true);
    }

    HIDDEN_FOR.store(-1, Ordering::Relaxed);
}

// ------------------------------------------------------------- race tick ----

/// Called from the race-view tick on the Unity main thread. Recomputes the
/// desired pose for the selected runner, or clears it.
pub unsafe fn on_race_tick(view: *mut c_void) {
    if !enabled() || !is_available() {
        return;
    }

    let selected = crate::race::selected_index();
    if view.is_null() || selected < 0 {
        clear_pose();
        return;
    }

    // Guard against a stale selection: the stored index can outlive the race it
    // came from, and `GetModelController` indexes a managed array. Method pointers
    // called directly have no exception handling, so an out-of-range index would
    // be a hard crash rather than a managed exception.
    let selection_is_valid = crate::race::snapshot()
        .runners
        .iter()
        .any(|runner| runner.index == selected);
    if !selection_is_valid {
        clear_pose();
        return;
    }

    let Some(classes) = CLASSES.get().copied() else {
        return;
    };

    let model = il2cpp::call_obj1_i32(classes.get_model_controller, view, selected);
    if model.is_null() {
        // Not every frame has models ready (gate-in, loading).
        clear_pose();
        return;
    }

    let tuning = tuning();

    // Hide the head once per runner change, not every frame. Reset the rotation
    // stabiliser too, so switching runners snaps to the new head instead of
    // slowly rotating across the track.
    if HIDDEN_FOR.load(Ordering::Relaxed) != selected {
        restore_head_parts(classes);
        if tuning.hide_head {
            hide_head_parts(classes, model, tuning.hide_body);
        }
        HIDDEN_FOR.store(selected, Ordering::Relaxed);
        reset_rotation_stabilizer();
    }

    let Some((base, target_rot)) = read_attach_pose(classes, model, tuning.attach) else {
        if should_log(&LAST_POSE_LOG_MS) {
            logging::warn(&format!(
                "POV: attach point {:?} unavailable for runner {selected}; no pose",
                tuning.attach
            ));
        }
        // No usable pose, so do not leave the model hidden.
        restore_head_parts(classes);
        clear_pose();
        return;
    };

    // TLG's stabiliser: never rotate more than the configured step from the
    // previous frame, which removes head bob and animation jitter.
    let rot = stabilize_rotation(target_rot, tuning.max_rot_step);

    // Forward is the attach point's local +Z; the vertical nudge is world space.
    let forward = rot.rotate_vec(Vec3::new(0.0, 0.0, tuning.forward));
    let pose = Pose {
        pos: Vec3::new(
            base.x + forward.x,
            base.y + forward.y + tuning.up,
            base.z + forward.z,
        ),
        rot,
    };

    match POSE.lock() {
        Ok(mut guard) => *guard = Some(pose),
        Err(poisoned) => *poisoned.into_inner() = Some(pose),
    }

    if should_log(&LAST_POSE_LOG_MS) {
        logging::info(&format!(
            "POV pose runner={} attach={:?} pos=({:.3}, {:.3}, {:.3})",
            selected, tuning.attach, pose.pos.x, pose.pos.y, pose.pos.z
        ));
    }
}

/// Reads the camera mount: either the midpoint of the two eye transforms, or a
/// single named attach point (chest / neck / head).
unsafe fn read_attach_pose(
    classes: PovClasses,
    model: il2cpp::Obj,
    attach: AttachPoint,
) -> Option<(Vec3, Quat)> {
    let get_position: GetVec3Fn = std::mem::transmute(classes.transform_get_position);
    let get_rotation: GetQuatFn = std::mem::transmute(classes.transform_get_rotation);

    if attach == AttachPoint::Eyes {
        let left =
            il2cpp::call_obj1_i32(classes.get_prefab_attach_transform, model, ATTACH_EYE_LEFT);
        let right =
            il2cpp::call_obj1_i32(classes.get_prefab_attach_transform, model, ATTACH_EYE_RIGHT);
        if left.is_null() || right.is_null() {
            return None;
        }

        let mut pos_left = Vec3::default();
        let mut pos_right = Vec3::default();
        let mut rot_left = Quat::default();
        let mut rot_right = Quat::default();
        get_position(left, &mut pos_left);
        get_position(right, &mut pos_right);
        get_rotation(left, &mut rot_left);
        get_rotation(right, &mut rot_right);

        if !pos_left.is_finite()
            || !pos_right.is_finite()
            || !rot_left.is_finite()
            || !rot_right.is_finite()
        {
            return None;
        }

        return Some((
            Vec3::midpoint(pos_left, pos_right),
            Quat::nlerp(rot_left, rot_right, 0.5),
        ));
    }

    let transform = il2cpp::call_obj1_i32(classes.get_prefab_attach_transform, model, attach.value());
    if transform.is_null() {
        return None;
    }

    let mut pos = Vec3::default();
    let mut rot = Quat::default();
    get_position(transform, &mut pos);
    get_rotation(transform, &mut rot);
    if !pos.is_finite() || !rot.is_finite() {
        return None;
    }

    Some((pos, rot))
}

// ---------------------------------------------------------------- detour ----

type OnPreCullFn = unsafe extern "C" fn(this: *mut c_void);

unsafe extern "C" fn course_on_pre_cull(this: *mut c_void) {
    apply_pov(this);

    let trampoline = ON_PRE_CULL_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline != 0 {
        let original: OnPreCullFn = std::mem::transmute(trampoline);
        original(this);
    }
}

/// Runs after every LateUpdate, immediately before culling. Writing here beats
/// the game's camera update for the current frame, and this is also the point
/// where cut-in suppression and head restoration are kept in sync.
/// Clip plane values last observed from the game itself, as `f32` bits (0 =
/// unknown), plus whether we currently have them overridden.
static GAME_NEAR_CLIP_BITS: AtomicU32 = AtomicU32::new(0);
static GAME_FAR_CLIP_BITS: AtomicU32 = AtomicU32::new(0);
static CLIPS_OVERRIDDEN: AtomicBool = AtomicBool::new(false);

fn apply_pov(this: *mut c_void) {
    reload_tuning();

    let Some(classes) = CLASSES.get().copied() else {
        return;
    };

    unsafe {
        // The manager comes from the component we are running on, so we never
        // depend on singleton resolution.
        let manager = il2cpp::field_object(this, classes.course_manager_field as il2cpp::Field);
        if manager.is_null() {
            log_apply("no camera manager on CourseCameraController");
            return;
        }

        let tuning = tuning();
        let enabled = enabled();
        sync_cutin_suppression(classes, manager, enabled && tuning.suppress_cutins);

        let pose = match POSE.lock() {
            Ok(guard) => *guard,
            Err(poisoned) => *poisoned.into_inner(),
        };

        // Only drive the camera when the race camera is the one being rendered,
        // so gate-in and the goal camera keep working.
        let current = il2cpp::call_obj0(classes.get_current_camera, manager);
        let main = il2cpp::call_obj0(classes.get_main_camera, manager);
        let is_race_camera = !current.is_null() && current == main;

        // Anything that means "not writing this frame" lands here. We hand the
        // head and the clip plane back, so other cameras render an intact runner
        // and the game keeps its own depth range.
        if !(enabled && is_race_camera) || pose.is_none() {
            restore_head_parts(classes);
            release_clips(classes, current);

            if enabled && !is_race_camera {
                log_apply(&format!(
                    "skipped: current={current:p} main={main:p} (not the race camera)"
                ));
            }
            return;
        }

        let pose = match pose {
            Some(pose) => pose,
            None => return, // unreachable: guarded above
        };

        let get_transform: GetTransformFn = std::mem::transmute(classes.component_get_transform);
        let transform = get_transform(current);
        if transform.is_null() {
            log_apply("skipped: camera has no transform");
            return;
        }

        let set_position: SetVec3Fn = std::mem::transmute(classes.transform_set_position);
        let set_rotation: SetQuatFn = std::mem::transmute(classes.transform_set_rotation);

        let mut pos = pose.pos;
        let mut rot = pose.rot;
        set_position(transform, &mut pos);
        set_rotation(transform, &mut rot);

        // Both of these only need to hold while POV is active: the game recomputes
        // FOV and clip planes every frame, so releasing them hands control back.
        let vertical_fov = horizontal_to_vertical(tuning.fov, camera_aspect(classes, current));
        il2cpp::call_void_f32(classes.camera_set_fov, current, vertical_fov);
        apply_clips(classes, current, tuning.near_clip, tuning.far_clip);

        log_apply(&format!(
            "wrote pose to race camera transform={transform:p} fov={:.1}h/{:.1}v near={:.3} far={:.0}",
            tuning.fov, vertical_fov, tuning.near_clip, tuning.far_clip
        ));
    }
}

/// Converts a horizontal field of view to Unity's vertical one.
///
/// The reference implementations expose FOV the Quake way (horizontal), while
/// `Camera.fieldOfView` is vertical. At 16:9, 90 horizontal is ~59 vertical —
/// which is why a literal `fieldOfView = 90` looks like a fisheye.
fn horizontal_to_vertical(horizontal_degrees: f32, aspect: f32) -> f32 {
    if !(horizontal_degrees > 0.0) || !(aspect > 0.0) || !aspect.is_finite() {
        return horizontal_degrees;
    }

    let half = (horizontal_degrees * 0.5).to_radians();
    (2.0 * (half.tan() / aspect).atan()).to_degrees()
}

unsafe fn camera_aspect(classes: PovClasses, camera: il2cpp::Obj) -> f32 {
    if camera.is_null() || classes.camera_get_aspect == 0 {
        return 16.0 / 9.0;
    }
    let get: unsafe extern "C" fn(*mut c_void) -> f32 =
        std::mem::transmute(classes.camera_get_aspect);
    let aspect = get(camera);
    if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        16.0 / 9.0
    }
}

/// Sets the clip planes. The game derives the near clip from the camera FOV and
/// the far clip from the camera transform, and both feed back into what the game
/// reads next frame — the near one cuts away nearby runners, the far one clips the
/// skydome. `<= 0` disables that override.
unsafe fn apply_clips(classes: PovClasses, camera: il2cpp::Obj, near: f32, far: f32) {
    if near <= 0.0 && far <= 0.0 {
        return;
    }
    if near > 0.0 {
        il2cpp::call_void_f32(classes.camera_set_near_clip, camera, near);
    }
    if far > 0.0 {
        il2cpp::call_void_f32(classes.camera_set_far_clip, camera, far);
    }
    CLIPS_OVERRIDDEN.store(true, Ordering::Relaxed);
}

/// Restores the last clip values we saw the game use, and starts tracking them
/// again. Only writes when we had actually overridden them, so we never fight the
/// game for ownership of the values.
unsafe fn release_clips(classes: PovClasses, camera: il2cpp::Obj) {
    if camera.is_null() {
        return;
    }

    if CLIPS_OVERRIDDEN.swap(false, Ordering::Relaxed) {
        let near = GAME_NEAR_CLIP_BITS.load(Ordering::Relaxed);
        if near != 0 {
            il2cpp::call_void_f32(classes.camera_set_near_clip, camera, f32::from_bits(near));
        }
        let far = GAME_FAR_CLIP_BITS.load(Ordering::Relaxed);
        if far != 0 {
            il2cpp::call_void_f32(classes.camera_set_far_clip, camera, f32::from_bits(far));
        }
        return;
    }

    // Not overriding: sample what the game is using so we can put it back later.
    record_clip(classes.camera_get_near_clip, camera, &GAME_NEAR_CLIP_BITS);
    record_clip(classes.camera_get_far_clip, camera, &GAME_FAR_CLIP_BITS);
}

unsafe fn record_clip(getter: usize, camera: il2cpp::Obj, slot: &AtomicU32) {
    if getter == 0 {
        return;
    }
    let get: unsafe extern "C" fn(*mut c_void) -> f32 = std::mem::transmute(getter);
    let value = get(camera);
    if value.is_finite() && value > 0.0 {
        slot.store(value.to_bits(), Ordering::Relaxed);
    }
}

/// Uses the game's own cut-in gate (`EventCamera._unPlayable`) rather than hooking
/// `PlayEventCamera`, which Hachimi already owns. Only transitions are written, so
/// the flag can always be handed back when POV is switched off.
unsafe fn sync_cutin_suppression(classes: PovClasses, manager: il2cpp::Obj, suppress: bool) {
    if CUTINS_SUPPRESSED.load(Ordering::Relaxed) == suppress {
        return;
    }

    let event_camera = il2cpp::call_obj0(classes.get_event_camera, manager);
    if event_camera.is_null() {
        return;
    }

    il2cpp::set_field_bool(
        event_camera,
        classes.event_camera_unplayable_field as il2cpp::Field,
        suppress,
    );
    CUTINS_SUPPRESSED.store(suppress, Ordering::Relaxed);

    logging::info(if suppress {
        "POV: cut-ins suppressed"
    } else {
        "POV: cut-ins restored"
    });
}

// ------------------------------------------------------------- culling ------
//
// `RaceViewBase.Culling(curCamera, targetHorseIndex, culling, cullingSqrMagnitude)`
// is the game's per-model visibility pass. Culling a model pauses its CySpring
// hair/cloth simulation (`ModelController.SetVisible` -> `CySpringController.
// IsSkipUpdatePre`), and it runs in the LateUpdate phase using the game's camera
// position -- before we move the camera in `OnPreCull`. So its notion of "on
// screen" does not match what you are actually looking at, and runners beside the
// selected one freeze mid-gallop.
//
// `CullingType` is `None, Default`, so `None` (0) is the game's own "do not cull".

static CULLING_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

type CullingFn = unsafe extern "C" fn(*mut c_void, *mut c_void, i32, i32, f32);

unsafe extern "C" fn race_view_culling(
    this: *mut c_void,
    camera: *mut c_void,
    target_horse_index: i32,
    culling_type: i32,
    culling_sqr_magnitude: f32,
) {
    let trampoline = CULLING_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline == 0 {
        return;
    }
    let original: CullingFn = std::mem::transmute(trampoline);

    let mode = if enabled() {
        tuning().culling
    } else {
        CullingMode::Default
    };

    match mode {
        CullingMode::Skip => {
            if should_log(&LAST_CULLING_LOG_MS) {
                logging::info("POV: culling skipped");
            }
        }
        CullingMode::None => {
            if should_log(&LAST_CULLING_LOG_MS) {
                logging::info("POV: culling forced to None");
            }
            original(this, camera, target_horse_index, 0, culling_sqr_magnitude);
        }
        CullingMode::Default => {
            original(this, camera, target_horse_index, culling_type, culling_sqr_magnitude);
        }
    }
}

fn install_culling_hook() {
    unsafe {
        let Some(api) = api::get() else {
            return;
        };
        let image = il2cpp::assembly_image("umamusume.dll");
        if image.is_null() {
            return;
        }
        let class = il2cpp::class(image, "Gallop", "RaceViewBase");
        if class.is_null() {
            logging::warn("POV: RaceViewBase not found; culling override unavailable");
            return;
        }

        let target = il2cpp::method_addr(class, "Culling", 4);
        if target == 0 {
            logging::warn("POV: RaceViewBase.Culling not found; culling override unavailable");
            return;
        }

        let hachimi = (api.hachimi_instance)();
        let interceptor = (api.hachimi_get_interceptor)(hachimi);
        if interceptor.is_null() {
            return;
        }

        let trampoline =
            (api.interceptor_hook)(interceptor, target as *mut c_void, race_view_culling as *mut c_void);
        if trampoline.is_null() {
            logging::warn("POV: hook failed for RaceViewBase.Culling");
            return;
        }

        CULLING_TRAMPOLINE.store(trampoline as usize, Ordering::Release);
        logging::info("POV: hooked RaceViewBase.Culling");
    }
}

fn log_apply(message: &str) {
    if should_log(&LAST_APPLY_LOG_MS) {
        logging::info(&format!("POV apply: {message}"));
    }
}
