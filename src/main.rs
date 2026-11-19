//! tomboy — a personal Raycast-style launcher built with Rune.
//!
//! Type to search installed applications, evaluate math (`2+2`), or open a URL.
//! ↑/↓ pick, Enter runs, Esc, Super+W (or clicking away) closes.
//!
//! "Ask agent" (or dictating with Superwhisper) runs the coding agent headless and streams
//! its answer into tomboy; the field then takes follow-ups in the same session.
//!
//! `tomboy clipboard` opens straight into the clipboard history (also "Clipboard History" in
//! the launcher); `tomboy clipboard-watch` records that history where Omarchy doesn't.
//!
//! Headless checks (no window):
//!   cargo run -- --snap shot.png --scene calc
//!   cargo run -- --tree tree.json --scene apps

mod agent;
mod agent_history;
mod answers;
mod apps;
mod calc;
mod clipboard;
mod config;
mod dictation;
mod emoji;
mod ext;
mod extui;
mod fonts;
mod frecency;
#[cfg(target_os = "macos")]
mod mac;
// Omarchy is Linux-only; on macOS the module compiles but `Platform::Omarchy` never occurs.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod omarchy;
mod platform;
mod quicklinks;
#[cfg(unix)]
mod single;
mod sys;
mod todo;
mod trace;
mod url;

use rune::tok::{self, type_};
use rune::{
    Anim, App, AppSpec, Color, Cursor, Cx, Drag, El, Phase, Family, Image, ImageCache, Key, KeyEvent, ScrollSpec, SurfaceFit, TextEdit, Tx,
    WindowOptions, Waker, GrowthSpec, col, el, row, spacer, text, text_input,
};
use rune_kit::Markdown;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use agent::{Block, Status};
use answers::{Answer, Kind};
use apps::AppEntry;
use clipboard::Clip;
use config::Hotkey;
use omarchy::{Conditions, MenuItem};
use platform::Platform;

const WIDTH: f32 = 720.0;
const HEIGHT: f32 = 440.0;
/// Apps listed for an empty query, and per search.
const MAX_APPS: usize = 30;
const MAX_APP_HITS: usize = 8;
const MAX_MENU_HITS: usize = 8;
const ICON: f32 = 28.0;

/// Text styles scaled by `[ui] font_size` (read once at launch), and sizes that follow text.
/// Icons keep their size.
mod ty {
    use rune::TextStyle;
    use rune::tok::type_;
    use std::sync::OnceLock;

    static SCALE: OnceLock<f32> = OnceLock::new();

    pub fn set_scale(k: f32) { let _ = SCALE.set(k); }
    pub fn scale() -> f32 { SCALE.get().copied().unwrap_or(1.0) }

    /// A height or line height that holds text, scaled with it.
    pub fn px(v: f32) -> f32 { (v * scale()).round() }
    fn sized(t: TextStyle) -> TextStyle {
        let k = scale();
        TextStyle { size: t.size * k, line_height: t.line_height * k, ..t }
    }
    /// The search field, 28 pt lines.
    pub fn title() -> TextStyle { sized(type_::TITLE.lh(28.0)) }
    pub fn body() -> TextStyle { sized(type_::BODY) }
    pub fn label() -> TextStyle { sized(type_::LABEL) }
    /// A glyph in the emoji picker.
    pub fn emoji() -> TextStyle { sized(TextStyle::new(32.0, 400.0).lh(40.0)) }

    /// Rune's neutral Markdown look with its text sizes scaled (sizes as `MarkdownStyle::from_kit`).
    pub fn markdown(dark: bool) -> rune_kit::MarkdownStyle {
        let kit = rune_kit::Kit::neutral(dark);
        let body = TextStyle { line_height: 22.4, ..kit.ty.body };
        let heading = |size, lh| sized(TextStyle { size, weight: 600.0, line_height: lh, ..body });
        rune_kit::MarkdownStyle::from_kit(&kit)
            .body_font(sized(body))
            .heading_font(1, heading(17.0, 24.0))
            .heading_font(2, heading(15.0, 24.0))
            .heading_font(3, heading(13.0, 20.0))
            .heading_font(4, heading(13.0, 20.0))
            .code_block_font(sized(TextStyle { size: 13.0, line_height: 20.0, ..kit.ty.mono }))
    }
}
/// Omarchy's menu icons are Nerd Font glyphs.
const NERD_FONT: Family = Family::Named("JetBrainsMono Nerd Font");
/// Nerd Font robot (nf-md-robot), drawn with `NERD_FONT`.
const AGENT_GLYPH: &str = "\u{f06a9}";
const AGENT_COLOR: Color = Color { r: 0.85, g: 0.47, b: 0.34, a: 1.0 };

struct Tomboy {
    query: TextEdit,
    picked: usize,
    apps: Vec<AppEntry>,
    icons: Vec<Option<Image>>,
    cache: Option<ImageCache>,
    platform: Platform,
    /// The platform's own commands (Omarchy's menu), with their conditions.
    menu: Vec<MenuItem>,
    conds: Conditions,
    /// A message for the footer (a launch error, "Copied").
    status: Option<String>,
    /// The field's text after the previous edit, to see what each edit inserted.
    last_text: String,
    /// Set by the dictation check with the prompt to hand to the agent.
    dictated: Arc<Mutex<Option<String>>>,
    waker: Option<Waker>,
    /// Set by every background wake (agent output, currency rates, menu conditions), so
    /// `poll` tells Rune the state changed and the window redraws without input.
    woke: Arc<AtomicBool>,
    /// The agent conversation; while it exists tomboy shows it instead of search results.
    chat: Option<agent::Run>,
    /// Past agent conversations ("Agent History"), newest first, while the list is open. A
    /// conversation opened from it shows over the list, and Esc comes back to it.
    history: Option<Vec<agent_history::Saved>>,
    /// The Omarchy submenu being browsed (`install`, `appearance.top-bar`); `None` at the root.
    submenu: Option<String>,
    /// Random bytes for this launch, so `uuid`/`password` answers hold still while typing.
    seed: [u8; 32],
    /// Wakes the window when a background answer (currency rates) arrives.
    wake: answers::Wake,
    /// The clipboard history being browsed, newest first; while set, tomboy lists it instead
    /// of search results.
    clips: Option<Vec<Clip>>,
    /// Opened as `tomboy clipboard` / `tomboy emoji`: Esc closes instead of going back to the
    /// launcher.
    direct: bool,
    /// The emoji & symbol picker; while set, tomboy shows its grid.
    emoji: Option<EmojiView>,
    /// `~/.config/tomboy/config.toml`, read at launch.
    config: config::Config,
    /// What was launched, when and under which query, to rank usual picks first.
    frecency: frecency::Store,
    /// The picked clip's picture (kept here: the cache drops an image nobody holds).
    clip_image: Option<(std::path::PathBuf, Image)>,
    /// `[quicklinks]` from the config.
    links: Vec<quicklinks::Quicklink>,
    /// The quicklink whose argument is being typed (argument mode).
    quicklink: Option<usize>,
    /// Today's todo list, while it's open.
    todo: Option<todo::Day>,
    /// A task being dragged by its grip to a new place in the list.
    task_drag: Option<TaskDrag>,
    /// The task whose text is in the field to edit: Enter saves it there, Esc cancels.
    editing_task: Option<todo::Task>,
    /// Today's (done, total), for the launcher row; kept up to date as the list changes.
    todo_counts: (usize, usize),
    /// Running as `tomboy daemon` (macOS): closing hides the window and resets it for the
    /// next hotkey instead of quitting.
    resident: bool,
    /// This app's handle, for the daemon's hotkeys to show and hide the window.
    me: Option<rune::AppHandle<Tomboy>>,
    /// The daemon's window is on screen, and the hotkey that showed it.
    shown: Option<Hotkey>,
    /// A clip or emoji was just copied: paste it into the front app once the window hides.
    paste: bool,
    /// The window has had the keyboard since it last showed. Until then a focus loss is
    /// stale (macOS reports one before a new window's first focus) and doesn't close it.
    had_focus: bool,
    /// Installed, enabled extensions (`ext::install::installed_dirs`), plus `tomboy ext dev`'s.
    extensions: Vec<(ext::manifest::Manifest, std::path::PathBuf)>,
    /// A running extension command, while its views are on screen.
    session: Option<extui::Session>,
    /// The extension store ("Install Extension"), while it's open.
    store: Option<extui::Store>,
    /// The action panel (Ctrl/⌘+K) over an extension view or the store.
    panel: Option<extui::Panel>,
    /// `tomboy ext dev <dir>`: open its first command once the window is up.
    dev_open: bool,
}

/// `tomboy ext dev <dir>`: the extension folder being developed.
static DEV_DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Set by `tomboy daemon` before the app starts, for `init`.
static RESIDENT: AtomicBool = AtomicBool::new(false);

/// The emoji picker's grid for the current query, and the picked cell.
struct EmojiView {
    rows: Vec<emoji::Row>,
    pos: Option<emoji::Pos>,
    /// Bumped whenever `rows` change, so the virtual grid re-keys its rows.
    version: u64,
}

/// One row in the result list.
#[derive(Clone, Debug, PartialEq)]
enum Item {
    Calc { expr: String, value: String },
    /// Unit/currency conversion, time zone, developer utility or process.
    Answer(Answer),
    Url(String),
    App(usize),
    Menu(usize),
    WebSearch(String),
    Agent(String),
    /// "Clipboard History" in the launcher.
    Clipboard,
    /// "Emoji & Symbols" in the launcher.
    Emoji,
    /// A clip in the history.
    Clip(usize),
    /// A quicklink, with the argument typed for it so far.
    Quicklink { link: usize, arg: Option<String> },
    /// "Today · 3/10 done" in the launcher.
    Todo,
    /// `todo buy milk`: add it to today's list.
    TodoAdd(String),
    /// A task in the open list, by its place in `Day::tasks`.
    Task(usize),
    /// An installed extension's command.
    Extension { ext: usize, command: usize },
    /// "Install Extension": the store.
    Store,
    /// A row of the open extension list, by its place in the view's items.
    ExtRow(usize),
    /// A row of the store, by its place in `extui::store_rows`.
    StoreRow(usize),
    /// "Agent History" in the launcher.
    AgentHistory,
    /// A past conversation in the history, by its place in `Tomboy::history`.
    Past(usize),
}

impl Item {
    fn action(&self, s: &Tomboy) -> &'static str {
        match self {
            Item::Calc { .. } => "Copy Answer",
            Item::Answer(a) => match a.action {
                answers::Action::Copy(_) => "Copy Answer",
                answers::Action::Kill { .. } => "Kill Process",
                answers::Action::None => "",
            },
            Item::Url(_) => "Open in Browser",
            Item::App(_) => "Open Application",
            Item::Menu(i) if omarchy::has_children(&s.menu, &s.menu[*i].id) => "Open Submenu",
            Item::Menu(i) if s.menu[*i].is_submenu() => "Open in Omarchy Menu",
            Item::Menu(_) => "Run Command",
            Item::WebSearch(_) => "Search the Web",
            Item::Agent(_) => "Ask Agent",
            Item::Clipboard => "Open Clipboard History",
            Item::Emoji => "Open Emoji Picker",
            Item::Clip(_) => clip_action(s),
            Item::Quicklink { link, arg } => {
                let l = &s.links[*link];
                if l.arg.is_some() && arg.is_none() { "Enter Argument" }
                else if l.is_web() { "Open in Browser" } else { "Open Link" }
            }
            Item::Todo => "Open Todo List",
            Item::TodoAdd(_) => "Add Todo",
            Item::Task(i) => match s.todo.as_ref().and_then(|d| d.tasks().get(*i).map(|t| t.done)) {
                Some(true) => "Mark Open",
                _ => "Mark Done",
            },
            Item::Extension { .. } => "Open Command",
            Item::Store => "Open Extension Store",
            Item::AgentHistory => "Open Agent History",
            Item::Past(_) => "Open Conversation",
            Item::ExtRow(_) | Item::StoreRow(_) => "",
        }
    }
}

/// What tomboy does after running an item.
enum After {
    Quit,
    Stay,
}

fn init() -> Tomboy {
    let started = std::time::Instant::now();
    let (config, config_error) = trace::span("init: config::load", config::load);
    let frecency = trace::span("init: frecency", || if config.frecency() { frecency::Store::load() } else { frecency::Store::off() });
    ty::set_scale(config.font_scale());
    let status = config_error.or_else(|| config.warning());
    let mut query = TextEdit::new("");
    query.focused = true;
    let apps = trace::span("init: apps::load_apps", apps::load_apps);
    let platform = trace::span("init: platform::detect", platform::detect);
    let resident = RESIDENT.load(Ordering::Relaxed);
    let menu = trace::span("init: menu", || match &platform {
        Platform::Omarchy { root } => omarchy::load(root),
        #[cfg(target_os = "macos")]
        Platform::MacOS => mac::menu::items(resident),
        _ => Vec::new(),
    });
    // Omarchy's conditions run in `setup`; the macOS commands have none, so all show at once.
    let conds = match &platform {
        Platform::MacOS => omarchy::evaluate(&menu, std::path::Path::new("/"), || {}),
        _ => Conditions::default(),
    };
    let links = trace::span("init: quicklinks::load", || quicklinks::load(&config));
    let todo_counts = trace::span("init: todo::peek", || config.todo_dir().map_or((0, 0), |d| todo::peek(&d, todo::today_date())));
    let extensions = trace::span("init: load_extensions", load_extensions);
    let seed = trace::span("init: devtools::session_seed", answers::devtools::session_seed);
    if trace::on() { trace::mark(&format!("init: {} apps, {} menu rows, {} extensions", apps.len(), menu.len(), extensions.len())); }
    let mut s = Tomboy {
        query, picked: 0, icons: vec![None; apps.len()], apps, cache: None,
        platform, menu, conds, status,
        last_text: String::new(), dictated: Arc::default(), waker: None, woke: Arc::default(), chat: None, history: None, submenu: None,
        seed, wake: Arc::new(|| {}),
        clips: None, direct: false, emoji: None, clip_image: None, frecency,
        links, quicklink: None,
        todo: None, task_drag: None, editing_task: None, todo_counts,
        config, resident, me: None, shown: None, paste: false, had_focus: false,
        extensions, session: None, store: None, panel: None, dev_open: DEV_DIR.get().is_some(),
    };
    if rune::extra_args().iter().any(|a| a == "clipboard") {
        enter_clipboard(&mut s);
        s.direct = true;
    } else if rune::extra_args().iter().any(|a| a == "emoji") {
        enter_emoji(&mut s);
        s.direct = true;
    } else if rune::extra_args().iter().any(|a| a == "todo") {
        enter_todo(&mut s);
        s.direct = true;
    } else if rune::extra_args().iter().any(|a| a == "agents") {
        enter_history(&mut s);
        s.direct = true;
    } else if rune::extra_args().iter().any(|a| a == "extensions") {
        extui::open_store(&mut s);
        s.direct = true;
    }
    trace::since("init (total)", started);
    s
}

