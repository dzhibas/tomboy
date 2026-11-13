//! An extension's `tomboy.toml`: what it is, its commands, the permissions it asks for and its
//! preferences. Parsed strictly (an unknown key is an error) so a typo never silently drops a
//! permission or a command.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The extension API version this build of tomboy runs. An extension whose `api` is higher
/// needs a newer tomboy.
pub const API_VERSION: u32 = 1;

/// The file every extension folder has.
pub const MANIFEST: &str = "tomboy.toml";

/// The Luau module whose table holds one function per command id.
pub const ENTRY: &str = "init.luau";

/// File types an extension may contain; anything else is refused by `check` and the installer.
pub const ALLOWED_EXTENSIONS: &[&str] = &["toml", "luau", "md", "png", "json"];

/// Most bytes one extension's files may take together.
pub const MAX_TOTAL_BYTES: u64 = 1024 * 1024;

/// Most bytes `icon.png` may take.
pub const MAX_ICON_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// `[a-z0-9-]+`, the same as the extension's folder name.
    pub id: String,
    pub name: String,
    /// `major.minor.patch`.
    pub version: String,
    pub api: u32,
    pub description: String,
    /// GitHub login of the author.
    pub author: String,
    pub license: String,
    /// `linux`, `macos`; empty means both.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub platforms: Vec<String>,
    /// A Nerd Font glyph; without one tomboy shows a generic puzzle-piece icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub commands: Vec<Command>,
    #[serde(default)]
    pub permissions: Permissions,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preferences: Vec<Preference>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Command {
    /// The name of the function in `init.luau`'s returned table.
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub mode: Mode,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Opens a view (a list or a detail) inside tomboy.
    #[default]
    View,
    /// Runs and closes tomboy (unless it shows a view or a toast).
    Action,
}

/// What an extension may do beyond its own storage. Checked by every host function.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Permissions {
    /// Exact host names `api.http.get` may reach, over https only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub net: Vec<String>,
}

impl Permissions {
    pub fn is_empty(&self) -> bool {
        self.net.is_empty()
    }

    /// Permissions `self` has that `accepted` doesn't: what an update would newly ask for.
    pub fn added_since(&self, accepted: &Permissions) -> Permissions {
        Permissions { net: self.net.iter().filter(|h| !accepted.net.contains(h)).cloned().collect() }
    }

    /// Each permission in plain words, for the store and the consent screen.
    pub fn describe(&self) -> Vec<String> {
        self.net.iter().map(|h| format!("Connects to {h}")).collect()
    }

