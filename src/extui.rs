//! Extensions in the launcher: their commands as rows, the views a running extension shows,
//! the store ("Install Extension") and the action panel (Ctrl/⌘+K) both of them use.
//!
//! A child of the crate root, so it reaches `Tomboy` and the row helpers in `main.rs`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rune::tok;
use rune::{Color, El, Key, KeyEvent, ScrollSpec, Tx, col, el, row, spacer, text};
use rune_kit::Markdown;

use crate::ext::index::{Entry, Index};
use crate::ext::install::{self, Installed, InstalledEntry, Status};
use crate::ext::manifest::{Manifest, Mode};
use crate::ext::runtime::{Event, Runtime};
use crate::ext::view::{Action, List, ListItem, View};
use crate::{After, Item, NERD_FONT, Tomboy, frecency, keycap, quit, run_item, sys, ty, url};

/// nf-md-puzzle: an extension without an icon of its own.
const EXT_GLYPH: &str = "\u{f0431}";
/// nf-md-store
const STORE_GLYPH: &str = "\u{f04ca}";
const EXT_COLOR: Color = Color { r: 0.42, g: 0.36, b: 0.90, a: 1.0 };
const ERROR_COLOR: Color = Color { r: 0.95, g: 0.35, b: 0.35, a: 1.0 };
/// Extension commands listed for one query.
const MAX_HITS: usize = 6;

/// A running extension command and the views it pushed.
pub struct Session {
    rt: Runtime,
    /// Index into `Tomboy::extensions`.
    pub ext: usize,
    command: String,
    stack: Vec<View>,
    error: Option<String>,
    /// The command (or the last action) hasn't finished.
    running: bool,
}

/// The store: the registry's index and what's installed.
pub struct Store {
    index: Option<Index>,
    error: Option<String>,
    installed: Installed,
    /// The extension waiting for Install / Cancel on the consent screen.
    consent: Option<String>,
    /// "Installing Hacker News…" while a background job runs.
    busy: Option<String>,
    inbox: Arc<Mutex<Vec<StoreMsg>>>,
}

enum StoreMsg {
    Index(Result<Index, String>),
    Done { what: String, result: Result<(), String> },
}

/// The action panel: every action of the picked row; Enter runs one.
pub struct Panel {
    actions: Vec<PanelAction>,
    pub picked: usize,
}

#[derive(Clone, Debug)]
enum PanelAction {
    Ext(Action),
    Store(StoreAction),
}

#[derive(Clone, Debug)]
enum StoreAction {
    Install(String),
    Update(String),
    Uninstall(String),
    Enable(String, bool),
    Open(String),
    Repo(String),
    Folder(String),
}

impl PanelAction {
    fn title(&self) -> String {
        match self {
            PanelAction::Ext(a) => a.title().to_string(),
            PanelAction::Store(a) => match a {
                StoreAction::Install(_) => "Install".into(),
                StoreAction::Update(_) => "Update".into(),
                StoreAction::Uninstall(_) => "Uninstall".into(),
                StoreAction::Enable(_, true) => "Enable".into(),
                StoreAction::Enable(_, false) => "Disable".into(),
                StoreAction::Open(_) => "Open Command".into(),
                StoreAction::Repo(_) => "View Source on GitHub".into(),
                StoreAction::Folder(_) => "Show Folder".into(),
            },
        }
    }
}

/// One row of the store: an extension in the index, installed, or both.
pub struct StoreRow {
    pub id: String,
    pub manifest: Manifest,
    entry: Option<Entry>,
    installed: Option<InstalledEntry>,
    pub status: Status,
    readme: String,
}

// ---- Launcher rows ----

/// Installed extensions' commands matching `q`, best first (frecency among equals). With an
/// empty field, only the ones you've used.
pub fn search(s: &Tomboy, q: &str) -> Vec<Item> {
    let now = frecency::now();
    let words: Vec<String> = q.split_whitespace().map(str::to_lowercase).collect();
    let mut hits: Vec<(u32, u32, Item)> = Vec::new();
    for (e, (m, _)) in s.extensions.iter().enumerate() {
        for (c, cmd) in m.commands.iter().enumerate() {
            let boost = s.frecency.boost(&frecency_key(&m.id, &cmd.id), q, now);
            let score = if words.is_empty() {
                if boost == 0 { continue }
                1
            } else {
                let hay: Vec<String> = cmd.title.split_whitespace().chain(m.name.split_whitespace())
                    .chain(cmd.keywords.iter().map(String::as_str)).map(str::to_lowercase).collect();
                if !words.iter().all(|w| hay.iter().any(|h| h.starts_with(w.as_str()))) { continue }
                let exact = cmd.keywords.iter().any(|k| k.eq_ignore_ascii_case(q.trim()));
                if exact { 3 } else if cmd.title.to_lowercase().starts_with(&words[0]) { 2 } else { 1 }
            };
            hits.push((score, boost, Item::Extension { ext: e, command: c }));
        }
    }
    hits.sort_by_key(|h| std::cmp::Reverse((h.0, h.1)));
    hits.into_iter().take(MAX_HITS).map(|(_, _, it)| it).collect()
}

/// `install extension`, `extensions`, `store`, `plugins`.
pub fn store_matches(q: &str) -> bool {
    let q = q.trim().to_lowercase();
    if q.len() < 3 { return false; }
    ["install extension", "install extensions", "extensions", "extension store", "store", "plugins"]
        .iter().any(|c| c.starts_with(&q))
}