/// The installed extensions, and the one `tomboy ext dev` runs (first, replacing an installed
/// copy of it).
fn load_extensions() -> Vec<(ext::manifest::Manifest, std::path::PathBuf)> {
    let mut exts = ext::install::installed_dirs();
    if let Some(dir) = DEV_DIR.get() {
        match ext::manifest::Manifest::load(dir) {
            Ok(m) => { exts.retain(|(x, _)| x.id != m.id); exts.insert(0, (m, dir.clone())); }
            Err(e) => eprintln!("tomboy ext dev: {e}"),
        }
    }
    exts
}

/// Opens once the window's first frame is encoded, for work that would slow that frame down
/// (`omarchy::evaluate_gated`). Waiters give up waiting after `LIMIT`, so a window that never
/// draws (hidden, headless without frames) still gets its work done.
mod first_frame {
    use std::sync::{Condvar, Mutex};
    use std::time::Duration;

    const LIMIT: Duration = Duration::from_millis(500);
    static OPEN: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

    pub fn open() {
        let mut open = OPEN.0.lock().unwrap_or_else(|e| e.into_inner());
        if !*open { *open = true; OPEN.1.notify_all(); }
    }

    pub fn wait() {
        let open = OPEN.0.lock().unwrap_or_else(|e| e.into_inner());
        let _ = OPEN.1.wait_timeout_while(open, LIMIT, |open| !*open);
    }

    /// For `App::on_frame_encoded`.
    pub fn on_frame(f: &rune::FrameEncoded) {
        crate::trace::frame(f);
        if f.outcome() == rune::FrameOutcome::Drawn { open(); }
    }
}

/// Start decoding every app's icon on Rune's image worker and checking the menu's shell
/// conditions on a background pool (after the first frame); each result wakes the window.
fn setup(s: &mut Tomboy, waker: Waker) {
    let started = std::time::Instant::now();
    let woke = s.woke.clone();
    let waker: Waker = match s.me.clone() {
        // A shown launcher panel (the daemon's) redraws only for an update that changed
        // something, never for a bare wake, so wake it with one.
        Some(me) => Arc::new(move || { trace::wake(); woke.store(true, Ordering::Release); me.update_if(|_, _| true) }),
        None => Arc::new(move || { trace::wake(); woke.store(true, Ordering::Release); waker() }),
    };
    let cache = ImageCache::new(waker.clone());
    s.icons = trace::span("setup: request app icons", || s.apps.iter().map(|a| a.icon.as_ref().map(|_| cache.app_icon(&a.path, ICON))).collect());
    s.cache = Some(cache);
    if let Platform::Omarchy { root } = &s.platform {
        let waker = waker.clone();
        s.conds = omarchy::evaluate_gated(&s.menu, root, first_frame::wait, move || waker());
    }
    s.wake = waker.clone();
    if s.dev_open || s.store.is_some() { waker(); }
    #[cfg(unix)]
    if !s.resident { single::set_waker(waker.clone()); }
    s.waker = Some(waker);
    sync_clip_image(s);
    trace::since("setup (total)", started);
}

/// After every edit: if one change inserted a burst of text, check on a thread whether it
/// was a Superwhisper dictation (the history entry can land just after the text does).
fn on_change(s: &mut Tomboy, tx: &mut Tx) {
    s.picked = 0;
    s.status = None;
    s.panel = None;
    if s.session.is_some() || s.store.is_some() {
        s.last_text = s.query.text.clone();
        extui::on_change(s, tx);
        return;
    }
    if s.clips.is_some() {
        s.last_text = s.query.text.clone();
        sync_clip_image(s);
        return;
    }
    if s.history.is_some() && s.chat.is_none() {
        s.last_text = s.query.text.clone();
        return;
    }
    if s.emoji.is_some() {
        s.last_text = s.query.text.clone();
        rebuild_emoji(s);
        return;
    }
    if s.todo.is_some() {
        s.last_text = s.query.text.clone();
        return;
    }
    let burst = dictation::inserted(&s.last_text, &s.query.text).to_string();
    s.last_text = s.query.text.clone();
    if burst.trim().chars().count() < dictation::MIN_BURST { return; }
    let (prompt, slot, waker) = (s.query.text.clone(), s.dictated.clone(), s.waker.clone());
    std::thread::spawn(move || {
        for _ in 0..10 {
            if dictation::is_from_superwhisper(&burst) {
                *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(prompt);
                if let Some(wake) = waker { wake(); }
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(60));
        }
    });
}

/// Redraw after a background wake; a confirmed dictation goes straight to the agent instead
/// of the result list.
fn poll(s: &mut Tomboy, now: f64, tx: &mut Tx) -> bool {
    let woke = s.woke.swap(false, Ordering::AcqRel);
    #[cfg(unix)]
    if single::close_requested() { quit(s, tx); return true; }
    if std::mem::take(&mut s.dev_open) && !s.extensions.is_empty() {
        run_item(s, &Item::Extension { ext: 0, command: 0 }, tx);
        s.direct = true;
        return true;
    }
    let woke = extui::poll(s, now, tx) || woke;
    let Some(prompt) = s.dictated.lock().unwrap_or_else(|e| e.into_inner()).take() else { return woke };
    if prompt != s.query.text { return woke; } // edited since; treat it as typed
    if chat_running(s) { return woke; }
    run_item(s, &Item::Agent(prompt), tx);
    true
}

fn results(s: &Tomboy) -> Vec<Item> {
    let started = std::time::Instant::now();
    let rows = search_results(s);
    trace::results(started, rows.len());
    rows
}

fn search_results(s: &Tomboy) -> Vec<Item> {
    if let Some(rows) = extui::session_results(s).or_else(|| extui::store_results(s)) { return rows; }
    let q = s.query.text.trim();
    if let Some(clips) = &s.clips {
        return clipboard::search(clips, q).into_iter().map(Item::Clip).collect();
    }
    if let (Some(past), None) = (&s.history, &s.chat) {
        return agent_history::search(past, q).into_iter().map(Item::Past).collect();
    }
    if s.emoji.is_some() { return Vec::new(); } // the picker draws its own grid
    if let Some(day) = &s.todo { return (0..day.tasks().len()).map(Item::Task).collect(); }
    if let Some(id) = &s.submenu {
        // Inside a submenu: its rows, or a search of everything under it.
        let rows = if q.is_empty() { omarchy::children_of(&s.menu, &s.conds, id) }
            else { omarchy::search(&s.menu, &s.conds, q, Some(id), MAX_APPS, menu_boost(s, q)) };
        return rows.into_iter().map(Item::Menu).collect();
    }
    if let Some(link) = s.quicklink {
        return if q.is_empty() { Vec::new() } else { vec![Item::Quicklink { link, arg: Some(q.to_string()) }] };
    }
    let mut items = search(s, q);
    if let Some((_, target)) = alias_target(s) {
        items.retain(|it| !same_item(it, &target));
        items.insert(0, target);
    }
    items
}

/// The launcher's results for `q` (no alias applied).
fn search(s: &Tomboy, q: &str) -> Vec<Item> {
    let now = frecency::now();
    let mut links = quicklinks::matches(&s.links, q);
    links.sort_by_key(|h| std::cmp::Reverse((h.exact, s.frecency.boost(&frecency::link_key(&s.links[h.link].keyword), q, now))));
    let link_item = |h: &quicklinks::Hit| Item::Quicklink { link: h.link, arg: h.arg.map(str::to_string) };
    // The keyword typed in full (`jira`, `jira ABC-1`) goes first; a prefix sits with the commands.
    let mut items: Vec<Item> = links.iter().filter(|h| h.exact).map(link_item).collect();
    if let Some(task) = todo::quick_add(q) {
        items.push(Item::TodoAdd(task.to_string()));
    }
    // Today's list: asked for, or first on an empty launcher when there's something on it.
    if todo::matches_command(q) || (q.is_empty() && s.todo_counts.1 > 0) {
        items.push(Item::Todo);
    }
    if let Some(v) = calc::eval(q) {
        items.push(Item::Calc { expr: q.to_string(), value: calc::format(v) });
    }
    items.extend(answers::collect(&s.query.text, &s.seed, &s.wake).into_iter().map(Item::Answer));
    if let Some(u) = url::detect(q) {
        items.push(Item::Url(u));
    }
    if clipboard::matches_command(q) {
        items.push(Item::Clipboard);
    }
    if emoji::matches_command(q) {
        items.push(Item::Emoji);
    }
    if agent_history::matches_command(q) {
        items.push(Item::AgentHistory);
    }
    // Typed: extension commands rank with the other commands. Empty: they follow your apps.
    let ext_items = extui::search(s, q);
    let ext_items = if q.is_empty() { Some(ext_items) } else { items.extend(ext_items); None };
    if extui::store_matches(q) {
        items.push(Item::Store);
    }
    items.extend(links.iter().filter(|h| !h.exact).map(link_item));
    let app_limit = if q.is_empty() { MAX_APPS } else { MAX_APP_HITS };
    let app_boost = |i: usize| s.frecency.boost(&frecency::app_key(&s.apps[i].id), q, now);
    items.extend(apps::search(&s.apps, q, app_limit, app_boost).into_iter().map(Item::App));
    items.extend(ext_items.into_iter().flatten());
    items.extend(omarchy::search(&s.menu, &s.conds, q, None, MAX_MENU_HITS, menu_boost(s, q)).into_iter().map(Item::Menu));
    if !q.is_empty() && !matches!(items.first(), Some(Item::Calc { .. } | Item::Answer(_) | Item::Url(_))) {
        items.push(Item::Agent(q.to_string()));
        items.push(Item::WebSearch(q.to_string()));
    }
    items
}

/// When the query is exactly an `[aliases]` key: the alias and the best launcher match for
/// what it stands for (an app, a menu command, a quicklink, `gh tomboy`).
fn alias_target(s: &Tomboy) -> Option<(String, Item)> {
    let q = s.query.text.trim().to_lowercase();
    if q.is_empty() { return None; }
    let (alias, target) = s.config.aliases.iter().find(|(a, _)| a.trim().to_lowercase() == q)?;
    let target = target.trim();
    if target.is_empty() { return None; }
    let hit = search(s, target).into_iter().find(|it| !matches!(it, Item::Agent(_) | Item::WebSearch(_)))?;
    Some((alias.trim().to_string(), hit))
}

/// The same row (for moving an alias's target to the top).
fn same_item(a: &Item, b: &Item) -> bool {
    match (a, b) {
        (Item::App(x), Item::App(y)) | (Item::Menu(x), Item::Menu(y)) => x == y,
        (Item::Quicklink { link: x, arg: p }, Item::Quicklink { link: y, arg: q }) => x == y && p == q,
        (Item::Clipboard, Item::Clipboard) | (Item::Emoji, Item::Emoji) | (Item::Store, Item::Store)
        | (Item::AgentHistory, Item::AgentHistory) => true,
        (Item::Extension { ext: a, command: x }, Item::Extension { ext: b, command: y }) => a == b && x == y,
        _ => false,
    }
}

/// Frecency points for Omarchy menu row `i` under query `q`.
fn menu_boost<'a>(s: &'a Tomboy, q: &'a str) -> impl Fn(usize) -> u32 + 'a {
    let now = frecency::now();
    move |i| s.frecency.boost(&frecency::menu_key(&s.menu[i].id), q, now)
}

/// The frecency key of an item that launches something (an app, a menu command); `None`
/// for answers, fallbacks and rows that open a submenu inside tomboy.
fn frecency_key(s: &Tomboy, item: &Item) -> Option<String> {
    match item {
        Item::App(i) => Some(frecency::app_key(&s.apps[*i].id)),
        Item::Menu(i) if !omarchy::has_children(&s.menu, &s.menu[*i].id) => Some(frecency::menu_key(&s.menu[*i].id)),
        Item::Quicklink { link, arg } if arg.is_some() || s.links[*link].arg.is_none() => Some(frecency::link_key(&s.links[*link].keyword)),
        Item::Extension { ext, command } => {
            let m = &s.extensions[*ext].0;
            Some(extui::frecency_key(&m.id, &m.commands[*command].id))
        }
        _ => None,
    }
}

fn run_item(s: &mut Tomboy, item: &Item, tx: &mut Tx) {
    let key = frecency_key(s, item);
    let outcome = match item {
        Item::Calc { value, .. } => copy_text(value, tx).map(|()| {
            s.status = Some(format!("Copied {value}"));
            After::Stay
        }),
        Item::Answer(a) => match &a.action {
            answers::Action::Copy(v) => copy_text(v, tx).map(|()| {
                s.status = Some(format!("Copied {v}"));
                After::Stay
            }),
            answers::Action::Kill { pid, name } => answers::kill::terminate(*pid).map(|()| {
                s.status = Some(format!("Sent SIGTERM to {name} ({pid})"));
                After::Stay
            }),
            answers::Action::None => Ok(After::Stay),
        },
        Item::Url(u) => url::open(u).map(|()| After::Quit),
        Item::App(i) => apps::launch(&s.apps[*i]).map(|()| After::Quit),
        // A submenu opens inside tomboy; one the shell fills at runtime (Fonts) has no rows
        // in the config, so it opens in Omarchy's own menu.
        Item::Menu(i) if omarchy::has_children(&s.menu, &s.menu[*i].id) => {
            let id = s.menu[*i].id.clone();
            enter_submenu(s, Some(id));
            Ok(After::Stay)
        }
        Item::Menu(i) => match &s.platform {
            Platform::Omarchy { root } => omarchy::run(&s.menu[*i], root).map(|()| After::Quit),
            #[cfg(target_os = "macos")]
            Platform::MacOS if s.menu[*i].id == mac::menu::QUIT_ID => { tx.quit(); Ok(After::Stay) }
            #[cfg(target_os = "macos")]
            Platform::MacOS => mac::menu::run(&s.menu[*i]).map(|()| After::Quit),
            _ => Ok(After::Stay),
        },
        Item::WebSearch(q) => url::open(&format!("https://duckduckgo.com/?q={}", encode(q))).map(|()| After::Quit),
        Item::Agent(prompt) => ask_agent(s, prompt),
        Item::Clipboard => { enter_clipboard(s); Ok(After::Stay) }
        Item::Emoji => { enter_emoji(s); Ok(After::Stay) }
        Item::Clip(i) => copy_clip(s, *i, tx),
        Item::Todo => { enter_todo(s); Ok(After::Stay) }
        Item::TodoAdd(text) => add_todo(s, text).map(|()| After::Stay),
        Item::Task(i) => toggle_task(s, *i).map(|()| After::Stay),
        Item::Quicklink { link, arg } => match (&s.links[*link].arg, arg) {
            (Some(_), None) => { enter_quicklink(s, Some(*link)); Ok(After::Stay) }
            (_, arg) => url::open(&s.links[*link].expand(arg.as_deref().unwrap_or(""))).map(|()| After::Quit),
        },
        Item::Extension { ext, command } => extui::open_command(s, *ext, *command, tx),
        Item::Store => { extui::open_store(s); Ok(After::Stay) }
        Item::ExtRow(i) => { extui::run_row(s, *i, tx); Ok(After::Stay) }
        Item::StoreRow(i) => { extui::run_store_row(s, *i, tx); Ok(After::Stay) }
        Item::AgentHistory => { enter_history(s); Ok(After::Stay) }
        Item::Past(i) => { open_past(s, *i); Ok(After::Stay) }
    };
    if let (Some(key), Ok(_)) = (&key, &outcome) {
        // In argument mode the field holds the argument; learn the keyword instead.
        let query = match s.quicklink { Some(l) => s.links[l].keyword.clone(), None => s.query.text.clone() };
        s.frecency.record(key, &query, frecency::now());
    }
    match outcome {
        Ok(After::Quit) => quit(s, tx),
        Ok(After::Stay) => {}
        Err(e) => s.status = Some(format!("Failed: {e}")),
    }
}

