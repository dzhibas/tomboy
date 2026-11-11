//! Omarchy's menu (`omarchy-menu.jsonc`) as searchable tomboy commands.
//!
//! The menu is a flat JSONC object of dotted ids (`appearance.top-bar.hide`) whose parents are
//! implied by the dots. Omarchy's defaults live in `$OMARCHY_PATH/default/omarchy/`, and the
//! user's `~/.config/omarchy/extensions/omarchy-menu.jsonc` adds entries or overrides fields of
//! existing ids. Rows carry shell conditions (`when`, `disabled`, `checked`) which run on a
//! background pool; a row with a condition stays hidden until its check passes.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

use serde_json::{Map, Value};

#[derive(Debug, Clone, Default)]
pub struct MenuItem {
    pub id: String,
    pub label: String,
    pub icon: Option<String>,
    /// Parent labels, e.g. `["Appearance", "Menu Bar"]`.
    pub parents: Vec<String>,
    /// Shell command; `None` makes the row open the Omarchy menu at this submenu.
    pub action: Option<String>,
    pub aliases: Vec<String>,
    pub description: Option<String>,
    pub when: Option<String>,
    pub disabled: Option<String>,
    pub checked: Option<String>,
}

impl MenuItem {
    pub fn breadcrumb(&self) -> String {
        self.parents.join(" › ")
    }
    pub fn is_submenu(&self) -> bool {
        self.action.is_none()
    }
    fn has_visibility_check(&self) -> bool {
        self.when.is_some() || self.disabled.is_some()
    }
}

/// Load Omarchy's default menu with the user's extensions merged over it.
pub fn load(root: &Path) -> Vec<MenuItem> {
    let mut merged: Map<String, Value> = Map::new();
    let user = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|c| c.join("omarchy/extensions/omarchy-menu.jsonc"));
    for path in [Some(root.join("default/omarchy/omarchy-menu.jsonc")), user].into_iter().flatten() {
        let Ok(src) = std::fs::read_to_string(&path) else { continue };
        match serde_json::from_str::<Map<String, Value>>(&strip_jsonc(&src)) {
            Ok(obj) => for (id, fields) in obj {
                let Value::Object(fields) = fields else { continue };
                match merged.get_mut(&id) {
                    Some(Value::Object(existing)) => existing.extend(fields),
                    _ => { merged.insert(id, Value::Object(fields)); }
                }
            },
            Err(e) => eprintln!("tomboy: can't parse {}: {e}", path.display()),
        }
    }
    items_from(&merged)
}

fn items_from(menu: &Map<String, Value>) -> Vec<MenuItem> {
    let label_of = |id: &str| menu.get(id).and_then(|v| v.get("label")).and_then(Value::as_str)
        .map(str::to_string).unwrap_or_else(|| id.rsplit('.').next().unwrap_or(id).to_string());
    let str_of = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).filter(|s| !s.trim().is_empty()).map(str::to_string);
    menu.iter().filter_map(|(id, v)| {
        // tomboy already lists applications itself, and other providers fill rows at runtime
        // inside the shell; their submenus still open through `omarchy menu summon`.
        if str_of(v, "provider").as_deref() == Some("apps") { return None; }
        let parts: Vec<&str> = id.split('.').collect();
        let parents = (1..parts.len()).map(|n| label_of(&parts[..n].join("."))).collect();
        Some(MenuItem {
            id: id.clone(),
            label: label_of(id),
            icon: str_of(v, "icon"),
            parents,
            action: str_of(v, "action"),
            aliases: v.get("aliases").and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default(),
            description: str_of(v, "description"),
            when: str_of(v, "when"),
            disabled: str_of(v, "disabled"),
            checked: str_of(v, "checked"),
        })
    }).collect()
}

/// JSONC → JSON: drop `//` and `/* */` comments and trailing commas, leaving strings intact.
pub fn strip_jsonc(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => {
                let start = i;
                i += 1;
                while i < b.len() && b[i] != b'"' { i += if b[i] == b'\\' { 2 } else { 1 }; }
                i = (i + 1).min(b.len());
                out.extend_from_slice(&b[start..i]);
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' { i += 1; }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') { i += 1; }
                i += 2;
            }
            b',' => {
                let mut j = i + 1;
                // Look past whitespace and comments for a closing bracket.
                loop {
                    while j < b.len() && b[j].is_ascii_whitespace() { j += 1; }
                    if b.get(j) == Some(&b'/') && b.get(j + 1) == Some(&b'/') {
                        while j < b.len() && b[j] != b'\n' { j += 1; }
                    } else if b.get(j) == Some(&b'/') && b.get(j + 1) == Some(&b'*') {
                        j += 2;
                        while j + 1 < b.len() && !(b[j] == b'*' && b[j + 1] == b'/') { j += 1; }
                        j += 2;
                    } else { break; }
                }
                if !matches!(b.get(j), Some(b'}' | b']')) { out.push(b','); }
                i += 1;
            }
            c => { out.push(c); i += 1; }
        }
    }
    String::from_utf8(out).unwrap_or_default()
}

// ---- Conditions --------------------------------------------------------------------------

