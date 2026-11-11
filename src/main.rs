//! tomboy — a personal Raycast-style launcher built with Rune.
//!
//! Type to search installed applications, evaluate math (`2+2`), or open a URL.
//! ↑/↓ pick, Enter runs, Esc (or clicking away) closes.
//!
//! "Ask agent" (or dictating with Superwhisper) runs the coding agent headless and streams
//! its answer into tomboy; the field then takes follow-ups in the same session.
//!
//! Headless checks (no window):
//!   cargo run -- --snap shot.png --scene calc
//!   cargo run -- --tree tree.json --scene apps

mod agent;
mod apps;
mod calc;
mod dictation;
// Omarchy is Linux-only; on macOS the module compiles but `Platform::Omarchy` never occurs.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod omarchy;
mod platform;
mod sys;
mod url;

use rune::tok::{self, type_};
use rune::{
    App, AppSpec, Color, Cx, El, Family, Image, ImageCache, Key, KeyEvent, ScrollSpec, TextEdit, Tx, WindowOptions,
    Waker, col, el, row, spacer, text, text_input,
};
use rune_kit::{Markdown, MarkdownStyle};

use std::sync::{Arc, Mutex};

use agent::{Block, Status};
use apps::AppEntry;
use omarchy::{Conditions, MenuItem};
use platform::Platform;

const WIDTH: f32 = 720.0;
const HEIGHT: f32 = 440.0;
/// Apps listed for an empty query, and per search.
const MAX_APPS: usize = 30;
const MAX_APP_HITS: usize = 8;
const MAX_MENU_HITS: usize = 8;
const ICON: f32 = 28.0;
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
}

/// One row in the result list.
#[derive(Clone, Debug, PartialEq)]
enum Item {
    Calc { expr: String, value: String },
    Url(String),
    App(usize),
    Menu(usize),
    WebSearch(String),
    Agent(String),
}

impl Item {
    fn action(&self, s: &Tomboy) -> &'static str {
        match self {
            Item::Calc { .. } => "Copy Answer",
            Item::Url(_) => "Open in Browser",
            Item::App(_) => "Open Application",
            Item::Menu(i) if s.menu[*i].is_submenu() => "Open Menu",
            Item::Menu(_) => "Run Command",
            Item::WebSearch(_) => "Search the Web",
            Item::Agent(_) => "Ask Agent",
        }
    }
}

/// What tomboy does after running an item.
enum After {
    Quit,
    Stay,
}

fn init() -> Tomboy {
    let mut query = TextEdit::new("");
    query.focused = true;
    let apps = apps::load_apps();
    let platform = platform::detect();
    let menu = match &platform {
        Platform::Omarchy { root } => omarchy::load(root),
        Platform::Linux | Platform::MacOS => Vec::new(),
    };
    Tomboy {
        query, picked: 0, icons: vec![None; apps.len()], apps, cache: None,
        platform, menu, conds: Conditions::default(), status: None,
        last_text: String::new(), dictated: Arc::default(), waker: None, chat: None,
    }
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
    s.waker = Some(waker);
}

/// After every edit: if one change inserted a burst of text, check on a thread whether it
/// was a Superwhisper dictation (the history entry can land just after the text does).
fn on_change(s: &mut Tomboy, _: &mut Tx) {
    s.picked = 0;
    s.status = None;
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
    let mut items = Vec::new();
    if let Some(v) = calc::eval(q) {
        items.push(Item::Calc { expr: q.to_string(), value: calc::format(v) });
    }
    if let Some(u) = url::detect(q) {
        items.push(Item::Url(u));
    }
    let app_limit = if q.is_empty() { MAX_APPS } else { MAX_APP_HITS };
    items.extend(apps::search(&s.apps, q, app_limit).into_iter().map(Item::App));
    items.extend(omarchy::search(&s.menu, &s.conds, q, MAX_MENU_HITS).into_iter().map(Item::Menu));
    if !q.is_empty() && !matches!(items.first(), Some(Item::Calc { .. } | Item::Url(_))) {
        items.push(Item::Agent(q.to_string()));
        items.push(Item::WebSearch(q.to_string()));
    }
    items
}