/// Run the agent headless inside tomboy (a follow-up when a conversation is open); agents
/// with no headless mode open in a terminal instead.
fn ask_agent(s: &mut Tomboy, prompt: &str) -> std::io::Result<After> {
    let prompt = prompt.trim();
    if prompt.is_empty() { return Ok(After::Stay); }
    let run = match &s.chat {
        Some(run) => run.clone(),
        None => {
            let forced = s.config.agent.name.as_deref();
            let name = agent::name(&s.platform, forced);
            if !agent::supports_headless(&name) {
                return agent::ask(prompt, &s.platform, forced).map(|()| After::Quit);
            }
            let model = s.config.agent_model(&name);
            agent::Run::new(name, s.config.agent_dir(), model)
        }
    };
    let waker = s.waker.clone();
    run.start(prompt, move || if let Some(w) = &waker { w() })?;
    s.chat = Some(run);
    s.query.clear();
    s.last_text.clear();
    s.picked = 0;
    s.status = None;
    Ok(After::Stay)
}

/// Browse the past agent conversations, read fresh from disk.
fn enter_history(s: &mut Tomboy) {
    s.history = Some(agent_history::load());
    s.chat = None;
    s.query.clear();
    s.last_text.clear();
    s.picked = 0;
    s.status = None;
}

fn leave_history(s: &mut Tomboy) {
    s.history = None;
    enter_submenu(s, s.submenu.clone());
}

/// Make past conversation `i` the live one, ready for a follow-up.
fn open_past(s: &mut Tomboy, i: usize) {
    let Some(saved) = s.history.as_ref().and_then(|h| h.get(i)).cloned() else { return };
    s.chat = Some(agent::Run::resume(saved));
    s.query.clear();
    s.last_text.clear();
    s.picked = 0;
    s.status = None;
}

/// From a conversation opened in the history back to the list (stopping a running turn),
/// with the list read again so that conversation's new turns show and it moves to the top.
fn back_to_history(s: &mut Tomboy) {
    if let Some(run) = s.chat.take() { run.stop(); }
    enter_history(s);
}

/// Delete the picked past conversation.
fn remove_past(s: &mut Tomboy) {
    let Some(Item::Past(i)) = results(s).get(s.picked).cloned() else { return };
    let Some(past) = s.history.as_mut() else { return };
    let saved = past.remove(i);
    match agent_history::remove(&saved.id) {
        Ok(()) => s.status = Some(format!("Deleted “{}”", saved.title())),
        Err(e) => { s.status = Some(format!("Failed: {e}")); past.insert(i, saved); }
    }
    let n = results(s).len();
    s.picked = s.picked.min(n.saturating_sub(1));
}

/// Esc, Backspace and Delete in the history list (arrows and Enter work as in any list).
fn history_key_event(s: &mut Tomboy, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let empty = s.query.text.is_empty();
    match ev.key {
        Key::Escape if !empty => { s.query.clear(); s.last_text.clear(); s.picked = 0; }
        Key::Escape if s.direct => quit(s, tx),
        Key::Escape | Key::Backspace if empty && !s.direct => leave_history(s),
        Key::Delete if empty => remove_past(s),
        _ => return false,
    }
    true
}

/// Browse the clipboard history, read fresh from disk.
fn enter_clipboard(s: &mut Tomboy) {
    s.clips = Some(clipboard::history_path(&s.platform).map(|p| clipboard::load(&p)).unwrap_or_default());
    s.query.clear();
    s.last_text.clear();
    s.picked = 0;
    s.status = None;
    sync_clip_image(s);
}

fn leave_clipboard(s: &mut Tomboy) {
    s.clips = None;
    s.clip_image = None;
    enter_submenu(s, s.submenu.clone());
}

/// Open the emoji & symbol picker.
fn enter_emoji(s: &mut Tomboy) {
    s.emoji = Some(EmojiView { rows: Vec::new(), pos: None, version: 0 });
    s.query.clear();
    s.last_text.clear();
    s.status = None;
    rebuild_emoji(s);
}

fn leave_emoji(s: &mut Tomboy) {
    s.emoji = None;
    enter_submenu(s, s.submenu.clone());
}

/// Frecency key of an emoji or symbol.
fn emoji_key(ch: &str) -> String { format!("emoji:{ch}") }

/// Lay out the grid for the current query: Recently Used and every group when empty, else the
/// matches (frecency first among equals).
fn rebuild_emoji(s: &mut Tomboy) {
    const RECENT: usize = 16;
    let q = s.query.text.trim().to_string();
    let now = frecency::now();
    let all = emoji::all();
    let boost = |i: usize| s.frecency.boost(&emoji_key(all[i].ch), &q, now);
    let sections = if q.is_empty() {
        let mut used: Vec<(u32, usize)> = (0..all.len()).map(|i| (boost(i), i)).filter(|(b, _)| *b > 0).collect();
        used.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        emoji::browse(used.into_iter().take(RECENT).map(|(_, i)| i).collect())
    } else {
        vec![("Results".to_string(), emoji::search(&q, boost))]
    };
    let rows = emoji::grid(sections);
    if let Some(v) = &mut s.emoji {
        v.pos = emoji::first(&rows);
        v.rows = rows;
        v.version += 1;
    }
}

/// The picked emoji or symbol.
fn picked_glyph(s: &Tomboy) -> Option<&'static emoji::Glyph> {
    let v = s.emoji.as_ref()?;
    emoji::at(&v.rows, v.pos?).map(|i| &emoji::all()[i])
}

/// Copy the picked emoji and close (stay open when the copy lives in tomboy itself).
fn copy_emoji(s: &mut Tomboy, tx: &mut Tx) {
    let Some(g) = picked_glyph(s) else { return };
    let query = s.query.text.clone();
    s.frecency.record(&emoji_key(g.ch), &query, frecency::now());
    match sys::copy_to_clipboard(g.ch) {
        Ok(true) => { s.paste = true; quit(s, tx) }
        Ok(false) => { tx.copy(g.ch); s.status = Some(format!("Copied {}", g.ch)); }
        Err(e) => s.status = Some(format!("Failed: {e}")),
    }
}

/// Esc, Space and Delete in the todo list (arrows and Enter work as in any list).
fn todo_key_event(s: &mut Tomboy, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let empty = s.query.text.is_empty();
    let result = match &ev.key {
        Key::Escape if s.editing_task.is_some() => { cancel_task_edit(s); s.status = None; Ok(()) }
        Key::Escape if !empty => { s.query.clear(); s.last_text.clear(); Ok(()) }
        Key::Escape if s.direct => { quit(s, tx); Ok(()) }
        Key::Escape | Key::Backspace if empty => { leave_todo(s); Ok(()) }
        Key::Text(t) if empty && t == " " && !ev.mods.cmd => toggle_task(s, s.picked),
        Key::Delete if empty => remove_task(s, s.picked),
        _ => return false,
    };
    if let Err(e) = result { s.status = Some(format!("Failed: {e}")); }
    true
}

/// Arrow keys, Tab and Esc in the picker.
fn emoji_key_event(s: &mut Tomboy, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let Some(v) = &mut s.emoji else { return false };
    let moved = |v: &mut EmojiView, f: &dyn Fn(&[emoji::Row], emoji::Pos) -> emoji::Pos| {
        if let Some(p) = v.pos { v.pos = Some(f(&v.rows, p)); }
        v.pos.map(|p| p.0)
    };
    let row = match ev.key {
        Key::Left => moved(v, &|r, p| emoji::step(r, p, -1, 0)),
        Key::Right => moved(v, &|r, p| emoji::step(r, p, 1, 0)),
        Key::Up => moved(v, &|r, p| emoji::step(r, p, 0, -1)),
        Key::Down => moved(v, &|r, p| emoji::step(r, p, 0, 1)),
        Key::Tab => { let dir = if ev.mods.shift { -1 } else { 1 }; moved(v, &|r, p| emoji::section(r, p, dir)) }
        Key::Escape if !s.query.text.is_empty() => {
            s.query.clear(); s.last_text.clear(); rebuild_emoji(s); return true;
        }
        Key::Escape if s.direct => { quit(s, tx); return true; }
        Key::Escape | Key::Backspace if s.query.text.is_empty() => { leave_emoji(s); return true; }
        _ => return false,
    };
    // Keep the header above the first row of a section in view too.
    if let Some(r) = row { tx.scroll_into_view(format!("er{}", r.saturating_sub(usize::from(r == 1)))); }
    true
}

/// Put clip `i` back on the clipboard and close.
fn copy_clip(s: &mut Tomboy, i: usize, tx: &mut Tx) -> std::io::Result<After> {
    let Some(clip) = s.clips.as_ref().and_then(|c| c.get(i)).cloned() else { return Ok(After::Stay) };
    if clip.copy()? { s.paste = true; return Ok(After::Quit); }
    match &clip {
        // No clipboard helper installed: copy in-process and stay open so the copy lives.
        Clip::Text(t) => { tx.copy(t); s.status = Some("Copied".into()); Ok(After::Stay) }
        Clip::Image { .. } => Err(std::io::Error::other(if cfg!(target_os = "linux") {
            "no clipboard helper for images (install wl-clipboard)"
        } else {
            "couldn't put the image on the clipboard"
        })),
    }
}

/// Load the picture for the picked clip, if it has one, and let go of the previous one.
fn sync_clip_image(s: &mut Tomboy) {
    let path = match results(s).get(s.picked) {
        Some(Item::Clip(i)) => s.clips.as_ref().and_then(|c| c[*i].image_path()),
        _ => None,
    };
    if s.clip_image.as_ref().map(|(p, _)| p) == path.as_ref() { return; }
    s.clip_image = match (path, &s.cache) {
        (Some(p), Some(cache)) => { let img = cache.file(&p, 0.0, 1.0); Some((p, img)) }
        _ => None,
    };
}

/// Show the rows of submenu `id` (`None`: back to the root search), with an empty field.
fn enter_submenu(s: &mut Tomboy, id: Option<String>) {
    s.submenu = id;
    s.query.clear();
    s.last_text.clear();
    s.picked = 0;
    s.status = None;
}

/// Open today's list (carrying over yesterday's open tasks on a day's first open).
fn enter_todo(s: &mut Tomboy) {
    let Some(dir) = s.config.todo_dir() else { s.status = Some("No home directory for the todo list".into()); return };
    match todo::today(&dir, todo::today_date()) {
        Ok(day) => {
            s.todo_counts = day.counts();
            s.todo = Some(day);
            s.query.clear();
            s.last_text.clear();
            s.picked = 0;
            s.status = None;
        }
        Err(e) => s.status = Some(format!("Todo list: {e}")),
    }
}

fn leave_todo(s: &mut Tomboy) {
    s.todo = None;
    s.task_drag = None;
    s.editing_task = None;
    enter_submenu(s, s.submenu.clone());
}

/// Add `text` to today's list: the open one, or (a quick add from the launcher) today's file
/// without opening the list.
fn add_todo(s: &mut Tomboy, text: &str) -> std::io::Result<()> {
    let counts = match s.todo.as_mut() {
        Some(day) => day.add(text).map(|_| day.counts()),
        None => {
            let dir = s.config.todo_dir().ok_or_else(|| std::io::Error::other("no home directory"))?;
            todo::today(&dir, todo::today_date()).and_then(|mut day| day.add(text).map(|_| day.counts()))
        }
    }?;
    s.todo_counts = counts;
    s.query.clear();
    s.last_text.clear();
    s.picked = 0;
    s.status = Some(format!("Added “{}” · {}/{} done", text.trim(), counts.0, counts.1));
    Ok(())
}

/// Check or uncheck task `i` of the open list.
fn toggle_task(s: &mut Tomboy, i: usize) -> std::io::Result<()> {
    let Some(day) = s.todo.as_mut() else { return Ok(()) };
    let Some(task) = day.tasks().get(i).cloned() else { return Ok(()) };
    if !day.toggle(&task)? { s.status = Some("The list changed on disk; reloaded".into()); }
    s.todo_counts = day.counts();
    Ok(())
}

/// What the action panel (ctrl+k) offers for a task.
#[derive(Clone, Debug)]
enum TodoAction { Edit(usize), Toggle(usize, bool), Delete(usize) }

impl TodoAction {
    fn title(&self) -> &'static str {
        match self {
            TodoAction::Edit(_) => "Edit Todo",
            TodoAction::Toggle(_, true) => "Mark Open",
            TodoAction::Toggle(_, false) => "Mark Done",
            TodoAction::Delete(_) => "Delete Todo",
        }
    }
}

/// The picked task's actions.
fn todo_actions(s: &Tomboy) -> Vec<TodoAction> {
    let i = s.picked;
    match s.todo.as_ref().and_then(|d| d.tasks().get(i).cloned()) {
        Some(t) => vec![TodoAction::Edit(i), TodoAction::Toggle(i, t.done), TodoAction::Delete(i)],
        None => Vec::new(),
    }
}