    /// Whether `host` (lowercase) may be reached.
    pub fn allows_host(&self, host: &str) -> bool {
        self.net.iter().any(|h| h.eq_ignore_ascii_case(host))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Preference {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: PreferenceKind,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<toml::Value>,
    /// The choices of a `dropdown`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PreferenceKind {
    Text,
    Checkbox,
    Dropdown,
}

impl Manifest {
    pub fn parse(raw: &str) -> Result<Manifest, String> {
        let m: Manifest = toml::from_str(raw).map_err(|e| format!("{MANIFEST}: {}", e.message()))?;
        m.validate()?;
        Ok(m)
    }

    pub fn load(dir: &Path) -> Result<Manifest, String> {
        let raw = std::fs::read_to_string(dir.join(MANIFEST))
            .map_err(|e| format!("{}: {e}", dir.join(MANIFEST).display()))?;
        Manifest::parse(&raw)
    }

    /// Checks what serde can't: id and version shapes, unique command ids, preference
    /// defaults. Not the API version: an extension for a newer tomboy still parses, so the
    /// store can say "Needs a newer tomboy".
    pub fn validate(&self) -> Result<(), String> {
        if !valid_id(&self.id) {
            return Err(format!("id \"{}\": use lowercase letters, digits and dashes", self.id));
        }
        if parse_version(&self.version).is_none() {
            return Err(format!("version \"{}\": use major.minor.patch, like 1.0.0", self.version));
        }
        if self.api == 0 {
            return Err("api: must be 1 or more".into());
        }
        for field in [("name", &self.name), ("description", &self.description), ("author", &self.author), ("license", &self.license)] {
            if field.1.trim().is_empty() {
                return Err(format!("{} is empty", field.0));
            }
        }
        for p in &self.platforms {
            if p != "linux" && p != "macos" {
                return Err(format!("platforms: unknown platform \"{p}\" (linux, macos)"));
            }
        }
        if self.commands.is_empty() {
            return Err("commands: an extension needs at least one [[commands]]".into());
        }
        let mut seen = Vec::new();
        for c in &self.commands {
            if !valid_id(&c.id) {
                return Err(format!("command id \"{}\": use lowercase letters, digits and dashes", c.id));
            }
            if seen.contains(&&c.id) {
                return Err(format!("command id \"{}\" is used twice", c.id));
            }
            seen.push(&c.id);
            if c.title.trim().is_empty() {
                return Err(format!("command \"{}\": title is empty", c.id));
            }
        }
        for h in &self.permissions.net {
            if !valid_host(h) {
                return Err(format!("permissions.net: \"{h}\" is not a host name (no scheme, path or port)"));
            }
        }
        let mut seen = Vec::new();
        for p in &self.preferences {
            if !valid_id(&p.id) && !p.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(format!("preference id \"{}\": use letters, digits, - or _", p.id));
            }
            if seen.contains(&&p.id) {
                return Err(format!("preference id \"{}\" is used twice", p.id));
            }
            seen.push(&p.id);
            match (p.kind, &p.default) {
                (PreferenceKind::Dropdown, _) if p.options.is_empty() => {
                    return Err(format!("preference \"{}\": a dropdown needs options", p.id));
                }
                (PreferenceKind::Dropdown, Some(toml::Value::String(d))) if !p.options.contains(d) => {
                    return Err(format!("preference \"{}\": default \"{d}\" is not one of its options", p.id));
                }
                (PreferenceKind::Checkbox, Some(d)) if !d.is_bool() => {
                    return Err(format!("preference \"{}\": a checkbox default is true or false", p.id));
                }
                (PreferenceKind::Text | PreferenceKind::Dropdown, Some(d)) if !d.is_str() => {
                    return Err(format!("preference \"{}\": default must be a string", p.id));
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Whether this tomboy can run it: API version and platform.
    pub fn supported(&self) -> Result<(), String> {
        if self.api > API_VERSION {
            return Err("Needs a newer tomboy".into());
        }
        if !self.platforms.is_empty() && !self.platforms.iter().any(|p| p == current_platform()) {
            return Err(format!("Not available on {}", current_platform()));
        }
        Ok(())
    }

    /// Preference values: the manifest's defaults with the user's `[extensions.<id>]` over them.
    pub fn preference_values(&self, user: Option<&toml::Table>) -> BTreeMap<String, toml::Value> {
        let mut out = BTreeMap::new();
        for p in &self.preferences {
            let user_value = user.and_then(|t| t.get(&p.id)).cloned();
            let value = user_value.or_else(|| p.default.clone()).unwrap_or(match p.kind {
                PreferenceKind::Checkbox => toml::Value::Boolean(false),
                PreferenceKind::Dropdown => toml::Value::String(p.options.first().cloned().unwrap_or_default()),
                PreferenceKind::Text => toml::Value::String(String::new()),
            });
            out.insert(p.id.clone(), value);
        }
        out
    }
}

pub fn current_platform() -> &'static str {
    if cfg!(target_os = "macos") { "macos" } else { "linux" }
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !id.starts_with('-') && !id.ends_with('-')
}

fn valid_host(h: &str) -> bool {
    !h.is_empty() && h.contains('.')
        && h.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

/// `1.2.3` → `(1, 2, 3)`.
pub fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let mut parts = v.split('.').map(|p| p.parse::<u64>().ok());
    let out = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(out)
}

/// Whether `a` is a newer version than `b`; an unparseable version is never newer.
pub fn is_newer(a: &str, b: &str) -> bool {
    match (parse_version(a), parse_version(b)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HN: &str = r#"
id = "hackernews"
name = "Hacker News"
version = "1.0.0"
api = 1
description = "Stories"
author = "dzhibas"
license = "MIT"

[[commands]]
id = "front-page"
title = "Hacker News"
keywords = ["hn"]

[permissions]
net = ["hn.algolia.com"]

[[preferences]]
id = "feed"
type = "dropdown"
title = "Feed"
default = "front_page"
options = ["front_page", "show_hn"]
"#;

    #[test]
    fn parses() {
        let m = Manifest::parse(HN).unwrap();
        assert_eq!(m.commands[0].mode, Mode::View);
        assert!(m.permissions.allows_host("hn.algolia.com"));
        assert!(!m.permissions.allows_host("evil.com"));
        assert_eq!(m.permissions.describe(), vec!["Connects to hn.algolia.com"]);
        assert!(m.supported().is_ok());
    }

    #[test]
    fn rejects_bad_manifests() {
        let bad = |from: &str, to: &str| Manifest::parse(&HN.replace(from, to)).unwrap_err();
        assert!(bad("id = \"hackernews\"", "id = \"Hacker News\"").contains("id"));
        assert!(bad("version = \"1.0.0\"", "version = \"1.0\"").contains("version"));
        assert!(bad("license = \"MIT\"", "license = \"MIT\"\nunknown = 1").contains("unknown"));
        assert!(bad("hn.algolia.com", "https://hn.algolia.com/").contains("host"));
        assert!(bad("default = \"front_page\"", "default = \"best\"").contains("options"));
    }

    #[test]
    fn newer_api_parses_but_is_unsupported() {
        let m = Manifest::parse(&HN.replace("api = 1", "api = 99")).unwrap();
        assert_eq!(m.supported().unwrap_err(), "Needs a newer tomboy");
    }

    #[test]
    fn versions() {
        assert!(is_newer("1.10.0", "1.9.9"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("x", "1.0.0"));
    }

    #[test]
    fn added_permissions() {
        let old = Permissions { net: vec!["a.com".into()] };
        let new = Permissions { net: vec!["a.com".into(), "b.com".into()] };
        assert_eq!(new.added_since(&old).net, vec!["b.com"]);
        assert!(old.added_since(&new).is_empty());
    }

    #[test]
    fn preference_values() {
        let m = Manifest::parse(HN).unwrap();
        assert_eq!(m.preference_values(None)["feed"].as_str(), Some("front_page"));
        let user: toml::Table = toml::from_str("feed = \"show_hn\"").unwrap();
        assert_eq!(m.preference_values(Some(&user))["feed"].as_str(), Some("show_hn"));
    }
}
