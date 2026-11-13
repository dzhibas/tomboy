--!strict
-- tomboy's side of every extension VM, run once before the sandbox freezes the globals.
-- It builds `@tomboy/ui` and `@tomboy/api` around the host functions Rust passes in, turns
-- the views an extension returns into plain data (functions become callback ids), and runs
-- commands and actions as tasks so they may wait on `api.http.get`.
local host, preferences, runSignal, actionSignal = ...

local emit: (string, any?) -> () = host.emit

-- Callbacks the views handed to tomboy, by id: `ui.run`, `ui.push`, a list's `onSearch`.
local callbacks: { [number]: { kind: string, fn: (...any) -> any } } = {}
local nextId = 0
local module: { [string]: any } = {}

local function remember(kind: string, fn: (...any) -> any): number
	nextId += 1
	callbacks[nextId] = { kind = kind, fn = fn }
	return nextId
end

local function str(v: any): string?
	if v == nil then
		return nil
	end
	return tostring(v)
end

local function action(a: any)
	if type(a) ~= "table" then
		error("an action must come from ui.open, ui.copy, ui.push or ui.run", 0)
	end
	if a.kind == "open" then
		return { kind = "open", title = str(a.title) or "Open", url = tostring(a.url) }
	elseif a.kind == "copy" then
		return { kind = "copy", title = str(a.title) or "Copy", text = tostring(a.text) }
	elseif a.kind == "push" or a.kind == "run" then
		return { kind = "callback", title = str(a.title) or "Open", id = remember(a.kind, a.fn) }
	end
	error(`unknown action kind {a.kind}`, 0)
end

local function actions(list: any)
	local out = {}
	for _, a in list or {} do
		table.insert(out, action(a))
	end
	return out
end

local function strings(list: any)
	local out = {}
	for _, s in list or {} do
		table.insert(out, tostring(s))
	end
	return out
end

-- A view as plain data, the shape `src/ext/view.rs` decodes.
local function serialize(view: any)
	if type(view) ~= "table" or (view.type ~= "list" and view.type ~= "detail") then
		error("a command must return ui.list{...}, ui.detail{...} or nil", 0)
	end
	if view.type == "detail" then
		return {
			type = "detail",
			title = str(view.title),
			markdown = tostring(view.markdown or ""),
			actions = actions(view.actions),
		}
	end
	local items = {}
	for i, item in view.items or {} do
		table.insert(items, {
			id = str(item.id) or tostring(i),
			title = tostring(item.title or ""),
			subtitle = str(item.subtitle),
			icon = str(item.icon),
			accessories = strings(item.accessories),
			keywords = strings(item.keywords),
			detail = str(item.detail),
			actions = actions(item.actions),
		})
	end
	return {
		type = "list",
		title = str(view.title),
		items = items,
		loading = view.loading == true,
		emptyText = str(view.emptyText),
		searchPlaceholder = str(view.searchPlaceholder),
		filtering = view.filtering ~= false and view.onSearch == nil,
		search = if type(view.onSearch) == "function" then remember("search", view.onSearch) else nil,
	}
end

local ui = {}

function ui.list(spec: any)
	spec = spec or {}
	spec.type = "list"
	return spec
end

function ui.detail(spec: any)
	spec = spec or {}
	spec.type = "detail"
	return spec
end

function ui.open(url: string, title: string?)
	return { kind = "open", url = url, title = title }
end

function ui.copy(text: string, title: string?)
	return { kind = "copy", text = text, title = title }
end

-- An action that opens the view `build()` returns on top of the current one (Esc goes back).
function ui.push(build: () -> any, title: string?)
	assert(type(build) == "function", "ui.push needs a function that returns a view")
	return { kind = "push", fn = build, title = title }
end

-- An action that calls `fn()`; it may wait, show a toast, update the view or close tomboy.
function ui.run(title: string, fn: () -> ())
	assert(type(fn) == "function", "ui.run needs a function")
	return { kind = "run", fn = fn, title = title }
end

-- Replaces the view on screen, e.g. a loading list with the loaded one.
function ui.update(view: any)
	emit("view", serialize(view))
end

function ui.toast(text: string)
	emit("toast", tostring(text))
end

function ui.close()
	emit("close")
end

local api = {}

api.http = {}
function api.http.get(url: string, options: { headers: { [string]: string }? }?)
	local res = host.httpGet(tostring(url), options and options.headers or {})
	if res == nil then
		error("api.http.get: no response", 2)
	end
	if res.error then
		error(`api.http.get {url}: {res.error}`, 2)
	end
	return res
end

api.json = { decode = host.jsonDecode, encode = host.jsonEncode }
api.storage = { get = host.storageGet, set = host.storageSet, remove = host.storageRemove }
api.cache = { get = host.cacheGet, set = host.cacheSet }
api.time = { now = host.timeNow, ago = host.timeAgo }
api.preferences = table.freeze(preferences)
function api.log(...)
	print(...)
end

-- Runs `fn(...)` and sends what it returns: a view (`as` = "view" or "push") or nothing.
local function invoke(fn: (...any) -> any, as: string?, ...)
	local ok, result = pcall(fn, ...)
	if not ok then
		emit("error", tostring(result))
	elseif result ~= nil and as ~= nil then
		local sent, err = pcall(function()
			emit(as, serialize(result))
		end)
		if not sent then
			emit("error", tostring(err))
		end
	end
	emit("done")
end

runSignal:Connect(function(command: string, ctx: any)
	local fn = module[command]
	if type(fn) ~= "function" then
		emit("error", `init.luau has no function for command "{command}"`)
		emit("done")
		return
	end
	ctx = ctx or {}
	ctx.preferences = api.preferences
	invoke(fn, "view", ctx)
end)

actionSignal:Connect(function(id: number, arg: any)
	local cb = callbacks[id]
	if not cb then
		emit("done")
		return
	end
	if cb.kind == "push" then
		invoke(cb.fn, "push")
	elseif cb.kind == "search" then
		invoke(cb.fn, "view", arg or "")
	else
		invoke(cb.fn, nil)
	end
end)

return {
	ui = table.freeze(ui),
	api = table.freeze(api),
	attach = function(m: any)
		module = if type(m) == "table" then m else {}
	end,
	has = function(command: string): boolean
		return type(module[command]) == "function"
	end,
}
