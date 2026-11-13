//! One running extension: a sandboxed Luau VM (`rune_luau::Script`) with `@tomboy/ui` and
//! `@tomboy/api` bound in. Commands and actions run as tasks, so they may wait on the
//! network; what they show comes back as [`Event`]s from [`Runtime::run`], [`Runtime::action`]
//! and [`Runtime::poll`].
//!
//! The sandbox: Luau has no file, process or network access of its own. Everything an
//! extension does outside its VM goes through the host functions below, and each checks the
//! manifest's permissions. `rune-luau` stops any entry that runs past 250 ms and caps the heap.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use rune::{Tx, Waker};
use rune_luau::mlua::{self, LuaSerdeExt, MultiValue, Table, Value};
use rune_luau::{Data, Script, ScriptOptions, Sources};
use sha2::{Digest, Sha256};

use super::manifest::{Manifest, Mode};
use super::view::View;

const LIB: &str = include_str!("tomboy.luau");
/// Where `tomboy.luau`'s table waits for the extension's module.
const LIB_KEY: &str = "tomboy.lib";
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const HTTP_MAX_BYTES: u64 = 5 * 1024 * 1024;
/// Most bytes `api.storage` keeps per extension.
const STORAGE_MAX_BYTES: usize = 1024 * 1024;
const MEMORY_LIMIT: usize = 64 << 20;

/// What a script asked tomboy to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Replace the view on screen.
    View(View),
    /// Show a view on top (Esc goes back).
    Push(View),
    Toast(String),
    Close,
    /// A command or action finished.
    Done,
    Error(String),
}

/// Overrides for tests and headless scenes: canned HTTP responses and private directories.
#[derive(Default, Clone)]
pub struct Options {
    /// URL → body, answered with status 200 instead of the network (after the permission check).
    pub fixtures: BTreeMap<String, String>,
    /// Instead of `~/.local/state/tomboy/ext/<id>` and `~/.cache/tomboy/ext/<id>`.
    pub state_dir: Option<PathBuf>,
    pub cache_dir: Option<PathBuf>,
}

pub struct Runtime {
    pub manifest: Manifest,
    script: Script,
    outbox: Rc<RefCell<Vec<(String, Data)>>>,
}

impl Runtime {
    /// Starts the extension in `dir` with these preference values. A script that fails to
    /// compile or start is an `Err` with the reason.
    pub fn start(manifest: Manifest, dir: &Path, prefs: BTreeMap<String, toml::Value>) -> Result<Runtime, String> {
        Runtime::start_with(manifest, dir, prefs, Options::default())
    }

    pub fn start_with(manifest: Manifest, dir: &Path, prefs: BTreeMap<String, toml::Value>, opts: Options) -> Result<Runtime, String> {
        let outbox: Rc<RefCell<Vec<(String, Data)>>> = Rc::default();
        let sources = Sources::from_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let ctx = Ctx { manifest: manifest.clone(), outbox: outbox.clone(), prefs, opts };
        let options = ScriptOptions::new()
            .memory_limit(MEMORY_LIMIT)
            .setup(move |host| bind(host, &ctx))
            .start(|lua, module, _| {
                let lib: Table = lua.named_registry_value(LIB_KEY)?;
                lib.get::<mlua::Function>("attach")?.call::<()>(module.clone())
            });
        let script = Script::with_options(&manifest.id, sources, options);
        if let Some(err) = script.error() {
            return Err(err.message.clone());
        }
        Ok(Runtime { manifest, script, outbox })
    }

    pub fn set_waker(&self, waker: Waker) {
        self.script.set_waker(waker);
    }

    /// Runs command `id`; `query` is what was typed after the command, if anything.
    pub fn run(&self, id: &str, query: &str) -> Vec<Event> {
        let mut ctx = BTreeMap::new();
        ctx.insert("command".to_string(), Data::String(id.into()));
        ctx.insert("query".to_string(), Data::String(query.into()));
        self.script.fire("run", [Data::String(id.into()), Data::Map(ctx)]);
        self.drain()
    }