fn run_todo_action(s: &mut Tomboy, a: TodoAction) {
    let result = match a {
        TodoAction::Edit(i) => { start_task_edit(s, i); Ok(()) }
        TodoAction::Toggle(i, _) => toggle_task(s, i),
        TodoAction::Delete(i) => remove_task(s, i),
    };
    if let Err(e) = result { s.status = Some(format!("Failed: {e}")); }
}

/// Put task `i`'s text in the field to edit; Enter saves it to the same task.
fn start_task_edit(s: &mut Tomboy, i: usize) {
    let Some(task) = s.todo.as_ref().and_then(|d| d.tasks().get(i).cloned()) else { return };
    // A fresh field, focused with the cursor at the end, to type into at once.
    s.query = TextEdit::new(task.text.clone());
    s.query.focused = true;
    s.last_text = task.text.clone();
    s.picked = i;
    s.status = None;
    s.editing_task = Some(task);
}

fn cancel_task_edit(s: &mut Tomboy) {
    s.editing_task = None;
    s.query.clear();
    s.last_text.clear();
}

/// Save the field's text to the task being edited.
fn save_task_edit(s: &mut Tomboy) -> std::io::Result<()> {
    let (Some(day), Some(task)) = (s.todo.as_mut(), s.editing_task.clone()) else { return Ok(()) };
    let text = s.query.text.trim().to_string();
    if text.is_empty() { s.status = Some("A todo needs some text · esc cancels".into()); return Ok(()); }
    let saved = day.edit(&task, &text)?;
    let still_there = day.tasks().iter().any(|t| t.text == text);
    s.status = Some(if saved { format!("Saved “{text}”") }
        else if still_there { "No changes".into() }
        else { "The list changed on disk; reloaded".into() });
    cancel_task_edit(s);
    Ok(())
}

/// Move task `from` of the open list to `to`'s place.
fn move_task(s: &mut Tomboy, from: usize, to: usize) -> std::io::Result<()> {
    let Some(day) = s.todo.as_mut() else { return Ok(()) };
    let tasks = day.tasks();
    let (Some(task), Some(target)) = (tasks.get(from), tasks.get(to)) else { return Ok(()) };
    if !day.reorder(task, target)? { s.status = Some("The list changed on disk; reloaded".into()); }
    s.picked = to.min(day.tasks().len().saturating_sub(1));
    Ok(())
}

/// Delete task `i` of the open list.
fn remove_task(s: &mut Tomboy, i: usize) -> std::io::Result<()> {
    let Some(day) = s.todo.as_mut() else { return Ok(()) };
    let Some(task) = day.tasks().get(i).cloned() else { return Ok(()) };
    if day.remove(&task)? {
        s.status = Some(format!("Removed “{}”", task.text));
    } else {
        s.status = Some("The list changed on disk; reloaded".into());
    }
    s.todo_counts = day.counts();
    let n = day.tasks().len();
    s.picked = s.picked.min(n.saturating_sub(1));
    Ok(())
}

/// Type the argument for quicklink `link`, or back to the launcher with `None`.
fn enter_quicklink(s: &mut Tomboy, link: Option<usize>) {
    enter_submenu(s, None);
    s.quicklink = link;
}

/// One level up from the open submenu.
fn leave_submenu(s: &mut Tomboy) {
    let up = s.submenu.as_deref().and_then(omarchy::parent_of).map(str::to_string);
    enter_submenu(s, up);
}

/// "Install" or "Appearance › Menu Bar" for the open submenu.
fn submenu_title(s: &Tomboy) -> Option<String> {
    let id = s.submenu.as_deref()?;
    let entry = s.menu.iter().find(|m| m.id == id)?;
    Some(entry.parents.iter().cloned().chain([entry.label.clone()]).collect::<Vec<_>>().join(" › "))
}

fn chat_running(s: &Tomboy) -> bool {
    s.chat.as_ref().is_some_and(|r| r.lock().is_running())
}

/// Quit, stopping a running agent first (it runs in its own process group). The daemon
/// hides instead, ready for the next hotkey.
fn quit(s: &mut Tomboy, tx: &mut Tx) {
    trace::mark("quit");
    if let Some(run) = &s.chat { run.stop(); }
    // Linux has no resident window: the paste runs from a helper that outlives tomboy.
    #[cfg(target_os = "linux")]
    if std::mem::take(&mut s.paste) && s.config.paste() {
        sys::paste_soon(std::time::Duration::from_millis(200));
    }
    if s.resident { hide(s) } else { tx.quit() }
}

/// "Paste" when picking a clip (or emoji) also pastes it into the front app: the macOS
/// daemon, or Linux with a key-pressing helper (`wtype`, `hyprctl`, `xdotool`).
fn clip_action(s: &Tomboy) -> &'static str {
    #[cfg(target_os = "macos")]
    let can = s.resident;
    #[cfg(target_os = "linux")]
    let can = sys::can_paste();
    if can && s.config.paste() { "Paste" } else { "Copy to Clipboard" }
}

// ---- The resident launcher (`tomboy daemon`, macOS) ----

/// Hide the daemon's window, paste what was just picked, and get ready for the next hotkey:
/// a fresh launcher, with the config and apps read again (as a new launch would).
fn hide(s: &mut Tomboy) {
    if s.shown.take().is_none() { return; }
    s.had_focus = false;
    if let Some(me) = &s.me { me.hide(); }
    #[cfg(target_os = "macos")]
    if std::mem::take(&mut s.paste) && s.config.paste() {
        mac::paste::paste_soon(std::time::Duration::from_millis(120));
    }
    s.paste = false;
    if let Some(run) = s.chat.take() { run.stop(); }
    reload(s);
    fresh_view(s);
}

/// Back to an empty launcher: no open list, picker, submenu or conversation.
fn fresh_view(s: &mut Tomboy) {
    s.chat = None;
    s.history = None;
    s.clips = None;
    s.clip_image = None;
    s.emoji = None;
    s.todo = None;
    s.quicklink = None;
    s.session = None;
    s.store = None;
    s.panel = None;
    s.direct = false;
    enter_submenu(s, None);
    s.query.focused = true;
    *s.dictated.lock().unwrap_or_else(|e| e.into_inner()) = None;
    s.seed = answers::devtools::session_seed();
}

/// Read what a launch reads: the config (and its quicklinks), today's todo count and the
/// installed apps, decoding icons only for apps that are new.
fn reload(s: &mut Tomboy) {
    let (config, error) = config::load();
    s.links = quicklinks::load(&config);
    s.todo_counts = config.todo_dir().map_or((0, 0), |d| todo::peek(&d, todo::today_date()));
    s.config = config;
    s.extensions = load_extensions();
    let apps = apps::load_apps();
    if apps.iter().map(|a| &a.path).ne(s.apps.iter().map(|a| &a.path)) {
        s.icons = apps.iter().map(|a| match (&a.icon, &s.cache) {
            (Some(_), Some(cache)) => Some(cache.app_icon(&a.path, ICON)),
            _ => None,
        }).collect();
        s.apps = apps;
    }
    s.status = error.or_else(|| s.config.warning());
}

/// A daemon hotkey: show the window in that mode, or hide it when that hotkey showed it.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn summon(s: &mut Tomboy, hotkey: Hotkey) {
    let Some(me) = s.me.clone() else { return };
    if s.shown == Some(hotkey) { hide(s); return; }
    if s.shown.is_some() { fresh_view(s); }
    match hotkey {
        Hotkey::Launcher => {}
        Hotkey::Clipboard => { enter_clipboard(s); s.direct = true; }
        Hotkey::Emoji => { enter_emoji(s); s.direct = true; }
        Hotkey::Todo => { enter_todo(s); s.direct = true; }
    }
    s.shown = Some(hotkey);
    me.place(rune::DisplayTarget::UnderPointer, rune::OverlayPlacement::Edge {
        edge: rune::ScreenEdge::Top, margin: 160.0, align: rune::Align::Center, inset: 0.0,
    });
    me.show();
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn hotkey_handler(hotkey: Hotkey) -> fn(&mut Tomboy, &mut Tx) {
    match hotkey {
        Hotkey::Launcher => |s, _| summon(s, Hotkey::Launcher),
        Hotkey::Clipboard => |s, _| summon(s, Hotkey::Clipboard),
        Hotkey::Emoji => |s, _| summon(s, Hotkey::Emoji),
        Hotkey::Todo => |s, _| summon(s, Hotkey::Todo),
    }
}

/// Copy through the OS helper (`wl-copy`, `pbcopy`, …) so the copy survives tomboy quitting;
/// without one, copy in-process (tomboy stays open after a copy either way).
fn copy_text(value: &str, tx: &mut Tx) -> std::io::Result<()> {
    if !sys::copy_to_clipboard(value)? { tx.copy(value); }
    Ok(())
}

fn encode(q: &str) -> String {
    q.bytes().map(|b| match b {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
        b' ' => "+".to_string(),
        _ => format!("%{b:02X}"),
    }).collect()
}

fn submit(s: &mut Tomboy, tx: &mut Tx) {
    if s.panel.is_some() { extui::run_panel(s, tx); return; }
    if extui::store_submit(s, tx) { return; }
    if s.emoji.is_some() { copy_emoji(s, tx); return; }
    if s.todo.is_some() && s.editing_task.is_some() {
        if let Err(e) = save_task_edit(s) { s.status = Some(format!("Failed: {e}")); }
        return;
    }
    if s.todo.is_some() && !s.query.text.trim().is_empty() {
        // Typing in the list adds a todo; with the field empty, Enter toggles the picked one.
        let text = s.query.text.clone();
        if let Err(e) = add_todo(s, &text) { s.status = Some(format!("Failed: {e}")); }
        return;
    }
    if let Some(run) = s.chat.clone() {
        if run.lock().is_running() { return; }
        if s.query.text.trim().is_empty() {
            // Enter on an empty field copies the last answer.
            let answer = run.lock().last_answer();
            match copy_text(&answer, tx) {
                Ok(()) => s.status = Some("Copied answer".into()),
                Err(e) => s.status = Some(format!("Failed: {e}")),
            }
        } else {
            let prompt = s.query.text.clone();
            run_item(s, &Item::Agent(prompt), tx);
        }
        return;
    }
    let items = results(s);
    if let Some(item) = items.get(s.picked).cloned() { run_item(s, &item, tx); }
}

/// Super+W (or Cmd/Ctrl+W) closes tomboy from any screen.
fn is_close_chord(ev: &KeyEvent) -> bool {
    ev.is_command('w') || (ev.mods.logo && matches!(&ev.key, Key::Text(t) if t.eq_ignore_ascii_case("w")))
}

/// Keys no focused control took (the search field lost focus to a click on the transcript,
/// say): Esc behaves as it does in the field, and the close chord closes.
fn on_window_key(s: &mut Tomboy, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if is_close_chord(ev) { quit(s, tx); return true; }
    if matches!(ev.key, Key::Escape) || (s.chat.is_some() && matches!(ev.key, Key::PageUp | Key::PageDown)) {
        return on_input_key(s, ev, tx);
    }
    false
}

fn on_input_key(s: &mut Tomboy, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if is_close_chord(ev) { quit(s, tx); return true; }
    if s.panel.is_some() { return extui::panel_key(s, ev, tx); }
    if ev.is_command('k') && extui::has_panel_actions(s) { return extui::open_panel(s); }
    if ev.is_command('r') && DEV_DIR.get().is_some() && s.session.is_some() {
        // `tomboy ext dev`: reload the folder and run the command again.
        s.extensions = load_extensions();
        s.session = None;
        run_item(s, &Item::Extension { ext: 0, command: 0 }, tx);
        return true;
    }
    if s.session.is_some() && extui::session_key(s, ev, tx) {
        if s.session.is_none() && s.direct { quit(s, tx); }
        return true;
    }
    if s.store.is_some() && extui::store_key(s, ev, tx) {
        if s.store.is_none() && s.direct { quit(s, tx); }
        return true;
    }
    if let Some(run) = s.chat.clone() {
        match ev.key {
            // Esc stops a running agent; once it is idle, Esc closes tomboy.
            Key::Escape if run.lock().is_running() => run.stop(),
            // Opened from the history: back to the list.
            Key::Escape if s.history.is_some() => back_to_history(s),
            Key::Escape => quit(s, tx),
            // Page through a long answer without leaving the field.
            Key::PageUp => tx.scroll_page("transcript-scroll", -1),
            Key::PageDown => tx.scroll_page("transcript-scroll", 1),
            _ => return false,
        }
        return true;
    }
    if s.emoji.is_some() { return emoji_key_event(s, ev, tx); }
    if s.todo.is_some() && todo_key_event(s, ev, tx) { return true; }
    if s.history.is_some() && history_key_event(s, ev, tx) { return true; }
    let n = results(s).len();
    match ev.key {
        Key::Down if n > 0 => s.picked = (s.picked + 1) % n,
        Key::Up if n > 0 => s.picked = (s.picked + n - 1) % n,
        // In the clipboard history Esc clears the search, then goes back (or closes).
        Key::Escape if s.clips.is_some() && !s.query.text.is_empty() => {
            s.query.clear(); s.last_text.clear(); s.picked = 0; sync_clip_image(s); return true;
        }
        Key::Escape if s.clips.is_some() && s.direct => { quit(s, tx); return true; }
        Key::Escape | Key::Backspace if s.clips.is_some() && s.query.text.is_empty() && !s.direct => {
            leave_clipboard(s);
            return true;
        }
        // Typing a quicklink's argument: Esc clears it, then (like Backspace on an empty
        // field) goes back to the launcher.
        Key::Escape if s.quicklink.is_some() && !s.query.text.is_empty() => {
            s.query.clear(); s.last_text.clear(); s.picked = 0; return true;
        }
        Key::Escape | Key::Backspace if s.quicklink.is_some() && s.query.text.is_empty() => {
            enter_quicklink(s, None);
            return true;
        }
        // In a submenu, Esc (or Backspace on an empty field) goes back up a level.
        Key::Escape | Key::Backspace if s.submenu.is_some() && s.query.text.is_empty() => {
            leave_submenu(s);
            return true;
        }
        Key::Escape if s.submenu.is_some() => { s.query.clear(); s.last_text.clear(); s.picked = 0; return true; }
        Key::Escape => { quit(s, tx); return true; }
        _ => return false,
    }
    sync_clip_image(s);
    tx.scroll_into_view(item_key(s, s.picked));
    true
}

