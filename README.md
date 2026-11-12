# tomboy

A personal Raycast-style launcher for Linux (Hyprland/Omarchy) and macOS, built with [Rune](https://github.com/anomalyco/rune).

## Install on Omarchy

Tested on Omarchy 4.5.

1. **Install Rust.** Open the Omarchy menu (Super+Space) → **Install** → **Development** →
   **Rust**, and let it finish in the terminal it opens.
2. **Install tomboy** in a new terminal:

   ```sh
   cargo install --git https://github.com/dzhibas/tomboy
   ```

   This builds the latest `main` (a few minutes the first time) into `~/.cargo/bin/tomboy`.
   Run the same command again later to update.
3. **Bind the keys.** Add these to the end of `~/.config/hypr/bindings.lua`:

   ```lua
   -- Super+Space: tomboy launcher (was: Omarchy menu).
   o.rebind("SUPER + SPACE", "Tomboy launcher", "~/.cargo/bin/tomboy")
   -- Super+Shift+C: tomboy clipboard history (was: Calendar).
   o.rebind("SUPER + SHIFT + C", "Tomboy clipboard", "~/.cargo/bin/tomboy clipboard")
   ```

   Hyprland reloads on save; `hyprctl configerrors` should print nothing. The Omarchy menu is
   still a keystroke away: every entry in it is searchable inside tomboy (`theme`, `lock`,
   `install docker`), and its submenus open in tomboy too.
4. **Try it**: press Super+Space and type `firefox`, `2+2`, `100 km to miles`, `emoji` or
   `clip`.

Optional: bind the emoji picker too, see [Emoji & symbols](#emoji--symbols), and set things up
in [Configuration](#configuration) (or just ask the agent: "make the font bigger").

| | Linux | macOS |
| --- | --- | --- |
| Apps | XDG `.desktop` entries, launched (via `uwsm app --` if present) | `.app` bundles in `/Applications`, `~/Applications`, `/System/Applications`…, launched with `open` |
| URLs | `xdg-open` | `open` |
| Copy | `wl-copy` (Wayland), `xclip`/`xsel` (X11) | `pbcopy` |
| Platform menu | Omarchy menu when detected | — |

All OS commands live in `src/sys.rs` and `src/apps/{linux,macos}.rs` behind `#[cfg(target_os)]`.

- **Apps**: type to fuzzy-search installed `.desktop` applications; Enter launches (via `uwsm app --` when available).
- **Ranking learns from you (frecency)**: every app or Omarchy command you launch is remembered
  with when you launched it and what you typed. Things you use often and recently move up, and
  a query you've used before favours what you picked last time: launch hackernews after typing
  `news` and `news` (or `ne`) ranks it above euronews, so Enter opens it. With an empty field,
  your usual apps come first. It only reorders matches, never adds one. Stored in
  `~/.local/state/tomboy/frecency.json`; delete it to start over, or ask the agent: "forget
  hackernews" drops one item, "news shouldn't pick hackernews" drops one learned query, "reset
  usage counts" clears it all. Turn it off with `[search] frecency = false` (or "turn off
  frecency").
- **Calculator**: `2+2`, `(2+2)*10/3`, `sqrt(16)`, `2^10`, `sin(pi/2)` — Enter copies the answer (`wl-copy`).
  Percentages work like a phone calculator: `15% of 200` = 30, `15% off 200` = 170,
  `200 + 15%` = 230, `200 - 15%` = 170; `10 % 3` is still modulo.
- **URLs**: `github.com/anomalyco/rune`, `https://…`, `localhost:3000` — Enter opens the browser (`xdg-open`).
- **Answers** (`src/answers/`) — Enter copies the answer:

  | Type | Get |
  | --- | --- |
  | `100 km to miles`, `20 c to f`, `1 tb to gib`, `2 cup to ml` | `62.1371 mi` — length, mass, volume, temperature, data (decimal and binary), speed, time, area |
  | `123 eur to usd`, `$120 to eur`, `50 euros to dollars`, `eur to usd` | `143.31 USD` — 166 currencies; rates from open.er-api.com, fetched on the first conversion typed, cached a day in `~/.cache/tomboy/` |
  | `days until 25 dec`, `days since 1 jan 2020`, `weeks until christmas` | `43 days` · `Friday, 25 December 2026` — a date without a year is its next occurrence (its last one after `since`) |
  | `monday in 3 weeks`, `in 3 weeks`, `3 days ago`, `today + 10 days`, `2 weeks after christmas`, `next friday` | `Monday, 30 November 2026` · `2026-11-30 · in 18 days` — a weekday with an offset is that day in the week the offset lands in |
  | `time in tokyo`, `now in utc`, `pst time` | `18:13` · `Asia/Tokyo · 6 hours ahead` — any city in the tz database, plus abbreviations |
  | `uuid`, `password 24`, `sha256 x`, `base64 x`, `b64d x`, `urlencode x`, `urldecode x`, `epoch`, `epoch 1787875351` | developer utilities; random values come from `/dev/urandom` and hold still until the next launch |
  | `kill chromium` | matching processes with pid, CPU and memory — Enter sends SIGTERM |
- **Clipboard history**: `tomboy clipboard`, or type `clip` in the launcher — see
  [Clipboard history](#clipboard-history).
- **Emoji & symbols**: `tomboy emoji`, or type `emoji` in the launcher — see
  [Emoji & symbols](#emoji--symbols).
- **Omarchy**: on Omarchy (detected via `$OMARCHY_PATH` or `/usr/share/omarchy`), every entry of the
  Omarchy menu is searchable — `lock`, `theme`, `hide bar`, `install docker`. Omarchy's defaults
  are merged with `~/.config/omarchy/extensions/omarchy-menu.jsonc`; `when`/`disabled`/`checked`
  conditions run in the background. Enter on a submenu browses it inside tomboy (Esc goes back);
  submenus the shell fills at runtime open via `omarchy-menu summon <id>`.
  `TOMBOY_PLATFORM=generic` turns this off.
- **Agent, inside tomboy**: "Ask agent" runs the coding agent headless — the
  [configured](#configuration) `agent.name`, else Omarchy's default (`omarchy-default-agent`),
  else `$TOMBOY_AGENT` (default `claude`) — in `~/Work` (or the
  [configured](#configuration) directory), and streams
  its answer into the window as Markdown, with tool calls, time and cost. The field then takes
  follow-ups in the same session (claude `--resume`). Enter on an empty field copies the
  answer; Esc stops a running agent, a second Esc closes. Claude streams JSON events;
  opencode, codex and crush stream plain text; other agents open in a terminal instead.
- **Voice → agent**: text dictated into tomboy with Superwhisper (one multi-character insert
  that matches `superwhisper read` from the last minute) goes straight to the agent. Typed
  text never triggers it.
- **Fallback**: anything else offers "Ask agent" and a DuckDuckGo search.

↑/↓ pick · Enter run · Esc or click away to close.

## Configuration

tomboy works with no config. To change a setting, create `~/.config/tomboy/config.toml`
(`$XDG_CONFIG_HOME/tomboy/config.toml` when that is set); anything left out keeps its default.
tomboy reads it when it opens, so a change applies on the next launch.

```toml
[agent]
# The agent CLI to use. Default: Omarchy's default agent, else $TOMBOY_AGENT, else claude.
name = "codex"
# Where "Ask agent" runs the coding agent. Default: ~/Work, or ~ when that doesn't exist.
working_directory = "~/sites"

[agent.models]
# The model each agent uses, by agent command. Default: the agent's own default.
claude = "haiku"

[search]
# Rank by what you launch, how often and what you typed (frecency). Default true.
frecency = false

[ui]
# Text size in points; titles, labels and agent answers scale with it. Default 13.
font_size = 15
```

| Setting | Default | What it does |
| --- | --- | --- |
| `agent.name` | Omarchy's default, else `$TOMBOY_AGENT`, else `claude` | The coding agent CLI for "Ask agent", overriding the system's choice (Omarchy's default agent is left as it is). `claude`, `opencode`, `codex` and `crush` answer inside tomboy; any other CLI opens in a terminal. |
| `agent.models.<agent>` | the agent's default | The model an agent runs with: `claude = "haiku"` (or `sonnet`, `opus`, a full id), `codex = "gpt-5"`, `opencode = "anthropic/claude-sonnet-5-5"`. Passed as the agent's `--model`/`-m`; the chat footer shows it. |
| `agent.working_directory` | `~/Work`, else `~` | The agent's working directory. `~` is home; relative paths start at home. A missing directory falls back to the default. |
| `search.frecency` | `true` | Learn from launches and rank by them (frecency, see the feature list). `false` ranks by name match only and records nothing; the history file is kept for when it's back on. |
| `ui.font_size` | `13` | Text size in points (9–24). The search field, titles, labels and agent answers scale with it and rows grow to fit; icons and the window size stay. |

A file that doesn't parse, or has an unknown key, is ignored as a whole; the footer says why
(`config.toml ignored: unknown field …`).

### Changing it by asking the agent

Every agent run carries tomboy's skill ([`src/skill.md`](src/skill.md), built into the binary):
what the config file is, every option, and how to edit it safely. So you can say (or dictate)
in the launcher:

> change the working directory to ~/sites

> make the font bigger

> switch the agent to codex

> use haiku

and the agent writes `config.toml` for you and tells you it applies on the next launch. Claude
gets the skill as an appended system prompt (`--append-system-prompt`, with `--add-dir
~/.config/tomboy` so it may write there); opencode, codex and crush get it ahead of the prompt.
When you add an option, document it in `src/skill.md` too.

## Emoji & symbols

A Raycast-style picker: a grid of big emoji, searchable, with your recent picks first.

- **Open it**: run `tomboy emoji` (bind it to a key, below), or type `emoji`, `symbols` or
  `smiley` in the launcher and press Enter on **Emoji & Symbols**.
- **Find one**: type to search names and keywords (`fire`, `thumbs`, `lol` → 🤣, `cmd` → ⌘,
  `euro`); every word must match. With an empty field the grid shows **Recently Used**, then
  every Unicode group, then **Text Symbols** (arrows, math, currency, typography, keyboard
  keys like ⌘ ⌥ ⇧, Greek letters, check marks).
- **Move**: ←/→/↑/↓ move through the grid, Tab / Shift+Tab jump between sections, hover picks.
  The footer shows the picked one's name.
- **Use it**: Enter (or click) copies it and closes tomboy; paste with Ctrl+V. Picks feed
  Recently Used and rank higher next time (frecency; off with `[search] frecency = false`).
- **Esc** clears the search, then closes (or returns to the launcher).

Data: 1,900+ emoji from Unicode's `emoji-test.txt` (v17, no skin-tone variants) with search
keywords from Omarchy's emoji picker (MIT), generated into `src/emoji/emoji.tsv` by
`scripts/gen-emoji.py`; text symbols are in `src/emoji/symbols.rs`.

Bind it in Hyprland:

```
bind = SUPER CTRL, E, exec, ~/.cargo/bin/tomboy emoji
```

On Omarchy SUPER+CTRL+E opens Omarchy's own emoji picker, so replace it in
`~/.config/hypr/bindings.lua`:

```lua
o.rebind("SUPER + CTRL + E", "Tomboy emoji", "~/.cargo/bin/tomboy emoji")
```

## Clipboard history

A Raycast-style clipboard manager: everything you copied, newest first, with a full preview.

- **Open it**: run `tomboy clipboard` (bind it to a key, below), or type `clip`, `clipboard`,
  `history` or `paste` in the launcher and press Enter on **Clipboard History**.
- **Find a clip**: type to filter; every word must match (`git main` finds `git push origin main`).
- **Preview**: ↑/↓ (or hover) picks a clip; the right pane shows it in full: text in monospace
  (selectable), images and copied image files as pictures, with its size in the footer
  (`Text · 3 lines · 45 characters`, `Image · PNG · 1920×1080`).
- **Use it**: Enter (or click) puts the clip back on the clipboard and closes tomboy, so you
  can paste it with Ctrl+V. Text goes back as text, images as images, copied files as files
  (`text/uri-list`), so a file manager pastes the files themselves.
- **Esc** clears the search, then closes (or returns to the launcher if you came from there).

### Where the history comes from

| Desktop | History | Recorded by |
| --- | --- | --- |
| Omarchy | `~/.local/state/omarchy/clipboard-history.json` | Omarchy's shell, already running; nothing to set up |
| Other Wayland | `~/.local/state/tomboy/clipboard-history.json` | `tomboy clipboard-watch` (runs `wl-paste --watch`; needs `wl-clipboard`) |
| macOS | `~/.local/state/tomboy/clipboard-history.json` | `tomboy clipboard-watch` (polls `pbpaste`, text only) |

`tomboy clipboard-watch` keeps the last 100 clips, moves a repeated copy to the top instead of
duplicating it, saves images under `~/.local/state/tomboy/clipboard-images/`, and skips copies
password managers mark as sensitive. Start it once at login:

```
# ~/.config/hypr/hyprland.conf (not needed on Omarchy)
exec-once = ~/.cargo/bin/tomboy clipboard-watch
```

### Bind it to a key in Hyprland

Classic config (`~/.config/hypr/hyprland.conf` or a file it sources, such as `bindings.conf`):

```
bind = SUPER CTRL, V, exec, ~/.cargo/bin/tomboy clipboard
```

Omarchy's Lua config (`~/.config/hypr/bindings.lua`). SUPER+CTRL+V opens Omarchy's own
clipboard panel by default, so `o.rebind` replaces it (use `o.bind` for a free key):

```lua
o.rebind("SUPER + CTRL + V", "Tomboy clipboard", "~/.cargo/bin/tomboy clipboard")
```

Hyprland reloads the config on save; check it with `hyprctl configerrors`. The window floats
on its own, like the launcher.

## Build from source

```sh
git clone https://github.com/dzhibas/tomboy && cd tomboy
cargo build --release
./target/release/tomboy
```

On a Hyprland without Omarchy's Lua config, bind it in `hyprland.conf` (or a file it sources);
the window floats on its own:

```
bind = SUPER, SPACE, exec, ~/.cargo/bin/tomboy
bind = SUPER SHIFT, C, exec, ~/.cargo/bin/tomboy clipboard
```

On Omarchy, see [Install on Omarchy](#install-on-omarchy).

## Layout

| File | What |
| --- | --- |
| `src/main.rs` | Rune UI: state, results list, keys |
| `src/apps/mod.rs` | App entry type and fuzzy search (shared) |
| `src/apps/linux.rs` | XDG desktop entry scanning and launching |
| `src/apps/macos.rs` | `.app` bundle scanning and launching |
| `src/dictation.rs` | Detecting Superwhisper dictation |
| `src/agent.rs` | Handing a prompt to the coding agent |
| `src/answers/` | Units, currency, date math, time zones, developer utilities, kill process |
| `src/emoji/` | Emoji & symbol data (`emoji.tsv`, `symbols.rs`), search and grid navigation |
| `src/clipboard.rs` | Clipboard history: reading, searching, copying back, recording |
| `src/frecency.rs` | Launch history: frecency and learned queries for ranking |
| `src/config.rs` | `~/.config/tomboy/config.toml`: loading and defaults |
| `src/skill.md` | What the agent knows about tomboy's config (sent with every agent run) |
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