pub fn frecency_key(ext: &str, command: &str) -> String { format!("ext:{ext}:{command}") }

/// The launcher row of an extension command: (icon glyph, title, subtitle, kind).
pub fn command_row(s: &Tomboy, ext: usize, command: usize) -> (String, String, Option<String>, String) {
    let (m, _) = &s.extensions[ext];
    let c = &m.commands[command];
    let sub = c.subtitle.clone().unwrap_or_else(|| m.description.clone());
    (m.icon.clone().unwrap_or_else(|| EXT_GLYPH.into()), c.title.clone(), Some(sub), m.name.clone())
}

pub fn store_row_parts() -> (String, String, Option<String>) {
    (STORE_GLYPH.into(), "Install Extension".into(), Some("Browse and install extensions for tomboy".into()))
}

pub fn ext_color() -> Color { EXT_COLOR }

// ---- Running an extension ----

/// Start extension `ext` and run its command `command`.
pub fn open_command(s: &mut Tomboy, ext: usize, command: usize, tx: &mut Tx) -> std::io::Result<After> {
    let (manifest, dir) = s.extensions[ext].clone();
    let cmd = manifest.commands[command].id.clone();
    let prefs = manifest.preference_values(s.config.extension_prefs(&manifest.id));
    let rt = Runtime::start(manifest, &dir, prefs).map_err(std::io::Error::other)?;
    if let Some(w) = &s.waker { rt.set_waker(w.clone()); }
    let events = rt.run(&cmd, "");
    s.session = Some(Session { rt, ext, command: cmd, stack: Vec::new(), error: None, running: true });
    s.store = None;
    clear_field(s);
    apply(s, events, tx);
    Ok(After::Stay)
}

/// Resume the running extension's tasks (network calls that finished) and the store's jobs.
pub fn poll(s: &mut Tomboy, now: f64, tx: &mut Tx) -> bool {
    let mut changed = false;
    if let Some(sess) = &s.session {
        let events = sess.rt.poll(now, tx);
        if !events.is_empty() { changed = true; apply(s, events, tx); }
    }
    if s.store.is_some() { changed |= store_poll(s); }
    changed
}

fn apply(s: &mut Tomboy, events: Vec<Event>, tx: &mut Tx) {
    for ev in events {
        let Some(sess) = s.session.as_mut() else { return };
        match ev {
            Event::View(v) => {
                let top_search = matches!(sess.stack.last(), Some(View::List(l)) if l.search.is_some());
                match sess.stack.last_mut() { Some(top) => *top = v, None => sess.stack.push(v) }
                // A list that searches itself keeps what's typed; any other replaced view starts at the top.
                if !top_search { s.picked = s.picked.min(row_count(s).saturating_sub(1)); }
            }
            Event::Push(v) => { sess.stack.push(v); clear_field(s); }
            Event::Toast(t) => s.status = Some(t),
            Event::Close => { quit(s, tx); return; }
            Event::Error(e) => { sess.error = Some(e.clone()); s.status = Some(format!("{}: {e}", sess.rt.manifest.name)); }
            Event::Done => {
                sess.running = false;
                let action_mode = sess.rt.mode(&sess.command) == Mode::Action;
                if action_mode && sess.stack.is_empty() && sess.error.is_none() {
                    s.session = None;
                    if s.status.is_none() { quit(s, tx); return; }
                }
            }
        }
    }
}

fn row_count(s: &Tomboy) -> usize {
    match s.session.as_ref().and_then(|x| x.stack.last()) {
        Some(View::List(l)) => l.matches(&s.query.text).len(),
        _ => 0,
    }
}

fn clear_field(s: &mut Tomboy) {
    s.query.clear();
    s.last_text.clear();
    s.picked = 0;
    s.status = None;
    s.panel = None;
}

/// The open view's rows (filtered by the field), for `results`.
pub fn session_results(s: &Tomboy) -> Option<Vec<Item>> {
    let sess = s.session.as_ref()?;
    Some(match sess.stack.last() {
        Some(View::List(l)) => l.matches(&s.query.text).into_iter().map(Item::ExtRow).collect(),
        _ => Vec::new(),
    })
}

fn top_list(s: &Tomboy) -> Option<&List> {
    match s.session.as_ref()?.stack.last()? { View::List(l) => Some(l), View::Detail(_) => None }
}

fn picked_item(s: &Tomboy) -> Option<&ListItem> {
    let l = top_list(s)?;
    let i = *l.matches(&s.query.text).get(s.picked)?;
    l.items.get(i)
}

/// The actions of the picked row (or the open detail).
fn session_actions(s: &Tomboy) -> Vec<Action> {
    let Some(sess) = &s.session else { return Vec::new() };
    match sess.stack.last() {
        Some(View::List(_)) => picked_item(s).map(|it| it.actions.clone()).unwrap_or_default(),
        Some(v @ View::Detail(_)) => v.actions_of(None).to_vec(),
        None => Vec::new(),
    }
}

/// Enter in an extension view: the first action of the picked row.
pub fn run_row(s: &mut Tomboy, item: usize, tx: &mut Tx) {
    let action = top_list(s).and_then(|l| l.items.get(item)).and_then(|it| it.actions.first().cloned());
    if let Some(a) = action { run_action(s, a, tx) }
}