/// The key of result row `i`. A task's row is keyed by the task, not its place, so a dropped
/// row slides from where it was let go into its new place.
fn item_key(s: &Tomboy, i: usize) -> String {
    match &s.todo {
        Some(day) => task_key(&day.tasks(), i),
        None => format!("item-{i}"),
    }
}

/// `task-<text>#<n>`: the nth task with that text.
fn task_key(tasks: &[todo::Task], i: usize) -> String {
    let Some(t) = tasks.get(i) else { return format!("item-{i}") };
    let n = tasks[..i].iter().filter(|o| o.text == t.text).count();
    format!("task-{}#{n}", t.text)
}

/// A task held by its grip: from its place `from`, `dy` points from where it was pressed,
/// over the place `to` it lands on if dropped now.
#[derive(Clone, Copy, Debug)]
struct TaskDrag { from: usize, to: usize, dy: f32 }

/// Points the pointer travels before a press on the grip becomes a drag.
const DRAG_SLOP: f32 = 4.0;

impl TaskDrag {
    /// How far row `i` sits from its place: the dragged row with the pointer, the rows between
    /// its place and its target a row toward the gap it left.
    fn shift(&self, i: usize, pitch: f32) -> f32 {
        let TaskDrag { from, to, dy } = *self;
        if i == from { dy }
        else if from < i && i <= to { -pitch }
        else if to <= i && i < from { pitch }
        else { 0.0 }
    }
}

fn view(s: &Tomboy, cx: &mut Cx<Tomboy>) -> El {
    let started = std::time::Instant::now();
    let el = build_view(s, cx);
    trace::view(started);
    el
}

fn build_view(s: &Tomboy, cx: &mut Cx<Tomboy>) -> El {
    let dark = cx.system_dark();
    let t = tok::theme(dark);
    let accent = tok::accent(0, dark);

    let crumb = if s.chat.is_some() { s.history.is_some().then(|| "Agent History".to_string()) }
        else if s.history.is_some() { Some("Agent History".to_string()) }
        else if let Some(c) = extui::crumb(s) { Some(c) }
        else if s.clips.is_some() { Some("Clipboard History".to_string()) }
        else if s.emoji.is_some() { Some("Emoji & Symbols".to_string()) }
        else if s.todo.is_some() { Some(format!("Today {}/{}", s.todo_counts.0, s.todo_counts.1)) }
        else if let Some(l) = s.quicklink { Some(s.links[l].name.clone()) }
        else { submenu_title(s) };
    let placeholder = if s.chat.is_some() { "Ask a follow-up…".to_string() }
        else if let Some(p) = extui::placeholder(s) { p }
        else if s.history.is_some() { "Search agent conversations…".to_string() }
        else if s.clips.is_some() { "Search clipboard…".to_string() }
        else if s.emoji.is_some() { "Search emoji and symbols…".to_string() }
        else if s.todo.is_some() && s.editing_task.is_some() { "Edit the todo…".to_string() }
        else if s.todo.is_some() { "Add a todo…".to_string() }
        else if let Some(l) = s.quicklink { s.links[l].prompt() }
        else if let Some(c) = &crumb { format!("Search {}…", c.rsplit(" › ").next().unwrap_or(c)) }
        else { "Search apps, calculate, or paste a URL…".to_string() };
    let field = text_input(&s.query, ty::title()).key("query").single_line().grow(1.0)
        .color(t.text).placeholder(placeholder, t.text_secondary)
        .on_edit(|s: &mut Tomboy| &mut s.query)
        .on_input_key(on_input_key)
        .on_input_change(on_change)
        .on_submit(submit);
    let glyph = if s.chat.is_some() {
        text(AGENT_GLYPH).font(ty::title().family(NERD_FONT)).color(AGENT_COLOR)
    } else {
        // nf-md-magnify: a Nerd Font glyph, since Inter lacks ⌕ and drawing that loaded the
        // system's math and symbol fonts inside the first frame (~12 ms).
        text("\u{f0349}").font(ty::title().family(NERD_FONT)).color(t.text_secondary)
    };
    let mut header = row().h(ty::px(60.0)).px(20.0).gap(14.0).center_y().child(glyph);
    if let Some(c) = crumb {
        // The open submenu, as a chip; clicking it goes back up.
        header = header.child(row().key("crumb").h(ty::px(28.0)).px(10.0).radius(7.0).center_y().fill(t.control_off)
            .child(text(c).font(ty::label()).color(t.text))
            .on_press(|s: &mut Tomboy| if s.chat.is_some() { back_to_history(s) }
                else if s.history.is_some() { if !s.direct { leave_history(s) } }
                else if s.session.is_some() || s.store.is_some() { if !s.direct { s.session = None; s.store = None; s.panel = None; enter_submenu(s, None) } }
                else if s.emoji.is_some() { if !s.direct { leave_emoji(s) } }
                else if s.todo.is_some() { if !s.direct { leave_todo(s) } }
                else if s.clips.is_some() { if !s.direct { leave_clipboard(s) } }
                else if s.quicklink.is_some() { enter_quicklink(s, None) }
                else { leave_submenu(s) }));
    }
    let header = header.child(field);

    let (body, left, action) = match &s.chat {
        Some(run) => chat_body(s, run, dark, &t),
        None if s.session.is_some() => extui::session_body(s, dark, &t, accent),
        None if s.store.is_some() => extui::store_body(s, dark, &t, accent),
        None if s.history.is_some() => history_body(s, dark, &t, accent),
        None if s.clips.is_some() => clipboard_body(s, &t, accent),
        None if s.emoji.is_some() => emoji_body(s, &t, accent),
        None if s.todo.is_some() => todo_body(s, &t, accent),
        None => results_body(s, &t, accent),
    };
    let footer = row().h(ty::px(40.0)).px(16.0).gap(10.0).center_y().fill(t.bg)
        .child(text(left).font(ty::label()).color(t.text_secondary).truncate().grow(1.0).min_w(0.0))
        .child(text(action.0).font(ty::label()).color(t.text))
        .child(keycap(action.1, &t));

    col().w_full().h_full().fill(t.card)
        .child(header)
        .child(el().h(1.0).w_full().fill(t.separator))
        .child(body)
        .child(el().h(1.0).w_full().fill(t.separator))
        .child(footer)
}

/// The search results, the footer's status line and its action hint.
fn results_body(s: &Tomboy, t: &tok::Theme, accent: Color) -> (El, String, (&'static str, &'static str)) {
    let items = results(s);
    let mut list = col().key("results").w_full().padding(8.0);
    let mut section = None;
    for (i, item) in items.iter().enumerate() {
        let name = match item {
            Item::App(_) => "Applications",
            Item::Menu(_) if s.submenu.is_some() => "Commands",
            Item::Menu(_) => s.platform.name(),
            Item::WebSearch(_) | Item::Agent(_) => "Fallback",
            Item::Clipboard | Item::Emoji | Item::Store => "Commands",
            Item::Extension { .. } => "Extensions",
            Item::Clip(_) => "Clipboard",
            Item::Quicklink { .. } => "Quicklinks",
            Item::Todo | Item::TodoAdd(_) => "Todo",
            Item::Answer(a) if a.kind == Kind::Process => "Processes",
            _ => "Results",
        };
        if section != Some(name) {
            section = Some(name);
            list = list.child(row().h(ty::px(28.0)).px(12.0).center_y()
                .child(text(name).font(ty::label()).color(t.text_secondary)));
        }
        list = list.child(item_row(s, item, i, i == s.picked, t, accent));
    }
    if items.is_empty() {
        let empty = match s.quicklink {
            Some(l) => format!("Type the {} and press Enter", s.links[l].prompt().trim_end_matches('…').to_lowercase()),
            None => "No results".into(),
        };
        list = list.child(col().h(120.0).w_full().center()
            .child(text(empty).font(ty::body()).color(t.text_secondary)));
    }
    let body = col().w_full().grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).child(list);
    let action = items.get(s.picked).map(|it| it.action(s)).unwrap_or("");
    let left = s.status.clone().unwrap_or_else(|| match (&s.submenu, s.quicklink) {
        (_, Some(l)) => format!("tomboy · quicklink {} · esc to go back", s.links[l].keyword),
        (Some(_), _) => format!("tomboy · {} · esc to go back", s.platform.name()),
        (None, None) => format!("tomboy {} · {}", env!("CARGO_PKG_VERSION"), s.platform.name()),
    });
    (body, left, (action, "↵"))
}

/// Today's todo list: a checkbox per task; typing adds one.
fn todo_body(s: &Tomboy, t: &tok::Theme, accent: Color) -> (El, String, (&'static str, &'static str)) {
    let tasks = s.todo.as_ref().map(|d| d.tasks()).unwrap_or_default();
    let mut list = col().key("results").w_full().padding(8.0);
    for (i, task) in tasks.iter().enumerate() {
        let editing = s.editing_task.as_ref().is_some_and(|e| e.line == task.line && e.text == task.text);
        list = list.child(task_row(&tasks, task, i, i == s.picked, editing, s.task_drag, t, accent));
    }
    if tasks.is_empty() {
        list = list.child(col().h(120.0).w_full().center()
            .child(text("Nothing on today's list. Type a todo and press Enter").font(ty::body()).color(t.text_secondary)));
    }
    let body = col().w_full().grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).child(list);
    let body = extui::with_panel(s, body, t, accent);
    let (done, total) = s.todo_counts;
    let left = s.status.clone().unwrap_or_else(|| {
        if let Some(task) = &s.editing_task { return format!("Editing “{}” · esc cancels", task.text); }
        let date = s.todo.as_ref().map(|d| d.date.strftime("%A %-d %B").to_string()).unwrap_or_default();
        let count = if total > 0 && done == total { "All done 🎉".to_string() } else { format!("{done}/{total} done") };
        format!("{count} · {date} · space toggles, del removes, {} actions", extui::panel_key_hint())
    });
    let action = if s.editing_task.is_some() { ("Save Todo", "↵") }
        else if !s.query.text.trim().is_empty() { ("Add Todo", "↵") }
        else { match tasks.get(s.picked) { Some(t) if t.done => ("Mark Open", "↵"), Some(_) => ("Mark Done", "↵"), None => ("", "↵") } };
    (body, left, action)
}

fn task_row(tasks: &[todo::Task], task: &todo::Task, i: usize, on: bool, editing: bool, drag: Option<TaskDrag>, t: &tok::Theme, accent: Color) -> El {
    let pitch = ty::px(40.0);
    let count = tasks.len();
    let lifted = drag.is_some_and(|d| d.from == i && d.dy.abs() >= DRAG_SLOP);
    let fg = match (on, task.done) {
        (true, false) => Color::WHITE,
        (true, true) => Color::WHITE.alpha(0.7),
        (false, false) => t.text,
        (false, true) => t.text_secondary,
    };
    // nf-md-checkbox_marked / checkbox_blank_outline
    let (glyph, glyph_color) = if task.done { ("\u{f0132}", if on { Color::WHITE } else { accent }) }
        else { ("\u{f0131}", fg) };
    // nf-md-drag_vertical: the grip to drag the task to a new place.
    let grip = text("\u{f01dd}").font(rune::TextStyle { size: ty::px(18.0), ..ty::body().lh(ty::px(22.0)).family(NERD_FONT) })
        .color(if on { Color::WHITE.alpha(0.75) } else { t.text_secondary.alpha(0.6) });
    let grip = row().key("grip").h_full().w(ty::px(18.0)).center()
        .cursor(if drag.is_some() { Cursor::Grabbing } else { Cursor::Grab })
        .child(grip)
        .on_drag(move |s: &mut Tomboy, d: &mut Drag, tx: &mut Tx| match d.phase {
            Phase::Down => { s.picked = i; s.task_drag = Some(TaskDrag { from: i, to: i, dy: 0.0 }); }
            Phase::Move => if let Some(drag) = s.task_drag.as_mut() {
                drag.dy = d.dy;
                if d.dy.abs() >= DRAG_SLOP {
                    drag.to = (i as f32 + (d.dy / pitch).round()).clamp(0.0, count.saturating_sub(1) as f32) as usize;
                }
                tx.animated();
            },
            Phase::Up => if let Some(drag) = s.task_drag.take() {
                if !d.cancelled() && drag.from != drag.to {
                    if let Err(e) = move_task(s, drag.from, drag.to) { s.status = Some(format!("Failed: {e}")); }
                }
            },
        });
    let fill = if lifted { if on { accent } else { t.control_off } }
        else if on { accent.alpha(0.85) } else { Color::TRANSPARENT };
    let mut r = row().key(task_key(tasks, i)).h(pitch).pl(ty::px(4.0)).pr(10.0).gap(8.0).radius(8.0).center_y()
        .fill(fill)
        .child(grip)
        .child(text(glyph).font(rune::TextStyle { size: ty::px(20.0), ..ty::body().lh(ty::px(22.0)).family(NERD_FONT) }).color(glyph_color))
        .child(text(task.text.clone()).font(ty::body()).color(fg).truncate().grow(1.0).min_w(0.0));
    // How long ago it was added, dimmed on the right (or that it's in the field to edit).
    let detail = if editing { Some("editing".to_string()) }
        else { task.created.map(|c| agent_history::ago(c, agent_history::now())) };
    if let Some(detail) = detail {
        r = r.child(text(detail).font(ty::label()).color(if on { Color::WHITE.alpha(0.75) } else { t.text_secondary }));
    }
    if let Some(d) = drag {
        r = r.offset(0.0, d.shift(i, pitch));
        // Follows the pointer exactly, above the rows it passes; they spring aside.
        if lifted { r = r.raise().offset_motion(Anim::INSTANT).shadow(4.0, 12.0, Color::BLACK.alpha(0.35)); }
    }
    r.on_hover(move |s: &mut Tomboy| if s.task_drag.is_none() { s.picked = i })
        .on_press(move |s: &mut Tomboy| {
            s.picked = i;
            if let Err(e) = toggle_task(s, i) { s.status = Some(format!("Failed: {e}")); }
        })
}

