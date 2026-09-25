# M1 test checklist

Milestone 1 is **read-only**. Nothing it does should change what you see on track.
The point of this pass is to confirm the plugin loads, finds the race classes, and
enumerates the field correctly.

## Already installed

- `hachimi\honse_pov.dll` — the plugin
- `hachimi\config.json` — `load_libraries` includes `hachimi\\honse_pov.dll`,
  and `enable_file_logging` is `true` so you can see the log
- `hachimi\config.json.bak` — the pre-install config

Log file: `UmamusumePrettyDerby\hachimi.log` (next to the exe). **It is truncated on
every launch**, so check it after starting the game, before relaunching.

Menu hotkey: **Right Arrow** (`menu_open_key: 39`, changeable in the Hachimi GUI).

---

## 1. Does the plugin load and hook? (same second, before the title screen)

Launch the game, then open `hachimi.log`. All of this should appear together, in one
batch, right after the `Hachimi <version>` banner:

```
Loaded library: hachimi\honse_pov.dll
Initializing plugin: hachimi\honse_pov.dll
honse_pov: honse_pov 0.1.0 loaded (host plugin API v3)
honse_pov: hooked Gallop.RaceViewReplay.UpdateView
honse_pov: hooked Gallop.RaceViewStoryReplay.UpdateView
honse_pov: race tracking installed
honse_pov: race tracking ready
```

There is **no** `game initialized; installing race tracking` line — see the host-quirk note
in `README.md`. Class resolution and hook installation deliberately happen during plugin
init on this host.

| What you see | Meaning |
|---|---|
| all lines above | good |
| `host is missing required API symbols: ...` | host build too old for this plugin — send me the line |
| `honse_pov requires plugin API v3 or newer, host reports v2` | same |
| `could not resolve class Gallop.X` | that class moved/renamed in your game build |
| `could not resolve method X` | that method moved/renamed |
| `Gallop.RaceViewStoryReplay not found; ...` | warning only — normal races still work |
| `failed to hook any race view tick` | the runner list will stay empty; send the log |
| `interceptor_hook failed for ...` | hooking was refused; send the log |
| nothing at all mentioning `honse_pov` | plugin never loaded — config/path problem, not a plugin bug |

Also copy the `Hachimi <version>` line at the top of the log — it tells me exactly which
host build you're on.

## 2. Does the window open?

Open the Hachimi menu (Right Arrow) and click the menu item **Open Race POV window**.

Expected: a standalone window titled `Race POV (experimental)`. From the home screen it
shows `No active race detected.` plus a hint line, with a fixed bottom bar containing
**Clear selection** and **Close**.

Failures:

- **menu item missing** → GUI registration failed even though the plugin
  loaded. Send the log.
- **`Waiting for game initialization (classes not resolved yet).`** → step 1 did not
  complete; the log tells you why.
- **`Race tracking failed to install. ...`** → step 1 tells you why.

- Clicking **Open** twice should not stack duplicate windows (the host replaces the
  window with the same id).
- **Close** should close it; the menu item should reopen it.
- The window should be draggable/resizable like other Hachimi windows.

## 3. Does the runner list populate? (main test)

**Test a normal race first** — single mode, daily race, or team stadium.

Start the race, and while it plays (gate-in or mid-race), open the window. Expected:

```
Race POV (experimental)
────────────────────────────────
Selected: none
N runners | player horse index: X
────────────────────────────────
select | #gate | name | popularity | tag
select | #1 | <name> | <pop> | YOU      <- exactly one row
select | #2 | <name> | <pop> |
...
select | gate | name | popularity | tag
```

Check:

- [ ] `N` equals the number of runners actually in the race (9 / 12 / 15 / 18)
- [ ] gate numbers are `1..N`, no gaps or duplicates
- [ ] names match the runners on screen
- [ ] **exactly one** row is tagged `YOU`, and it's your uma
- [ ] `player horse index` is a plausible 0-based index (usually 0)
- [ ] the header selection line is green