const PENDING: u8 = 0;
const SHOWN: u8 = 1;
const HIDDEN: u8 = 2;
const CHECKED: u8 = 4;

/// Per-row results of the shell conditions, filled in by a background pool.
#[derive(Clone, Default)]
pub struct Conditions(Arc<Vec<AtomicU8>>);

impl Conditions {
    pub fn visible(&self, i: usize) -> bool {
        self.0.get(i).is_some_and(|s| s.load(Ordering::Relaxed) & SHOWN != 0)
    }
    pub fn checked(&self, i: usize) -> bool {
        self.0.get(i).is_some_and(|s| s.load(Ordering::Relaxed) & CHECKED != 0)
    }
}

/// Rows without conditions are visible at once; the rest are evaluated on a few threads,
/// calling `wake` as results land so the window redraws.
pub fn evaluate(items: &[MenuItem], root: &Path, wake: impl Fn() + Send + Sync + 'static) -> Conditions {
    let states: Vec<AtomicU8> = items.iter()
        .map(|it| AtomicU8::new(if it.has_visibility_check() || it.checked.is_some() { PENDING } else { SHOWN }))
        .collect();
    let conds = Conditions(Arc::new(states));
    let jobs: Arc<Vec<(usize, MenuItem)>> = Arc::new(items.iter().cloned().enumerate()
        .filter(|(_, it)| it.has_visibility_check() || it.checked.is_some()).collect());
    if jobs.is_empty() { return conds; }
    let next = Arc::new(AtomicUsize::new(0));
    let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
    let path = shell_path(root);
    for _ in 0..8.min(jobs.len()) {
        let (jobs, next, conds, wake, path) = (jobs.clone(), next.clone(), conds.clone(), wake.clone(), path.clone());
        std::thread::spawn(move || loop {
            let n = next.fetch_add(1, Ordering::Relaxed);
            let Some((i, it)) = jobs.get(n) else { break };
            let shown = it.when.as_deref().is_none_or(|c| check(c, &path))
                && !it.disabled.as_deref().is_some_and(|c| check(c, &path));
            let mut state = if shown { SHOWN } else { HIDDEN };
            if shown && it.checked.as_deref().is_some_and(|c| check(c, &path)) { state |= CHECKED; }
            conds.0[*i].store(state, Ordering::Relaxed);
            if shown { wake(); }
        });
    }
    conds
}

/// PATH with Omarchy's bin directory first, since conditions and actions call its commands.
fn shell_path(root: &Path) -> String {
    let bin = root.join("bin");
    match std::env::var("PATH") {
        Ok(p) => format!("{}:{p}", bin.display()),
        Err(_) => bin.display().to_string(),
    }
}

fn check(cond: &str, path: &str) -> bool {
    Command::new("bash").arg("-c").arg(cond).env("PATH", path)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .status().is_ok_and(|s| s.success())
}

// ---- Running -----------------------------------------------------------------------------

/// Run a row's command detached, or open the Omarchy menu at a submenu.
pub fn run(item: &MenuItem, root: &Path) -> std::io::Result<()> {
    let mut cmd = Command::new("bash");
    match &item.action {
        Some(action) => cmd.arg("-c").arg(action),
        None => cmd.arg("-c").arg(format!("omarchy-menu summon {}", item.id)),
    };
    cmd.env("PATH", shell_path(root));
    if let Some(home) = crate::sys::home_dir() { cmd.current_dir(home); }
    crate::sys::spawn_detached(&mut cmd)
}

// ---- Search ------------------------------------------------------------------------------

/// Rank visible rows against `query` (empty matches nothing); indices into `items`, best first.
/// With `scope`, only rows under that submenu id count.
pub fn search(items: &[MenuItem], conds: &Conditions, query: &str, scope: Option<&str>, limit: usize) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() { return Vec::new(); }
    let prefix = scope.map(|id| format!("{id}."));
    let mut scored: Vec<(u32, usize, usize)> = items.iter().enumerate()
        .filter(|(i, it)| conds.visible(*i) && prefix.as_deref().is_none_or(|p| it.id.starts_with(p)))
        .filter_map(|(i, it)| score(it, &q).map(|s| (s, it.parents.len(), i)))
        .collect();
    // Higher score first; shallower rows (Lock over Appearance › … › Lock) win ties.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    scored.into_iter().take(limit).map(|(_, _, i)| i).collect()
}

fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

fn score(it: &MenuItem, q: &str) -> Option<u32> {
    let label = it.label.to_lowercase();
    if label == q { return Some(1000); }
    if label.starts_with(q) { return Some(900); }
    let label_words = words(&label);
    if label_words.iter().any(|w| w.starts_with(q)) { return Some(800); }
    let aliases: Vec<String> = it.aliases.iter().map(|a| a.to_lowercase()).collect();
    if aliases.iter().any(|a| a.starts_with(q)) { return Some(750); }
    if label.contains(q) { return Some(700); }
    // Every query word starts a word of the breadcrumb + label: "bar hide", "theme font".
    let mut all: Vec<String> = it.parents.iter().flat_map(|p| words(p)).collect();
    all.extend(label_words);
    let qw: Vec<&str> = q.split_whitespace().collect();
    if qw.iter().all(|w| all.iter().any(|a| a.starts_with(w))) { return Some(600); }
    if it.description.as_deref().is_some_and(|d| d.to_lowercase().contains(q)) { return Some(400); }
    None
}