fn run_action(s: &mut Tomboy, action: Action, tx: &mut Tx) {
    s.panel = None;
    match action {
        Action::Open { url, .. } => match url::open(&url) {
            Ok(()) => quit(s, tx),
            Err(e) => s.status = Some(format!("Failed: {e}")),
        },
        Action::Copy { text, .. } => match sys::copy_to_clipboard(&text) {
            Ok(true) => { s.paste = true; quit(s, tx) }
            Ok(false) => { tx.copy(&text); s.status = Some("Copied".into()); }
            Err(e) => s.status = Some(format!("Failed: {e}")),
        },
        Action::Callback { id, .. } => {
            let Some(sess) = s.session.as_mut() else { return };
            sess.running = true;
            sess.error = None;
            let events = sess.rt.action(id, None);
            apply(s, events, tx);
        }
    }
}

/// After an edit: a list that searches itself gets the new text.
pub fn on_change(s: &mut Tomboy, tx: &mut Tx) -> bool {
    let Some(sess) = &s.session else { return false };
    if let Some(View::List(List { search: Some(id), .. })) = sess.stack.last() {
        let events = sess.rt.action(*id, Some(&s.query.text));
        apply(s, events, tx);
    }
    true
}

/// Esc / Backspace in an extension view: clear the field, then go back a view, then leave.
pub fn session_key(s: &mut Tomboy, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let empty = s.query.text.is_empty();
    match ev.key {
        Key::Escape if !empty => { s.query.clear(); s.last_text.clear(); s.picked = 0; on_change(s, tx); }
        Key::Escape | Key::Backspace if empty => {
            let depth = s.session.as_ref().map_or(0, |x| x.stack.len());
            if depth > 1 {
                if let Some(sess) = s.session.as_mut() { sess.stack.pop(); }
                clear_field(s);
            } else {
                s.session = None;
                clear_field(s);
            }
        }
        _ => return false,
    }
    true
}

// ---- The store ----

pub fn open_store(s: &mut Tomboy) {
    let base = s.config.registry();
    let inbox: Arc<Mutex<Vec<StoreMsg>>> = Arc::default();
    let cached = install::cached_index(&base);
    s.store = Some(Store {
        index: cached, error: None, installed: Installed::load(), consent: None,
        busy: Some("Loading extensions…".into()), inbox: inbox.clone(),
    });
    s.session = None;
    clear_field(s);
    let waker = s.waker.clone();
    std::thread::spawn(move || {
        let result = install::fetch_index(&base, install::INDEX_MAX_AGE);
        push(&inbox, StoreMsg::Index(result), &waker);
    });
}

fn push(inbox: &Arc<Mutex<Vec<StoreMsg>>>, msg: StoreMsg, waker: &Option<rune::Waker>) {
    inbox.lock().unwrap_or_else(|e| e.into_inner()).push(msg);
    if let Some(w) = waker { w() }
}

fn store_poll(s: &mut Tomboy) -> bool {
    let Some(store) = s.store.as_mut() else { return false };
    let msgs = std::mem::take(&mut *store.inbox.lock().unwrap_or_else(|e| e.into_inner()));
    if msgs.is_empty() { return false; }
    let mut reload = false;
    for msg in msgs {
        match msg {
            StoreMsg::Index(Ok(index)) => { store.index = Some(index); store.error = None; }
            StoreMsg::Index(Err(e)) => store.error = Some(e),
            StoreMsg::Done { what, result } => {
                match result {
                    Ok(()) => s.status = Some(what),
                    Err(e) => s.status = Some(format!("Failed: {e}")),
                }
                reload = true;
            }
        }
        store.busy = None;
    }
    if reload {
        store.installed = Installed::load();
        s.extensions = install::installed_dirs();
    }
    true
}

/// Installed extensions first, then the rest of the index; filtered by the field.
pub fn store_rows(s: &Tomboy) -> Vec<StoreRow> {
    let Some(store) = &s.store else { return Vec::new() };
    let mut rows = Vec::new();
    let data = crate::ext::data_dir();
    for (id, inst) in &store.installed.0 {
        let entry = store.index.as_ref().and_then(|i| i.get(id)).cloned();
        let dir = data.as_ref().map(|d| d.join(id));
        let local = dir.as_deref().and_then(|d| Manifest::load(d).ok());
        let Some(manifest) = local.or_else(|| entry.as_ref().map(|e| e.manifest.clone())) else { continue };
        let readme = dir.and_then(|d| std::fs::read_to_string(d.join("README.md")).ok())
            .or_else(|| entry.as_ref().map(|e| e.readme.clone())).unwrap_or_default();
        // Without an index (offline) nothing is "removed from the registry".
        let status = if store.index.is_none() { Status::Installed } else { install::status(entry.as_ref(), Some(inst)) };
        rows.push(StoreRow { id: id.clone(), manifest, entry, installed: Some(inst.clone()), status, readme });
    }
    if let Some(index) = &store.index {
        for e in &index.extensions {
            if store.installed.0.contains_key(&e.manifest.id) { continue }
            rows.push(StoreRow {
                id: e.manifest.id.clone(), manifest: e.manifest.clone(), status: install::status(Some(e), None),
                readme: e.readme.clone(), entry: Some(e.clone()), installed: None,
            });
        }
    }
    let words: Vec<String> = s.query.text.split_whitespace().map(str::to_lowercase).collect();
    rows.retain(|r| {
        let m = &r.manifest;
        let hay = format!("{} {} {} {}", m.name, m.description, m.id,
            m.commands.iter().flat_map(|c| c.keywords.iter().chain([&c.title])).cloned().collect::<Vec<_>>().join(" ")).to_lowercase();
        words.iter().all(|w| hay.contains(w.as_str()))
    });
    rows
}

