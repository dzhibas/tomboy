# Writing tomboy extensions

An extension adds commands to tomboy. It is a folder with a manifest (`tomboy.toml`) and a
Luau script (`init.luau`). A command shows a list or a Markdown page inside tomboy, with
actions such as open a URL or copy text. tomboy draws every view with its own rows, so an
extension looks and behaves like the rest of the launcher.

Extensions run in a sandbox. Luau has no file, process or network access of its own:
everything outside the script goes through `@tomboy/api`, and the network only reaches the
hosts the manifest lists. Users see those permissions before they install.

Extensions live in the [`extensions/`](../extensions) folder of tomboy's repository and reach
users through pull requests; see [CONTRIBUTING.md](../CONTRIBUTING.md#extensions).

- [Quick start](#quick-start)
- [Folder layout](#folder-layout)
- [`tomboy.toml`](#tomboytoml)
- [`init.luau`](#initluau)
- [`@tomboy/ui`](#tomboyui)
- [`@tomboy/api`](#tomboyapi)
- [Async: loading data](#async-loading-data)
- [Keyboard](#keyboard)
- [Preferences](#preferences)
- [Sandbox and limits](#sandbox-and-limits)
- [Developing](#developing)
- [API versions](#api-versions)
- [Walkthrough: Hacker News](#walkthrough-hacker-news)

## Quick start

```sh
cd extensions                     # in your fork of tomboy
tomboy ext new my-ext             # writes my-ext/tomboy.toml, init.luau, README.md
tomboy ext dev my-ext             # opens tomboy on its first command
```

Edit `init.luau`, press Ctrl+R (⌘R on macOS) in tomboy to reload, and run
`tomboy ext check my-ext` before you open a pull request.

## Folder layout

```
extensions/my-ext/
  tomboy.toml     the manifest (required)
  init.luau       the entry script (required)
  README.md       shown in the store's detail pane (required for the registry)
  *.luau          more modules, loaded with require("@self/name")
  icon.png        optional, at most 64 KB
```

- The folder name is the extension's `id`.
- Allowed file types: `.toml`, `.luau`, `.md`, `.png`, `.json`. Anything else, a symlink or a
  path that leaves the folder fails `tomboy ext check` and the installer.
- All files together: at most 1 MB.
- `icon.png` is accepted but not drawn yet; tomboy shows the manifest's `icon` glyph, or a
  puzzle piece when there is none.

## `tomboy.toml`

```toml
id = "hackernews"
name = "Hacker News"
version = "1.0.0"
api = 1
description = "Front page, newest, Show HN and Ask HN stories from Hacker News"
author = "dzhibas"
license = "MIT"
platforms = ["linux", "macos"]
icon = "\U000F0CC6"

[[commands]]
id = "front-page"
title = "Hacker News"
subtitle = "Top stories, points and comments"
mode = "view"
keywords = ["hn", "news", "ycombinator", "hacker"]

[permissions]
net = ["hn.algolia.com"]

[[preferences]]
id = "feed"
type = "dropdown"
title = "Feed"
description = "Which stories to list"
default = "front_page"
options = ["front_page", "newest", "show_hn", "ask_hn"]
```

The manifest is parsed strictly: an unknown key is an error, so a typo never drops a
permission or a command silently.

### Top level

| Key | Required | What it is |
| --- | --- | --- |
| `id` | yes | Lowercase letters, digits and `-` (not at the start or end), at most 64 characters. Must equal the folder name. |
| `name` | yes | Shown in the store and as the label on the extension's launcher rows. |
| `version` | yes | `major.minor.patch`, numbers only (`1.0.0`). Must increase with every change to the folder. |
| `api` | yes | The extension API version it is written for. `1` today. See [API versions](#api-versions). |
| `description` | yes | One line. The store shows it; launcher rows show it when a command has no `subtitle`. |
| `author` | yes | Your GitHub login. |
| `license` | yes | An SPDX id, such as `MIT` or `Apache-2.0`. |
| `platforms` | no | `["linux"]`, `["macos"]` or both. Left out: both. On another platform the store shows "Not available on …". |
| `icon` | no | One Nerd Font glyph, written as an escape (`"\U000F0CC6"`, `""`). |
| `[[commands]]` | yes | At least one. |
| `[permissions]` | no | What the extension may reach. Left out: nothing. |
| `[[preferences]]` | no | Settings the user can change in `config.toml`. |

### `[[commands]]`

Each command is one row in the launcher.

| Key | Required | What it is |
| --- | --- | --- |
| `id` | yes | Same rules as the extension id; unique in the manifest. The key of its function in `init.luau`. |
| `title` | yes | The row's title, and what the launcher matches first. |
| `subtitle` | no | The row's second line (default: the extension's `description`). |
| `mode` | no | `view` (default) or `action`. See below. |
| `keywords` | no | More words the launcher matches. Typing one exactly ranks the command above other extension commands. |

- **`view`**: the command returns a view (`ui.list` or `ui.detail`) and tomboy shows it.
- **`action`**: the command does something and returns `nil`. When it finishes without a
  view, a toast or an error, tomboy closes. A toast or an error keeps the window open so the
  user can read it.

### `[permissions]`

| Key | What it is |
| --- | --- |
| `net` | Host names `api.http.get` may reach, exactly as written: `"hn.algolia.com"`, not `"algolia.com"`, no scheme, path or port. Every subdomain you use needs its own entry. |

The store and the consent screen show each permission in plain words ("Connects to
hn.algolia.com"). An update that adds a host asks the user again before it installs.

Copying text and opening URLs are built-in actions and need no permission.

### `[[preferences]]`

Settings the user writes in `config.toml` (see [Preferences](#preferences)).

| Key | Required | What it is |
| --- | --- | --- |
| `id` | yes | Letters, digits, `-` or `_`; unique in the manifest. The key in `config.toml` and in `api.preferences`. |
| `type` | yes | `text`, `checkbox` or `dropdown`. |
| `title` | yes | A short name. |
| `description` | no | What it changes; list the choices of a dropdown here, since this is what the README and store show. |
| `default` | no | A string for `text` and `dropdown` (one of `options`), `true` or `false` for `checkbox`. |
| `options` | for `dropdown` | The allowed values. |
| `required` | no | `true` or `false` (default). Parsed but not enforced yet; check for an empty value yourself. |

Without a `default`, a `checkbox` is `false`, a `dropdown` its first option and a `text` `""`.

## `init.luau`

`init.luau` returns a table with one function per command id:

```luau
--!strict
local api = require("@tomboy/api")
local ui = require("@tomboy/ui")

return {
    ["my-command"] = function(ctx)
        return ui.list({
            items = {
                { id = "1", title = "Hello", actions = { ui.copy("Hello", "Copy") } },
            },
        })
    end,
}
```

The function gets `ctx`:

| Field | What it is |
| --- | --- |
| `ctx.command` | The command id being run. |
| `ctx.query` | Text typed with the command. Today always `""`: commands open from their row. |
| `ctx.preferences` | The preference values, the same table as `api.preferences`. |

It returns a view (`view` mode) or `nil` (`action` mode). An error it raises is shown in
tomboy's footer as `<extension name>: <message>`; tomboy keeps running.

The command function runs as a task, so it may call `api.http.get` directly and wait. While
it waits the window shows "Loading…". To show something at once, return a loading list and
fetch in a task (see [Async](#async-loading-data)).

`init.luau` runs once when the command is opened (one VM per extension, kept while the
window is open). Top-level code should only define functions; it must not call the network.

More files: `require("@self/util")` from `init.luau` loads `util.luau` in the same folder.
Nothing outside the folder can be required.

## `@tomboy/ui`

```luau
local ui = require("@tomboy/ui")
```

Views are plain tables; tomboy draws them. Functions inside them (`ui.run`, `ui.push`,
`onSearch`) are kept by tomboy and called back.

### Views

#### `ui.list(spec)`

| Field | Type | What it is |
| --- | --- | --- |
| `items` | `{ item }` | The rows. |
| `title` | `string?` | The chip next to the search field (default: the extension name). |
| `loading` | `boolean?` | Shows a loading state while you fetch. Default `false`. |
| `emptyText` | `string?` | What an empty list says (also a good place for an error). |
| `searchPlaceholder` | `string?` | The search field's placeholder. |
| `filtering` | `boolean?` | Default `true`: tomboy filters `items` as the user types (every word must appear in the title, subtitle or keywords). `false` shows every item. |
| `onSearch` | `(text: string) -> view?` | Instead of filtering, called with the field's text on every edit. Return the new view (or call `ui.update`). Setting it turns `filtering` off. |

Each item:

| Field | Type | What it is |
| --- | --- | --- |
| `id` | `string?` | Stable id (default: its position). |
| `title` | `string` | The row's title. |
| `subtitle` | `string?` | Second line. |
| `icon` | `string?` | A Nerd Font glyph. |
| `accessories` | `{ string }?` | Short labels on the right (`"▲ 512"`, `"3h"`). |
| `keywords` | `{ string }?` | More words filtering matches. |
| `detail` | `string?` | Markdown shown in a pane beside the list for the picked row. |
| `actions` | `{ action }` | What the row can do. The first is Enter; all are in the action panel. |

Numbers in `title`, `accessories` and so on are turned into strings.

#### `ui.detail(spec)`

| Field | Type | What it is |
| --- | --- | --- |
| `markdown` | `string` | The page, as Markdown. |
| `title` | `string?` | The chip next to the search field. |
| `actions` | `{ action }?` | First = Enter; all are in the action panel. |

### Actions

| Call | What it does |
| --- | --- |
| `ui.open(url, title?)` | Opens `url` in the browser (or its app, for other schemes) and closes tomboy. Title default "Open". |
| `ui.copy(text, title?)` | Copies `text` and closes tomboy (on macOS it pastes into the app in front when `[clipboard] paste` is on). Title default "Copy". |
| `ui.push(build, title?)` | Calls `build()` and shows the view it returns on top of the current one. Esc goes back. Title default "Open". |
| `ui.run(title, fn)` | Calls `fn()`. It may wait, toast, update the view or close tomboy. Its return value is ignored. Note the order: title first. |

`ui.push` needs a function and `ui.run` needs a function; anything else is an error when the
action is made. An item's `actions` may only contain values from these four calls.

### Effects

| Call | What it does |
| --- | --- |
| `ui.update(view)` | Replaces the view on top of the stack (or shows it, when there is none yet). Use it after a load. |
| `ui.toast(text)` | Shows `text` in the footer. |
| `ui.close()` | Closes tomboy. |

`ui.update` replaces whatever view is on top. If a background task of an earlier view calls it
after the user pushed a new view, it replaces the pushed one. Keep loads short, or check your
own state before updating.

## `@tomboy/api`

```luau
local api = require("@tomboy/api")
```

### `api.http.get(url, options?) -> { status, body }`

```luau
local res = api.http.get("https://hn.algolia.com/api/v1/search?tags=front_page", {
    headers = { Accept = "application/json" },
})
if res.status ~= 200 then error(`HTTP {res.status}`, 0) end
local data = api.json.decode(res.body)
```

- `url` must start with `https://`. Plain `http://`, other schemes and URLs with
  credentials (`user@host`) are refused.
- The host must be in `[permissions] net`, matched exactly (case does not matter). Otherwise
  it raises `api.http.get: <host> is not in this extension's permissions (add it to
  [permissions] net in tomboy.toml)`.
- `options.headers`: a table of header name → string value.
- Waits for the response without blocking tomboy (the request runs on a background thread).
  Call it from a command, an action or a task, never at the top level of a module.
- Returns `{ status: number, body: string }` for any HTTP status; a 404 or 500 is not an
  error, so check `status`.
- Redirects are not followed: a 3xx comes back as it is.
- Raises `api.http.get <url>: <reason>` when there is no response: DNS or TLS failure,
  timeout (10 seconds for the whole request), or a body over 5 MB.
- The request's User-Agent is `tomboy-extension`.

### `api.json`

| Call | What it does |
| --- | --- |
| `api.json.decode(text) -> any` | Parses JSON into tables. JSON `null` becomes `nil` (so a field holding null is missing). Raises `api.json.decode: …` on invalid JSON. |
| `api.json.encode(value) -> string` | Turns a table, string, number or boolean into compact JSON. An empty table is `{}`; a table with keys 1..n is an array. |

### `api.storage`

Small values that persist between runs: settings the user picked, read markers, a token the
user pasted.

| Call | What it does |
| --- | --- |
| `api.storage.get(key) -> any?` | The value stored under `key`, or `nil`. |
| `api.storage.set(key, value)` | Stores a JSON-able value (table, string, number, boolean). |
| `api.storage.remove(key)` | Deletes `key`. |

All keys of one extension share one file, `~/.local/state/tomboy/ext/<id>/storage.json`
(`$XDG_STATE_HOME` honoured), at most 1 MB as JSON; a `set` that would go over raises
`api.storage: over 1024 KB`. Each `set` rewrites the file, so don't call it in a tight loop.

### `api.cache`

Strings that may be thrown away: API responses.

| Call | What it does |
| --- | --- |
| `api.cache.get(key, maxAgeSecs?) -> string?` | The cached string, or `nil` when there is none or it is older than `maxAgeSecs`. Without `maxAgeSecs`, any age. |
| `api.cache.set(key, value)` | Stores the string `value`. Failures are ignored. |

Files live in `~/.cache/tomboy/ext/<id>/`, one per key. Values must be strings: store
`res.body` as it came, or `api.json.encode(t)`.

### `api.preferences`

A read-only table of the extension's preference values: the user's `[extensions.<id>]`
values over the manifest's defaults. `ctx.preferences` is the same table.

```luau
local feed = tostring(api.preferences.feed or "front_page")
```

### `api.time`

| Call | What it does |
| --- | --- |
| `api.time.now() -> number` | Seconds since the Unix epoch. |
| `api.time.ago(epochSecs) -> string` | How long ago, short: `now`, `5m`, `3h`, `2d`, `3w`, `4mo`, `2y`. A time in the future is `now`. |

### `api.log(...)`

Writes its arguments to the extension's log, like `print`. `tomboy ext check` prints what
`init.luau` logged while it loaded.

### Also available

- **`task`**: `task.spawn(fn, ...)`, `task.defer(fn, ...)`, `task.delay(secs, fn, ...)`,
  `task.wait(secs)`, `task.cancel(thread)`.
- The Luau standard library: `string`, `table`, `math`, `utf8`, `bit32`, `buffer`,
  `coroutine`, string interpolation, `--!strict` types. Luau's `os` has only `time`, `clock`,
  `date` and `difftime`.
- **Not available**: `io`, any file or process access, and `require` of anything outside the
  folder.

## Async: loading data

`api.http.get` waits. The command function is a task, so this works, but the user sees
"Loading…" until it returns:

```luau
["simple"] = function(ctx)
    local res = api.http.get("https://example.com/items.json")
    return ui.list({ items = toItems(api.json.decode(res.body)) })
end,
```

Better: return a loading list at once, fetch in a task, and replace it with `ui.update`:

```luau
["better"] = function(ctx)
    task.defer(function()
        local ok, result = pcall(api.http.get, "https://example.com/items.json")
        if ok and result.status == 200 then
            ui.update(ui.list({ items = toItems(api.json.decode(result.body)) }))
        else
            ui.update(ui.list({ items = {}, emptyText = "Couldn't load items" }))
        end
    end)
    return ui.list({ loading = true, items = {} })
end,
```

- `task.defer` runs the function after the command has returned, so the loading list is on
  screen first.
- Wrap the work in `pcall`. An error inside a task you started is not shown to the user; it
  only goes to the log. An error in the command function itself, or in a `ui.run` or
  `ui.push` callback, is shown in the footer.
- Cache the response (`api.cache`) and show the cached list right away; fetch only when it is
  stale. The Hacker News sample does this.
- The same works in a `ui.run` action: `ui.run("Refresh", function() ui.update(loading); load() end)`.

Each entry into the VM (a command, a callback, one resumed step of a task) may run for at most
250 ms of Luau code. Waiting on `api.http.get` or `task.wait` does not count. A long loop is
stopped with an error; split heavy work with `task.wait()`.

## Keyboard

| Key | In an extension view |
| --- | --- |
| Type | Filters the list (or calls `onSearch`). |
| ↑ / ↓ | Pick a row; the `detail` pane follows. |
| Enter | Runs the picked row's first action (in a detail, its first action). |
| Ctrl+K (⌘K) | Opens the action panel with all of the row's actions; ↑/↓ and Enter pick one, Esc or Ctrl+K closes it. |
| Esc | Clears the field; on an empty field goes back one view, then leaves the extension (closes tomboy when it was opened directly). |
| Backspace | On an empty field, the same as Esc. |
| Ctrl+R (⌘R) | Only under `tomboy ext dev`: reload the folder and run the command again. |

So put the action users want most first, and give every action a clear title: the action
panel lists them by title.

## Preferences

Users set preference values in `~/.config/tomboy/config.toml`, under the extension's id:

```toml
[extensions.hackernews]
feed = "show_hn"
```

- Keys are the preference ids from the manifest; others are ignored.
- Values are read when tomboy opens.
- tomboy does not check the value against `options` or the type. Treat values defensively
  (`tostring(api.preferences.feed or "front_page")`).
- Document every preference, with its choices, in your README.

## Sandbox and limits

| Limit | Value |
| --- | --- |
| Network | `https://` only, exact hosts from `[permissions] net`, no redirects followed |
| HTTP timeout | 10 s per request, from connect to the end of the body |
| HTTP body | 5 MB |
| `api.storage` | 1 MB per extension |
| Luau time | 250 ms per entry into the VM (a command, callback or task step) |
| Luau heap | 64 MB per extension |
| Files | `.toml .luau .md .png .json`, 1 MB in all, `icon.png` ≤ 64 KB |

Nothing an extension does can crash tomboy. A script error, a missing permission, a runaway
loop or a memory blow-up ends that call with an error, and tomboy carries on.

## Developing

| Command | What it does |
| --- | --- |
| `tomboy ext new <id> [<dir>]` | Writes `<dir>/<id>/` (default: the current folder) with `tomboy.toml`, `init.luau` and `README.md`. |
| `tomboy ext dev <dir>` | Checks the folder, then opens tomboy on its first command, running it from `<dir>` (an installed copy with the same id is ignored). Ctrl+R (⌘R) reloads after you save. Its commands are also in the launcher for that window. |
| `tomboy ext check <dir>` | Validates the folder: file types and sizes, the manifest, `id` equal to the folder name, `init.luau` loads in the sandbox and returns a function for every command. Prints `ok: <id> <version> (<n> files)`; exits non-zero on a problem. It does not run the commands. |
| `tomboy ext index [--check] [<dir>]` | Writes `<dir>/index.json` (default `extensions`) from every extension folder in it; `--check` only verifies it is up to date. Run from the repository root. |

A loop that works:

1. `tomboy ext dev extensions/my-ext` in a terminal.
2. Edit, save, Ctrl+R in tomboy. Errors show in the footer.
3. `tomboy ext check extensions/my-ext` and `tomboy ext index` when it works.

Try the store against your checkout before you open a pull request: set
`[extensions] registry = "/path/to/your/tomboy"` (a folder, `file://…` or an `https://` base
that has `extensions/index.json` under it), open tomboy, type `extensions`.

## API versions

- `api = 1` is the only version today. Within a version, changes are additive only: new
  functions, new optional fields. Code written for `api = 1` keeps working.
- A breaking change is a new version, `api = 2`. tomboy keeps running `api = 1` extensions
  for at least two releases after that.
- An extension whose `api` is higher than the running tomboy supports still appears in the
  store, marked "Needs a newer tomboy", and cannot be installed.
- Use the lowest `api` that has what you need.

## Walkthrough: Hacker News

[`extensions/hackernews/`](../extensions/hackernews) is the sample to copy from. It lists
stories from the Algolia Hacker News API with points, comments and age, a detail pane, five
actions, a preference and a cache.

**Manifest.** One `view` command, `front-page`, with keywords `hn`, `news`, `ycombinator`,
`hacker`. One host, `hn.algolia.com`. One dropdown preference, `feed`.

**Rows.** `row(hit, refresh)` turns one API hit into a list item:

```luau
return {
    id = hit.objectID,
    title = title,
    subtitle = site(hit.url),                         -- the story's domain
    accessories = { `▲ {points}`, `💬 {count}`, age }, -- age from api.time.ago
    detail = detail,                                  -- Markdown for the side pane
    actions = {
        ui.open(link, "Open Story"),                  -- Enter
        ui.open(comments, "Open Comments"),           -- the rest: Ctrl/⌘+K
        ui.copy(link, "Copy Link"),
        ui.copy(`[{title}]({link})`, "Copy as Markdown"),
        refresh,
    },
}
```

**One list builder.** `list(feed, body, loading, note)` builds the whole view from a response
body (or `nil`). Every state, loading, loaded, failed, goes through it, so they look the same.
It also makes the shared `Refresh` action:

```luau
local refresh = ui.run("Refresh", function()
    ui.update(list(feed, body, true))   -- keep the stories, show loading
    load(feed, body)
end)
```

**Loading in the background.** `load` fetches after the current view is up and replaces it:

```luau
load = function(feed, previous)
    task.defer(function()
        local ok, result = pcall(fetch, feed)
        if ok then
            ui.update(list(feed, result, false))
        else
            local message = `Couldn't load Hacker News: {result}`
            ui.update(list(feed, previous, false, message))
            ui.toast(message)
        end
    end)
end
```

`fetch` calls `api.http.get`, raises on a non-200 status and stores the body in
`api.cache`. On failure the previous stories stay on screen and the footer says why, which is
also how it behaves offline.

**The command.** A fresh cache (under five minutes) shows at once with no request. Otherwise
it shows the stale cache, or an empty loading list, and starts `load`:

```luau
["front-page"] = function(ctx)
    local feed = tostring(ctx.preferences.feed or "front_page")
    local fresh = api.cache.get("feed:" .. feed, FRESH)
    if fresh then
        return list(feed, fresh, false)
    end
    local stale = api.cache.get("feed:" .. feed)
    load(feed, stale)
    return list(feed, stale, true)
end,
```

Things to copy: return quickly, load in `task.defer` inside `pcall`, keep old data on screen
when a refresh fails, put the main action first, and document the preference in the README.
