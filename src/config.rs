//! tomboy's optional settings, from `~/.config/tomboy/config.toml` (`$XDG_CONFIG_HOME` honoured).
//! No file means defaults; a setting left out means its default. Read once at launch, so an
//! edit takes effect the next time tomboy opens.
//!
//! ```toml
//! [agent]
//! # The agent CLI to use. Default: Omarchy's default agent, else $TOMBOY_AGENT, else claude.
//! name = "codex"
//! # Where "Ask agent" runs the coding agent. Default: ~/Work, or ~ when that doesn't exist.
//! working_directory = "~/sites"
//!
//! [agent.models]
//! # The model each agent uses, by agent command. Default: the agent's own default.
//! claude = "haiku"
//!
//! [search]
//! # Rank by what you launch, how often and what you typed (frecency). Default true.
//! frecency = false
//!
//! [ui]
//! # Text size in points for list rows; titles and labels scale with it. Default 13.
//! font_size = 15
//!
//! [quicklinks]
//! # keyword = URL; `{a}` (any `{name}`) is replaced by what you type after the keyword.
//! gh = "https://github.com/search?q={query}"
//! jira = { url = "issues.jira.com/browse/{a}", name = "Jira issue" }
//!
//! [aliases]
//! # Typing the alias puts the target's best match first.
//! bubu = "chromium"
//!
//! [todo]
//! # Where the daily todo lists (2026-11-12.md) live. Default ~/Documents/tomboy/todo.
//! directory = "~/notes/todo"
//!
//! [hotkeys]
//! # macOS (`tomboy daemon`): global shortcuts; "" turns one off. Defaults below.
//! launcher = "alt+space"
//! clipboard = "cmd+shift+c"
//!
//! [clipboard]
//! # macOS: Enter on a clip or emoji also pastes it into the app in front. Default true.
//! paste = false
//!
//! [extensions]
//! # Where "Install Extension" lists extensions from. Default tomboy's GitHub repo.
//! registry = "https://raw.githubusercontent.com/dzhibas/tomboy/main"
//!
//! [extensions.hackernews]
//! # An extension's preferences, by the ids in its tomboy.toml.
//! feed = "show_hn"
//! ```
//!
//! Every option is documented for the agent in `src/skill.md`; keep the two in step.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::sys;

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub agent: AgentConfig,
    pub ui: UiConfig,
    pub search: SearchConfig,
    /// Keyword → URL pattern (`crate::quicklinks`).
    pub quicklinks: BTreeMap<String, QuicklinkConfig>,
    /// What you type → what it stands for (`bubu = "chromium"`).
    pub aliases: BTreeMap<String, String>,
    pub todo: TodoConfig,
    pub hotkeys: HotkeysConfig,
    pub clipboard: ClipboardConfig,
    pub extensions: ExtensionsConfig,
}

/// `[extensions]`: where the store reads from, and each extension's preference values as
/// `[extensions.<id>]` tables.
#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ExtensionsConfig {
    /// The registry's base URL (or a local checkout); default tomboy's GitHub repo.
    pub registry: Option<String>,
    #[serde(flatten)]
    pub prefs: BTreeMap<String, toml::Table>,
}

/// Global shortcuts for the resident launcher (`tomboy daemon`, macOS). Unset is the
/// default chord; `""` registers none.
#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct HotkeysConfig {
    pub launcher: Option<String>,
    pub clipboard: Option<String>,
    pub emoji: Option<String>,
    pub todo: Option<String>,
}

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ClipboardConfig {
    /// After picking a clip or emoji, paste it into the app in front (macOS daemon); default on.
    pub paste: Option<bool>,
}

/// What each `tomboy daemon` shortcut opens, with its default chord.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hotkey { Launcher, Clipboard, Emoji, Todo }

#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // the daemon is macOS-only
impl Hotkey {
    pub const ALL: [Hotkey; 4] = [Hotkey::Launcher, Hotkey::Clipboard, Hotkey::Emoji, Hotkey::Todo];
    pub fn default_chord(self) -> &'static str {
        match self { Hotkey::Launcher => "alt+space", Hotkey::Clipboard => "cmd+shift+c", Hotkey::Emoji => "ctrl+alt+e", Hotkey::Todo => "ctrl+alt+t" }
    }
}

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct TodoConfig {
    /// Where the daily lists live; `~` expands to $HOME.
    pub directory: Option<String>,
}

