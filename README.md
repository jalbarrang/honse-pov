# honse_pov

Experimental **Hachimi Edge plugin** for a "select a runner, see from its point of view" race camera.

This is a sibling project to [Hachimi-Edge](../Hachimi-Edge) and is loaded by it as a plugin. It does **not** modify or depend on Hachimi's source.

## Status

| Milestone | Scope | State |
|---|---|---|
| **1** | Resolve race classes, hook a main-thread race tick, list runners, let the user pick one | **verified in-game** |
| 2 | Drive an actual POV camera from the selected runner (eyes / head node) | not started |

M1 was verified on `Hachimi v0.31.2-83dd99f` (Global) in a Champions Meeting race: both
race-view hooks installed, the runner list populated, and runner selection round-tripped
through `race::selected_index()`. Still unverified: the story-race hook firing at runtime
(it installs cleanly but has not been exercised in a race since), and the two field
mappings that M2 depends on — `HorseData.get_IsUser()` marking the correct row as `YOU`,
and `HorseData.get_Popularity()` matching the game's own entry table.

Milestone 1 is deliberately read-only: it never touches the game camera. `race::selected_index()` is the single input milestone 2 consumes.

## Why a plugin and not a core patch

Hachimi Edge already ships a free camera with a race first-person mode (`src/windows/free_camera.rs` in the core repo, `FreeCameraMode::{Free, FirstPerson, SelfieStick}`, plus `RaceViewBase.LateUpdateView` hooking and `free_camera.race_target_index`).

This repo exists to iterate on POV variants **without rebuilding the core**, and to keep the experiment isolated if it turns out badly.

The plugin only has access to the public plugin C API, so it cannot read Hachimi's internal free-camera state or config. That is intentional.

## Requirements

- Windows x86_64 (only platform supported by this milestone)
- Hachimi Edge with **plugin API v3 or newer**
- Rust (edition 2021) with the `x86_64-pc-windows-msvc` target
- The game's IL2CPP dump, if you need to re-check class/member names (see *Game coupling* below)

## Build

```bash
cargo build --release
# -> target/release/honse_pov.dll
```

## Install

1. Copy `honse_pov.dll` into the game's `hachimi` folder.
2. Edit `hachimi/config.json` and add it to `load_libraries`:

```json
{
  "load_libraries": [
    "hachimi\\honse_pov.dll"
  ]
}
```

3. Start the game.

To unload, remove the entry from `load_libraries` and restart.

## Usage

1. Launch the game and start a race (any normal race uses `RaceViewReplay`).
2. Open the Hachimi menu and click **Open Race POV window** to get the standalone picker window.
3. The window lists every runner in the current race:

   `select | gate | name | popularity | tag`

   `tag` shows `YOU` for the player's own horse. Clicking a row stores that runner index as the selection; the header shows the current selection and the player horse index reported by the game.

## Verify the install

Hachimi writes `hachimi.log` next to the game executable when `enable_file_logging` is
`true` in `config.json` (recommended while testing this plugin). A successful load looks
like:

```
Loaded library: hachimi\honse_pov.dll
Initializing plugin: hachimi\honse_pov.dll
honse_pov: honse_pov 0.1.0 loaded (host plugin API v3)
honse_pov: race tracking ready
honse_pov: hooked Gallop.RaceViewReplay.UpdateView
honse_pov: hooked Gallop.RaceViewStoryReplay.UpdateView
honse_pov: race tracking installed
```

Selection then logs `honse_pov: selected runner index -> 7`.

A failed load tells you exactly what to fix, e.g.:

```
honse_pov: host is missing required API symbols: gui_ui_grid
honse_pov: could not resolve method HorseData.get_Popularity
honse_pov: could not resolve class Gallop.RaceViewReplay
```

The plugin never aborts the process on failure; the window stays open and reports the
problem.

To uninstall, delete `honse_pov.dll` and remove its entry from `load_libraries`.

## Host quirk: `hachimi_register_on_game_initialized` never fires

