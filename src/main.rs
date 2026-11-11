//! tomboy — a personal Raycast-style launcher built with Rune.
//!
//! Type to search installed applications, evaluate math (`2+2`), or open a URL.
//! ↑/↓ pick, Enter runs, Esc (or clicking away) closes.
//!
//! Headless checks (no window):
//!   cargo run -- --snap shot.png --scene calc
//!   cargo run -- --tree tree.json --scene apps

mod apps;
mod calc;
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
}

/// One row in the result list.
#[derive(Clone, Debug, PartialEq)]
enum Item {
    Calc { expr: String, value: String },
    Url(String),
    App(usize),
    Menu(usize),
    WebSearch(String),
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
        }
    }
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
    }
}

/// Start decoding every app's icon on Rune's image worker and checking the menu's shell
/// conditions on a background pool; each result wakes the window.
fn setup(s: &mut Tomboy, waker: Waker) {
    let cache = ImageCache::new(waker.clone());
    s.icons = s.apps.iter().map(|a| a.icon.as_ref().map(|_| cache.app_icon(&a.path, ICON))).collect();
    s.cache = Some(cache);
    if let Platform::Omarchy { root } = &s.platform {
        s.conds = omarchy::evaluate(&s.menu, root, move || waker());
    }
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
        items.push(Item::WebSearch(q.to_string()));
    }
    items
}

fn run_item(s: &mut Tomboy, item: &Item, tx: &mut Tx) {
    let outcome = match item {
        Item::Calc { value, .. } => copy_text(value, tx).map(|()| false),
        Item::Url(u) => url::open(u).map(|()| true),
        Item::App(i) => apps::launch(&s.apps[*i]).map(|()| true),
        Item::Menu(i) => match &s.platform {
            Platform::Omarchy { root } => omarchy::run(&s.menu[*i], root).map(|()| true),
            Platform::Linux | Platform::MacOS => Ok(false),
        },
        Item::WebSearch(q) => url::open(&format!("https://duckduckgo.com/?q={}", encode(q))).map(|()| true),
    };
    match outcome {
        Ok(true) => tx.quit(),
        Ok(false) => s.status = Some(format!("Copied {}", match item { Item::Calc { value, .. } => value.as_str(), _ => "" })),
        Err(e) => s.status = Some(format!("Failed: {e}")),
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
    let items = results(s);
    if let Some(item) = items.get(s.picked).cloned() { run_item(s, &item, tx); }
}

fn on_input_key(s: &mut Tomboy, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let n = results(s).len();
    match ev.key {
        Key::Down if n > 0 => s.picked = (s.picked + 1) % n,
        Key::Up if n > 0 => s.picked = (s.picked + n - 1) % n,
        Key::Escape => { tx.quit(); return true; }
        _ => return false,
    }
    tx.scroll_into_view(format!("item-{}", s.picked));
    true
}

fn view(s: &Tomboy, cx: &mut Cx<Tomboy>) -> El {
    let dark = cx.system_dark();
    let t = tok::theme(dark);
    let accent = tok::accent(0, dark);
    let items = results(s);

    let field = text_input(&s.query, type_::TITLE.lh(28.0)).key("query").single_line().grow(1.0)
        .color(t.text).placeholder("Search apps, calculate, or paste a URL…", t.text_secondary)
        .on_edit(|s: &mut Tomboy| &mut s.query)
        .on_input_key(on_input_key)
        .on_input_change(|s: &mut Tomboy, _| { s.picked = 0; s.status = None; })
        .on_submit(submit);
    let header = row().h(60.0).px(20.0).gap(14.0).center_y()
        .child(text("⌕").font(type_::TITLE.lh(28.0)).color(t.text_secondary))
        .child(field);

    let mut list = col().key("results").w_full().padding(8.0);
    let mut section = None;
    for (i, item) in items.iter().enumerate() {
        let name = match item {
            Item::App(_) => "Applications",
            Item::Menu(_) => s.platform.name(),
            Item::WebSearch(_) => "Fallback",
            _ => "Results",
        };
        if section != Some(name) {
            section = Some(name);
            list = list.child(row().h(28.0).px(12.0).center_y()
                .child(text(name).font(type_::LABEL).color(t.text_secondary)));
        }
        list = list.child(item_row(s, item, i, i == s.picked, &t, accent));
    }
    if items.is_empty() {
        list = list.child(col().h(120.0).w_full().center()
            .child(text("No results").font(type_::BODY).color(t.text_secondary)));
    }
    let body = col().w_full().grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).child(list);

    let action = items.get(s.picked).map(|it| it.action(s)).unwrap_or("");
    let left = s.status.clone().unwrap_or_else(|| format!("tomboy · {}", s.platform.name()));
    let footer = row().h(40.0).px(16.0).gap(10.0).center_y().fill(t.bg)
        .child(text(left).font(type_::LABEL).color(t.text_secondary))
        .child(spacer())
        .child(text(action).font(type_::LABEL).color(t.text))
        .child(keycap("↵", &t));

    col().w_full().h_full().fill(t.card)
        .child(header)
        .child(el().h(1.0).w_full().fill(t.separator))
        .child(body)
        .child(el().h(1.0).w_full().fill(t.separator))
        .child(footer)
}

fn keycap(label: &str, t: &tok::Theme) -> El {
    row().h(20.0).min_w(22.0).px(6.0).radius(5.0).center().fill(t.control_off)
        .child(text(label.to_string()).font(type_::LABEL).color(t.text))
}

fn item_row(s: &Tomboy, item: &Item, i: usize, on: bool, t: &tok::Theme, accent: Color) -> El {
    let glyph = |g: &str, c: Color| row().size(ICON, ICON).radius(7.0).center().fill(c)
        .child(text(g.to_string()).font(type_::BODY).color(Color::WHITE));
    let (icon, title, subtitle, kind): (El, String, Option<String>, &str) = match item {
        Item::Calc { expr, value } => (glyph("=", Color { r: 1.0, g: 0.58, b: 0.0, a: 1.0 }),
            value.clone(), Some(expr.clone()), "Calculator"),
        Item::Url(u) => (glyph("↗", Color { r: 0.2, g: 0.5, b: 1.0, a: 1.0 }), u.clone(), None, "Open URL"),
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

fn main() {
    rune::set_app_id("tomboy");
    rune::prewarm_gpu();
    let spec = AppSpec {
        min_size: Some((WIDTH, HEIGHT)),
        setup: Some(setup),
        scenes: &[
            ("apps", |s| s.query = TextEdit::new("fi")),
            ("calc", |s| s.query = TextEdit::new("(2+2)*10/3")),
            ("url", |s| s.query = TextEdit::new("github.com/anomalyco/rune")),
            ("web", |s| s.query = TextEdit::new("rust rune ui")),
            ("omarchy", |s| s.query = TextEdit::new("bar")),
        ],
        ..AppSpec::new("tomboy", (WIDTH, HEIGHT), init, view)
    };
    App::new(spec)
        .window_options(WindowOptions { borderless: true, resizable: false, max_size: Some((WIDTH, HEIGHT)), ..Default::default() })
        .float_on_open()
        .on_focus(|_, focused, tx| if !focused { tx.quit() })
        .run();
}