pub fn store_results(s: &Tomboy) -> Option<Vec<Item>> {
    let store = s.store.as_ref()?;
    if store.consent.is_some() { return Some(Vec::new()); }
    Some((0..store_rows(s).len()).map(Item::StoreRow).collect())
}

fn store_actions(row: &StoreRow) -> Vec<StoreAction> {
    let id = row.id.clone();
    let mut out = Vec::new();
    match (&row.status, &row.installed) {
        (Status::NotInstalled, _) => out.push(StoreAction::Install(id.clone())),
        (Status::UpdateAvailable(_), _) => out.push(StoreAction::Update(id.clone())),
        _ => {}
    }
    if let Some(inst) = &row.installed {
        if inst.enabled && row.manifest.supported().is_ok() { out.insert(0, StoreAction::Open(id.clone())); }
        out.push(StoreAction::Enable(id.clone(), !inst.enabled));
        out.push(StoreAction::Uninstall(id.clone()));
        out.push(StoreAction::Folder(id.clone()));
    }
    if row.entry.is_some() { out.push(StoreAction::Repo(id)); }
    out
}

/// Enter on a store row: open an installed extension, else install (or update) it.
pub fn run_store_row(s: &mut Tomboy, i: usize, tx: &mut Tx) {
    let rows = store_rows(s);
    let Some(row) = rows.get(i) else { return };
    if let Some(a) = store_actions(row).into_iter().next() { run_store_action(s, a, tx) }
}

fn run_store_action(s: &mut Tomboy, action: StoreAction, tx: &mut Tx) {
    s.panel = None;
    let base = s.config.registry();
    let name_of = |s: &Tomboy, id: &str| store_rows(s).into_iter().find(|r| r.id == id).map_or(id.to_string(), |r| r.manifest.name);
    match action {
        // A new install, or an update that asks for more, waits for the consent screen.
        StoreAction::Install(id) | StoreAction::Update(id) if needs_consent(s, &id) => {
            if let Some(store) = s.store.as_mut() { store.consent = Some(id); }
        }
        StoreAction::Install(id) | StoreAction::Update(id) => install_now(s, id),
        StoreAction::Uninstall(id) => {
            let name = name_of(s, &id);
            background(s, format!("Removing {name}…"), move || install::uninstall(&id).map(|()| format!("{name} removed")));
        }
        StoreAction::Enable(id, on) => {
            let name = name_of(s, &id);
            background(s, String::new(), move || install::set_enabled(&id, on)
                .map(|()| format!("{name} {}", if on { "enabled" } else { "disabled" })));
        }
        StoreAction::Open(id) => {
            s.extensions = install::installed_dirs();
            if let Some(e) = s.extensions.iter().position(|(m, _)| m.id == id) {
                run_item(s, &Item::Extension { ext: e, command: 0 }, tx);
            }
        }
        StoreAction::Repo(id) => {
            let url = repo_url(&base, &id);
            if let Err(e) = url::open(&url) { s.status = Some(format!("Failed: {e}")); } else { quit(s, tx) }
        }
        StoreAction::Folder(id) => {
            let dir = crate::ext::data_dir().map(|d| d.join(&id)).unwrap_or_else(|| PathBuf::from("."));
            if let Err(e) = sys::open_url(&dir.to_string_lossy()) { s.status = Some(format!("Failed: {e}")); }
        }
    }
}

/// A fresh install always asks; an update only when it adds permissions.
fn needs_consent(s: &Tomboy, id: &str) -> bool {
    store_rows(s).into_iter().find(|r| r.id == id).is_none_or(|r| match (&r.entry, &r.installed) {
        (Some(e), Some(inst)) => !install::needs_consent(e, Some(inst)).is_empty(),
        _ => true,
    })
}

fn install_now(s: &mut Tomboy, id: String) {
    let base = s.config.registry();
    let name = store_rows(s).into_iter().find(|r| r.id == id).map_or(id.clone(), |r| r.manifest.name);
    let updating = s.store.as_ref().is_some_and(|st| st.installed.0.contains_key(&id));
    let verb = if updating { ("Updating", "updated") } else { ("Installing", "installed") };
    background(s, format!("{} {name}…", verb.0), move || {
        install::install_fresh(&base, &id).map(|_| format!("{name} {}", verb.1))
    });
}

/// Enter (or Install) on the consent screen.
fn accept(s: &mut Tomboy, _: &mut Tx) {
    let Some(id) = s.store.as_mut().and_then(|st| st.consent.take()) else { return };
    install_now(s, id);
}

fn background(s: &mut Tomboy, busy: String, job: impl FnOnce() -> Result<String, String> + Send + 'static) {
    let Some(store) = s.store.as_mut() else { return };
    store.busy = (!busy.is_empty()).then_some(busy);
    let (inbox, waker) = (store.inbox.clone(), s.waker.clone());
    std::thread::spawn(move || {
        let result = job();
        let what = result.clone().unwrap_or_default();
        push(&inbox, StoreMsg::Done { what, result: result.map(|_| ()) }, &waker);
    });
}

