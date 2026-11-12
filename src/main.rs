//! tomboy — a personal Raycast-style launcher built with Rune.
//!
//! Type to search installed applications, evaluate math (`2+2`), or open a URL.
//! ↑/↓ pick, Enter runs, Esc (or clicking away) closes.
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
mod answers;
mod apps;
mod calc;
mod clipboard;
mod config;
mod dictation;
mod emoji;
mod frecency;
// Omarchy is Linux-only; on macOS the module compiles but `Platform::Omarchy` never occurs.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod omarchy;
mod platform;
mod quicklinks;
mod sys;
mod todo;
mod url;

use rune::tok::{self, type_};
use rune::{
    App, AppSpec, Color, Cx, El, Family, Image, ImageCache, Key, KeyEvent, ScrollSpec, SurfaceFit, TextEdit, Tx,
    WindowOptions, Waker, GrowthSpec, col, el, row, spacer, text, text_input,
};
use rune_kit::Markdown;

use std::sync::{Arc, Mutex};

use agent::{Block, Status};
use answers::{Answer, Kind};
use apps::AppEntry;
use clipboard::Clip;
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
    /// The agent conversation; while it exists tomboy shows it instead of search results.
    chat: Option<agent::Run>,
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
    /// Today's (done, total), for the launcher row; kept up to date as the list changes.
    todo_counts: (usize, usize),
}

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
            Item::Clip(_) => "Copy to Clipboard",
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
        }
    }
}

/// What tomboy does after running an item.
enum After {
    Quit,
    Stay,
}

fn init() -> Tomboy {
    let (config, config_error) = config::load();
    let frecency = if config.frecency() { frecency::Store::load() } else { frecency::Store::off() };
    ty::set_scale(config.font_scale());
    let status = config_error.or_else(|| config.warning());
    let mut query = TextEdit::new("");
    query.focused = true;
    let apps = apps::load_apps();
    let platform = platform::detect();
    let menu = match &platform {
        Platform::Omarchy { root } => omarchy::load(root),
        Platform::Linux | Platform::MacOS => Vec::new(),
    };
    let mut s = Tomboy {
        query, picked: 0, icons: vec![None; apps.len()], apps, cache: None,
        platform, menu, conds: Conditions::default(), status,
        last_text: String::new(), dictated: Arc::default(), waker: None, chat: None, submenu: None,
        seed: answers::devtools::session_seed(), wake: Arc::new(|| {}),
        clips: None, direct: false, emoji: None, clip_image: None, frecency,
        links: quicklinks::load(&config), quicklink: None,
        todo: None, todo_counts: config.todo_dir().map_or((0, 0), |d| todo::peek(&d, todo::today_date())),
        config,
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
    }
    s
}

/// Start decoding every app's icon on Rune's image worker and checking the menu's shell
/// conditions on a background pool; each result wakes the window.
fn setup(s: &mut Tomboy, waker: Waker) {
    let cache = ImageCache::new(waker.clone());
    s.icons = s.apps.iter().map(|a| a.icon.as_ref().map(|_| cache.app_icon(&a.path, ICON))).collect();
    s.cache = Some(cache);
    if let Platform::Omarchy { root } = &s.platform {
        let waker = waker.clone();
        s.conds = omarchy::evaluate(&s.menu, root, move || waker());
    }
    s.wake = waker.clone();
    s.waker = Some(waker);
    sync_clip_image(s);
}

