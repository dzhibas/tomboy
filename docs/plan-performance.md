# Make tomboy open, close and navigate with no perceptible delay

## Context

On Linux every hotkey press starts a new `tomboy` process (macOS already has a resident
`tomboy daemon`). The review looked at the launch, close and per-keystroke paths and at what
Rune (pinned rev `2749796`) offers.

What was measured (release build, this machine, headless `--tree`/`--snap --scene apps`):

- Whole process: 100–165 ms wall, ~0.5 s CPU across threads. Rune reports GPU init 26–38 ms
  and first frame 34–40 ms. A real window adds surface creation and compositor mapping, not
  measured yet.
- The binary is 51 MB, unstripped, `lto = "thin"`.

What was found by reading (not yet timed individually):

| # | Finding | Where |
|---|---------|-------|
| 1 | Hyprland animates tomboy's window: `windowsIn` popin ≈ 410 ms, `windowsOut` ≈ 150 ms, plus fades. No `no_anim` rule exists for tomboy. This is probably the largest *perceived* open/close delay and is outside the app. | `/usr/share/omarchy/default/hypr/looknfeel.lua:76-83`; no rule in `~/.config/hypr` |
| 2 | Typing `kill …` sleeps 150 ms on the UI thread (two `/proc` samples), again every 3 s while the query stays. | `src/answers/kill.rs` `list_procs` / `SAMPLE_GAP`, called from `results()` |
| 3 | `results()` (the full search) runs 3× per arrow key (`on_input_key`, `sync_clip_image`, `view`) and once per hover/redraw. `item_row` calls `alias_target()` per row, which reruns a full `search()` per row when the query is an alias. | `src/main.rs:417`, `:755`, `:1031`, `:1136`, `:1480` |
| 4 | Every launch spawns a `bash -c` per Omarchy menu condition (the menu has 261 `when`/`disabled`/`checked` lines) on 8 threads, competing with the first frame; each visible result calls `wake()`, so the window redraws up to hundreds of times in its first moments. | `src/omarchy.rs:158-183`, `src/main.rs:349-352` |
| 5 | Every launch asks Rune to decode an icon for all ~97 apps, though at most 30 rows show. | `src/main.rs:347` |
| 6 | Clipboard view lowercases every clip's full text on each `results()` call and builds a row per clip (126 now) without virtualization. | `src/clipboard.rs:198`, `src/main.rs:1224` |
| 7 | Quit runs Rune's full teardown (event loop exit, GPU drop) before the process ends; the frecency file is read and rewritten before the window goes away. | `src/main.rs:585-592`, `:863` |
| 8 | Rune on Wayland supports `keep_running` + `hide`/`open` (hide destroys the surface, keeps GPU device, fonts, state). It has no Linux single-instance, so a resident mode needs tomboy's own socket. | rune `window/lifecycle.rs:375-393`, `open_url/instance.rs` |

Intended outcome: the window appears and disappears within a frame or two of the keypress,
and no keystroke ever blocks.

## Plan (in order; each step is independently shippable)

### Step 0 — Trace, so every later step is measured
- `TOMBOY_TRACE=1` prints ms since process start to stderr at: `main` entry, each part of
  `init()` (config, `apps::load_apps`, `omarchy::load`, `load_extensions`, `todo::peek`),
  `setup()`, first `view()`, first `poll()` after the first frame, and `quit()`.
- Small `trace` module (an `Instant` in a `OnceLock`, a `mark(&str)` fn); no-op when unset.
- Record baseline numbers for windowed launch (hotkey → first frame, Esc → process gone).

### Step 1 — Remove the compositor animation (largest perceived win, no code)
- Add a Hyprland window rule for class `tomboy` with `no_anim` (Lua form, as Omarchy does
  for its own layers in `default/hypr/apps/omarchy-shell.lua`). Goes in the user's
  `~/.config/hypr/` (load the `omarchy` skill before editing) and in the README install
  steps next to the key bindings.

### Step 2 — Nothing blocks the UI thread; one search per change
- **kill** (`src/answers/kill.rs`): `answers()` takes the `Wake` (as `currency::answer`
  does). A stale cache returns a single-sample listing at once (CPU shown as pending) and
  starts the two-sample measurement on a thread, which fills `CACHE` and wakes. Pass
  `&s.wake` from `answers::collect`.