    /// Fires a view's callback (`ui.run`, `ui.push`, `onSearch` with the typed text).
    pub fn action(&self, id: i64, arg: Option<&str>) -> Vec<Event> {
        let arg = arg.map_or(Data::Nil, |a| Data::String(a.into()));
        self.script.fire("action", [Data::Integer(id), arg]);
        self.drain()
    }

    /// Resumes tasks whose network calls finished, and returns what they asked for.
    pub fn poll(&self, now: f64, tx: &mut Tx) -> Vec<Event> {
        self.script.poll(now, tx);
        self.drain()
    }

    pub fn mode(&self, command: &str) -> Mode {
        self.manifest.commands.iter().find(|c| c.id == command).map(|c| c.mode).unwrap_or_default()
    }

    /// Recent `print`/`warn`/error lines.
    pub fn log(&self) -> Vec<String> {
        self.script.log().into_iter().map(|l| format!("{:?}: {}", l.level, l.message)).collect()
    }

    /// Whether `init.luau` has a function for `command`.
    pub fn has_command(&self, command: &str) -> bool {
        self.script.with_lua(|lua| {
            let lib: Table = lua.named_registry_value(LIB_KEY).ok()?;
            lib.get::<mlua::Function>("has").ok()?.call::<bool>(command).ok()
        }).flatten().unwrap_or(false)
    }

    /// `poll` without a window: runs due tasks with a throwaway transaction.
    #[cfg(test)]
    fn script_poll(&self) -> Vec<Event> {
        let mut tx = Tx::default();
        self.script.poll(f64::MAX / 4.0, &mut tx);
        self.drain()
    }

    fn drain(&self) -> Vec<Event> {
        let msgs = std::mem::take(&mut *self.outbox.borrow_mut());
        msgs.into_iter().map(|(kind, data)| event(&kind, data)).collect()
    }
}

fn event(kind: &str, data: Data) -> Event {
    let view = |data: Data| serde_json::from_value::<View>(data.to_json())
        .map_err(|e| format!("bad view from the extension: {e}"));
    match kind {
        "view" => view(data).map_or_else(Event::Error, Event::View),
        "push" => view(data).map_or_else(Event::Error, Event::Push),
        "toast" => Event::Toast(data.as_str().unwrap_or_default().to_string()),
        "close" => Event::Close,
        "done" => Event::Done,
        _ => Event::Error(data.as_str().unwrap_or("unknown error").to_string()),
    }
}

/// What the host functions of one extension share.
struct Ctx {
    manifest: Manifest,
    outbox: Rc<RefCell<Vec<(String, Data)>>>,
    prefs: BTreeMap<String, toml::Value>,
    opts: Options,
}

