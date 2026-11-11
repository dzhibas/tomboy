# tomboy

A personal Raycast-style launcher for Linux (Hyprland/Omarchy) and macOS, built with [Rune](https://github.com/anomalyco/rune).

| | Linux | macOS |
| --- | --- | --- |
| Apps | XDG `.desktop` entries, launched (via `uwsm app --` if present) | `.app` bundles in `/Applications`, `~/Applications`, `/System/Applications`…, launched with `open` |
| URLs | `xdg-open` | `open` |
| Copy | `wl-copy` (Wayland), `xclip`/`xsel` (X11) | `pbcopy` |
| Platform menu | Omarchy menu when detected | — |

All OS commands live in `src/sys.rs` and `src/apps/{linux,macos}.rs` behind `#[cfg(target_os)]`.

- **Apps**: type to fuzzy-search installed `.desktop` applications; Enter launches (via `uwsm app --` when available).
- **Calculator**: `2+2`, `(2+2)*10/3`, `sqrt(16)`, `2^10`, `sin(pi/2)` — Enter copies the answer (`wl-copy`).
- **URLs**: `github.com/anomalyco/rune`, `https://…`, `localhost:3000` — Enter opens the browser (`xdg-open`).
- **Answers** (`src/answers/`) — Enter copies the answer:

  | Type | Get |
  | --- | --- |
  | `100 km to miles`, `20 c to f`, `1 tb to gib`, `2 cup to ml` | `62.1371 mi` — length, mass, volume, temperature, data (decimal and binary), speed, time, area |
  | `123 eur to usd`, `$120 to eur`, `50 euros to dollars`, `eur to usd` | `143.31 USD` — 166 currencies; rates from open.er-api.com, fetched on the first conversion typed, cached a day in `~/.cache/tomboy/` |
  | `time in tokyo`, `now in utc`, `pst time` | `18:13` · `Asia/Tokyo · 6 hours ahead` — any city in the tz database, plus abbreviations |
  | `uuid`, `password 24`, `sha256 x`, `base64 x`, `b64d x`, `urlencode x`, `urldecode x`, `epoch`, `epoch 1787875351` | developer utilities; random values come from `/dev/urandom` and hold still until the next launch |
  | `kill chromium` | matching processes with pid, CPU and memory — Enter sends SIGTERM |
- **Clipboard history**: `tomboy clipboard` (or type `clip` / `clipboard` / `paste` in the
  launcher) lists what you copied, newest first: type to filter, ↑/↓ shows each clip in full on
  the right (text in monospace, images as pictures), Enter puts it back on the clipboard and
  closes. Copied files go back as files (`text/uri-list`), images as images. On Omarchy the
  history is Omarchy's own (`~/.local/state/omarchy/clipboard-history.json`, recorded by the
  shell); elsewhere run `tomboy clipboard-watch` at login to record
  `~/.local/state/tomboy/clipboard-history.json` (Wayland `wl-paste --watch`, or `pbpaste`
  polling on macOS; last 100 clips, password-manager copies skipped).
- **Omarchy**: on Omarchy (detected via `$OMARCHY_PATH` or `/usr/share/omarchy`), every entry of the
  Omarchy menu is searchable — `lock`, `theme`, `hide bar`, `install docker`. Omarchy's defaults
  are merged with `~/.config/omarchy/extensions/omarchy-menu.jsonc`; `when`/`disabled`/`checked`
  conditions run in the background. Enter on a submenu browses it inside tomboy (Esc goes back);
  submenus the shell fills at runtime open via `omarchy-menu summon <id>`.
  `TOMBOY_PLATFORM=generic` turns this off.
- **Agent, inside tomboy**: "Ask agent" runs the coding agent headless — Omarchy's default
  (`omarchy-default-agent`), else `$TOMBOY_AGENT` (default `claude`) — in `~/Work`, and streams
  its answer into the window as Markdown, with tool calls, time and cost. The field then takes
  follow-ups in the same session (claude `--resume`). Enter on an empty field copies the
  answer; Esc stops a running agent, a second Esc closes. Claude streams JSON events;
  opencode, codex and crush stream plain text; other agents open in a terminal instead.
- **Voice → agent**: text dictated into tomboy with Superwhisper (one multi-character insert
  that matches `superwhisper read` from the last minute) goes straight to the agent. Typed
  text never triggers it.
- **Fallback**: anything else offers "Ask agent" and a DuckDuckGo search.

↑/↓ pick · Enter run · Esc or click away to close.

## Build & run

```sh
cargo build --release
./target/release/tomboy
```

Bind it in Hyprland (e.g. `~/.config/hypr/bindings.conf`); the window floats on its own:

```
bind = SUPER ALT, SPACE, exec, ~/Work/tomboy/target/release/tomboy
bind = SUPER CTRL, V, exec, ~/Work/tomboy/target/release/tomboy clipboard
```

On Omarchy's Lua config (`~/.config/hypr/bindings.lua`):

```lua
o.rebind("SUPER + SPACE", "Tomboy launcher", "/home/me/Work/tomboy/target/release/tomboy")
o.rebind("SUPER + CTRL + V", "Tomboy clipboard", "/home/me/Work/tomboy/target/release/tomboy clipboard")
```

Off Omarchy, record the clipboard history at login (e.g. `exec-once = …/tomboy clipboard-watch`).

## Layout

| File | What |
| --- | --- |
| `src/main.rs` | Rune UI: state, results list, keys |
| `src/apps/mod.rs` | App entry type and fuzzy search (shared) |
| `src/apps/linux.rs` | XDG desktop entry scanning and launching |
| `src/apps/macos.rs` | `.app` bundle scanning and launching |
| `src/dictation.rs` | Detecting Superwhisper dictation |
| `src/agent.rs` | Handing a prompt to the coding agent |
| `src/answers/` | Units, currency, time zones, developer utilities, kill process |
| `src/clipboard.rs` | Clipboard history: reading, searching, copying back, recording |
| `src/sys.rs` | OS commands: open URL, clipboard, detached spawn |
| `src/calc.rs` | Expression evaluator |
| `src/url.rs` | URL detection and opening |
| `src/platform.rs` | Platform detection (Omarchy / generic Linux) |
| `src/omarchy.rs` | Omarchy menu loading (JSONC), conditions, search, running |

Type-check the macOS build from Linux (blake3's C code needs the macOS SDK, so use its pure-Rust path).
This currently stops at `ring` (ureq's TLS), whose C code also needs the SDK; build on a Mac for a full check:

```sh
rustup target add aarch64-apple-darwin
CARGO_FEATURE_PURE=1 cargo check --target aarch64-apple-darwin
```

## Headless checks

```sh
cargo test
cargo test live_claude -- --ignored   # runs the real claude CLI (tiny paid request)
cargo run -- --snap shot.png --scene calc   # scenes: apps calc url web omarchy chat submenu units time dev kill currency
cargo run -- --tree tree.json --scene apps
```