/// The clipboard history: the clips on the left, the picked one's full text or picture on
/// the right.
fn clipboard_body(s: &Tomboy, t: &tok::Theme, accent: Color) -> (El, String, (&'static str, &'static str)) {
    let clips = s.clips.as_deref().unwrap_or_default();
    let items = results(s);
    let mut list = col().key("results").w_full().padding(8.0);
    for (i, item) in items.iter().enumerate() {
        let Item::Clip(c) = item else { continue };
        list = list.child(clip_row(&clips[*c], i, i == s.picked, t, accent));
    }
    if items.is_empty() {
        let empty = if clips.is_empty() { "No clipboard history yet" } else { "No matching clips" };
        list = list.child(col().h(120.0).w_full().center()
            .child(text(empty).font(ty::body()).color(t.text_secondary)));
    }
    let picked = match items.get(s.picked) { Some(Item::Clip(c)) => clips.get(*c), _ => None };
    let preview = match picked {
        Some(clip) => clip_preview(s, clip, t),
        None => col().grow(1.0),
    };
    let body = row().w_full().grow(1.0).min_h(0.0)
        .child(col().w(280.0).h_full().min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).child(list))
        .child(el().w(1.0).h_full().fill(t.separator))
        .child(preview);
    let left = s.status.clone().unwrap_or_else(|| {
        let n = clips.len();
        let back = if s.direct { "esc to close" } else { "esc to go back" };
        format!("tomboy · {n} clip{} · {back}", if n == 1 { "" } else { "s" })
    });
    let action = if picked.is_some() { clip_action(s) } else { "" };
    (body, left, (action, "↵"))
}

fn clip_row(clip: &Clip, i: usize, on: bool, t: &tok::Theme, accent: Color) -> El {
    let (g, c) = match clip {
        Clip::Image { .. } => ("\u{f02e9}", Color { r: 0.85, g: 0.45, b: 0.65, a: 1.0 }), // nf-md-image
        _ if clip.image_path().is_some() => ("\u{f02e9}", Color { r: 0.85, g: 0.45, b: 0.65, a: 1.0 }),
        Clip::Text(_) if clip.is_link() => ("\u{f0337}", Color { r: 0.2, g: 0.5, b: 1.0, a: 1.0 }), // nf-md-link
        Clip::Text(_) if clip.kind().starts_with("File") || clip.kind().ends_with("Files") =>
            ("\u{f0214}", Color { r: 0.95, g: 0.65, b: 0.2, a: 1.0 }), // nf-md-file
        Clip::Text(_) => ("\u{f09a8}", Color { r: 0.45, g: 0.45, b: 0.5, a: 1.0 }), // nf-md-text
    };
    let fg = if on { Color::WHITE } else { t.text };
    row().key(format!("item-{i}")).h(ty::px(40.0)).px(10.0).gap(10.0).radius(8.0).center_y()
        .fill(if on { accent.alpha(0.85) } else { Color::TRANSPARENT })
        .child(row().size(24.0, 24.0).radius(6.0).center().fill(c)
            .child(text(g).font(type_::LABEL.lh(16.0).family(NERD_FONT)).color(Color::WHITE)))
        .child(text(clip.title()).font(ty::body()).color(fg).truncate().grow(1.0).min_w(0.0))
        .on_hover(move |s: &mut Tomboy| { s.picked = i; sync_clip_image(s); })
        .on_press_tx(move |s: &mut Tomboy, tx: &mut Tx| {
            s.picked = i;
            let item = results(s).get(i).cloned();
            if let Some(item) = item { run_item(s, &item, tx); }
        })
}

/// The picked clip in full: text in monospace, or the picture fitted to the pane.
fn clip_preview(s: &Tomboy, clip: &Clip, t: &tok::Theme) -> El {
    let content = match (clip.image_path(), &s.clip_image) {
        // The pixels fill in once the worker decodes them; Fit letterboxes the picture.
        (Some(path), Some((p, img))) if *p == path => match img.error() {
            Some(e) => text(format!("Can't show image: {e}")).font(ty::label()).color(t.text_secondary).wrap(),
            None => el().w_full().grow(1.0).min_h(0.0).pixels_fit(img.pixels(), SurfaceFit::Fit),
        },
        (Some(_), _) => text("Loading…").font(ty::label()).color(t.text_secondary),
        (None, _) => match clip {
            Clip::Text(body) => col().w_full().grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: false })
                .child(text(clipboard::preview_text(body)).font(ty::label().family(Family::Mono))
                    .color(t.text).wrap().selectable(true)),
            Clip::Image { .. } => col(),
        },
    };
    col().key("clip-preview").grow(1.0).min_w(0.0).h_full().padding(16.0).gap(10.0)
        .child(col().w_full().grow(1.0).min_h(0.0).child(content))
        .child(el().h(1.0).w_full().fill(t.separator))
        .child(text(match s.clip_image.as_ref().and_then(|(_, img)| img.size()) {
            Some([w, h]) if clip.image_path().is_some() => format!("{} · {w}×{h}", clip.kind()),
            _ => clip.kind(),
        }).font(ty::label()).color(t.text_secondary).truncate())
}

/// The emoji grid: section headers and rows of big glyphs, built only where visible.
fn emoji_body(s: &Tomboy, t: &tok::Theme, accent: Color) -> (El, String, (&'static str, &'static str)) {
    let v = s.emoji.as_ref().expect("emoji mode");
    let (text2, sel) = (t.text_secondary, accent.alpha(0.35));
    let grid = col().key("emoji-grid").w_full().px(8.0).growth(GrowthSpec::default())
        .virtual_rows(v.rows.len(), ty::px(64.0),
            |_: &Tomboy, r| format!("er{r}").into(),
            move |s: &Tomboy, r| {
                let Some(v) = &s.emoji else { return el() };
                match v.rows.get(r) {
                    Some(emoji::Row::Header(title)) => row().h(ty::px(30.0)).px(8.0).center_y()
                        .child(text(title.clone()).font(ty::label()).color(text2)),
                    Some(emoji::Row::Cells(cells)) => {
                        // Eight equal cells across, whatever the text size; a short last row
                        // keeps them in their columns.
                        let mut line = row().w_full().h(ty::px(64.0));
                        for (c, &g) in cells.iter().enumerate() {
                            let on = v.pos == Some((r, c));
                            line = line.child(row().key(format!("e{r}-{c}")).w(0.0).grow(1.0).h_full().radius(10.0).center()
                                .fill(if on { sel } else { Color::TRANSPARENT })
                                .child(text(emoji::all()[g].ch).font(ty::emoji()))
                                .on_hover(move |s: &mut Tomboy| if let Some(v) = &mut s.emoji { v.pos = Some((r, c)) })
                                .on_press_tx(move |s: &mut Tomboy, tx: &mut Tx| {
                                    if let Some(v) = &mut s.emoji { v.pos = Some((r, c)) }
                                    copy_emoji(s, tx);
                                }));
                        }
                        for _ in cells.len()..emoji::COLS { line = line.child(el().w(0.0).grow(1.0)); }
                        line
                    }
                    None => el(),
                }
            })
        .virtual_rows_version(v.version);
    let body = if v.rows.is_empty() {
        col().w_full().grow(1.0).center().child(text("No matching emoji").font(ty::body()).color(t.text_secondary))
    } else {
        col().key("emoji-scroll").w_full().grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).child(grid)
    };
    let left = s.status.clone().unwrap_or_else(|| match picked_glyph(s) {
        Some(g) => format!("{} · {}", g.title(), g.group),
        None => format!("tomboy · {}", if s.direct { "esc to close" } else { "esc to go back" }),
    });
    let action = match picked_glyph(s) {
        Some(_) if clip_action(s) == "Paste" => "Paste Emoji",
        Some(_) => "Copy Emoji",
        None => "",
    };
    (body, left, (action, "↵"))
}

/// The agent conversation, streaming: prompts, Markdown answers, tool calls.
fn chat_body(s: &Tomboy, run: &agent::Run, dark: bool, t: &tok::Theme) -> (El, String, (&'static str, &'static str)) {
    let tr = run.lock();
    let md = ty::markdown(dark);
    let running = tr.is_running();
    let log = transcript_log(&tr.blocks, running, &md, t);
    let body = col().key("transcript-scroll").w_full().grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: true }).child(log);
    let state = match &tr.status {
        Status::Idle if s.history.is_some() => format!("{} · Esc to go back", agent_history::ago(tr.created, agent_history::now())),
        Status::Idle => "Ready".to_string(),
        Status::Running => "Running… (Esc to stop)".to_string(),
        Status::Done { secs, cost_usd: Some(c) } => format!("Done in {secs:.1}s · ${c:.3}"),
        Status::Done { secs, cost_usd: None } => format!("Done in {secs:.1}s"),
        Status::Failed(e) => format!("Failed: {e}"),
    };
    let who = match &tr.model { Some(m) => format!("{} ({m})", tr.agent), None => tr.agent.clone() };
    let left = s.status.clone().unwrap_or_else(|| format!("{who} · {state}"));
    let action = if running { ("Stop", "esc") }
        else if s.query.text.trim().is_empty() { ("Copy Answer", "↵") }
        else { ("Send Follow-up", "↵") };
    (body, left, action)
}

/// A conversation's blocks: prompts, Markdown answers (selectable, links open in the browser),
/// tool calls and errors. `running`: the last answer is still streaming.
fn transcript_log(blocks: &[Block], running: bool, md: &rune_kit::MarkdownStyle, t: &tok::Theme) -> El {
    let mut log = col().key("transcript").w_full().px(20.0).py(14.0).gap(10.0);
    for (i, block) in blocks.iter().enumerate() {
        let last = i + 1 == blocks.len();
        log = log.child(match block {
            Block::Prompt(p) => row().w_full().gap(10.0).padding(10.0).radius(9.0).fill(t.control_off)
                .child(text("›").font(ty::body()).color(AGENT_COLOR))
                .child(text(p.clone()).font(ty::body()).color(t.text).wrap().selectable(true).grow(1.0).min_w(0.0)),
            Block::Text(md_src) => col().w_full()
                .child(Markdown::new(md_src.trim()).streaming(running && last).selectable(true)
                    .on_link(|s: &mut Tomboy, url: &str| {
                        if let Err(e) = url::open(url) { s.status = Some(format!("Failed: {e}")); }
                    })
                    .render(md)),
            Block::Tool(summary) => row().w_full().gap(8.0).center_y()
                .child(text("⚙").font(ty::label()).color(t.text_secondary))
                .child(text(summary.clone()).font(ty::label().family(Family::Mono)).color(t.text_secondary).truncate().selectable(true)
                    .grow(1.0).min_w(0.0)),
            Block::Error(e) => text(e.clone()).font(ty::body()).color(Color { r: 0.95, g: 0.35, b: 0.35, a: 1.0 }).wrap().selectable(true),
        });
    }
    if running && !matches!(blocks.last(), Some(Block::Text(_))) {
        log = log.child(text("Thinking…").font(ty::label()).color(t.text_secondary));
    }
    log
}

/// The past agent conversations on the left, the picked one's transcript on the right.
fn history_body(s: &Tomboy, dark: bool, t: &tok::Theme, accent: Color) -> (El, String, (&'static str, &'static str)) {
    let past = s.history.as_deref().unwrap_or_default();
    let items = results(s);
    let now = agent_history::now();
    let mut list = col().key("results").w_full().padding(8.0);
    for (i, item) in items.iter().enumerate() {
        let Item::Past(p) = item else { continue };
        list = list.child(past_row(&past[*p], i, i == s.picked, now, t, accent));
    }
    if items.is_empty() {
        let empty = if past.is_empty() { "No agent conversations yet" } else { "No matching conversations" };
        list = list.child(col().h(120.0).w_full().center()
            .child(text(empty).font(ty::body()).color(t.text_secondary)));
    }
    let picked = match items.get(s.picked) { Some(Item::Past(p)) => past.get(*p), _ => None };
    let preview = match picked {
        Some(saved) => {
            let md = ty::markdown(dark);
            let where_ = saved.dir.as_ref().map(|d| format!(" · {}", sys::tilde(d))).unwrap_or_default();
            let who = match &saved.model { Some(m) => format!("{} ({m})", saved.agent), None => saved.agent.clone() };
            col().key("history-preview").grow(1.0).min_w(0.0).h_full()
                .child(col().key("history-preview-scroll").w_full().grow(1.0).min_h(0.0)
                    .scroll(ScrollSpec { stick_bottom: true })
                    .child(transcript_log(&saved.blocks, false, &md, t)))
                .child(el().h(1.0).w_full().fill(t.separator))
                .child(text(format!("{who}{where_}")).font(ty::label()).color(t.text_secondary).truncate().px(16.0).py(8.0))
        }
        None => col().grow(1.0),
    };
    let body = row().w_full().grow(1.0).min_h(0.0)
        .child(col().w(300.0).h_full().min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).child(list))
        .child(el().w(1.0).h_full().fill(t.separator))
        .child(preview);
    let left = s.status.clone().unwrap_or_else(|| {
        let n = past.len();
        let back = if s.direct { "esc to close" } else { "esc to go back" };
        format!("{n} conversation{} · del deletes · {back}", if n == 1 { "" } else { "s" })
    });
    (body, left, (if picked.is_some() { "Open Conversation" } else { "" }, "↵"))
}

fn past_row(saved: &agent_history::Saved, i: usize, on: bool, now: i64, t: &tok::Theme, accent: Color) -> El {
    let fg = if on { Color::WHITE } else { t.text };
    let fg2 = if on { Color::WHITE.alpha(0.75) } else { t.text_secondary };
    let turns = saved.turns();
    let sub = format!("{} · {} · {turns} turn{}", agent_history::ago(saved.updated, now), saved.agent,
        if turns == 1 { "" } else { "s" });
    row().key(format!("item-{i}")).h(ty::px(52.0)).px(10.0).gap(10.0).radius(8.0).center_y()
        .fill(if on { accent.alpha(0.85) } else { Color::TRANSPARENT })
        .child(col().grow(1.0).min_w(0.0).gap(2.0)
            .child(text(saved.title()).font(ty::body()).color(fg).truncate())
            .child(text(sub).font(ty::label()).color(fg2).truncate()))
        .on_hover(move |s: &mut Tomboy| s.picked = i)
        .on_press_tx(move |s: &mut Tomboy, tx: &mut Tx| {
            s.picked = i;
            let item = results(s).get(i).cloned();
            if let Some(item) = item { run_item(s, &item, tx); }
        })
}

fn keycap(label: &str, t: &tok::Theme) -> El {
    // ↵ as nf-md-keyboard_return: any character in U+2100–2BFF makes Rune load the system's
    // math fonts (a fontconfig scan, ~12 ms) inside the first frame, even one Inter has.
    let (label, font) = if label == "↵" { ("\u{f0311}", ty::label().family(NERD_FONT)) } else { (label, ty::label()) };
    row().h(ty::px(20.0)).min_w(ty::px(22.0)).px(6.0).radius(5.0).center().fill(t.control_off)
        .child(text(label.to_string()).font(font).color(t.text))
}