Hachimi Edge v0.31.2 (`Hachimi::on_hooking_finished`) dispatches the game-initialized
callbacks **before** it runs plugin init:

```rust
GameSystem::on_game_initialized();               // fires plugin_init_callbacks
...
for plugin in self.plugins { plugin.init(); }    // a plugin can only register here
```

The second dispatch site (`GameSystem::InitializeGame_MoveNext`) is only installed when
`ui_scale != 1.0`, so with the default config the callback never runs at all.

This plugin therefore resolves classes and installs its hooks directly in
`hachimi_init_v3`. That is safe because plugin init runs immediately after
`il2cpp::hook::init()`, i.e. once the IL2CPP runtime is up and `umamusume.dll` is loaded —
the same phase in which Hachimi resolves and hooks its own race classes.

## Host quirk: plugin grids wrapped mid-word and did not fill the window

`gui_ui_grid` is built on `egui::Grid`, which sizes each column to its content. Two
defaults combine badly with plugin windows:

- egui's grid measures cells assuming they do not wrap, but plugin windows set
  `TextWrapMode::Wrap` so paragraph labels wrap. Inside a grid that collapses every
  column's measurement to `interact_size.x` (~34-40px), so cell text wraps mid-word —
  `select` renders as two lines and names break after ~8 characters.
- `Grid`'s minimum column width also defaults to `interact_size.x`, so even without the
  wrapping the columns stay one word-fragment wide and the grid sits in a corner of a
  wide window, leaving most of it empty.

Both are fixed in the host. `gui_ui_grid` now shares the available width across the
columns (`min_col_width`) and forces `TextWrapMode::Extend` while the cells are measured,
restoring the no-wrap assumption the grid relies on. On a host without those fixes this
plugin still loads and works, but the runner grid looks cramped.

## Expected log output

With Hachimi's log level at Info or lower you should see, in order:

```
honse_pov 0.1.0 loaded (host plugin API v3)
game initialized; installing race tracking
hooked Gallop.RaceViewReplay.UpdateView
race tracking installed
selected runner index -> 7
```

If class resolution fails you get a specific error instead, e.g.:

```
could not resolve method HorseData.get_Popularity
```

Failure is non-fatal: the window stays open and reports that classes were not resolved.

## How it works

```
hachimi_init_v3(get_api, version)
  |-- resolve API symbols by name (no Vtable dependency)
  |-- register GUI (menu item + window)
  |-- register on_game_initialized
        |-- resolve Gallop classes / fields / methods
        |-- hook RaceViewReplay.UpdateView(float)   <-- main-thread tick
        '-- cache descriptors

RaceViewReplay.UpdateView(this, dt)
  '-- every 100 ms: refresh()
        RaceManager.get_Instance()
          -> _horseManager
          -> GetHorseRaceInfos()
          -> [ HorseRaceInfo.get_HorseData()
                 -> HorseData.get_charaName / get_GateNo / get_Popularity / get_IsUser ]
```

### Deliberate design choices

- **`RaceViewReplay.UpdateView(float)` as the tick.** It runs once per frame on the Unity main thread during a race, and Hachimi Edge does **not** hook it. Hooking a target the host already hooks (e.g. `RaceViewBase.LateUpdateView`, `RaceCameraManager.AlterLateUpdate`) would mean sharing a MinHook target and re-enabling an already-enabled hook, which is fragile.
- **Refresh throttled to 100 ms.** Avoids re-allocating 18 name strings every frame; still feels live.
- **Snapshot behind a `Mutex`, cloned before rendering.** GUI callbacks run on the render thread, so no IL2CPP calls happen there and no lock is held across a GUI call.
- **No `unwrap` in callback paths.** Unwinding out of an `extern "C"` callback aborts the process.
- **Name-resolved API.** Avoids recompiling when the host appends symbols to its table.

## Game coupling

Everything resolved by name can break on a game update. The names currently used:

| Class | Member |
|---|---|
| `Gallop.RaceManager` | `get_Instance`, field `_horseManager` |
| `Gallop.RaceHorseManagerBase` | `GetHorseRaceInfos()`, `GetPlayerHorseIndex()` |
| `Gallop.HorseRaceInfo` | `get_HorseData()` |
| `Gallop.HorseData` | `get_charaName()`, `get_GateNo()`, `get_Popularity()`, `get_IsUser()` |
| `Gallop.RaceViewReplay` | `UpdateView(float)` |

Array length/data offsets (`24` / `32`) come from the standard IL2CPP layout for Unity 2020+ and are isolated in `src/il2cpp.rs`.

If a name changes, the plugin logs which one failed and the runner list stops populating; nothing crashes.

## Milestone 2 notes

### Decisions

| Question | Decision |
|---|---|
| Activation | **Explicit `Enable POV` toggle + selected runner.** Selection alone never touches the camera; toggling off must fully restore the game camera. |
| Look control | **Fixed forward from the eyes.** No look input in the first cut — the plugin API exposes no key/mouse events, so free-look would require polling `UnityEngine.Input` through il2cpp. Add later, not now. |

### Why the camera has to be forced

The game owns the race camera; there is no "set camera to runner POV" entry point. Two
separate overrides are needed:

1. **Suppress the game's camera decisions** — `RaceCameraManager` keeps switching cameras
   (course camera events, jikkyou/commentary cameras, skill cut-ins, gate camera). Core
   does this by hooking `RaceCameraManager.ChangeCameraMode` / `PlayEventCamera` and
   returning early while its free camera is active.
2. **Make the pose stick** — the game's own update rewrites the camera transform. Core
   wraps the original `RaceCameraManager.AlterLateUpdate` in
   `Transform::set_update_race_camera(active)`, so the game's camera logic still runs but
   the Transform hooks skip applying its result.

### Phase matters

`RaceViewReplay` overrides `UpdateView(float)` but **not** `LateUpdateView()`. Our tick is
in the **Update** phase; the camera manager writes in **LateUpdate**, i.e. after us, so a
camera write from the existing hook would be clobbered. Options:

- **Preferred:** keep the `UpdateView` tick for state, and set the race-camera override
  flag there. Update runs before LateUpdate, so the flag is still set when the game
  applies transforms — this may avoid hooking `LateUpdateView` entirely. Needs testing.
- **Fallback:** hook `RaceViewBase.LateUpdateView` (base-only, no virtual dispatch
  issues). Problem: Hachimi Edge already hooks it, and re-enabling an already-enabled
  MinHook target is the fragile case this plugin avoids everywhere else.

Also unresolved: clean restore on race end / pause / goal, and what happens if the
selected runner is not present in the current race.

### Upstream bug: core's race first-person never runs

`Gallop.RaceModelController.GetPrefabAttachTransform` takes **one enum argument**:

```
GetPrefabAttachTransform(Gallop.CoursePrefabParam.CharaAttachTransform type) -> UnityEngine.Transform
```

Hachimi Edge resolves it in `src/il2cpp/hook/umamusume/RaceModelController.rs` with
`args_count = 2` (a `part`/`name` pair), so the lookup fails:

```
[WARN] hachimi::il2cpp::symbols: get_method_addr: GetPrefabAttachTransform = NULL
```

`GET_PREFAB_ATTACH_TRANSFORM_ADDR` therefore stays 0 and
`RaceViewBase.LateUpdateView`'s race first-person / selfie-stick path is dead code. It is
latent only because `free_camera.enabled` defaults to false — with the free camera on and
`mode` set to `FirstPerson`, that path would call address 0.

This plugin resolves the same method with `args_count = 1` and passes the enum value
directly. Enum ordering:

```text
Root=0 M_Face=1 Waist=2 Spine=3 Chest=4 Neck=5 Head=6
Eye_L=7 Eye_R=8 Toe_L=9 Toe_R=10 Nose=11
```

### Cross-reference: Trainers' Legend G