fn run_item(s: &mut Tomboy, item: &Item, tx: &mut Tx) {
    let outcome = match item {
        Item::Calc { value, .. } => copy_text(value, tx).map(|()| {
            s.status = Some(format!("Copied {value}"));
            After::Stay
        }),
        Item::Url(u) => url::open(u).map(|()| After::Quit),
        Item::App(i) => apps::launch(&s.apps[*i]).map(|()| After::Quit),
        Item::Menu(i) => match &s.platform {
            Platform::Omarchy { root } => omarchy::run(&s.menu[*i], root).map(|()| After::Quit),
            Platform::Linux | Platform::MacOS => Ok(After::Stay),
        },
        Item::WebSearch(q) => url::open(&format!("https://duckduckgo.com/?q={}", encode(q))).map(|()| After::Quit),
        Item::Agent(prompt) => ask_agent(s, prompt),
    };
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
            let name = agent::name(&s.platform);
            if !agent::supports_headless(&name) {
                return agent::ask(prompt, &s.platform).map(|()| After::Quit);
            }
            agent::Run::new(name)
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
    let n = results(s).len();
    match ev.key {
        Key::Down if n > 0 => s.picked = (s.picked + 1) % n,
        Key::Up if n > 0 => s.picked = (s.picked + n - 1) % n,
        Key::Escape => { quit(s, tx); return true; }
        _ => return false,
    }
    tx.scroll_into_view(format!("item-{}", s.picked));
    true
}

fn view(s: &Tomboy, cx: &mut Cx<Tomboy>) -> El {
    let dark = cx.system_dark();
    let t = tok::theme(dark);
    let accent = tok::accent(0, dark);

    let placeholder = if s.chat.is_some() { "Ask a follow-up…" } else { "Search apps, calculate, or paste a URL…" };
    let field = text_input(&s.query, type_::TITLE.lh(28.0)).key("query").single_line().grow(1.0)
        .color(t.text).placeholder(placeholder, t.text_secondary)
        .on_edit(|s: &mut Tomboy| &mut s.query)
        .on_input_key(on_input_key)
        .on_input_change(on_change)
        .on_submit(submit);
    let glyph = if s.chat.is_some() {
        text(AGENT_GLYPH).font(type_::TITLE.lh(28.0).family(NERD_FONT)).color(AGENT_COLOR)
    } else {
        text("⌕").font(type_::TITLE.lh(28.0)).color(t.text_secondary)
    };
    let header = row().h(60.0).px(20.0).gap(14.0).center_y()
        .child(glyph)
        .child(field);

    let (body, left, action) = match &s.chat {
        Some(run) => chat_body(s, run, dark, &t),
        None => results_body(s, &t, accent),
    };
    let footer = row().h(40.0).px(16.0).gap(10.0).center_y().fill(t.bg)
        .child(text(left).font(type_::LABEL).color(t.text_secondary).truncate().grow(1.0).min_w(0.0))
        .child(text(action.0).font(type_::LABEL).color(t.text))
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
            Item::Menu(_) => s.platform.name(),
            Item::WebSearch(_) | Item::Agent(_) => "Fallback",
            _ => "Results",
        };
        if section != Some(name) {
            section = Some(name);
            list = list.child(row().h(28.0).px(12.0).center_y()
                .child(text(name).font(type_::LABEL).color(t.text_secondary)));
        }
        list = list.child(item_row(s, item, i, i == s.picked, t, accent));
    }
    if items.is_empty() {
        list = list.child(col().h(120.0).w_full().center()
            .child(text("No results").font(type_::BODY).color(t.text_secondary)));
    }
    let body = col().w_full().grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).child(list);
    let action = items.get(s.picked).map(|it| it.action(s)).unwrap_or("");
    let left = s.status.clone().unwrap_or_else(|| format!("tomboy · {}", s.platform.name()));
    (body, left, (action, "↵"))
}