/// `https://github.com/dzhibas/tomboy/tree/main/extensions/<id>` for the default registry.
fn repo_url(base: &str, id: &str) -> String {
    match base.strip_prefix("https://raw.githubusercontent.com/") {
        Some(rest) => {
            let mut parts = rest.splitn(3, '/');
            let (owner, repo, branch) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or("main"));
            format!("https://github.com/{owner}/{repo}/tree/{branch}/extensions/{id}")
        }
        None => format!("{base}/extensions/{id}"),
    }
}

/// Esc / Backspace in the store: cancel the consent screen, clear the field, then go back.
pub fn store_key(s: &mut Tomboy, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let Some(store) = s.store.as_mut() else { return false };
    if store.consent.is_some() {
        match ev.key {
            Key::Escape | Key::Backspace => { store.consent = None; return true; }
            Key::Enter => { accept(s, tx); return true; }
            _ => return false,
        }
    }
    let empty = s.query.text.is_empty();
    match ev.key {
        Key::Escape if !empty => { s.query.clear(); s.last_text.clear(); s.picked = 0; }
        Key::Escape | Key::Backspace if empty => { s.store = None; clear_field(s); }
        _ => return false,
    }
    true
}

/// Enter while the consent screen is up (the field's submit).
pub fn store_submit(s: &mut Tomboy, tx: &mut Tx) -> bool {
    if s.store.as_ref().is_some_and(|st| st.consent.is_some()) { accept(s, tx); return true; }
    false
}

// ---- The action panel ----

/// Ctrl/⌘+K: every action of the picked row.
pub fn open_panel(s: &mut Tomboy) -> bool {
    let actions: Vec<PanelAction> = if s.session.is_some() {
        session_actions(s).into_iter().map(PanelAction::Ext).collect()
    } else if s.store.is_some() {
        store_rows(s).get(s.picked).map(|r| store_actions(r).into_iter().map(PanelAction::Store).collect()).unwrap_or_default()
    } else {
        return false;
    };
    if actions.is_empty() { return true; }
    s.panel = Some(Panel { actions, picked: 0 });
    true
}

/// Keys while the panel is open.
pub fn panel_key(s: &mut Tomboy, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let Some(p) = s.panel.as_mut() else { return false };
    let n = p.actions.len();
    match ev.key {
        Key::Down => p.picked = (p.picked + 1) % n,
        Key::Up => p.picked = (p.picked + n - 1) % n,
        Key::Escape => s.panel = None,
        Key::Enter => run_panel(s, tx),
        _ if ev.is_command('k') => s.panel = None,
        _ => return true, // the panel has the keyboard
    }
    true
}

pub fn run_panel(s: &mut Tomboy, tx: &mut Tx) {
    let Some(p) = s.panel.take() else { return };
    match p.actions.get(p.picked).cloned() {
        Some(PanelAction::Ext(a)) => run_action(s, a, tx),
        Some(PanelAction::Store(a)) => run_store_action(s, a, tx),
        None => {}
    }
}

pub fn has_panel_actions(s: &Tomboy) -> bool {
    s.session.is_some() || s.store.is_some()
}

/// How the action panel opens on this platform.
fn panel_key_hint() -> &'static str { if cfg!(target_os = "macos") { "⌘K" } else { "ctrl+k" } }

fn panel_view(p: &Panel, t: &tok::Theme, accent: Color) -> El {
    let mut list = col().key("action-panel").w(260.0).padding(6.0).gap(2.0).radius(10.0).fill(t.card)
        .border(1.0, t.separator).shadow(8.0, 24.0, Color::BLACK.alpha(0.35));
    for (i, a) in p.actions.iter().enumerate() {
        let on = i == p.picked;
        list = list.child(row().key(format!("panel-{i}")).h(ty::px(32.0)).px(10.0).radius(6.0).center_y()
            .fill(if on { accent.alpha(0.85) } else { Color::TRANSPARENT })
            .child(text(a.title()).font(ty::body()).color(if on { Color::WHITE } else { t.text }).truncate().grow(1.0).min_w(0.0))
            .child(text(if i == 0 { "↵" } else { "" }).font(ty::label()).color(if on { Color::WHITE } else { t.text_secondary }))
            .on_hover(move |s: &mut Tomboy| if let Some(p) = &mut s.panel { p.picked = i })
            .on_press_tx(move |s: &mut Tomboy, tx: &mut Tx| { if let Some(p) = &mut s.panel { p.picked = i } run_panel(s, tx) }));
    }
    list.pin(None, None, Some(12.0), Some(12.0))
}

/// `body` with the action panel over its bottom-right corner when it's open.
fn with_panel(s: &Tomboy, body: El, t: &tok::Theme, accent: Color) -> El {
    match &s.panel {
        Some(p) => col().w_full().grow(1.0).min_h(0.0).child(body).child(panel_view(p, t, accent)),
        None => body,
    }
}

// ---- Drawing ----