[Trainers' Legend G](https://github.com/MinamiChiwa/Trainers-Legend-G) (C++, a
`umamusume-localify` fork) implements the same race first-person feature. Comparing the two
was useful because it independently confirms some choices and corrects others.

**Confirmed — we had these right:**

- `GetPrefabAttachTransform(model, 0x7)` / `0x8` for the eyes, midpoint of the two
  positions, `slerp(rotL, rotR, 0.5)` for the rotation.
- Hiding exactly `M_Hair` and `M_Face` child objects of the model owner, restored by a
  native-alive-checked `SetActive(true)`.
- Blocking cut-ins while the POV is active. TLG hooks `RaceCameraManager.PlayEventCamera`
  and returns false; we cannot (Hachimi owns that target) so we use the game's own
  `EventCamera._unPlayable` flag instead.

**Ported from TLG:**

- **Offsets.** TLG's `liveFirstPersonOffset` is `(0, 0.075, 0.015)` — a small forward
  nudge and most of the correction *upward*. Hachimi's own `live_first_person_offset`
  default is the same pair. We had been guessing a forward-only 0.05; we now use
  0.015 forward / 0.075 up.
- **Rotation stabiliser.** TLG's `SmoothQuaternion(q, lastRot, 0.01f)` clamps the applied
  orientation to at most 0.01 rad from the previous frame's, filtering animated head bob
  and jitter. Ported as `MAX_ROT_STEP_RAD`. It resets on runner change and on POV toggle
  so switching runners snaps rather than panning across the track.

**Not ported, deliberately:**

- TLG still resolves `GetPrefabAttachTransform` with `args_count = 2` (a `part`/`name`
  pair), the same stale signature Hachimi uses. The shipped game method takes a single
  `CharaAttachTransform` enum argument, so TLG's path is broken on current builds too.
- TLG overrides the FOV *getter* (`Camera.get_fieldOfView`). Hachimi already hooks that
  function, so a plugin cannot; writing FOV from `OnPreCull` achieves the same result.
- TLG's gallop bob (`raceFirstShakeStrength`, a pitch offset driven by vertical velocity)
  and their `cutin_first_person` mode. Both are optional extras; the bob is a one-constant
  addition if we ever want it.
- TLG rewrites the value passed into `Transform.set_position_Injected` inside its
  `RaceCameraManager.AlterLateUpdate` hook. That is the same mechanism Hachimi uses, and
  MinHook allows only one hook per target, so it is unavailable to a plugin either way.

### Available primitives

The dump work behind this project showed which primitives are available for a real POV camera:

- Eye/head attachment: `RaceModelController.GetPrefabAttachTransform(controller, part, "")` — `0x7` = left eye, `0x8` = right eye (this is what the core free camera averages for race first-person), and `RaceViewBase.GetModelController(index)` to reach a specific runner's model.
- Per-character camera params already exist in the data (`RaceEpisodeCameraEvent.CharacterEyeCamera*`, `CharacterParameter.Node { Head, Neck, Chest, Spine, Waist }`).
- Camera targeting is data-driven: `CourseCameraTargetType { Player, IndexDirect, OrderDirect, Popularity, JikkyouOrder, ... }` with `RaceCameraEventBase.CameraParameter.targetHorseIndex`.
- Custom camera modes can be registered via `MasterRacePlayerCamera.RacePlayerCamera { Id, Priority, PrefabName, Category }` and `PlayerEventCameraSetting`.

The natural next step is to hook `RaceViewReplay.UpdateView` (already hooked here) or `RaceViewBase.LateUpdateView`, resolve the selected runner's model controller, read the eye transforms, and write the resulting pose to the race camera — i.e. the same shape as the core free camera, but standalone.

## Layout

```
src/
  lib.rs       entry point (hachimi_init_v3), lifecycle wiring
  api.rs       name-resolved bindings to the plugin C API
  il2cpp.rs    IL2CPP helpers (classes, fields, arrays, strings, invocation)
  race.rs      class resolution, UpdateView hook, runner snapshot
  ui.rs        standalone runner picker window
  logging.rs   thin wrapper over the host logger
```