/// The agent conversation, streaming: prompts, Markdown answers, tool calls.
fn chat_body(s: &Tomboy, run: &agent::Run, dark: bool, t: &tok::Theme) -> (El, String, (&'static str, &'static str)) {
    let tr = run.lock();
    let md = MarkdownStyle::neutral(dark);
    let running = tr.is_running();
    let mut log = col().key("transcript").w_full().px(20.0).py(14.0).gap(10.0);
    for (i, block) in tr.blocks.iter().enumerate() {
        let last = i + 1 == tr.blocks.len();
        log = log.child(match block {
            Block::Prompt(p) => row().w_full().gap(10.0).padding(10.0).radius(9.0).fill(t.control_off)
                .child(text("›").font(type_::BODY).color(AGENT_COLOR))
                .child(text(p.clone()).font(type_::BODY).color(t.text).wrap().grow(1.0).min_w(0.0)),
            Block::Text(md_src) => col().w_full()
                .child(Markdown::new(md_src.trim()).streaming(running && last).render(&md)),
            Block::Tool(summary) => row().w_full().gap(8.0).center_y()
                .child(text("⚙").font(type_::LABEL).color(t.text_secondary))
                .child(text(summary.clone()).font(type_::LABEL.family(Family::Mono)).color(t.text_secondary).truncate()
                    .grow(1.0).min_w(0.0)),
            Block::Error(e) => text(e.clone()).font(type_::BODY).color(Color { r: 0.95, g: 0.35, b: 0.35, a: 1.0 }).wrap(),
        });
    }
    if running && !matches!(tr.blocks.last(), Some(Block::Text(_))) {
        log = log.child(text("Thinking…").font(type_::LABEL).color(t.text_secondary));
    }
    let body = col().w_full().grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: true }).child(log);
    let state = match &tr.status {
        Status::Running => "Running… (Esc to stop)".to_string(),
        Status::Done { secs, cost_usd: Some(c) } => format!("Done in {secs:.1}s · ${c:.3}"),
        Status::Done { secs, cost_usd: None } => format!("Done in {secs:.1}s"),
        Status::Failed(e) => format!("Failed: {e}"),
    };
    let left = s.status.clone().unwrap_or_else(|| format!("{} · {state}", tr.agent));
    let action = if running { ("Stop", "esc") }
        else if s.query.text.trim().is_empty() { ("Copy Answer", "↵") }
        else { ("Send Follow-up", "↵") };
    (body, left, action)
}

fn keycap(label: &str, t: &tok::Theme) -> El {
    row().h(20.0).min_w(22.0).px(6.0).radius(5.0).center().fill(t.control_off)
        .child(text(label.to_string()).font(type_::LABEL).color(t.text))
}

fn item_row(s: &Tomboy, item: &Item, i: usize, on: bool, t: &tok::Theme, accent: Color) -> El {
    let glyph_in = |g: &str, c: Color, family: Option<Family>| row().size(ICON, ICON).radius(7.0).center().fill(c)
        .child(text(g.to_string()).font(family.map_or(type_::BODY, |f| type_::BODY.lh(18.0).family(f))).color(Color::WHITE));
    let glyph = |g: &str, c: Color| glyph_in(g, c, None);
    let (icon, title, subtitle, kind): (El, String, Option<String>, &str) = match item {
        Item::Calc { expr, value } => (glyph("=", Color { r: 1.0, g: 0.58, b: 0.0, a: 1.0 }),
            value.clone(), Some(expr.clone()), "Calculator"),
        Item::Url(u) => (glyph("↗", Color { r: 0.2, g: 0.5, b: 1.0, a: 1.0 }), u.clone(), None, "Open URL"),
        Item::Agent(q) => (glyph_in(AGENT_GLYPH, AGENT_COLOR, Some(NERD_FONT)), format!("Ask agent: “{q}”"), None, "Agent"),
        Item::WebSearch(q) => (glyph("⌕", Color { r: 0.45, g: 0.45, b: 0.5, a: 1.0 }),
            format!("Search the web for “{q}”"), None, "DuckDuckGo"),
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
            (icon, title, sub, if entry.is_submenu() { "Submenu" } else { "Command" })
        }
    };
    let fg = if on { Color::WHITE } else { t.text };
    let fg2 = if on { Color::WHITE.alpha(0.75) } else { t.text_secondary };
    let mut r = row().key(format!("item-{i}")).h(46.0).px(12.0).gap(12.0).radius(9.0).center_y()
        .fill(if on { accent.alpha(0.85) } else { Color::TRANSPARENT })
        .child(icon)
        .child(text(title).font(type_::BODY).color(fg).truncate());
    if let Some(sub) = subtitle {
        r = r.child(text(sub).font(type_::LABEL).color(fg2).truncate().grow(1.0).min_w(0.0));
    } else {
        r = r.child(spacer());
    }
    let item = item.clone();
    r.child(text(kind).font(type_::LABEL).color(fg2))
        .on_hover(move |s: &mut Tomboy| s.picked = i)
        .on_press_tx(move |s: &mut Tomboy, tx: &mut Tx| { s.picked = i; run_item(s, &item, tx); })
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

fn main() {
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