fn item_row(s: &Tomboy, item: &Item, i: usize, on: bool, t: &tok::Theme, accent: Color) -> El {
    let glyph_in = |g: &str, c: Color, family: Option<Family>| row().size(ICON, ICON).radius(7.0).center().fill(c)
        .child(text(g.to_string()).font(family.map_or(type_::BODY, |f| type_::BODY.lh(18.0).family(f))).color(Color::WHITE));
    let glyph = |g: &str, c: Color| glyph_in(g, c, None);
    let (icon, title, subtitle, kind): (El, String, Option<String>, &str) = match item {
        Item::Calc { expr, value } => (glyph("=", Color { r: 1.0, g: 0.58, b: 0.0, a: 1.0 }),
            value.clone(), Some(expr.clone()), "Calculator"),
        Item::Answer(a) => {
            let (g, c, label) = answer_style(a.kind);
            (glyph_in(g, c, Some(NERD_FONT)), a.title.clone(), Some(a.subtitle.clone()), label)
        }
        Item::Url(u) => (glyph("↗", Color { r: 0.2, g: 0.5, b: 1.0, a: 1.0 }), u.clone(), None, "Open URL"),
        Item::Agent(q) => (glyph_in(AGENT_GLYPH, AGENT_COLOR, Some(NERD_FONT)), format!("Ask agent: “{q}”"), None, "Agent"),
        Item::WebSearch(q) => (glyph_in("\u{f0349}", Color { r: 0.45, g: 0.45, b: 0.5, a: 1.0 }, Some(NERD_FONT)), // nf-md-magnify
            format!("Search the web for “{q}”"), None, "DuckDuckGo"),
        Item::Emoji => (glyph_in("\u{f0c79}", Color { r: 0.98, g: 0.70, b: 0.15, a: 1.0 }, Some(NERD_FONT)), // nf-md-emoticon_happy
            "Emoji & Symbols".into(), Some("Search emoji and symbols; Enter copies".into()), "Command"),
        Item::Clipboard => (glyph_in("\u{f0147}", Color { r: 0.85, g: 0.45, b: 0.65, a: 1.0 }, Some(NERD_FONT)), // nf-md-clipboard_text
            "Clipboard History".into(), Some("Browse and copy what you copied".into()), "Command"),
        Item::Clip(c) => {
            let clip = &s.clips.as_ref().expect("clips listed in clipboard mode")[*c];
            (glyph("⧉", accent), clip.title(), None, "Clip")
        }
        Item::Todo => {
            let (done, total) = s.todo_counts;
            let sub = if total == 0 { "Nothing yet. Add some".to_string() }
                else if done == total { "All done 🎉".to_string() } else { format!("{} left", total - done) };
            (glyph_in("\u{f0134}", Color { r: 0.20, g: 0.65, b: 0.45, a: 1.0 }, Some(NERD_FONT)), // nf-md-checkbox_marked_circle_outline
                format!("Today · {done}/{total} done"), Some(sub), "Todo")
        }
        Item::TodoAdd(text) => (glyph_in("\u{f0415}", Color { r: 0.20, g: 0.65, b: 0.45, a: 1.0 }, Some(NERD_FONT)), // nf-md-plus
            format!("Add todo: {text}"), Some(format!("to today's list · {}/{} done", s.todo_counts.0, s.todo_counts.1)), "Todo"),
        Item::Task(_) => (glyph("☐", accent), String::new(), None, "Task"), // drawn by `task_row`
        Item::Extension { ext, command } => {
            let (g, title, sub, kind) = extui::command_row(s, *ext, *command);
            (glyph_in(&g, extui::ext_color(), Some(NERD_FONT)), title, sub, leak_kind(kind))
        }
        Item::Store => {
            let (g, title, sub) = extui::store_row_parts();
            (glyph_in(&g, extui::ext_color(), Some(NERD_FONT)), title, sub, "Command")
        }
        Item::ExtRow(_) | Item::StoreRow(_) => (glyph("•", accent), String::new(), None, ""), // drawn by `extui`
        Item::AgentHistory => (glyph_in(AGENT_GLYPH, AGENT_COLOR, Some(NERD_FONT)),
            "Agent History".into(), Some("Past agent conversations; open one to continue it".into()), "Command"),
        Item::Past(_) => (glyph("•", accent), String::new(), None, ""), // drawn by `past_row`
        Item::Quicklink { link, arg } => {
            let l = &s.links[*link];
            let (title, sub) = match arg {
                Some(a) => (format!("Open {}: {a}", l.name), l.expand(a)),
                None if l.arg.is_none() => (l.name.clone(), l.expand("")),
                None => (l.name.clone(), l.url.clone()),
            };
            (glyph_in("\u{f0337}", Color { r: 0.25, g: 0.55, b: 0.95, a: 1.0 }, Some(NERD_FONT)), // nf-md-link_variant
                title, Some(sub), "Quicklink")
        }
        Item::App(a) => {
            let app = &s.apps[*a];
            let icon = match &s.icons[*a] {
                Some(img) if img.error().is_none() => el().size(ICON, ICON).pixels(img.pixels()),
                _ => glyph(&app.name.chars().next().unwrap_or('?').to_uppercase().to_string(), accent),
            };
            (icon, app.name.clone(), app.comment.clone().or_else(|| app.generic_name.clone()), "Application")
        }
        Item::Menu(m) => {
            let entry = &s.menu[*m];
            let icon = row().size(ICON, ICON).radius(7.0).center().fill(t.control_off)
                .child(text(entry.icon.clone().unwrap_or_else(|| "•".into()))
                    .font(type_::BODY.lh(18.0).family(NERD_FONT)).color(t.text));
            let title = if s.conds.checked(*m) { format!("{} ✓", entry.label) } else { entry.label.clone() };
            let crumb = entry.breadcrumb();
            let sub = entry.description.clone().or((!crumb.is_empty()).then_some(crumb));
            // Inside a submenu the breadcrumb is in the header; show it only for deeper rows.
            let sub = match &s.submenu {
                Some(open) => {
                    let below = entry.parents.len().saturating_sub(open.split('.').count());
                    entry.description.clone().or_else(|| (below > 0).then(|| entry.parents[entry.parents.len() - below..].join(" › ")))
                }
                None => sub,
            };
            let kind = if omarchy::has_children(&s.menu, &entry.id) { "Submenu ›" }
                else if entry.is_submenu() { "Omarchy Menu" } else { "Command" };
            (icon, title, sub, kind)
        }
    };
    // The row an alias pinned says so.
    let subtitle = match alias_target(s) {
        Some((alias, _)) if i == 0 && s.submenu.is_none() && s.quicklink.is_none() =>
            Some(subtitle.map_or(format!("alias {alias}"), |sub| format!("alias {alias} · {sub}"))),
        _ => subtitle,
    };
    let fg = if on { Color::WHITE } else { t.text };
    let fg2 = if on { Color::WHITE.alpha(0.75) } else { t.text_secondary };
    let mut r = row().key(format!("item-{i}")).h(ty::px(46.0)).px(12.0).gap(12.0).radius(9.0).center_y()
        .fill(if on { accent.alpha(0.85) } else { Color::TRANSPARENT })
        .child(icon)
        .child(text(title).font(ty::body()).color(fg).truncate());
    if let Some(sub) = subtitle {
        r = r.child(text(sub).font(ty::label()).color(fg2).truncate().grow(1.0).min_w(0.0));
    } else {
        r = r.child(spacer());
    }
    let item = item.clone();
    r.child(text(kind).font(ty::label()).color(fg2))
        .on_hover(move |s: &mut Tomboy| s.picked = i)
        .on_press_tx(move |s: &mut Tomboy, tx: &mut Tx| { s.picked = i; run_item(s, &item, tx); })
}

/// Icon (Nerd Font), icon colour and right-hand label for an answer row.
fn answer_style(kind: Kind) -> (&'static str, Color, &'static str) {
    let rgb = |r, g, b| Color { r, g, b, a: 1.0 };
    match kind {
        Kind::Units => ("\u{f046d}", rgb(0.30, 0.65, 0.45), "Convert"),        // nf-md-ruler
        Kind::Currency => ("\u{f01c1}", rgb(0.20, 0.60, 0.35), "Currency"),    // nf-md-currency_usd
        Kind::Time => ("\u{f017}", rgb(0.35, 0.50, 0.95), "Time"),             // nf-fa-clock_o
        Kind::Dev => ("\u{f121}", rgb(0.55, 0.40, 0.85), "Developer"),         // nf-fa-code
        Kind::Process => ("\u{f068c}", rgb(0.85, 0.30, 0.30), "Process"),      // nf-md-skull
        Kind::Date => ("\u{f00ed}", rgb(0.15, 0.60, 0.60), "Date"),          // nf-md-calendar
    }
}

/// An extension's name as a row's right-hand label (`item_row` takes `&'static str`; names
/// are few and fixed for the launch).
fn leak_kind(name: String) -> &'static str {
    static NAMES: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    let mut names = NAMES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(n) = names.iter().find(|n| **n == name) { return n; }
    let n: &'static str = Box::leak(name.into_boxed_str());
    names.push(n);
    n
}

/// A finished conversation for the headless `chat` scene.
fn chat_scene(s: &mut Tomboy) {
    let run = agent::Run::demo("claude", &[
        Block::Prompt("what's using port 3000?".into()),
        Block::Tool("Bash: ss -ltnp | grep :3000".into()),
        Block::Text("**node** (pid `48211`) is listening on port 3000 — that's your `vite` dev server:\n\n\
            ```\nLISTEN 0 511 *:3000 *:* users:((\"node\",pid=48211,fd=23))\n```\n\n\
            Stop it with `kill 48211`, or press `q` in its terminal.".into()),
    ], Status::Done { secs: 6.4, cost_usd: Some(0.0213) });
    s.chat = Some(run);
}

/// Past conversations for the headless `agent-history` scene.
fn history_scene(s: &mut Tomboy) {
    let now = agent_history::now();
    let conv = |id: &str, ago: i64, agent: &str, blocks: Vec<Block>| agent_history::Saved {
        id: id.into(), agent: agent.into(), model: None, dir: sys::home_dir().map(|h| h.join("Work")),
        session: Some(format!("session-{id}")), created: now - ago, updated: now - ago, blocks,
    };
    s.history = Some(vec![
        conv("a", 300, "claude", vec![
            Block::Prompt("what's using port 3000?".into()),
            Block::Tool("Bash: ss -ltnp | grep :3000".into()),
            Block::Text("**node** (pid `48211`) is listening on port 3000 — your `vite` dev server.\n\nStop it with `kill 48211`.".into()),
            Block::Prompt("and port 5432?".into()),
            Block::Text("That's **postgres**, started by systemd ([docs](https://www.postgresql.org/docs/)).".into()),
        ]),
        conv("b", 3 * 3600, "claude", vec![
            Block::Prompt("change the working directory to ~/sites".into()),
            Block::Tool("Edit: ~/.config/tomboy/config.toml".into()),
            Block::Text("Done: `[agent] working_directory = \"~/sites\"`.".into()),
        ]),
        conv("c", 2 * 86_400, "opencode", vec![
            Block::Prompt("summarize the last 5 commits in tomboy".into()),
            Block::Text("1. Performance plan\n2. Extension fixes\n3. Luau extensions".into()),
        ]),
    ]);
    s.direct = true;
}

/// A clipboard history for the headless `clipboard` scene.
fn clipboard_scene(s: &mut Tomboy) {
    s.clips = Some(vec![
        Clip::Text("fn main() {\n    println!(\"hello, tomboy\");\n}\n".into()),
        Clip::Text("https://github.com/dzhibas/tomboy/issues/1".into()),
        Clip::Text("bind = SUPER SHIFT, V, exec, ~/Work/tomboy/target/release/tomboy clipboard".into()),
        Clip::Text("file:///home/me/Pictures/screenshot.png".into()),
        Clip::Text("The quick brown fox jumps over the lazy dog.".into()),
    ]);
    s.direct = true;
}

/// A day's list in memory (nothing written) for the headless `todo` scene.
fn todo_scene(s: &mut Tomboy) {
    let day = todo::Day::demo(jiff::civil::date(2026, 11, 12), &[
        (true, "Reply to Jonas about the offsite", 3 * 86_400), (true, "Ship tomboy 0.2", 26 * 3600),
        (false, "Review PR #42", 5 * 3600), (false, "Buy milk", 2 * 3600), (false, "Book the dentist", 45 * 60),
        (true, "Pay the electricity bill", 15 * 60), (false, "Call mum", 20),
    ]);
    s.todo_counts = day.counts();
    s.todo = Some(day);
    s.picked = 2;
    s.direct = true;
}

/// Quicklinks and an alias for the headless scenes, instead of the real config's.
fn demo_links(s: &mut Tomboy) {
    s.links = vec![
        quicklinks::Quicklink::new("gh", "https://github.com/search?q={query}", Some("GitHub search")),
        quicklinks::Quicklink::new("jira", "issues.jira.com/browse/{a}", Some("Jira issue")),
    ];
    s.config.aliases = [("bubu".to_string(), "chromium".to_string())].into();
}

fn main() {
    trace::start();
    // Headless subcommands that record the clipboard history; no window.
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("clipboard-watch") => exit_on_error(clipboard::watch()),
        Some("clipboard-capture") => exit_on_error(clipboard::capture(args.get(1).map_or("text", String::as_str))),
        Some("todo") if args.get(1).map(String::as_str) == Some("add") => {
            let dir = config::load().0.todo_dir();
            exit_on_error(dir.ok_or_else(|| std::io::Error::other("no home directory"))
                .and_then(|d| todo::add_cli(&d, &args[2..].join(" "))))
        }
        Some("ext") if args.get(1).map(String::as_str) == Some("dev") => {
            let Some(dir) = args.get(2) else { eprintln!("usage: tomboy ext dev <dir>"); std::process::exit(2) };
            let dir = std::fs::canonicalize(dir).unwrap_or_else(|e| { eprintln!("tomboy ext dev: {dir}: {e}"); std::process::exit(1) });
            if let Err(e) = ext::runtime::check(&dir) { eprintln!("tomboy ext dev: {e}"); std::process::exit(1) }
            let _ = DEV_DIR.set(dir);
        }
        Some("ext") => std::process::exit(ext::cli::run(&args[1..], &config::load().0.registry())),
        #[cfg(target_os = "macos")]
        Some("daemon") => return run_daemon(),
        #[cfg(target_os = "macos")]
        Some("install-launch-agent") => exit_on_error(mac::launch_agent::install()),
        #[cfg(target_os = "macos")]
        Some("uninstall-launch-agent") => exit_on_error(mac::launch_agent::uninstall()),
        #[cfg(not(target_os = "macos"))]
        Some("daemon" | "install-launch-agent" | "uninstall-launch-agent") => exit_on_error(Err(std::io::Error::other(
            "the background launcher is for macOS; on Linux bind `tomboy` to a key in your compositor"))),
        _ => {}
    }
    // The launch hotkey toggles one window: a second launch closes the open one and exits.
    #[cfg(unix)]
    if !single::claim() { return; }
    rune::set_app_id("tomboy");
    // macOS: device and pipelines on a thread. Linux: `prewarm_gpu` does nothing there, and
    // `warm_gpu` is its equivalent, so the window's first frame doesn't wait ~40 ms for them.
    rune::prewarm_gpu();
    #[cfg(not(target_os = "macos"))]
    rune::warm_gpu(Default::default());
    // Read the font files while the GPU warms, before Rune builds the text system.
    fonts::spec();
    trace::mark("gpu warm-up started; building app");
    App::new(spec())
        .window_options(WindowOptions { borderless: true, resizable: false, max_size: Some((WIDTH, HEIGHT)), ..Default::default() })
        .float_on_open()
        .on_focus(on_focus)
        // The compositor closing the window (Hyprland's Super+W) stops a running agent too.
        .on_close(|s, _| if let Some(run) = &s.chat { run.stop() })
        .on_frame_encoded(first_frame::on_frame)
        .on_ready(|_| trace::mark("on_ready (first frame on the GPU)"))
        .run();
    #[cfg(unix)]
    single::release();
    trace::mark("event loop exited");
    trace::summary();
}