/// A `[quicklinks]` entry: just the URL, or a table with a display name.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum QuicklinkConfig {
    Url(String),
    Full(QuicklinkTable),
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct QuicklinkTable {
    pub url: String,
    pub name: Option<String>,
}

impl QuicklinkConfig {
    pub fn url(&self) -> &str {
        match self { QuicklinkConfig::Url(u) => u, QuicklinkConfig::Full(t) => &t.url }
    }
    pub fn name(&self) -> Option<&str> {
        match self { QuicklinkConfig::Url(_) => None, QuicklinkConfig::Full(t) => t.name.as_deref() }
    }
}

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct SearchConfig {
    /// Rank by launch history (`crate::frecency`); on unless set to `false`.
    pub frecency: Option<bool>,
}

/// The default `ui.font_size`: Rune's body text size.
pub const DEFAULT_FONT_SIZE: f32 = 13.0;
/// The range `ui.font_size` is clamped to, so the launcher stays usable.
pub const FONT_SIZE_RANGE: std::ops::RangeInclusive<f32> = 9.0..=24.0;

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    /// Body text size in points; every other text size scales with it.
    pub font_size: Option<f32>,
}

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct AgentConfig {
    /// The agent CLI to use (`claude`, `codex`, …), instead of Omarchy's default or `$TOMBOY_AGENT`.
    pub name: Option<String>,
    /// Where the agent runs; `~` expands to $HOME.
    pub working_directory: Option<String>,
    /// The model each agent uses, by agent command (`claude = "haiku"`); unset is its default.
    pub models: BTreeMap<String, String>,
}

impl Config {
    /// The agent's working directory: the configured one when it exists, else ~/Work, else ~.
    pub fn agent_dir(&self) -> Option<PathBuf> {
        let home = sys::home_dir()?;
        if let Some(dir) = self.agent.working_directory.as_deref().map(|d| expand(d, &home)) {
            if dir.is_dir() { return Some(dir); }
        }
        let work = home.join("Work");
        Some(if work.is_dir() { work } else { home })
    }

    /// The configured model for `agent`, if any.
    pub fn agent_model(&self, agent: &str) -> Option<String> {
        self.agent.models.get(agent).map(|m| m.trim().to_string()).filter(|m| !m.is_empty())
    }

    /// Where the daily todo lists live: the configured folder, else ~/Documents/tomboy/todo.
    /// (Created on the first write, so it needn't exist.)
    pub fn todo_dir(&self) -> Option<PathBuf> {
        let home = sys::home_dir()?;
        Some(match self.todo.directory.as_deref().filter(|d| !d.trim().is_empty()) {
            Some(d) => expand(d, &home),
            None => home.join("Documents/tomboy/todo"),
        })
    }