/// After every edit: if one change inserted a burst of text, check on a thread whether it
/// was a Superwhisper dictation (the history entry can land just after the text does).
fn on_change(s: &mut Tomboy, _: &mut Tx) {
    s.picked = 0;
    s.status = None;
    if s.clips.is_some() {
        s.last_text = s.query.text.clone();
        sync_clip_image(s);
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

/// A confirmed dictation goes straight to the agent instead of the result list.
fn poll(s: &mut Tomboy, _: f64, tx: &mut Tx) -> bool {
    let Some(prompt) = s.dictated.lock().unwrap_or_else(|e| e.into_inner()).take() else { return false };
    if prompt != s.query.text { return false; } // edited since; treat it as typed
    if chat_running(s) { return false; }
    run_item(s, &Item::Agent(prompt), tx);
    true
}

fn results(s: &Tomboy) -> Vec<Item> {
    let q = s.query.text.trim();
    if let Some(clips) = &s.clips {
        return clipboard::search(clips, q).into_iter().map(Item::Clip).collect();
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
    items.extend(links.iter().filter(|h| !h.exact).map(link_item));
    let app_limit = if q.is_empty() { MAX_APPS } else { MAX_APP_HITS };
    let app_boost = |i: usize| s.frecency.boost(&frecency::app_key(&s.apps[i].id), q, now);
    items.extend(apps::search(&s.apps, q, app_limit, app_boost).into_iter().map(Item::App));
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
        (Item::Clipboard, Item::Clipboard) | (Item::Emoji, Item::Emoji) => true,
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
            Platform::Linux | Platform::MacOS => Ok(After::Stay),
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
        Ok(true) => quit(s, tx),
        Ok(false) => { tx.copy(g.ch); s.status = Some(format!("Copied {}", g.ch)); }
        Err(e) => s.status = Some(format!("Failed: {e}")),
    }
}

/// Esc, Space and Delete in the todo list (arrows and Enter work as in any list).
fn todo_key_event(s: &mut Tomboy, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let empty = s.query.text.is_empty();
    let result = match &ev.key {
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
    if clip.copy()? { return Ok(After::Quit); }
    match &clip {
        // No clipboard helper installed: copy in-process and stay open so the copy lives.
        Clip::Text(t) => { tx.copy(t); s.status = Some("Copied".into()); Ok(After::Stay) }
        Clip::Image { .. } => Err(std::io::Error::other("no clipboard helper for images (install wl-clipboard)")),
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

/// Quit, stopping a running agent first (it runs in its own process group).
fn quit(s: &mut Tomboy, tx: &mut Tx) {
    if let Some(run) = &s.chat { run.stop(); }
    tx.quit();
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
    if s.emoji.is_some() { copy_emoji(s, tx); return; }
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

fn on_input_key(s: &mut Tomboy, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if let Some(run) = s.chat.clone() {
        match ev.key {
            // Esc stops a running agent; once it is idle, Esc closes tomboy.
            Key::Escape if run.lock().is_running() => run.stop(),
            Key::Escape => quit(s, tx),
            _ => return false,
        }
        return true;
    }
    if s.emoji.is_some() { return emoji_key_event(s, ev, tx); }
    if s.todo.is_some() && todo_key_event(s, ev, tx) { return true; }
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
    tx.scroll_into_view(format!("item-{}", s.picked));
    true
}

fn view(s: &Tomboy, cx: &mut Cx<Tomboy>) -> El {
    let dark = cx.system_dark();
    let t = tok::theme(dark);
    let accent = tok::accent(0, dark);

    let crumb = if s.chat.is_some() { None }
        else if s.clips.is_some() { Some("Clipboard History".to_string()) }
        else if s.emoji.is_some() { Some("Emoji & Symbols".to_string()) }
        else if s.todo.is_some() { Some(format!("Today {}/{}", s.todo_counts.0, s.todo_counts.1)) }
        else if let Some(l) = s.quicklink { Some(s.links[l].name.clone()) }
        else { submenu_title(s) };
    let placeholder = if s.chat.is_some() { "Ask a follow-up…".to_string() }
        else if s.clips.is_some() { "Search clipboard…".to_string() }
        else if s.emoji.is_some() { "Search emoji and symbols…".to_string() }
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
        text("⌕").font(ty::title()).color(t.text_secondary)
    };
    let mut header = row().h(ty::px(60.0)).px(20.0).gap(14.0).center_y().child(glyph);
    if let Some(c) = crumb {
        // The open submenu, as a chip; clicking it goes back up.
        header = header.child(row().key("crumb").h(ty::px(28.0)).px(10.0).radius(7.0).center_y().fill(t.control_off)
            .child(text(c).font(ty::label()).color(t.text))
            .on_press(|s: &mut Tomboy| if s.emoji.is_some() { if !s.direct { leave_emoji(s) } }
                else if s.todo.is_some() { if !s.direct { leave_todo(s) } }
                else if s.clips.is_some() { if !s.direct { leave_clipboard(s) } }
                else if s.quicklink.is_some() { enter_quicklink(s, None) }
                else { leave_submenu(s) }));
    }
    let header = header.child(field);

    let (body, left, action) = match &s.chat {
        Some(run) => chat_body(s, run, dark, &t),
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
            Item::Clipboard | Item::Emoji => "Commands",
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
        list = list.child(task_row(task, i, i == s.picked, t, accent));
    }
    if tasks.is_empty() {
        list = list.child(col().h(120.0).w_full().center()
            .child(text("Nothing on today's list. Type a todo and press Enter").font(ty::body()).color(t.text_secondary)));
    }
    let body = col().w_full().grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).child(list);
    let (done, total) = s.todo_counts;
    let left = s.status.clone().unwrap_or_else(|| {
        let date = s.todo.as_ref().map(|d| d.date.strftime("%A %-d %B").to_string()).unwrap_or_default();
        let count = if total > 0 && done == total { "All done 🎉".to_string() } else { format!("{done}/{total} done") };
        format!("{count} · {date} · space toggles, del removes")
    });
    let action = if !s.query.text.trim().is_empty() { ("Add Todo", "↵") }
        else { match tasks.get(s.picked) { Some(t) if t.done => ("Mark Open", "↵"), Some(_) => ("Mark Done", "↵"), None => ("", "↵") } };
    (body, left, action)
}

fn task_row(task: &todo::Task, i: usize, on: bool, t: &tok::Theme, accent: Color) -> El {
    let fg = match (on, task.done) {
        (true, false) => Color::WHITE,
        (true, true) => Color::WHITE.alpha(0.7),
        (false, false) => t.text,
        (false, true) => t.text_secondary,
    };
    // nf-md-checkbox_marked / checkbox_blank_outline
    let (glyph, glyph_color) = if task.done { ("\u{f0132}", if on { Color::WHITE } else { accent }) }
        else { ("\u{f0131}", fg) };
    row().key(format!("item-{i}")).h(ty::px(40.0)).px(10.0).gap(10.0).radius(8.0).center_y()
        .fill(if on { accent.alpha(0.85) } else { Color::TRANSPARENT })
        .child(text(glyph).font(rune::TextStyle { size: ty::px(20.0), ..ty::body().lh(ty::px(22.0)).family(NERD_FONT) }).color(glyph_color))
        .child(text(task.text.clone()).font(ty::body()).color(fg).truncate().grow(1.0).min_w(0.0))
        .on_hover(move |s: &mut Tomboy| s.picked = i)
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
    let action = if picked.is_some() { "Copy to Clipboard" } else { "" };
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
    let action = if picked_glyph(s).is_some() { "Copy Emoji" } else { "" };
    (body, left, (action, "↵"))
}

/// The agent conversation, streaming: prompts, Markdown answers, tool calls.
fn chat_body(s: &Tomboy, run: &agent::Run, dark: bool, t: &tok::Theme) -> (El, String, (&'static str, &'static str)) {
    let tr = run.lock();
    let md = ty::markdown(dark);
    let running = tr.is_running();
    let mut log = col().key("transcript").w_full().px(20.0).py(14.0).gap(10.0);
    for (i, block) in tr.blocks.iter().enumerate() {
        let last = i + 1 == tr.blocks.len();
        log = log.child(match block {
            Block::Prompt(p) => row().w_full().gap(10.0).padding(10.0).radius(9.0).fill(t.control_off)
                .child(text("›").font(ty::body()).color(AGENT_COLOR))
                .child(text(p.clone()).font(ty::body()).color(t.text).wrap().grow(1.0).min_w(0.0)),
            Block::Text(md_src) => col().w_full()
                .child(Markdown::new(md_src.trim()).streaming(running && last).render(&md)),
            Block::Tool(summary) => row().w_full().gap(8.0).center_y()
                .child(text("⚙").font(ty::label()).color(t.text_secondary))
                .child(text(summary.clone()).font(ty::label().family(Family::Mono)).color(t.text_secondary).truncate()
                    .grow(1.0).min_w(0.0)),
            Block::Error(e) => text(e.clone()).font(ty::body()).color(Color { r: 0.95, g: 0.35, b: 0.35, a: 1.0 }).wrap(),
        });
    }
    if running && !matches!(tr.blocks.last(), Some(Block::Text(_))) {
        log = log.child(text("Thinking…").font(ty::label()).color(t.text_secondary));
    }
    let body = col().w_full().grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: true }).child(log);
    let state = match &tr.status {
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

fn keycap(label: &str, t: &tok::Theme) -> El {
    row().h(ty::px(20.0)).min_w(ty::px(22.0)).px(6.0).radius(5.0).center().fill(t.control_off)
        .child(text(label.to_string()).font(ty::label()).color(t.text))
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
        Item::WebSearch(q) => (glyph("⌕", Color { r: 0.45, g: 0.45, b: 0.5, a: 1.0 }),
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
        (true, "Reply to Jonas about the offsite"), (true, "Ship tomboy 0.2"), (false, "Review PR #42"),
        (false, "Buy milk"), (false, "Book the dentist"), (true, "Pay the electricity bill"), (false, "Call mum"),
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
        _ => {}
    }
    rune::set_app_id("tomboy");
    rune::prewarm_gpu();
    let spec = AppSpec {
        min_size: Some((WIDTH, HEIGHT)),
        // Code in agent answers uses the system monospace font (fontconfig's `monospace`).
        families: ("Inter", "ui-monospace"),
        setup: Some(setup),
        poll: Some(poll),
        scenes: &[
            ("apps", |s| s.query = TextEdit::new("fi")),
            ("calc", |s| s.query = TextEdit::new("(2+2)*10/3")),
            ("url", |s| s.query = TextEdit::new("github.com/anomalyco/rune")),
            ("web", |s| s.query = TextEdit::new("rust rune ui")),
            ("omarchy", |s| s.query = TextEdit::new("bar")),
            ("chat", chat_scene),
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
            ("todo-command", |s| { s.todo_counts = (3, 7); s.query = TextEdit::new("todo"); }),
            ("todo-add", |s| { s.todo_counts = (3, 7); s.query = TextEdit::new("todo buy milk"); }),
        ],
        ..AppSpec::new("tomboy", (WIDTH, HEIGHT), init, view)
    };
    App::new(spec)
        .window_options(WindowOptions { borderless: true, resizable: false, max_size: Some((WIDTH, HEIGHT)), ..Default::default() })
        .float_on_open()
        // Clicking away closes the launcher, but not while a conversation is open.
        .on_focus(|s, focused, tx| if !focused && s.chat.is_none() { tx.quit() })
        .run();
}

fn exit_on_error(r: std::io::Result<()>) -> ! {
    match r {
        Ok(()) => std::process::exit(0),
        Err(e) => { eprintln!("tomboy: {e}"); std::process::exit(1) }
    }
}