Then **repeat with a story/episode race** — that's what exercises the second hook.

If the list stays empty while racing, note **which race type** and send the log.

## 4. Does selection work?

- [ ] click **select** on a row → that row becomes `* selected`, the header becomes
      `Selected: gate N - <name> (#idx)`, and `hachimi.log` gets
      `selected runner index -> N`
- [ ] click a different row → header follows, new log line
- [ ] **Clear selection** → header returns to `Selected: none`, log gets
      `selected runner index -> -1`

## 5. Milestone 1 assertions (must NOT happen)

- [ ] **the race camera never changes** — no camera jump, no lost cut-ins, no POV.
      Selection must be purely cosmetic in this build.
- [ ] no stutter when the list refreshes (it ticks 10x/sec)
- [ ] no crash on: race start, race end, leaving to home, pausing, opening the menu
      mid-race, opening the window mid-race
- [ ] after the race ends the list goes back to `No active race detected.`, and
      starting another race repopulates it

## 6. Known limitations (not bugs)

- **No camera effect.** By design for M1.
- The list refreshes 10x/sec, so gate/name/popularity are near-live but not
  frame-perfect.
- The list can be empty for the first frames of a race while models load.
- Selection is in-memory only; it resets to `none` on game restart.
- `popularity` is the popularity **rank** (1 = race favourite), as stored on
  `HorseData.Popularity` — confirm it matches what the UI shows in-game.

## 7. If something fails, send me

1. `hachimi.log` (at minimum every line containing `honse_pov` or `could not` / `failed`)
2. which step number failed and what you saw instead
3. the race type you were in (single mode / daily / story / legend / team stadium)
4. a screenshot of the window if it's a layout problem

## Milestone 2: POV camera

M2 adds an **Enable POV** checkbox to the picker window.
Selection alone still does nothing — the camera only moves while the checkbox is on.

### Expected log lines at startup

```
honse_pov: POV installed (hooked Gallop.CourseCameraController.OnPreCull)
```

or, if something is missing:

```
honse_pov: POV unavailable: could not resolve <name>
```

The reason is also shown in red in the picker UI, so you do not need the log to see it.

### Procedure

1. Start a race.
2. Open the picker window. Confirm the checkbox is present and the status reads `POV off`.
3. Click **select** on a runner.
4. Tick **Enable POV**.

### Expected result

- [ ] the camera snaps to that runner's eye level and moves with them
- [ ] unticking the checkbox hands the camera straight back to the game
- [ ] the status line reads `POV on - eye (x, y, z)`
- [ ] gate-in, skill cut-ins and the goal camera still use the game's cameras
      (the override only applies while the race camera is the active one)

### Diagnostic lines (once per second while POV is on)

```
POV pose runner=7 eye=(123.456, 1.234, 789.012)
POV apply: wrote pose to race camera transform=0x...
```

| Log line | Meaning |
|---|---|
| `POV pose ...` | the eye transforms were found and the pose computed |
| `POV apply: wrote pose ...` | the camera was actually moved |
| `POV apply: skipped: current=0x.. main=0x.. (not the race camera)` | the game is on a cut-in/goal camera, so the override stepped aside |
| `POV apply: no camera manager on CourseCameraController` | `_cameraManager` was null — send me the line |
| `POV: eye attach transforms missing for runner N` | the model has no eye attach points (not loaded, or a non-standard model) |
| `POV on - waiting for a selected runner with a loaded model` (UI) | selected index has no model yet, or the selection is stale |

### Known M2 limitations

- **No look control.** The camera inherits the eye rotation and always faces forward.
- **Camera changes are not blocked.** Blocking them needs `ChangeCameraMode` /
  `PlayEventCamera`, which Hachimi already hooks and which MinHook will not let us
  share, so cut-ins intentionally win.
- A one-frame lag is possible if the course camera culls after the race camera in a
  given frame.