pub fn crumb(s: &Tomboy) -> Option<String> {
    if let Some(sess) = &s.session {
        let m = &s.extensions[sess.ext].0;
        let title = sess.stack.last().and_then(|v| match v { View::List(l) => l.title.clone(), View::Detail(d) => d.title.clone() });
        return Some(title.unwrap_or_else(|| m.name.clone()));
    }
    s.store.as_ref().map(|_| "Extensions".to_string())
}

pub fn placeholder(s: &Tomboy) -> Option<String> {
    if s.session.is_some() {
        return Some(top_list(s).and_then(|l| l.search_placeholder.clone()).unwrap_or_else(|| "Search…".into()));
    }
    s.store.as_ref().map(|_| "Search extensions…".to_string())
}

fn icon_box(glyph: &str, c: Color) -> El {
    row().size(28.0, 28.0).radius(7.0).center().fill(c)
        .child(text(glyph.to_string()).font(rune::tok::type_::BODY.lh(18.0).family(NERD_FONT)).color(Color::WHITE))
}

#[allow(clippy::too_many_arguments)]
fn list_row(i: usize, on: bool, icon: El, title: String, sub: Option<String>, accessories: Vec<String>,
            t: &tok::Theme, accent: Color, on_press: impl Fn(&mut Tomboy, &mut Tx) + 'static) -> El {
    let fg = if on { Color::WHITE } else { t.text };
    let fg2 = if on { Color::WHITE.alpha(0.75) } else { t.text_secondary };
    let mut r = row().key(format!("item-{i}")).h(ty::px(46.0)).px(12.0).gap(10.0).radius(9.0).center_y()
        .fill(if on { accent.alpha(0.85) } else { Color::TRANSPARENT })
        .child(icon)
        .child(col().grow(1.0).min_w(0.0).gap(1.0)
            .child(text(title).font(ty::body()).color(fg).truncate())
            .child(match sub { Some(sub) => text(sub).font(ty::label()).color(fg2).truncate(), None => el() }));
    for a in accessories {
        r = r.child(text(a).font(ty::label()).color(fg2));
    }
    r.on_hover(move |s: &mut Tomboy| s.picked = i)
        .on_press_tx(move |s: &mut Tomboy, tx: &mut Tx| { s.picked = i; on_press(s, tx) })
}

fn empty_note(msg: &str, t: &tok::Theme) -> El {
    col().h(120.0).w_full().center().child(text(msg.to_string()).font(ty::body()).color(t.text_secondary))
}

fn markdown_pane(src: &str, dark: bool) -> El {
    col().grow(1.0).min_w(0.0).h_full().scroll(ScrollSpec { stick_bottom: false }).padding(16.0)
        .child(Markdown::new(src.trim()).render(&ty::markdown(dark)))
}

/// A running extension's view: its list (with the picked item's detail beside it) or a detail.
pub fn session_body(s: &Tomboy, dark: bool, t: &tok::Theme, accent: Color) -> (El, String, (&'static str, &'static str)) {
    let sess = s.session.as_ref().expect("session");
    let name = &s.extensions[sess.ext].0.name;
    let ext_icon = s.extensions[sess.ext].0.icon.clone().unwrap_or_else(|| EXT_GLYPH.into());
    let back = if sess.stack.len() > 1 { "esc to go back" } else { "esc to close" };
    let body = match sess.stack.last() {
        None if sess.error.is_some() => col().w_full().grow(1.0).center().padding(24.0)
            .child(text(sess.error.clone().unwrap_or_default()).font(ty::body()).color(ERROR_COLOR).wrap()),
        None => col().w_full().grow(1.0).center().child(text("Loading…").font(ty::body()).color(t.text_secondary)),
        Some(View::Detail(d)) => col().w_full().grow(1.0).min_h(0.0).child(markdown_pane(&d.markdown, dark)),
        Some(View::List(l)) => {
            let matches = l.matches(&s.query.text);
            let mut list = col().key("results").w_full().padding(8.0);
            for (i, &it) in matches.iter().enumerate() {
                let item = &l.items[it];
                let icon = icon_box(item.icon.as_deref().unwrap_or(&ext_icon), EXT_COLOR);
                list = list.child(list_row(i, i == s.picked, icon, item.title.clone(), item.subtitle.clone(),
                    item.accessories.clone(), t, accent, move |s, tx| run_row(s, it, tx)));
            }
            if matches.is_empty() {
                let msg = if l.loading { "Loading…" } else if l.items.is_empty() { l.empty_text.as_deref().unwrap_or("Nothing here") } else { "No matches" };
                list = list.child(empty_note(msg, t));
            } else if l.loading {
                list = list.child(row().h(ty::px(28.0)).px(12.0).center_y().child(text("Refreshing…").font(ty::label()).color(t.text_secondary)));
            }
            let has_detail = l.items.iter().any(|i| i.detail.is_some());
            let scroller = |w: Option<f32>| {
                let c = col().h_full().min_h(0.0).scroll(ScrollSpec { stick_bottom: false });
                match w { Some(w) => c.w(w), None => c.w_full() }
            };
            if has_detail {
                let detail = picked_item(s).and_then(|i| i.detail.clone()).unwrap_or_default();
                row().w_full().grow(1.0).min_h(0.0)
                    .child(scroller(Some(420.0)).child(list))
                    .child(el().w(1.0).h_full().fill(t.separator))
                    .child(markdown_pane(&detail, dark))
            } else {
                col().w_full().grow(1.0).min_h(0.0).child(scroller(None).child(list))
            }
        }
    };
    let actions = session_actions(s);
    let left = s.status.clone().unwrap_or_else(|| {
        let more = if actions.len() > 1 { format!(" · {} actions", panel_key_hint()) } else { String::new() };
        format!("{name}{} · {back}{more}", if sess.running { " · working…" } else { "" })
    });
    let action = actions.first().map(|a| leak(a.title())).unwrap_or("");
    (with_panel(s, body, t, accent), left, (action, "↵"))
}

