//! Frecency: what you launch, how often, how recently, and what you typed to launch it, so
//! search ranks your usual pick first (`news` → hackernews, not euronews).
//!
//! Kept in `~/.local/state/tomboy/frecency.json` (`$XDG_STATE_HOME` honoured); delete it to
//! start over, or remove one item's entry to forget it (the agent can, see `skill.md`). Items
//! are keyed `app:<desktop id>`, `menu:<omarchy id>`, `link:<quicklink keyword>` or `emoji:<char>`.
//!
//! [`Store::boost`] is added to a match's tier score (exact 1000, prefix 900, word 800,
//! substring 700, …): up to 150 for frecency, so a habit wins ties and lifts about a tier,
//! and up to 300 when you launched the item before with a query that starts with what you're
//! typing now, so a learned pick can beat a closer name match. It never makes a non-match show.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::sys;

/// Launch times kept per item.
const MAX_VISITS: usize = 10;
/// Queries kept per item (the most used).
const MAX_QUERIES: usize = 20;
/// Items not launched for this long are forgotten.
const FORGET_AFTER: u64 = 180 * DAY;
const DAY: u64 = 24 * 60 * 60;
/// The most frecency (recent use) adds to a score.
const MAX_FRECENCY: u32 = 150;
/// What one earlier launch under a matching query adds, and the most that adds up to.
const QUERY_POINTS: u32 = 60;
const MAX_QUERY: u32 = 300;

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
struct Entry {
    /// Unix seconds, newest last.
    visits: Vec<u64>,
    /// Lowercased, trimmed query → launches under it.
    #[serde(default)]
    queries: BTreeMap<String, u32>,
}

#[derive(Debug, Default)]
pub struct Store {
    path: Option<PathBuf>,
    items: BTreeMap<String, Entry>,
    /// `[search] frecency = false`: no boosts, nothing recorded, the file left as it is.
    off: bool,
}

/// Now, in unix seconds.
pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The key of an app (its desktop file id) and of an Omarchy menu command.
pub fn app_key(id: &str) -> String { format!("app:{id}") }
pub fn menu_key(id: &str) -> String { format!("menu:{id}") }
/// The key of a quicklink, by keyword.
pub fn link_key(keyword: &str) -> String { format!("link:{keyword}") }

impl Store {
    /// The store on disk; empty when there is none yet or it doesn't parse.
    pub fn load() -> Store {
        let path = sys::state_dir().map(|d| d.join("tomboy/frecency.json"));
        let items = path.as_deref().map(read).unwrap_or_default();
        Store { path, items, off: false }
    }

    /// A store that ranks nothing and records nothing (frecency turned off in the config).
    /// The history file stays, so turning it back on picks up where it was.
    pub fn off() -> Store {
        Store { off: true, ..Store::default() }
    }

    /// Points to add to `key`'s match score for `query`, at `now`.
    pub fn boost(&self, key: &str, query: &str, now: u64) -> u32 {
        if self.off { return 0; }
        let Some(e) = self.items.get(key) else { return 0 };
        let weight: u32 = e.visits.iter().map(|&t| age_weight(now.saturating_sub(t))).sum();
        // Ten visits this week (1000) is the cap; one is 15, enough to win a tie.
        let frecency = (weight * 15 / 100).min(MAX_FRECENCY);
        let q = normalize(query);
        let learned = if q.is_empty() { 0 } else {
            let times: u32 = e.queries.iter().filter(|(k, _)| k.starts_with(&q)).map(|(_, n)| n).sum();
            (times * QUERY_POINTS).min(MAX_QUERY)
        };
        frecency + learned
    }

    /// Note a launch of `key` typed as `query`, and save. The file is read again first, so an
    /// edit made while tomboy was open (the agent forgetting an item) isn't written back over.
    pub fn record(&mut self, key: &str, query: &str, now: u64) {
        if self.off { return; }
        if let Some(path) = &self.path { self.items = read(path); }
        let e = self.items.entry(key.to_string()).or_default();
        e.visits.push(now);
        let extra = e.visits.len().saturating_sub(MAX_VISITS);
        e.visits.drain(..extra);
        let q = normalize(query);
        if !q.is_empty() {
            *e.queries.entry(q).or_default() += 1;
            if e.queries.len() > MAX_QUERIES {
                let rarest = e.queries.iter().min_by_key(|(_, n)| **n).map(|(k, _)| k.clone());
                if let Some(k) = rarest { e.queries.remove(&k); }
            }
        }
        self.items.retain(|_, e| e.visits.last().is_some_and(|&t| now.saturating_sub(t) < FORGET_AFTER));
        if let Some(path) = &self.path {
            if let Err(e) = write(path, &self.items) { eprintln!("tomboy: saving {}: {e}", path.display()); }
        }
    }
}