- The head/hair geometry may intrude into the near plane. `FORWARD_OFFSET` in
  `src/pov.rs` (currently 0.10 m) is the knob for that.

### Tuning without rebuilding

`hachimi\honse_pov.ini` is re-read every second, so values can be changed while the game
is running — no rebuild, no restart.

```ini
fov = 90            # HORIZONTAL FOV in degrees (Quake convention), converted to
                    # Unity's vertical fieldOfView via the camera aspect
attach = eyes       # eyes | head | neck | chest
forward = 0.015     # metres along the attach point's forward axis
up = 0.075          # metres straight up (negative moves down)
max_rot_step = 0.01 # max rotation change per frame, radians; 0 = rigid head tracking
near_clip = 0.05    # near clip plane while POV is on; 0 = leave the game's value alone
far_clip = 2500     # far clip plane while POV is on; 0 = leave the game's value alone
hide_head = 1       # hide the M_Face / M_Hair meshes
hide_body = 1       # also hide the runner's own body meshes
culling = none      # default | none | skip
suppress_cutins = 1 # drop skill cut-ins at the source while POV is on
```

Nothing is hardcoded: every POV value above is read from this file.

Changes are logged as `POV tuning: fov=... forward=... ...`, so you can confirm the file
was picked up.

### Why `near_clip` and `far_clip` exist

The game derives both from things we override, and both feed back into what it reads on
the next frame.

**Near.** `RaceCameraManager` scales the near clip by FOV (`NEARCLIP_FOV_SCALE_MIN/MAX`,
`MIN_NEARCLIP_CHANGE_FOV`). Our FOV override persists on the Camera object, so the game
reads it back and inflates the clip plane — which cut away runners several metres ahead.

**Far.** `RaceCameraManager.CalcFarClipPlane(targetCamera)` recomputes the far clip from
the camera transform, so at some track positions it shrinks enough to clip the skydome,
which reads as the sky being eaten.

Hachimi Edge dodges the near case by hooking `Camera.set_nearClipPlane` and forcing
`0.001` whenever *its* free camera is active for the race — which never covers this
plugin. We instead write both planes ourselves while POV is drawing (0.05 / 2500, the
latter matching what core forces), and put the game's own values back when we release the
camera. The values are sampled from the game while POV is off, so a release always
restores what the game was actually using.

### Why `hide_body` exists

The camera sits at the eye midpoint, and the runner's shoulders are only ~20 cm below
that. At a wide FOV the runner's own torso and outfit fill the bottom of the frame, so the
head meshes alone are not enough — `hide_body` hides `M_Body`, `M_Cheek`, `M_Mayu`, `Eyes`
and `M_Tail` as well, giving a "no self model" first person view. As with the head parts,
everything is re-shown the moment we stop driving the camera.

### Player identity and popularity

`HorseData.get_IsUser()` is not trustworthy on this build: in a nine runner field it
reports true for the last three entries. The player's runner is identified from
`RaceHorseManagerBase.GetPlayerHorseIndex()` instead, and the raw `get_IsUser` value is
still logged alongside for comparison.

`HorseData.get_Popularity()` is stored 0-based, so the UI shows `popularity + 1` (1 =
favourite).

### Why `FindReserve` is the cut-in hook that matters

The obvious target is `RaceSkillCutInReserveCreator.AddCutInInfo`, which is where a
cut-in joins the reserve list. In practice it **never fires**: the reserve is built once
at race load by `CreatePlayReserved`, before the race runs, so hooking the incremental
adder does nothing (confirmed — both overloads installed cleanly and logged zero calls).

The gate the race actually consults is `FindReserve`, asked before entering the cut-in
state. Answering `false` is equivalent to having an empty reserve list, so it covers
reserves built at load time and every other piece of race code runs as it always did.
`AddCutInInfo` is still hooked (both overloads) to catch incremental additions and avoid
the asset load that queuing triggers.

Suppression is independent of which runner is selected — it is a global POV setting.