fn parent_id(id: &str) -> &str {
    id.rsplit_once('.').map_or("", |(p, _)| p)
}

/// The visible rows directly under submenu `parent`, in menu order.
pub fn children_of(items: &[MenuItem], conds: &Conditions, parent: &str) -> Vec<usize> {
    items.iter().enumerate()
        .filter(|(i, it)| parent_id(&it.id) == parent && conds.visible(*i))
        .map(|(i, _)| i).collect()
}

/// Whether the menu config lists rows under `id` (provider submenus fill theirs at runtime
/// inside the Omarchy shell, so they have none here).
pub fn has_children(items: &[MenuItem], id: &str) -> bool {
    items.iter().any(|it| parent_id(&it.id) == id)
}

/// The submenu above `id`, `None` at the root.
pub fn parent_of(id: &str) -> Option<&str> {
    id.rsplit_once('.').map(|(p, _)| p)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MENU: &str = r#"{
      // comment with "quotes", and a trailing comma,
      "system": {"icon":"x","label":"System"},
      "system.lock": {"label":"Lock","action":"omarchy-system-lock"}, /* block */
      "appearance": {"label":"Appearance"},
      "appearance.top-bar": {"label":"Menu Bar"},
      "appearance.top-bar.hide": {"label":"Hide Bar","when":"true","action":"echo // not a comment"},
      "apps": {"label":"Apps","provider":"apps"},
    }"#;

    fn items() -> Vec<MenuItem> {
        items_from(&serde_json::from_str(&strip_jsonc(MENU)).unwrap())
    }

    #[test]
    fn jsonc_strips_comments_and_trailing_commas_but_not_strings() {
        let it = items();
        let hide = it.iter().find(|i| i.id == "appearance.top-bar.hide").unwrap();
        assert_eq!(hide.action.as_deref(), Some("echo // not a comment"));
        assert_eq!(hide.parents, ["Appearance", "Menu Bar"]);
        assert!(it.iter().all(|i| i.id != "apps"), "the apps provider is skipped");
        assert!(it.iter().find(|i| i.id == "system").unwrap().is_submenu());
        let shown = Conditions(Arc::new(it.iter().map(|_| AtomicU8::new(SHOWN)).collect()));
        let ids = |v: Vec<usize>| v.into_iter().map(|i| it[i].id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(children_of(&it, &shown, "system")), ["system.lock"]);
        assert_eq!(ids(children_of(&it, &shown, "")), ["system", "appearance"]);
        assert!(has_children(&it, "appearance.top-bar") && !has_children(&it, "system.lock"));
        assert_eq!(parent_of("appearance.top-bar"), Some("appearance"));
        assert_eq!(parent_of("system"), None);
    }

    #[test]
    fn search_ranks_labels_then_breadcrumbs() {
        let it = items();
        let all_visible = Conditions(Arc::new(it.iter().map(|_| AtomicU8::new(SHOWN)).collect()));
        let ids = |q| search(&it, &all_visible, q, None, 10).into_iter().map(|i| it[i].id.clone()).collect::<Vec<_>>();
        assert_eq!(ids("lock"), ["system.lock"]);
        assert_eq!(ids("bar hide")[0], "appearance.top-bar.hide");
        assert_eq!(ids("hide")[0], "appearance.top-bar.hide");
        assert!(ids("").is_empty());
        let scoped = search(&it, &all_visible, "lock", Some("appearance"), 10);
        assert!(scoped.is_empty(), "Lock is under System, not Appearance");
    }

    #[test]
    fn conditions_hide_rows_until_they_pass() {
        let mut it = items();
        it.push(MenuItem { id: "x.never".into(), label: "Never".into(), when: Some("false".into()), ..Default::default() });
        it.push(MenuItem { id: "x.done".into(), label: "Done".into(), disabled: Some("true".into()), ..Default::default() });
        it.push(MenuItem { id: "x.on".into(), label: "On".into(), checked: Some("true".into()), ..Default::default() });
        let c = evaluate(&it, Path::new("/nonexistent"), || {});
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while c.0.iter().any(|s| s.load(Ordering::Relaxed) == PENDING) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let at = |id: &str| it.iter().position(|i| i.id == id).unwrap();
        assert!(c.visible(at("system.lock")));
        assert!(c.visible(at("appearance.top-bar.hide")));
        assert!(!c.visible(at("x.never")));
        assert!(!c.visible(at("x.done")));
        assert!(c.visible(at("x.on")) && c.checked(at("x.on")));
    }

    #[test]
    fn loads_the_real_menu_when_present() {
        let root = Path::new("/usr/share/omarchy");
        if root.join("default/omarchy/omarchy-menu.jsonc").is_file() {
            let it = load(root);
            assert!(it.len() > 50, "{} rows", it.len());
        }
    }
}