    /// The chord for `hotkey`, or `None` when it's turned off (`""`).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn hotkey(&self, hotkey: Hotkey) -> Option<String> {
        let h = &self.hotkeys;
        let set = match hotkey { Hotkey::Launcher => &h.launcher, Hotkey::Clipboard => &h.clipboard, Hotkey::Emoji => &h.emoji, Hotkey::Todo => &h.todo };
        let chord = set.as_deref().unwrap_or(hotkey.default_chord()).trim();
        (!chord.is_empty()).then(|| chord.to_string())
    }

    /// Whether a picked clip or emoji is pasted into the front app too (default on).
    pub fn paste(&self) -> bool {
        self.clipboard.paste.unwrap_or(true)
    }

    /// The extension registry's base URL (`crate::ext::install::DEFAULT_REGISTRY` by default).
    pub fn registry(&self) -> String {
        self.extensions.registry.as_deref().map(str::trim).filter(|r| !r.is_empty())
            .unwrap_or(crate::ext::install::DEFAULT_REGISTRY).trim_end_matches('/').to_string()
    }

    /// The user's `[extensions.<id>]` preference values.
    pub fn extension_prefs(&self, id: &str) -> Option<&toml::Table> {
        self.extensions.prefs.get(id)
    }

    /// Whether search learns from launches and ranks by them (default on).
    pub fn frecency(&self) -> bool {
        self.search.frecency.unwrap_or(true)
    }

    /// How much bigger than default the text is: `ui.font_size / 13`, within [`FONT_SIZE_RANGE`].
    pub fn font_scale(&self) -> f32 {
        let size = self.ui.font_size.filter(|s| s.is_finite()).unwrap_or(DEFAULT_FONT_SIZE);
        size.clamp(*FONT_SIZE_RANGE.start(), *FONT_SIZE_RANGE.end()) / DEFAULT_FONT_SIZE
    }

    /// A warning for the footer when a setting can't be used as written.
    pub fn warning(&self) -> Option<String> {
        if let Some(size) = self.ui.font_size.filter(|s| !FONT_SIZE_RANGE.contains(s)) {
            let (lo, hi) = (FONT_SIZE_RANGE.start(), FONT_SIZE_RANGE.end());
            return Some(format!("config: ui.font_size {size} is outside {lo}–{hi}; using {}", self.font_scale() * DEFAULT_FONT_SIZE));
        }
        for (keyword, link) in &self.quicklinks {
            if keyword.trim().is_empty() || keyword.trim().contains(char::is_whitespace) {
                return Some(format!("config: quicklink keyword \"{keyword}\" can't contain spaces; ignored"));
            }
            if link.url().trim().is_empty() {
                return Some(format!("config: quicklinks.{keyword} has no url; ignored"));
            }
        }
        if let Some((alias, _)) = self.aliases.iter().find(|(a, t)| a.trim().is_empty() || t.trim().is_empty()) {
            return Some(format!("config: alias \"{alias}\" is empty; ignored"));
        }
        let home = sys::home_dir()?;
        let dir = self.agent.working_directory.as_deref()?;
        (!expand(dir, &home).is_dir()).then(|| format!("config: agent.working_directory {dir} doesn't exist; using the default"))
    }
}

/// `~/sites` → `/home/me/sites`; relative paths are taken from $HOME.
fn expand(dir: &str, home: &Path) -> PathBuf {
    let dir = dir.trim();
    match dir.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => home.join(rest.trim_start_matches('/')),
        _ => home.join(dir), // an absolute path replaces `home` in `join`
    }
}

/// `~/.config/tomboy/config.toml` (or under `$XDG_CONFIG_HOME`).
pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME").filter(|d| !d.is_empty()).map(PathBuf::from)
        .or_else(|| sys::home_dir().map(|h| h.join(".config")))?;
    Some(base.join("tomboy/config.toml"))
}

/// The config, or the defaults and an error to show when the file doesn't parse.
pub fn load() -> (Config, Option<String>) {
    let Some(path) = path() else { return (Config::default(), None) };
    match std::fs::read_to_string(&path) {
        Ok(raw) => parse(&raw),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Config::default(), None),
        Err(e) => (Config::default(), Some(format!("config: {}: {e}", path.display()))),
    }
}