### Why `culling` exists (frozen hair / cloth)

The game runs a per-model visibility pass, `RaceViewBase.Culling(curCamera,
targetHorseIndex, culling, cullingSqrMagnitude)`, and culling a model **pauses its CySpring
hair and cloth simulation** (`ModelController.SetVisible` →
`CySpringController.IsSkipUpdatePre`).

That pass runs in the **LateUpdate** phase, using the game's camera position. We only move
the camera later, in `OnPreCull`. So the game's notion of "on screen" is its own camera,
not your POV — which is why runners beside the one you are watching freeze mid-gallop while
everything else keeps moving. It is a draw-distance effect, just measured from the wrong
viewpoint.

```ini
culling = none   # default | none | skip
```

- `default` — leave the game's culling alone.
- `none` — keep the call but pass `CullingType.None`, so nothing is culled. `CullingType`
  is `None, Default`, so `None` is the game's own "do not cull".
- `skip` — drop the call entirely, if `none` turns out not to be enough.

Cost: every runner keeps full spring simulation and LOD. If that hurts performance, the
proper fix is to make the pass use *our* camera pose rather than disabling it — the hook
already has the `curCamera` argument, so setting the camera transform to the POV pose at
the top of the hook would make the distance checks match what is actually rendered.

### Diagnostic lines

```
POV field (9 runners, player index 7): 0|gate1|pop5(+1=6)|-  1|gate2|pop3(+1=4)|-  ...
  7|gate8|pop6(+1=7)|PLAYER  ...
```

`(rawUser)` is appended when `get_IsUser` disagrees with the player index, which is how the
last-three-entries bug above was found.

Once per runner change, the model owner's child object names and how many were hidden:

```
POV: owner children (12) = [M_Body, M_Face, M_Hair, ...] -> hid 2
```

If `hid` is 0, the head meshes are named differently on this model and the camera will be
looking at the inside of them — send that line.

### Window hotkey

The picker window toggles with a global chord, default **Ctrl+Shift+P**:

```ini
window_hotkey = ctrl+shift+p   # or alt+p, ctrl+shift+f1, none, ...
```

Recognised modifiers are `ctrl`, `shift`, `alt`; keys are `a`-`z`, `0`-`9`, `f1`-`f12`,
and `space tab insert delete home end pageup pagedown` plus the punctuation names
(`minus`, `equals`, `comma`, `period`, `slash`, `semicolon`, `quote`, `backquote`,
`bracketleft`, `bracketright`, `backslash`). A typo leaves the previous binding alone
rather than silently unbinding it.

#### Why it needs Ctrl or Alt

Key state is polled globally (`GetAsyncKeyState`), so a chord that types characters would
fire while you type anywhere:

- a bare key, or Shift alone, produces characters;
- **Ctrl+Alt is AltGr on Windows** — on non-US layouts that is how everyday characters are
typed (Spanish `AltGr+2` is `@`), so a Ctrl+Alt chord fires during ordinary typing.

Ctrl+Shift produces no characters on any layout. The plugin only warns if you pick
something without Ctrl or Alt; it does not refuse it.

#### Why polling rather than a key hook

Hachimi Edge has no WndProc plugin hook, so a plugin cannot observe key messages. Polling
happens on the host's present callback — the same approach honse-tracker uses. Two
consequences:

- it is gated on the foreground window belonging to the game process, so chords do not
  fire while another application has focus;
- it needs the present callback, which only exists when the Hachimi GUI is enabled
  (`disable_gui = false`). With the GUI off, this hotkey does nothing.

Toggling asks the host for the window's real state, so closing the window with its **X**
and then pressing the chord reopens it in one press.

## Revert

```bash
GAME="/c/Program Files (x86)/Steam/steamapps/common/UmamusumePrettyDerby"
cp "$GAME/hachimi/config.json.bak" "$GAME/hachimi/config.json"
rm "$GAME/hachimi/honse_pov.dll"
```
