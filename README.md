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
- **Omarchy**: on Omarchy (detected via `$OMARCHY_PATH` or `/usr/share/omarchy`), every entry of the
  Omarchy menu is searchable — `lock`, `theme`, `hide bar`, `install docker`. Omarchy's defaults
  are merged with `~/.config/omarchy/extensions/omarchy-menu.jsonc`; `when`/`disabled`/`checked`
  conditions run in the background; submenus open via `omarchy-menu summon <id>`.
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
```

## Layout

| File | What |
| --- | --- |
| `src/main.rs` | Rune UI: state, results list, keys |
| `src/apps/mod.rs` | App entry type and fuzzy search (shared) |
| `src/apps/linux.rs` | XDG desktop entry scanning and launching |
| `src/apps/macos.rs` | `.app` bundle scanning and launching |
| `src/dictation.rs` | Detecting Superwhisper dictation |
| `src/agent.rs` | Handing a prompt to the coding agent |
| `src/sys.rs` | OS commands: open URL, clipboard, detached spawn |
| `src/calc.rs` | Expression evaluator |
| `src/url.rs` | URL detection and opening |
| `src/platform.rs` | Platform detection (Omarchy / generic Linux) |
| `src/omarchy.rs` | Omarchy menu loading (JSONC), conditions, search, running |

Type-check the macOS build from Linux (blake3's C code needs the macOS SDK, so use its pure-Rust path):

```sh
rustup target add aarch64-apple-darwin
CARGO_FEATURE_PURE=1 cargo check --target aarch64-apple-darwin
```

## Headless checks

```sh
cargo test
cargo test live_claude -- --ignored   # runs the real claude CLI (tiny paid request)
cargo run -- --snap shot.png --scene calc   # scenes: apps calc url web omarchy chat
cargo run -- --tree tree.json --scene apps
```