fn parse(raw: &str) -> (Config, Option<String>) {
    match toml::from_str(raw) {
        Ok(c) => (c, None),
        Err(e) => {
            let msg = e.message().lines().next().unwrap_or("invalid TOML").to_string();
            (Config::default(), Some(format!("config.toml ignored: {msg}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_defaults() {
        assert_eq!(parse("").0, Config::default());
        let (c, err) = parse("[agent]\nname = \"codex\"\nworking_directory = \"~/sites\"\n");
        assert_eq!(err, None);
        assert_eq!(c.agent.name.as_deref(), Some("codex"));
        let (m, err) = parse("[agent]\nname = \"claude\"\n\n[agent.models]\nclaude = \"haiku\"\ncodex = \" \"\n");
        assert_eq!(err, None);
        assert_eq!(m.agent_model("claude").as_deref(), Some("haiku"));
        assert_eq!(m.agent_model("codex"), None);
        assert_eq!(m.agent_model("opencode"), None);
        assert_eq!(c.agent.working_directory.as_deref(), Some("~/sites"));
        let (c, err) = parse("[agent]\nworking_dir = \"x\"\n");
        assert_eq!(c, Config::default());
        assert!(err.unwrap().contains("unknown field"));
        assert!(parse("[agent").1.is_some());
    }

    #[test]
    fn frecency_switch() {
        assert!(parse("").0.frecency());
        assert!(parse("[search]\nfrecency = true\n").0.frecency());
        let (c, err) = parse("[search]\nfrecency = false\n");
        assert_eq!(err, None);
        assert!(!c.frecency());
    }

    #[test]
    fn hotkeys_and_paste() {
        let c = parse("").0;
        assert_eq!(c.hotkey(Hotkey::Launcher).as_deref(), Some("alt+space"));
        assert!(c.paste());
        let (c, err) = parse("[hotkeys]\nlauncher = \"cmd+space\"\nemoji = \"\"\n\n[clipboard]\npaste = false\n");
        assert_eq!(err, None);
        assert_eq!(c.hotkey(Hotkey::Launcher).as_deref(), Some("cmd+space"));
        assert_eq!(c.hotkey(Hotkey::Emoji), None);
        assert_eq!(c.hotkey(Hotkey::Clipboard).as_deref(), Some("cmd+shift+c"));
        assert!(!c.paste());
    }

    #[test]
    fn font_size() {
        assert_eq!(parse("").0.font_scale(), 1.0);
        let (c, err) = parse("[ui]\nfont_size = 26\n");
        assert_eq!(err, None);
        assert_eq!(c.ui.font_size, Some(26.0));
        assert_eq!(c.font_scale(), 2.0 * 24.0 / 26.0);
        assert!(c.warning().unwrap().contains("outside"));
        let (c, _) = parse("[ui]\nfont_size = 14.5\n");
        assert_eq!(c.font_scale(), 14.5 / 13.0);
        assert_eq!(c.warning(), None);
    }

    #[test]
    fn quicklinks_and_aliases() {
        let (c, err) = parse("[quicklinks]\ngh = \"https://github.com/search?q={query}\"\n\
            jira = { url = \"issues.jira.com/browse/{a}\", name = \"Jira issue\" }\n\n[aliases]\nbubu = \"chromium\"\n");
        assert_eq!(err, None);
        assert_eq!(c.quicklinks["gh"].url(), "https://github.com/search?q={query}");
        assert_eq!(c.quicklinks["gh"].name(), None);
        assert_eq!(c.quicklinks["jira"].name(), Some("Jira issue"));
        assert_eq!(c.aliases["bubu"], "chromium");
        assert_eq!(c.warning(), None);
        assert!(parse("[quicklinks]\njira = { url = \"x/{a}\", title = \"J\" }\n").1.is_some());
        assert!(parse("[quicklinks]\n\"my link\" = \"x/{a}\"\n").0.warning().unwrap().contains("spaces"));
        assert!(parse("[quicklinks]\njira = \" \"\n").0.warning().unwrap().contains("no url"));
        assert!(parse("[aliases]\nbubu = \"\"\n").0.warning().unwrap().contains("empty"));
    }

    #[test]
    fn extensions() {
        let (c, err) = parse("[extensions]\nregistry = \"https://example.com/r/\"\n[extensions.hackernews]\nfeed = \"ask_hn\"\n");
        assert_eq!(err, None);
        assert_eq!(c.registry(), "https://example.com/r");
        assert_eq!(c.extension_prefs("hackernews").and_then(|t| t.get("feed")).and_then(|v| v.as_str()), Some("ask_hn"));
        assert_eq!(Config::default().registry(), crate::ext::install::DEFAULT_REGISTRY);
    }

    #[test]
    fn todo_dir() {
        let Some(home) = sys::home_dir() else { return };
        assert_eq!(parse("").0.todo_dir(), Some(home.join("Documents/tomboy/todo")));
        let (c, err) = parse("[todo]\ndirectory = \"~/notes/todo\"\n");
        assert_eq!(err, None);
        assert_eq!(c.todo_dir(), Some(home.join("notes/todo")));
    }

    #[test]
    fn expands_home() {
        let home = Path::new("/home/me");
        assert_eq!(expand("~", home), PathBuf::from("/home/me"));
        assert_eq!(expand("~/sites", home), PathBuf::from("/home/me/sites"));
        assert_eq!(expand("/srv/www", home), PathBuf::from("/srv/www"));
        assert_eq!(expand("sites", home), PathBuf::from("/home/me/sites"));
        assert_eq!(expand("~bob", home), PathBuf::from("/home/me/~bob"));
    }
}