/// Losing focus never closes a launched window: focus-follows-mouse would close it the moment
/// the pointer left. Esc, Cmd/Super+W or the compositor's close does. Only the macOS daemon's
/// panel hides on click-away (not while a conversation is open).
fn on_focus(s: &mut Tomboy, focused: bool, tx: &mut Tx) {
    if focused { trace::first("window focused"); s.had_focus = true; return; }
    if s.resident && s.had_focus && s.chat.is_none() { quit(s, tx) }
}

/// `tomboy daemon`: stay running with no Dock icon, the window a hidden launcher panel that
/// the `[hotkeys]` chords show, and record the clipboard history on a thread.
#[cfg(target_os = "macos")]
fn run_daemon() {
    // One daemon at a time: a second would find the hotkeys taken.
    let lock = sys::state_dir().map(|d| d.join("tomboy")).and_then(|d| {
        std::fs::create_dir_all(&d).ok()?;
        std::fs::File::create(d.join("daemon.lock")).ok()
    });
    if let Some(file) = &lock && file.try_lock().is_err() {
        eprintln!("tomboy: the daemon is already running");
        std::process::exit(0);
    }
    RESIDENT.store(true, Ordering::Relaxed);
    std::thread::spawn(|| if let Err(e) = clipboard::watch() { eprintln!("tomboy: clipboard-watch: {e}") });
    rune::set_app_id("tomboy");
    rune::prewarm_gpu();
    let config = config::load().0;
    let mut app = App::new(spec()).accessory().keep_running()
        .configured_options(WindowOptions::launcher_panel().macos(|native| native.corner_radius(12.0)))
        // The panel preset's portable half, at tomboy's fixed size (its AppKit half stays).
        .window_options(WindowOptions {
            visible: false, borderless: true, transparent: true, resizable: false,
            min_size: Some((WIDTH, HEIGHT)), max_size: Some((WIDTH, HEIGHT)), ..Default::default()
        })
        .on_focus(on_focus)
        .on_frame_encoded(first_frame::on_frame)
        .on_ready(|_| trace::mark("on_ready (first frame on the GPU)"));
    let me = app.handle();
    app = app.init(move || Tomboy { me: Some(me), ..init() });
    for hotkey in Hotkey::ALL {
        if let Some(chord) = config.hotkey(hotkey) { app = app.global_shortcut(&chord, hotkey_handler(hotkey)); }
    }
    app.run();
    drop(lock);
}

fn spec() -> AppSpec<Tomboy> {
    AppSpec {
        min_size: Some((WIDTH, HEIGHT)),
        // Code in agent answers uses the mono family: the Nerd Font when `fonts` found it,
        // else the system monospace (fontconfig's `monospace`).
        fonts: fonts::spec().0,
        families: fonts::spec().1,
        setup: Some(setup),
        poll: Some(poll),
        on_key: Some(on_window_key),
        scenes: &[
            ("apps", |s| s.query = TextEdit::new("fi")),
            ("calc", |s| s.query = TextEdit::new("(2+2)*10/3")),
            ("url", |s| s.query = TextEdit::new("github.com/anomalyco/rune")),
            ("web", |s| s.query = TextEdit::new("rust rune ui")),
            ("omarchy", |s| s.query = TextEdit::new("bar")),
            ("system", |s| s.query = TextEdit::new("dark")),
            ("chat", chat_scene),
            ("agent-history", history_scene),
            ("agent-history-open", |s| { history_scene(s); open_past(s, 0); }),
            ("agent-history-command", |s| s.query = TextEdit::new("agent hist")),
            ("submenu", |s| s.submenu = Some("learn".into())),
            ("units", |s| s.query = TextEdit::new("100 km to miles")),
            ("time", |s| s.query = TextEdit::new("time in tokyo")),
            ("dev", |s| s.query = TextEdit::new("password 24")),
            ("kill", |s| s.query = TextEdit::new("kill sleep")),
            ("currency", |s| s.query = TextEdit::new("123 eur to usd")),
            ("clipboard-command", |s| s.query = TextEdit::new("clip")),
            ("clipboard", clipboard_scene),
            ("emoji", |s| { enter_emoji(s); s.direct = true; }),
            ("emoji-search", |s| { enter_emoji(s); s.direct = true; s.query = TextEdit::new("heart"); rebuild_emoji(s); }),
            ("emoji-command", |s| s.query = TextEdit::new("emoji")),
            ("date", |s| s.query = TextEdit::new("days until 25 dec")),
            ("date-offset", |s| s.query = TextEdit::new("monday in 3 weeks")),
            ("percent", |s| s.query = TextEdit::new("15% of 200")),
            ("quicklink", |s| { demo_links(s); s.query = TextEdit::new("jira ABC-1"); }),
            ("quicklink-command", |s| { demo_links(s); s.query = TextEdit::new("ji"); }),
            ("quicklink-arg", |s| { demo_links(s); s.quicklink = Some(1); }),
            ("alias", |s| { demo_links(s); s.query = TextEdit::new("bubu"); }),
            ("todo", todo_scene),
            // "Review PR #42" dragged down past two rows.
            ("todo-panel", |s| { todo_scene(s); extui::open_panel(s); }),
            ("todo-edit", |s| { todo_scene(s); start_task_edit(s, 2); }),
            ("todo-drag", |s| { todo_scene(s); s.task_drag = Some(TaskDrag { from: 2, to: 4, dy: ty::px(86.0) }); }),
            ("todo-command", |s| { s.todo_counts = (3, 7); s.query = TextEdit::new("todo"); }),
            ("todo-add", |s| { s.todo_counts = (3, 7); s.query = TextEdit::new("todo buy milk"); }),
            ("store", |s| extui::store_scene(s, false)),
            ("store-consent", |s| extui::store_scene(s, true)),
            ("store-command", |s| s.query = TextEdit::new("install ext")),
            ("ext-hackernews", extui::hackernews_scene),
            ("ext-command", extui::command_scene),
            ("ext-hackernews-panel", |s| { extui::hackernews_scene(s); extui::open_panel(s); }),
        ],
        ..AppSpec::new("tomboy", (WIDTH, HEIGHT), init, view)
    }
}

fn exit_on_error(r: std::io::Result<()>) -> ! {
    match r {
        Ok(()) => std::process::exit(0),
        Err(e) => { eprintln!("tomboy: {e}"); std::process::exit(1) }
    }
}

#[cfg(test)]
mod extension_flow_tests {
    use super::*;

    fn key(k: Key) -> KeyEvent { KeyEvent { key: k, mods: Default::default() } }

    #[test]
    fn todo_panel_edits_in_the_field() {
        let dir = std::env::temp_dir().join(format!("tomboy-todo-panel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let date = jiff::civil::date(2026, 11, 12);
        std::fs::write(dir.join("2026-11-12.md"), "# 2026-11-12\n\n- [ ] a\n- [ ] typo\n").unwrap();
        let mut s = init();
        let mut tx = Tx::default();
        s.todo = Some(todo::Day::load(&dir, date).unwrap());
        s.picked = 1;

        // Ctrl/⌘+K, then Enter on the first action: Edit puts the text in the field.
        assert!(on_input_key(&mut s, &key(Key::Command('k')), &mut tx));
        assert!(s.panel.is_some());
        submit(&mut s, &mut tx);
        assert_eq!(s.query.text, "typo");
        assert!(s.query.focused);
        assert_eq!(s.query.focus, "typo".len()); // the cursor at the end
        assert!(s.editing_task.is_some());

        // Esc cancels without touching the file; editing again and Enter saves the same task.
        assert!(on_input_key(&mut s, &key(Key::Escape), &mut tx));
        assert!(s.editing_task.is_none() && s.query.text.is_empty() && s.todo.is_some());
        start_task_edit(&mut s, 1);
        s.query = TextEdit::new("fixed");
        submit(&mut s, &mut tx);
        assert!(s.editing_task.is_none() && s.query.text.is_empty());
        assert_eq!(std::fs::read_to_string(dir.join("2026-11-12.md")).unwrap(), "# 2026-11-12\n\n- [ ] a\n- [ ] fixed\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn store_consent_and_extension_keys() {
        let mut s = init();
        let mut tx = Tx::default();

        // Enter on an available extension asks first; Esc cancels and stays in the store.
        extui::store_scene(&mut s, false);
        assert!(matches!(results(&s).first(), Some(Item::StoreRow(0))));
        submit(&mut s, &mut tx);
        assert!(extui::consent_open(&s));
        assert!(on_input_key(&mut s, &key(Key::Escape), &mut tx));
        assert!(!extui::consent_open(&s) && s.store.is_some());

        // The Hacker News sample: its stories are the rows, the field filters them.
        s.store = None;
        extui::hackernews_scene(&mut s);
        assert_eq!(results(&s).len(), 12);
        s.query = TextEdit::new("rust");
        s.picked = 0; // as on_change does after an edit
        assert!((1..12).contains(&results(&s).len()));

        // Ctrl/⌘+K opens the panel, which keeps the keyboard until Esc.
        assert!(on_input_key(&mut s, &key(Key::Command('k')), &mut tx));
        assert!(s.panel.is_some());
        assert!(on_input_key(&mut s, &key(Key::Down), &mut tx));
        assert_eq!(s.panel.as_ref().map(|p| p.picked), Some(1));
        assert!(on_input_key(&mut s, &key(Key::Escape), &mut tx));
        assert!(s.panel.is_none() && s.session.is_some());

        // Esc clears the filter, then leaves the extension.
        assert!(on_input_key(&mut s, &key(Key::Escape), &mut tx));
        assert!(s.query.text.is_empty() && s.session.is_some());
        assert!(on_input_key(&mut s, &key(Key::Escape), &mut tx));
        assert!(s.session.is_none());
    }

    /// The history lists past conversations; one opens as the live chat with its turns, Esc
    /// comes back to the list, and the search filters it.
    #[test]
    fn agent_history_opens_and_returns() {
        let mut s = init();
        let mut tx = Tx::default();
        s.query = TextEdit::new("agents");
        assert!(results(&s).contains(&Item::AgentHistory));
        s.query = TextEdit::new("");
        history_scene(&mut s);
        s.direct = false;
        assert_eq!(results(&s), [Item::Past(0), Item::Past(1), Item::Past(2)]);
        s.query = TextEdit::new("postgres");
        assert_eq!(results(&s), [Item::Past(0)]);
        s.query = TextEdit::new("");
        s.picked = 1;
        submit(&mut s, &mut tx);
        let run = s.chat.clone().expect("opened");
        {
            let t = run.lock();
            assert_eq!(t.session.as_deref(), Some("session-b"));
            assert_eq!(t.history_id.as_deref(), Some("b"));
            assert_eq!(t.status, Status::Idle);
            assert_eq!(t.last_answer(), "Done: `[agent] working_directory = \"~/sites\"`.");
        }
        // Esc goes back to the list (read again from disk), not out of tomboy.
        assert!(on_input_key(&mut s, &key(Key::Escape), &mut tx));
        assert!(s.chat.is_none() && s.history.is_some());
        // Esc on the list goes back to the launcher.
        assert!(on_input_key(&mut s, &key(Key::Escape), &mut tx));
        assert!(s.history.is_none());
    }

    /// Esc and Super+W close the conversation even when the search field has lost focus.
    #[test]
    fn window_keys_close_from_anywhere() {
        let mut s = init();
        let mut tx = Tx::default();
        chat_scene(&mut s);
        let logo = |k: Key| KeyEvent { key: k, mods: rune::Mods { logo: true, ..Default::default() } };
        assert!(on_window_key(&mut s, &key(Key::Escape), &mut tx));
        assert!(on_window_key(&mut s, &logo(Key::Text("w".into())), &mut tx));
        assert!(on_window_key(&mut s, &key(Key::Command('w')), &mut tx));
        assert!(on_window_key(&mut s, &key(Key::PageUp), &mut tx));
        assert!(!on_window_key(&mut s, &key(Key::Text("w".into())), &mut tx));
    }

    #[test]
    fn launcher_lists_extension_commands_and_the_store() {
        let mut s = init();
        extui::command_scene(&mut s);
        assert!(matches!(results(&s).first(), Some(Item::Extension { ext: 0, command: 0 })));
        s.query = TextEdit::new("install ext");
        assert!(results(&s).contains(&Item::Store));
    }

    /// With an empty field, a used extension command comes after the applications.
    #[test]
    fn empty_launcher_lists_apps_before_extensions() {
        let mut s = init();
        extui::command_scene(&mut s);
        let id = s.extensions[0].0.id.clone();
        let cmd = s.extensions[0].0.commands[0].id.clone();
        s.frecency = frecency::Store::default(); // in memory: never writes the real history file
        s.frecency.record(&extui::frecency_key(&id, &cmd), "", frecency::now());
        s.query = TextEdit::new("");
        let items = results(&s);
        let ext = items.iter().position(|i| matches!(i, Item::Extension { .. })).expect("used extension listed");
        if let Some(app) = items.iter().position(|i| matches!(i, Item::App(_))) {
            assert!(app < ext, "apps first: {items:?}");
        }
    }
}