/// The footer shows `&'static str` actions; extension titles are few and short-lived views
/// reuse the same strings, so intern them.
fn leak(title: &str) -> &'static str {
    static SEEN: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = seen.iter().find(|s| **s == title) { return s; }
    let s: &'static str = Box::leak(title.to_string().into_boxed_str());
    seen.push(s);
    s
}

/// The store: installed and available extensions, the picked one's README and permissions.
pub fn store_body(s: &Tomboy, dark: bool, t: &tok::Theme, accent: Color) -> (El, String, (&'static str, &'static str)) {
    let store = s.store.as_ref().expect("store");
    if let Some(id) = &store.consent {
        return consent_body(s, id, t, accent);
    }
    let rows = store_rows(s);
    let mut list = col().key("results").w_full().padding(8.0);
    let mut section = None;
    for (i, r) in rows.iter().enumerate() {
        let name = if r.installed.is_some() { "Installed" } else { "Available" };
        if section != Some(name) {
            section = Some(name);
            list = list.child(row().h(ty::px(28.0)).px(12.0).center_y().child(text(name).font(ty::label()).color(t.text_secondary)));
        }
        let mut acc = vec![format!("v{}", r.installed.as_ref().map_or(&r.manifest.version, |x| &x.version))];
        match &r.status {
            Status::NotInstalled => {}
            other => acc.push(other.label()),
        }
        if r.installed.as_ref().is_some_and(|x| !x.enabled) { acc.push("Disabled".into()); }
        let icon = icon_box(r.manifest.icon.as_deref().unwrap_or(EXT_GLYPH), EXT_COLOR);
        list = list.child(list_row(i, i == s.picked, icon, r.manifest.name.clone(), Some(r.manifest.description.clone()),
            acc, t, accent, move |s, tx| run_store_row(s, i, tx)));
    }
    if rows.is_empty() {
        let msg = if store.busy.is_some() && store.index.is_none() { "Loading extensions…" }
            else if store.index.is_none() { "Couldn't reach the extension registry" }
            else if s.query.text.is_empty() { "No extensions yet" } else { "No matching extensions" };
        list = list.child(empty_note(msg, t));
    }
    let detail = match rows.get(s.picked) {
        Some(r) => store_detail(r),
        None => String::new(),
    };
    let body = row().w_full().grow(1.0).min_h(0.0)
        .child(col().w(340.0).h_full().min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).child(list))
        .child(el().w(1.0).h_full().fill(t.separator))
        .child(markdown_pane(&detail, dark));
    let available = store.index.as_ref().map_or(0, |i| i.extensions.len());
    let updates = rows.iter().filter(|r| matches!(r.status, Status::UpdateAvailable(_))).count();
    let left = s.status.clone()
        .or_else(|| store.busy.clone())
        .or_else(|| store.error.as_ref().filter(|_| store.index.is_none()).map(|_| "Couldn't reach the extension registry".to_string()))
        .unwrap_or_else(|| {
            let mut parts = vec![format!("{available} available"), format!("{} installed", store.installed.0.len())];
            if updates > 0 { parts.push(format!("{updates} update{}", if updates == 1 { "" } else { "s" })); }
            format!("{} · {} actions", parts.join(" · "), panel_key_hint())
        });
    let action = rows.get(s.picked).and_then(|r| store_actions(r).into_iter().next())
        .map(|a| leak(&PanelAction::Store(a).title())).unwrap_or("");
    (with_panel(s, body, t, accent), left, (action, "↵"))
}

fn store_detail(r: &StoreRow) -> String {
    let m = &r.manifest;
    let perms = m.permissions.describe();
    let perms = if perms.is_empty() { "- None beyond its own storage".to_string() }
        else { perms.iter().map(|p| format!("- {p}")).collect::<Vec<_>>().join("\n") };
    let commands = m.commands.iter().map(|c| format!("- **{}**{}", c.title,
        if c.keywords.is_empty() { String::new() } else { format!(" (`{}`)", c.keywords.join("`, `")) })).collect::<Vec<_>>().join("\n");
    let unsupported = match m.supported() { Err(e) => format!("\n\n**{e}**"), Ok(()) => String::new() };
    let readme = strip_title(&r.readme);
    format!("## {}\n\n{}{unsupported}\n\nv{} · by {} · {}\n\n### Permissions\n\n{perms}\n\n### Commands\n\n{commands}\n\n{readme}",
        m.name, m.description, m.version, m.author, m.license)
}

/// A README's own `# Title` repeats the name above it.
fn strip_title(readme: &str) -> String {
    let t = readme.trim_start();
    match t.strip_prefix("# ") {
        Some(rest) => rest.split_once('\n').map_or("", |(_, body)| body).trim_start().to_string(),
        None => t.to_string(),
    }
}