/// Binds `@tomboy/ui` and `@tomboy/api` into a new VM.
fn bind(host: &rune_luau::Host<'_>, ctx: &Ctx) -> mlua::Result<()> {
    let lua = host.lua();
    let fns = lua.create_table()?;
    let id = ctx.manifest.id.clone();

    let outbox = ctx.outbox.clone();
    fns.set("emit", lua.create_function(move |_, (kind, data): (String, Data)| {
        outbox.borrow_mut().push((kind, data));
        Ok(())
    })?)?;

    let net = ctx.manifest.permissions.clone();
    let fixtures = ctx.opts.fixtures.clone();
    let get = host.async_function("api.http.get", move |lua, args: MultiValue, done| {
        let mut args = args.into_iter();
        let url = match args.next() { Some(Value::String(s)) => s.to_str()?.to_string(), _ => {
            return Err(mlua::Error::runtime("api.http.get needs a URL"));
        }};
        let headers = match args.next() { Some(v @ Value::Table(_)) => lua.unpack::<Data>(v)?, _ => Data::Nil };
        let host_name = https_host(&url).map_err(mlua::Error::runtime)?;
        if !net.allows_host(&host_name) {
            return Err(mlua::Error::runtime(format!(
                "api.http.get: {host_name} is not in this extension's permissions (add it to [permissions] net in tomboy.toml)")));
        }
        if let Some(body) = fixtures.get(&url) {
            let res = [("status".to_string(), Data::Integer(200)), ("body".to_string(), Data::String(body.clone()))];
            done.complete(Data::Map(res.into_iter().collect()));
            return Ok(());
        }
        rune_luau::spawn_blocking(move || done.complete(http_get(&url, &headers)));
        Ok(())
    })?;
    fns.set("httpGet", get)?;

    fns.set("jsonDecode", lua.create_function(|lua, s: String| {
        let v: serde_json::Value = serde_json::from_str(&s).map_err(|e| mlua::Error::runtime(format!("api.json.decode: {e}")))?;
        lua.to_value_with(&v, mlua::serde::SerializeOptions::new().serialize_none_to_null(false).serialize_unit_to_null(false))
    })?)?;
    fns.set("jsonEncode", lua.create_function(|_, v: Data| {
        Ok(v.to_json().to_string())
    })?)?;

    let storage = ctx.opts.state_dir.clone().or_else(|| super::state_dir(&id)).map(|d| d.join("storage.json"));
    let path = storage.clone();
    fns.set("storageGet", lua.create_function(move |lua, key: String| {
        let map = read_storage(path.as_deref());
        match map.get(&key) { Some(v) => lua.to_value(v), None => Ok(Value::Nil) }
    })?)?;
    let path = storage.clone();
    fns.set("storageSet", lua.create_function(move |_, (key, value): (String, Data)| {
        let mut map = read_storage(path.as_deref());
        map.insert(key, value.to_json());
        write_storage(path.as_deref(), &map).map_err(mlua::Error::runtime)
    })?)?;
    let path = storage;
    fns.set("storageRemove", lua.create_function(move |_, key: String| {
        let mut map = read_storage(path.as_deref());
        map.remove(&key);
        write_storage(path.as_deref(), &map).map_err(mlua::Error::runtime)
    })?)?;

    let cache = ctx.opts.cache_dir.clone().or_else(|| super::cache_dir(&id));
    let dir = cache.clone();
    fns.set("cacheGet", lua.create_function(move |_, (key, max_age): (String, Option<f64>)| {
        Ok(dir.as_deref().and_then(|d| cache_get(d, &key, max_age.map(|a| a.max(0.0) as u64))))
    })?)?;
    let dir = cache;
    fns.set("cacheSet", lua.create_function(move |_, (key, value): (String, String)| {
        if let Some(d) = dir.as_deref() { cache_set(d, &key, &value); }
        Ok(())
    })?)?;

    fns.set("timeNow", lua.create_function(|_, ()| Ok(crate::frecency::now() as f64))?)?;
    fns.set("timeAgo", lua.create_function(|_, t: f64| Ok(ago(crate::frecency::now() as i64 - t as i64)))?)?;

    let prefs = lua.to_value(&ctx.prefs.iter().map(|(k, v)| (k.clone(), toml_to_json(v))).collect::<serde_json::Map<_, _>>())?;
    let run = host.signal("run")?;
    let action = host.signal("action")?;
    let lib: Table = lua.load(LIB).set_name("=tomboy").call((fns, prefs, run, action))?;
    host.register_module("@tomboy/ui", lib.get::<Table>("ui")?)?;
    host.register_module("@tomboy/api", lib.get::<Table>("api")?)?;
    lua.set_named_registry_value(LIB_KEY, lib)?;
    Ok(())
}

/// The host of an `https://` URL, lowercase; anything else is refused.
fn https_host(url: &str) -> Result<String, String> {
    let rest = url.strip_prefix("https://").ok_or_else(|| format!("api.http.get: only https:// URLs are allowed, not {url}"))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return Err("api.http.get: URLs with credentials are not allowed".into());
    }
    let host = authority.split(':').next().unwrap_or_default().to_ascii_lowercase();
    if host.is_empty() { return Err(format!("api.http.get: no host in {url}")); }
    Ok(host)
}