/// Firefox's frecency buckets: a visit counts less the older it is.
fn age_weight(age: u64) -> u32 {
    match age {
        a if a < 4 * DAY => 100,
        a if a < 14 * DAY => 70,
        a if a < 31 * DAY => 50,
        a if a < 90 * DAY => 30,
        _ => 10,
    }
}

fn normalize(query: &str) -> String {
    query.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn read(path: &Path) -> BTreeMap<String, Entry> {
    std::fs::read_to_string(path).ok().and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or_default()
}

fn write(path: &Path, items: &BTreeMap<String, Entry>) -> std::io::Result<()> {
    if let Some(dir) = path.parent() { std::fs::create_dir_all(dir)?; }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(items)? + "\n")?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000;

    fn store() -> Store { Store::default() }

    #[test]
    fn recent_visits_count_more() {
        let mut s = store();
        s.record("app:a", "", NOW - 2 * DAY);
        s.record("app:b", "", NOW - 60 * DAY);
        assert_eq!(s.boost("app:a", "", NOW), 15);
        assert_eq!(s.boost("app:b", "", NOW), 4);
        assert_eq!(s.boost("app:none", "", NOW), 0);
        for _ in 0..20 { s.record("app:c", "", NOW); }
        assert_eq!(s.items["app:c"].visits.len(), MAX_VISITS);
        assert_eq!(s.boost("app:c", "", NOW), MAX_FRECENCY);
    }

    #[test]
    fn learns_the_query() {
        let mut s = store();
        s.record("app:hackernews.desktop", "News ", NOW);
        // 15 for the visit, 60 for one launch under a query starting with what's typed.
        assert_eq!(s.boost("app:hackernews.desktop", "news", NOW), 75);
        assert_eq!(s.boost("app:hackernews.desktop", "ne", NOW), 75);
        assert_eq!(s.boost("app:hackernews.desktop", "hacker", NOW), 15);
        for _ in 0..10 { s.record("app:hackernews.desktop", "news", NOW); }
        assert_eq!(s.boost("app:hackernews.desktop", "news", NOW), MAX_FRECENCY + MAX_QUERY);
    }

    #[test]
    fn off_ranks_and_records_nothing() {
        let mut s = Store::off();
        s.record("app:a", "a", NOW);
        assert!(s.items.is_empty());
        assert_eq!(s.boost("app:a", "a", NOW), 0);
    }

    #[test]
    fn forgets_old_items_and_rare_queries() {
        let mut s = store();
        s.record("app:old", "x", NOW - 200 * DAY);
        s.record("app:new", "y", NOW);
        assert!(!s.items.contains_key("app:old"));
        for i in 0..=MAX_QUERIES { s.record("app:new", &format!("q{i}"), NOW); }
        assert_eq!(s.items["app:new"].queries.len(), MAX_QUERIES);
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!("tomboy-frecency-{}", std::process::id()));
        let path = dir.join("tomboy/frecency.json");
        let mut s = Store { path: Some(path.clone()), ..Store::default() };
        s.record("menu:system.lock", "lock", NOW);
        let back = read(&path);
        assert_eq!(back, s.items);
        assert!(read(&dir.join("missing.json")).is_empty());
        // An item forgotten on disk while tomboy is open stays forgotten after the next launch.
        s.record("app:hackernews.desktop", "news", NOW);
        std::fs::write(&path, r#"{"menu:system.lock":{"visits":[1800000000],"queries":{"lock":1}}}"#).unwrap();
        s.record("menu:system.lock", "lock", NOW);
        assert!(!read(&path).contains_key("app:hackernews.desktop"));
        assert_eq!(read(&path)["menu:system.lock"].visits.len(), 2);
        std::fs::write(&path, "not json").unwrap();
        assert!(read(&path).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
