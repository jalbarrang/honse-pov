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

## 2. Does the menu section appear?

Open the Hachimi menu (Right Arrow) **from the home screen**. You should find a
section **Race POV (experimental)** containing:

- a heading
- an **Open runner picker window** button
- `No active race detected.` plus a hint line (because you're not racing)

Failures:

- **section missing entirely** → GUI registration failed even though the plugin
  loaded. Send the log.
- **`Waiting for game initialization (classes not resolved yet).`** → step 1 did not
  complete; the log tells you why.
- **`Race tracking failed to install. ...`** → step 1 tells you why.

## 3. Does the window open?

Click **Open runner picker window**, or the menu item **Open Race POV window**.

Expected: a standalone window titled `Race POV (experimental)`, showing the same
"no active race" message, with a fixed bottom bar containing **Clear selection** and
**Close**.

- Clicking **Open** twice should not stack duplicate windows (the host replaces the
  window with the same id).
- **Close** should close it; the menu button should reopen it.
- The window should be draggable/resizable like other Hachimi windows.

## 4. Does the runner list populate? (main test)

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

## 5. Does selection work?

- [ ] click **select** on a row → that row becomes `* selected`, the header becomes
      `Selected: gate N - <name> (#idx)`, and `hachimi.log` gets
      `selected runner index -> N`
- [ ] click a different row → header follows, new log line
- [ ] **Clear selection** → header returns to `Selected: none`, log gets
      `selected runner index -> -1`

## 6. Milestone 1 assertions (must NOT happen)

- [ ] **the race camera never changes** — no camera jump, no lost cut-ins, no POV.
      Selection must be purely cosmetic in this build.
- [ ] no stutter when the list refreshes (it ticks 10x/sec)
- [ ] no crash on: race start, race end, leaving to home, pausing, opening the menu
      mid-race, opening the window mid-race
- [ ] after the race ends the list goes back to `No active race detected.`, and
      starting another race repopulates it

## 7. Known limitations (not bugs)

- **No camera effect.** By design for M1.
- The list refreshes 10x/sec, so gate/name/popularity are near-live but not
  frame-perfect.
- The list can be empty for the first frames of a race while models load.
- Selection is in-memory only; it resets to `none` on game restart.
- `popularity` is the popularity **rank** (1 = race favourite), as stored on
  `HorseData.Popularity` — confirm it matches what the UI shows in-game.

## 8. If something fails, send me

1. `hachimi.log` (at minimum every line containing `honse_pov` or `could not` / `failed`)
2. which step number failed and what you saw instead
3. the race type you were in (single mode / daily / story / legend / team stadium)
4. a screenshot of the window or menu section if it's a layout problem

## Revert

```bash
GAME="/c/Program Files (x86)/Steam/steamapps/common/UmamusumePrettyDerby"
cp "$GAME/hachimi/config.json.bak" "$GAME/hachimi/config.json"
rm "$GAME/hachimi/honse_pov.dll"
```