fn consent_body(s: &Tomboy, id: &str, t: &tok::Theme, accent: Color) -> (El, String, (&'static str, &'static str)) {
    let rows = store_rows(s);
    let row_ = rows.iter().find(|r| r.id == id);
    let (name, perms, updating, glyph) = match row_ {
        Some(r) => {
            let asked = r.entry.as_ref().map(|e| install::needs_consent(e, r.installed.as_ref())).unwrap_or_default();
            (r.manifest.name.clone(), asked.describe(), r.installed.is_some(), r.manifest.icon.clone().unwrap_or_else(|| EXT_GLYPH.into()))
        }
        None => (id.to_string(), Vec::new(), false, EXT_GLYPH.to_string()),
    };
    let title = if updating { format!("Update {name}?") } else { format!("Install {name}?") };
    let intro = if updating { "This update asks for new permissions:" }
        else if perms.is_empty() { "It needs no permissions beyond its own storage." }
        else { "It asks for these permissions:" };
    let mut card = col().w(420.0).padding(24.0).gap(12.0).radius(12.0).fill(t.control_off)
        .child(row().gap(12.0).center_y().child(icon_box(&glyph, EXT_COLOR))
            .child(text(title).font(ty::title()).color(t.text)))
        .child(text(intro).font(ty::body()).color(t.text_secondary).wrap());
    for p in perms {
        card = card.child(row().gap(8.0).child(text("•").font(ty::body()).color(accent)).child(text(p).font(ty::body()).color(t.text)));
    }
    card = card.child(text("Every extension is reviewed before it is listed, and runs sandboxed.")
        .font(ty::label()).color(t.text_secondary).wrap());
    card = card.child(row().gap(10.0).child(spacer())
        .child(row().key("consent-cancel").h(ty::px(30.0)).px(14.0).gap(8.0).radius(7.0).center().fill(t.card)
            .child(text("Cancel").font(ty::body()).color(t.text)).child(keycap("esc", t))
            .on_press(|s: &mut Tomboy| if let Some(st) = &mut s.store { st.consent = None }))
        .child(row().key("consent-install").h(ty::px(30.0)).px(14.0).gap(8.0).radius(7.0).center().fill(accent)
            .child(text(if updating { "Update" } else { "Install" }).font(ty::body()).color(Color::WHITE))
            .on_press_tx(|s: &mut Tomboy, tx: &mut Tx| accept(s, tx))));
    let body = col().w_full().grow(1.0).center().child(card);
    let left = s.status.clone().unwrap_or_else(|| "Extensions · esc to cancel".into());
    (body, left, (if updating { "Update" } else { "Install" }, "↵"))
}

// ---- Headless scenes (`--snap … --scene store`) ----

/// The store listing this checkout's `extensions/` (no network, nothing installed).
pub fn store_scene(s: &mut Tomboy, consent: bool) {
    let index = crate::ext::index::build(std::path::Path::new("extensions")).ok();
    let first = index.as_ref().and_then(|i| i.extensions.first()).map(|e| e.manifest.id.clone());
    s.store = Some(Store {
        index, error: None, installed: Installed::default(), consent: if consent { first } else { None },
        busy: None, inbox: Arc::default(),
    });
    s.direct = true;
}

/// The Hacker News sample running against `tests/fixtures/hn-front-page.json`.
pub fn hackernews_scene(s: &mut Tomboy) {
    use crate::ext::runtime::Options;
    let dir = std::path::Path::new("extensions/hackernews").to_path_buf();
    let Ok(manifest) = Manifest::load(&dir) else { return };
    let fixture = std::fs::read_to_string("tests/fixtures/hn-front-page.json").unwrap_or_default();
    let tmp = std::env::temp_dir().join(format!("tomboy-scene-{}", std::process::id()));
    let opts = Options {
        fixtures: [("https://hn.algolia.com/api/v1/search?tags=front_page&hitsPerPage=50".to_string(), fixture)].into(),
        state_dir: Some(tmp.join("state")), cache_dir: Some(tmp.join("cache")),
    };
    let prefs = manifest.preference_values(None);
    let Ok(rt) = Runtime::start_with(manifest.clone(), &dir, prefs, opts) else { return };
    let mut events = rt.run("front-page", "");
    for _ in 0..5 { events.extend(rt.poll(f64::MAX / 4.0, &mut Tx::default())); }
    let _ = std::fs::remove_dir_all(&tmp);
    let mut stack: Vec<View> = Vec::new();
    for ev in events {
        if let Event::View(v) = ev { match stack.last_mut() { Some(top) => *top = v, None => stack.push(v) } }
    }
    s.extensions = vec![(manifest, dir)];
    s.session = Some(Session { rt, ext: 0, command: "front-page".into(), stack, error: None, running: false });
    s.picked = 1;
    s.direct = true;
}

/// The launcher with the Hacker News sample installed and `hn` typed.
pub fn command_scene(s: &mut Tomboy) {
    let dir = std::path::Path::new("extensions/hackernews").to_path_buf();
    if let Ok(m) = Manifest::load(&dir) { s.extensions = vec![(m, dir)]; }
    s.query = rune::TextEdit::new("hn");
}

/// Whether the store's consent screen is up (for tests).
#[cfg(test)]
pub fn consent_open(s: &Tomboy) -> bool {
    s.store.as_ref().is_some_and(|st| st.consent.is_some())
}