/// `{status, body}`, or `{error}` when there was no response.
fn http_get(url: &str, headers: &Data) -> Data {
    let mut out = BTreeMap::new();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(HTTP_TIMEOUT))
        .http_status_as_error(false)
        .max_redirects(0)
        .user_agent("tomboy-extension")
        .build().into();
    let mut req = agent.get(url);
    if let Data::Map(h) = headers {
        for (k, v) in h {
            if let Some(v) = v.as_str() { req = req.header(k.as_str(), v); }
        }
    }
    match req.call() {
        Ok(mut res) => {
            out.insert("status".into(), Data::Integer(res.status().as_u16() as i64));
            match res.body_mut().with_config().limit(HTTP_MAX_BYTES).read_to_string() {
                Ok(body) => { out.insert("body".into(), Data::String(body)); }
                Err(e) => { out.insert("error".into(), Data::String(e.to_string())); }
            }
        }
        Err(e) => { out.insert("error".into(), Data::String(e.to_string())); }
    }
    Data::Map(out)
}

fn read_storage(path: Option<&Path>) -> serde_json::Map<String, serde_json::Value> {
    path.and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn write_storage(path: Option<&Path>, map: &serde_json::Map<String, serde_json::Value>) -> Result<(), String> {
    let path = path.ok_or("api.storage: no home directory")?;
    let raw = serde_json::to_string(map).map_err(|e| e.to_string())?;
    if raw.len() > STORAGE_MAX_BYTES {
        return Err(format!("api.storage: over {} KB", STORAGE_MAX_BYTES / 1024));
    }
    if let Some(dir) = path.parent() { std::fs::create_dir_all(dir).map_err(|e| e.to_string())?; }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, raw).and_then(|_| std::fs::rename(&tmp, path)).map_err(|e| format!("api.storage: {e}"))
}

fn cache_file(dir: &Path, key: &str) -> PathBuf {
    let hash = Sha256::digest(key.as_bytes());
    dir.join(hash.iter().take(16).map(|b| format!("{b:02x}")).collect::<String>())
}

fn cache_get(dir: &Path, key: &str, max_age: Option<u64>) -> Option<String> {
    let path = cache_file(dir, key);
    if let Some(max) = max_age {
        let age = std::fs::metadata(&path).ok()?.modified().ok()?.elapsed().map(|d| d.as_secs()).unwrap_or(0);
        if age > max { return None; }
    }
    std::fs::read_to_string(path).ok()
}

fn cache_set(dir: &Path, key: &str, value: &str) {
    if std::fs::create_dir_all(dir).is_err() { return; }
    let path = cache_file(dir, key);
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, value).is_ok() { let _ = std::fs::rename(&tmp, &path); }
}

/// `45s`, `3m`, `5h`, `2d`, `3w`, `4mo`, `2y`.
pub fn ago(secs: i64) -> String {
    let s = secs.max(0);
    match s {
        0..60 => "now".into(),
        60..3600 => format!("{}m", s / 60),
        3600..86_400 => format!("{}h", s / 3600),
        86_400..604_800 => format!("{}d", s / 86_400),
        604_800..2_592_000 => format!("{}w", s / 604_800),
        2_592_000..31_536_000 => format!("{}mo", s / 2_592_000),
        _ => format!("{}y", s / 31_536_000),
    }
}

fn toml_to_json(v: &toml::Value) -> serde_json::Value {
    match v {
        toml::Value::String(s) => s.clone().into(),
        toml::Value::Integer(i) => (*i).into(),
        toml::Value::Float(f) => (*f).into(),
        toml::Value::Boolean(b) => (*b).into(),
        other => other.to_string().into(),
    }
}