- **Memoize `results()`** (`src/main.rs`): `results_cache: RefCell<Option<(String, u64, Vec<Item>)>>`
  keyed by query text and a `rev` counter. Bump `rev` wherever non-query inputs change:
  `enter_*`/`leave_*`, `fresh_view`, `reload`, todo/clip mutations, extension session
  changes, and any background wake seen in `poll` (conditions, currency, processes).
  Arrow keys and hover then reuse the list. Keying on the text keeps the existing tests
  (which assign `s.query` directly) valid.
- **Alias**: compute `alias_target(s)` once in `results_body` and pass it to `item_row`.
- **Clipboard**: store the lowercased haystack in each `Clip` at load; render the list with
  `virtual_rows` as `emoji_body` already does.

### Step 3 — Cheaper cold start and exit
- **Menu conditions** (`src/omarchy.rs`): cache results in `~/.cache/tomboy/` keyed by the
  condition string. `evaluate` seeds `Conditions` from the cache so rows show on frame one,
  re-checks in the background, wakes once per batch (not per row), and writes the cache
  back. Start the re-check after the first frame rather than in `setup`.
- **Icons** (`src/main.rs`): request an app's icon the first time its row is built
  (`Vec<OnceCell<Option<Image>>>`, filled from `item_row` through `s.cache`) instead of all
  apps in `setup`/`reload`.
- **Release profile** (`Cargo.toml`): `strip = "symbols"`, `codegen-units = 1`; try
  `lto = "fat"` and keep it only if the trace shows a gain.
- **Exit**: for the one-off process, record frecency after the window is hidden, and if the
  trace shows teardown costs more than a few ms, end with `std::process::exit(0)` once the
  agent is stopped and writes are flushed (children are already detached via
  `sys::spawn_detached`).
- Only if the trace shows them above ~2 ms: cache the parsed app list (keyed by directory
  mtimes) and the parsed Omarchy menu.

### Step 4 — Resident mode on Linux (removes process start, GPU init, font and app loading)
Biggest launch win, largest change; do a short spike first to confirm Hyprland focuses the
recreated window and that `float_on_open` applies on each reopen.

- **Client**: at the top of `main`, for the window modes (no args, `clipboard`, `emoji`,
  `todo`), connect to `$XDG_RUNTIME_DIR/tomboy.sock`, send the mode and
  `XDG_ACTIVATION_TOKEN` if set, exit. If the connect fails, launch one-off as today, so
  nothing depends on the daemon running.
- **Daemon**: make `run_daemon` cross-platform. On Linux: `App::new(spec()).keep_running()`
  with an initially hidden window, a listener thread that calls
  `me.update(|s, _| summon(s, hotkey))`. Reuse `summon`, `hide`, `reload`, `fresh_view`
  and the `resident`/`shown`/`had_focus` fields unchanged; on Linux `summon` uses
  `me.activate(token)`/`me.open()` in place of `place` + `show`. The same mode sent twice
  toggles, as the macOS hotkeys do.
- `hide()` additionally re-runs the Omarchy condition check so the next show is current.
- After a rebuild the daemon would be stale: on hide, compare the executable's mtime with
  the one at start and `relaunch` if it changed.
- Autostart: a line in `~/.config/hypr/autostart.lua` running `tomboy daemon` (user config,
  via the `omarchy` skill) and a README section. Cost to state there: the process stays
  resident with its GPU device (measure RSS in the spike).

## Critical files
- `src/main.rs` — trace marks, results memo, alias, lazy icons, quit order, daemon/client.
- `src/answers/kill.rs`, `src/answers/mod.rs` — background sampling.
- `src/omarchy.rs` — condition cache, batched wake.
- `src/clipboard.rs` — precomputed haystack.
- `Cargo.toml` — release profile.
- `README.md` — window rule, daemon setup.
- User config (not in repo): `~/.config/hypr/` window rule and autostart.

## Verification
- `cargo test` (existing `extension_flow_tests`, kill, omarchy, clipboard tests) plus new
  tests: results memo invalidates on mode change and wake; kill returns immediately with a
  cold cache; condition cache round-trips.
- Headless scenes still render the same: `cargo run --release -- --snap out.png --scene X`
  for `apps`, `kill`, `omarchy`, `clipboard`, `alias`, `emoji`, compared before/after.
- `TOMBOY_TRACE=1` numbers before and after each step, for hotkey → first frame and
  Esc → exit; report them rather than assume.
- By hand on Hyprland: open/close with the hotkey (no animation), hold ↓ through the list,
  type `kill fire` (no stall), open clipboard and scroll, launch an app and confirm
  frecency still records; with the daemon: toggle each mode, click away, rebuild and
  confirm it restarts itself.
