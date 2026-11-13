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

## Install on macOS

On a Mac tomboy runs in the background like Raycast: no Dock icon, a hotkey brings the window
up instantly, and it records your clipboard history.

1. **Install Rust** (`curl https://sh.rustup.rs -sSf | sh`) and the icon font
   (`brew install --cask font-jetbrains-mono-nerd-font`).
2. **Install tomboy**: `cargo install --git https://github.com/dzhibas/tomboy`
3. **Start it at login**: `tomboy install-launch-agent`. This writes
   `~/Library/LaunchAgents/io.github.dzhibas.tomboy.plist`, which runs `tomboy daemon` now, at
   every login and again if it crashes, with your shell's `PATH` (so `claude` is found). Run it
   again after updating tomboy or changing `[hotkeys]` to restart the daemon;
   `tomboy uninstall-launch-agent` removes it. Log: `~/Library/Logs/tomboy.log`.
4. **Use it**:

   | Hotkey | Opens |
   | --- | --- |
   | ⌥Space | the launcher |
   | ⌘⇧C | clipboard history |
   | ⌃⌥E | emoji & symbols |
   | ⌃⌥T | today's todos |

   Change them under [`[hotkeys]`](#configuration). To use ⌘Space, first turn off Spotlight's
   shortcut (System Settings › Keyboard › Keyboard Shortcuts › Spotlight), or Raycast's. The
   same hotkey again, Esc or a click elsewhere hides the window.
5. **Allow pasting** (optional): picking a clip or an emoji copies it and pastes it into the
   app you were in, as Raycast does. That needs System Settings › Privacy & Security ›
   Accessibility; macOS asks the first time. Until then picks are only copied. Rebuilding
   tomboy may need the permission granted again. Turn it off with `[clipboard] paste = false`.

Type `lock`, `sleep`, `restart`, `empty trash`, `dark mode`, `mute` or `eject` for macOS's
system commands; `quit tomboy` stops the daemon until the next login (or
`tomboy install-launch-agent`). Plain `tomboy`, `tomboy clipboard` and so on still open a
one-off window, as on Linux.

| | Linux | macOS |
| --- | --- | --- |
| Apps | XDG `.desktop` entries, launched (via `uwsm app --` if present) | `.app` bundles in `/Applications`, `~/Applications`, `/System/Applications`…, launched with `open` |
| URLs | `xdg-open` | `open` |
| Copy | `wl-copy` (Wayland), `xclip`/`xsel` (X11) | `pbcopy`; images and files through AppKit's pasteboard |
| Platform menu | Omarchy menu when detected | System commands (lock, sleep, restart, empty Trash, dark mode…) |
| Hotkeys | your compositor's bindings run `tomboy` | `tomboy daemon` registers `[hotkeys]` itself |

All OS commands live in `src/sys.rs`, `src/apps/{linux,macos}.rs` and `src/mac/` behind `#[cfg(target_os)]`.

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
- **Todos**: today's checklist, `tomboy todo`, or type `todo` (`todo buy milk` adds one) — see
  [Todos](#todos).
- **Quicklinks & aliases**: `jira ABC-1` opens your Jira ticket, `gh tomboy` a GitHub search;
  `bubu` opens Chromium. Set up in the config — see [Quicklinks & aliases](#quicklinks--aliases).
- **Extensions**: more commands, installed from inside tomboy (type `extensions`), such as
  Hacker News (`hn`). Sandboxed Luau scripts, each reviewed in a pull request to this repo —
  see [Extensions](#extensions).
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

[quicklinks]
# keyword = URL; {…} is where what you type goes.
gh = "https://github.com/search?q={query}"
jira = { url = "issues.jira.com/browse/{issue}", name = "Jira issue" }

[aliases]
bubu = "chromium"

[todo]
# Where the daily todo lists live. Default ~/Documents/tomboy/todo.
directory = "~/notes/todo"

[hotkeys]
# macOS (`tomboy daemon`): global shortcuts; "" turns one off.
launcher = "cmd+space"
todo = ""

[clipboard]
# macOS: picking a clip or emoji also pastes it into the app in front. Default true.
paste = false

[extensions]
# Where "Install Extension" lists extensions from. Default tomboy's GitHub repo.
registry = "https://raw.githubusercontent.com/dzhibas/tomboy/main"

[extensions.hackernews]
# An extension's preferences, by the ids in its tomboy.toml.
feed = "show_hn"
```

| Setting | Default | What it does |
| --- | --- | --- |
| `agent.name` | Omarchy's default, else `$TOMBOY_AGENT`, else `claude` | The coding agent CLI for "Ask agent", overriding the system's choice (Omarchy's default agent is left as it is). `claude`, `opencode`, `codex` and `crush` answer inside tomboy; any other CLI opens in a terminal. |
| `agent.models.<agent>` | the agent's default | The model an agent runs with: `claude = "haiku"` (or `sonnet`, `opus`, a full id), `codex = "gpt-5"`, `opencode = "anthropic/claude-sonnet-5-5"`. Passed as the agent's `--model`/`-m`; the chat footer shows it. |
| `agent.working_directory` | `~/Work`, else `~` | The agent's working directory. `~` is home; relative paths start at home. A missing directory falls back to the default. |
| `search.frecency` | `true` | Learn from launches and rank by them (frecency, see the feature list). `false` ranks by name match only and records nothing; the history file is kept for when it's back on. |
| `ui.font_size` | `13` | Text size in points (9–24). The search field, titles, labels and agent answers scale with it and rows grow to fit; icons and the window size stay. |
| `quicklinks.<keyword>` | none | A URL pattern opened by a keyword, see [Quicklinks & aliases](#quicklinks--aliases). |
| `aliases.<word>` | none | Another word for an app, command or quicklink, see [Quicklinks & aliases](#quicklinks--aliases). |
| `todo.directory` | `~/Documents/tomboy/todo` | Where the daily todo lists live, see [Todos](#todos). |
| `hotkeys.launcher` / `clipboard` / `emoji` / `todo` | `alt+space` / `cmd+shift+c` / `ctrl+alt+e` / `ctrl+alt+t` | macOS only: the daemon's global shortcuts (`cmd`, `ctrl`, `alt`, `shift` + a key). `""` turns one off. Read when the daemon starts: run `tomboy install-launch-agent` again to apply. |
| `clipboard.paste` | `true` | macOS only: after a clip or emoji is picked, paste it into the app in front (needs the Accessibility permission). `false` only copies. |
| `extensions.registry` | `https://raw.githubusercontent.com/dzhibas/tomboy/main` | Where the extension store reads `extensions/index.json` and files from: an `https://` base, a `file://` URL or a local checkout of tomboy (for forks and testing). |
| `extensions.<id>.<pref>` | the extension's default | An extension's preference, by the ids in its `tomboy.toml`; see [Extensions](#extensions). |

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

> add a quicklink jira for issues.jira.com/browse with the issue as argument

> alias bubu to chromium

and the agent writes `config.toml` for you and tells you it applies on the next launch. Claude
gets the skill as an appended system prompt (`--append-system-prompt`, with `--add-dir
~/.config/tomboy` so it may write there); opencode, codex and crush get it ahead of the prompt.
When you add an option, document it in `src/skill.md` too.

## Todos

A checklist for the day. Open it with `tomboy todo` (bind it to a key), or type `todo` in the
launcher; whenever today has tasks, the empty launcher shows **Today · 3/10 done** at the top.

- **Add**: type and press Enter. From the launcher, `todo buy milk` (or `task …`) + Enter adds
  without opening the list, so you can add a few in a row.
- **Check off**: ↑/↓ and Enter (or Space), or click a task. Enter on a done task opens it again.
- **Remove**: Delete.
- Esc clears the field, then goes back (or closes, when opened as `tomboy todo`).

Each day is a Markdown file in `~/Documents/tomboy/todo/` (`[todo] directory`), so any editor,
Obsidian or the agent can read and change it; tomboy re-reads it on every change:

```markdown
# 2026-11-12

- [x] Reply to Jonas
- [ ] Review PR #42
```

Opening the list moves the open tasks from your last earlier list into today, and they're
marked `- [>]` (moved) in the old file, so nothing is open in two places and old days keep
their history.

From a script or another keybinding: `tomboy todo add buy milk` (prints `3/11 done`).
Or ask the agent: "add review PR 42 to my todos", "mark buy milk done", "what's left today?",
"move the dentist to tomorrow".

Bind it on Omarchy (`~/.config/hypr/bindings.lua`; Super+Alt+T is free there):

```lua
o.bind("SUPER + ALT + T", "Tomboy todo", "~/.cargo/bin/tomboy todo")
```

## Quicklinks & aliases

Two small things in `config.toml` that save typing.

**Quicklinks** open a URL with what you type filled in:

```toml
[quicklinks]
gh = "https://github.com/search?q={query}"
jira = { url = "issues.jira.com/browse/{issue}", name = "Jira issue" }
yt = "https://www.youtube.com/results?search_query={q}"
google = { url = "https://www.google.com/?q={q}", name = "google" }
translate = { url = "https://translate.google.com/?sl=auto&tl=en&text={text}&op=translate", name = "Google Translate" }
news = "https://news.ycombinator.com"   # no {…}: a bookmark
```

- Type `jira ABC-1` and press Enter: `https://issues.jira.com/browse/ABC-1` opens in your
  browser. The row shows the URL before you open it.
- Type `jira` (or the start of it, `ji`) and press Enter: tomboy asks for the argument, with a
  **Jira issue** chip and an `Issue…` prompt. Esc or Backspace on an empty field goes back.
- `{…}` takes any one-word name, shown as the prompt; the argument is URL-encoded (a space is
  `%20`). A URL without `https://` gets it; other schemes (`obsidian://`, `mailto:`) open in
  their app. `name` is optional (default: the keyword).
- Quicklinks you use rank first, like apps (frecency key `link:<keyword>`).

**Aliases** are your own words for things tomboy can already find:

```toml
[aliases]
bubu = "chromium"        # an app
lk = "lock screen"       # an Omarchy command
tb = "gh tomboy"         # a quicklink with its argument
```

Typing exactly `bubu` puts the best match for `chromium` first (its row says `alias bubu`),
so Enter opens it; the usual results follow. An alias whose target matches nothing does
nothing.

Or ask the agent: "add a quicklink yt for youtube search", "remove the jira quicklink",
"alias bubu to chromium".

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
- **Use it**: Enter (or click) copies it and closes tomboy; paste with Ctrl+V (on macOS the
  daemon pastes it for you, see [Install on macOS](#install-on-macos)). Picks feed
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
  can paste it with Ctrl+V (the macOS daemon pastes it for you). Text goes back as text, images as images, copied files as files
  (`text/uri-list`), so a file manager pastes the files themselves.
- **Esc** clears the search, then closes (or returns to the launcher if you came from there).

### Where the history comes from

| Desktop | History | Recorded by |
| --- | --- | --- |
| Omarchy | `~/.local/state/omarchy/clipboard-history.json` | Omarchy's shell, already running; nothing to set up |
| Other Wayland | `~/.local/state/tomboy/clipboard-history.json` | `tomboy clipboard-watch` (runs `wl-paste --watch`; needs `wl-clipboard`) |
| macOS | `~/.local/state/tomboy/clipboard-history.json` | `tomboy daemon` (or `tomboy clipboard-watch`): watches the pasteboard for text, images and copied files; skips copies password managers mark private |

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

## Extensions

Extensions add commands to tomboy, such as **Hacker News**: type `hn`, pick a story, Enter
opens it. They come from the [`extensions/`](extensions) folder of this repo.

**Install from inside tomboy.** Type `install extension` or `extensions` and press Enter on
**Install Extension** (or run `tomboy extensions` to open it directly):

- The store lists **Installed** and **Available** extensions with version and status
  (`Installed`, `Update 1.1.0`, `Needs a newer tomboy`). Type to filter.
- The right pane shows the picked one's README, author, licence, commands and permissions in
  plain words ("Connects to hn.algolia.com").
- **Enter** on an available extension shows a consent screen with its permissions; Enter
  installs, Esc cancels. Its commands are in the launcher at once.
- **Enter** on an installed one opens its first command. **Ctrl/⌘+K** opens the action panel:
  Update, Uninstall, Disable/Enable, View Source on GitHub, Show Folder.
- An update that asks for a new permission shows the consent screen again.
- Offline, the store lists only what is installed and says "Couldn't reach the extension
  registry". An installed extension that was taken out of the registry is marked
  `Removed from registry`; uninstalling it is up to you.

Inside an extension: Enter runs the first action, Ctrl/⌘+K shows the rest, Esc goes back.

**From a terminal**, `tomboy ext …` does the same:

| Command | What it does |
| --- | --- |
| `tomboy ext list` | Installed extensions |
| `tomboy ext available` | Extensions in the registry |
| `tomboy ext install <id> [--yes]` | Install one (asks to accept its permissions) |
| `tomboy ext update [<id>] [--yes]` | Update one or all |
| `tomboy ext uninstall <id>` | Remove one |
| `tomboy ext enable <id>` / `disable <id>` | Turn one on or off |
| `tomboy ext new` / `dev` / `check` / `index` | For writing extensions, see below |

**Preferences** go in `config.toml` under the extension's id; each extension's README lists
its own:

```toml
[extensions.hackernews]
feed = "show_hn"   # front_page (default), newest, show_hn, ask_hn
```

**Files**:

| Path | What |
| --- | --- |
| `~/.local/share/tomboy/extensions/<id>/` | Installed extensions, as in the repo |
| `~/.local/share/tomboy/extensions/installed.json` | Version, source, on/off and the permissions you accepted, per extension |
| `~/.local/state/tomboy/ext/<id>/storage.json` | What an extension saved (`api.storage`) |
| `~/.cache/tomboy/ext/<id>/` | An extension's cache (`api.cache`); safe to delete |
| `~/.cache/tomboy/extensions-index.json` | The registry index, cached for an hour |

**Security.** Extensions are Luau scripts in a sandbox: no file or process access, and the
network only reaches the https hosts the extension declares, which you accept before
installing. Every extension, and every change to one, is reviewed in a pull request here
before the store offers it, and installs are checked against the SHA-256 of each file.

**Write one**: `tomboy ext new my-ext`, then `tomboy ext dev my-ext` (Ctrl/⌘+R reloads) and
`tomboy ext check my-ext`. The guide and API reference are in
[docs/extensions.md](docs/extensions.md); to publish it, see
[CONTRIBUTING.md](CONTRIBUTING.md#extensions). Or ask the agent: "make me a tomboy extension
that lists my GitHub notifications".

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
| `src/quicklinks.rs` | Quicklinks: keyword matching and URL expansion |
| `src/todo.rs` | Daily todo lists: Markdown files, carry-over, add/toggle/remove |
| `src/config.rs` | `~/.config/tomboy/config.toml`: loading and defaults |
| `src/skill.md` | What the agent knows about tomboy's config (sent with every agent run) |
| `src/sys.rs` | OS commands: open URL, clipboard, detached spawn |
| `src/calc.rs` | Expression evaluator |
| `src/url.rs` | URL detection and opening |
| `src/platform.rs` | Platform detection (Omarchy / generic Linux / macOS) |
| `src/mac/` | macOS: native pasteboard, paste into the front app, LaunchAgent, system commands |
| `src/omarchy.rs` | Omarchy menu loading (JSONC), conditions, search, running |
| `src/ext/` | Extensions: manifest, registry index, install, the Luau runtime and `@tomboy/*` API (`tomboy.luau`), `tomboy ext` CLI |
| `src/extui.rs` | Extension views, the store, the consent screen and the action panel (Ctrl/⌘+K) |
| `extensions/` | The extension registry: one folder per extension, and the generated `index.json` |

Type-check the macOS build from Linux (blake3's C code needs the macOS SDK, so use its pure-Rust path).
This currently stops at `ring` (ureq's TLS), whose C code also needs the SDK; build on a Mac for a full check.
The other way round, from a Mac, `ring` needs a C compiler for Linux: `clang` with stub `assert.h`
and `string.h` in a folder on `-I` is enough for `cargo check`:

```sh
rustup target add aarch64-apple-darwin
CARGO_FEATURE_PURE=1 cargo check --target aarch64-apple-darwin

# from a Mac:
rustup target add x86_64-unknown-linux-gnu
CC_x86_64_unknown_linux_gnu=clang AR_x86_64_unknown_linux_gnu=ar \
  CFLAGS_x86_64_unknown_linux_gnu="--target=x86_64-unknown-linux-gnu -ffreestanding -I$STUBS" \
  cargo check --target x86_64-unknown-linux-gnu --all-targets
```

## Headless checks

```sh
cargo test
cargo test live_claude -- --ignored   # runs the real claude CLI (tiny paid request)
cargo run -- --snap shot.png --scene calc   # scenes: apps calc url web omarchy system chat submenu units time dev kill currency quicklink quicklink-arg alias todo todo-command todo-add store ext-hackernews
cargo run -- --tree tree.json --scene apps
```