/// `tomboy ext check`: starts the extension headless and checks every command has a function.
/// Returns notes to print; an `Err` fails the check.
pub fn check(dir: &Path) -> Result<Vec<String>, String> {
    let manifest = Manifest::load(dir)?;
    let prefs = manifest.preference_values(None);
    let rt = Runtime::start(manifest.clone(), dir, prefs)?;
    let mut notes = Vec::new();
    for c in &manifest.commands {
        if !rt.has_command(&c.id) {
            return Err(format!("init.luau returns no function for command \"{}\"", c.id));
        }
        notes.push(format!("command {} ok", c.id));
    }
    for line in rt.log() { notes.push(line); }
    Ok(notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_ext(name: &str, manifest_extra: &str, init: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tomboy-rt-{}-{name}", std::process::id())).join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tomboy.toml"), format!(r#"
id = "{name}"
name = "Test"
version = "1.0.0"
api = 1
description = "t"
author = "me"
license = "MIT"
{manifest_extra}
[[commands]]
id = "main"
title = "Main"
"#)).unwrap();
        std::fs::write(dir.join("init.luau"), init).unwrap();
        dir
    }

    fn start(dir: &Path) -> Runtime {
        let m = Manifest::load(dir).unwrap();
        let prefs = m.preference_values(None);
        Runtime::start(m, dir, prefs).unwrap()
    }

    #[test]
    fn runs_a_command_and_an_action() {
        let dir = temp_ext("list", "", r#"
local ui = require("@tomboy/ui")
return { main = function(ctx)
    return ui.list({ items = {
        { id = 1, title = "One " .. ctx.query, accessories = { 5 }, actions = {
            ui.copy("x", "Copy"),
            ui.run("Say", function() ui.toast("hi") end),
        } },
    } })
end }
"#);
        let rt = start(&dir);
        assert!(rt.has_command("main"));
        let events = rt.run("main", "q");
        let Some(Event::View(View::List(list))) = events.first() else { panic!("{events:?}") };
        assert_eq!(list.items[0].title, "One q");
        assert_eq!(list.items[0].accessories, vec!["5"]);
        let crate::ext::view::Action::Callback { id, .. } = list.items[0].actions[1] else { panic!() };
        assert_eq!(rt.action(id, None), vec![Event::Toast("hi".into()), Event::Done]);
    }

    #[test]
    fn refuses_undeclared_hosts_and_plain_http() {
        let dir = temp_ext("net", "[permissions]\nnet = [\"example.com\"]\n", r#"
local api = require("@tomboy/api")
return { main = function(ctx)
    api.http.get(ctx.query)
end }
"#);
        let rt = start(&dir);
        let events = rt.run("main", "https://evil.com/x");
        assert!(matches!(&events[0], Event::Error(e) if e.contains("evil.com is not in this extension's permissions")), "{events:?}");
        let events = rt.run("main", "http://example.com/");
        assert!(matches!(&events[0], Event::Error(e) if e.contains("only https")), "{events:?}");
    }

    #[test]
    fn sandbox_has_no_io_and_stops_loops() {
        let dir = temp_ext("sandbox", "", r#"
return {
    main = function(ctx)
        if ctx.query == "io" then return io.open("/etc/passwd") end
        if ctx.query == "os" then return os.execute("true") end
        if ctx.query == "loop" then while true do end end
        if ctx.query == "require" then return require("../../etc") end
    end,
}
"#);
        let rt = start(&dir);
        for q in ["io", "os", "loop", "require"] {
            let events = rt.run("main", q);
            assert!(matches!(&events[0], Event::Error(_)), "{q}: {events:?}");
        }
        // Still alive after all of that.
        assert!(rt.has_command("main"));
    }

    #[test]
    fn missing_command_fails_check() {
        let dir = temp_ext("missing", "", "return {}");
        assert!(check(&dir).unwrap_err().contains("no function for command \"main\""));
    }

    #[test]
    fn json_round_trip() {
        let dir = temp_ext("json", "", r#"
local api = require("@tomboy/api")
local ui = require("@tomboy/ui")
return { main = function()
    local v = api.json.decode('{"hits":[{"n":1,"s":"a"}],"x":null}')
    return ui.detail({ markdown = v.hits[1].s .. tostring(v.hits[1].n) .. tostring(v.x) })
end }
"#);
        let rt = start(&dir);
        let events = rt.run("main", "");
        assert_eq!(events[0], Event::View(View::Detail(crate::ext::view::Detail { title: None, markdown: "a1nil".into(), actions: vec![] })));
    }

    /// The real Hacker News extension against a saved API response: loading list first, then
    /// the stories, then the actions, then the cache serving the next open.
    #[test]
    fn hacker_news_sample() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("extensions/hackernews");
        let fixture = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hn-front-page.json")).unwrap();
        let tmp = std::env::temp_dir().join(format!("tomboy-hn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let opts = Options {
            fixtures: [("https://hn.algolia.com/api/v1/search?tags=front_page&hitsPerPage=50".to_string(), fixture)].into(),
            state_dir: Some(tmp.join("state")),
            cache_dir: Some(tmp.join("cache")),
        };
        let m = Manifest::load(&dir).unwrap();
        let prefs = m.preference_values(None);
        let rt = Runtime::start_with(m.clone(), &dir, prefs.clone(), opts.clone()).unwrap();
        let first = rt.run("front-page", "");
        let Some(Event::View(View::List(loading))) = first.first() else { panic!("{first:?}") };
        assert!(loading.loading && loading.items.is_empty(), "{loading:?}");
        // The deferred fetch runs on the next poll.
        let mut events = Vec::new();
        for _ in 0..5 { events.extend(rt.script_poll()); }
        let Some(Event::View(View::List(list))) = events.iter().find(|e| matches!(e, Event::View(_))) else { panic!("{events:?}") };
        assert!(!list.loading);
        assert_eq!(list.items.len(), 12);
        let story = &list.items[0];
        assert!(story.accessories[0].starts_with('▲'), "{:?}", story.accessories);
        let titles: Vec<&str> = story.actions.iter().map(|a| a.title()).collect();
        assert_eq!(titles, ["Open Story", "Open Comments", "Copy Link", "Copy as Markdown", "Refresh"]);
        assert!(story.detail.as_deref().unwrap().contains("points"));
        assert_eq!(list.matches(&story.title[..4]).first(), Some(&0));
        // Reopened within five minutes: straight from the cache, no loading state.
        let again = Runtime::start_with(m, &dir, prefs, Options { fixtures: BTreeMap::new(), ..opts }).unwrap();
        let events = again.run("front-page", "");
        let Some(Event::View(View::List(cached))) = events.first() else { panic!("{events:?}") };
        assert!(!cached.loading && cached.items.len() == 12);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The Hacker News sample against the real API: `cargo test live_hackernews -- --ignored`.
    #[test]
    #[ignore]
    fn live_hackernews() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("extensions/hackernews");
        let tmp = std::env::temp_dir().join(format!("tomboy-hn-live-{}", std::process::id()));
        let opts = Options { state_dir: Some(tmp.join("state")), cache_dir: Some(tmp.join("cache")), ..Default::default() };
        let m = Manifest::load(&dir).unwrap();
        let rt = Runtime::start_with(m.clone(), &dir, m.preference_values(None), opts).unwrap();
        rt.run("front-page", "");
        let start = std::time::Instant::now();
        let list = loop {
            let events = rt.script_poll();
            if let Some(Event::View(View::List(l))) = events.into_iter().find(|e| matches!(e, Event::View(_))) { break l; }
            assert!(start.elapsed().as_secs() < 15, "no stories in 15 s");
            std::thread::sleep(Duration::from_millis(50));
        };
        let _ = std::fs::remove_dir_all(&tmp);
        assert!(!list.loading && list.items.len() >= 20, "{} stories, {:?}", list.items.len(), list.empty_text);
        println!("live: {} stories, first: {}", list.items.len(), list.items[0].title);
    }

    #[test]
    fn hosts() {
        assert_eq!(https_host("https://HN.algolia.com:443/api?x").unwrap(), "hn.algolia.com");
        assert!(https_host("http://hn.algolia.com").is_err());
        assert!(https_host("https://user@evil.com").is_err());
        assert_eq!(ago(30), "now");
        assert_eq!(ago(3 * 3600 + 5), "3h");
    }
}
