//! tomboy's optional settings, from `~/.config/tomboy/config.toml` (`$XDG_CONFIG_HOME` honoured).
//! No file means defaults; a setting left out means its default. Read once at launch, so an
//! edit takes effect the next time tomboy opens.
//!
//! ```toml
//! [agent]
//! # Where "Ask agent" runs the coding agent. Default: ~/Work, or ~ when that doesn't exist.
//! working_directory = "~/sites"
//! ```
//!
//! Every option is documented for the agent in `src/skill.md`; keep the two in step.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::sys;

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub agent: AgentConfig,
}

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct AgentConfig {
    /// Where the agent runs; `~` expands to $HOME.
    pub working_directory: Option<String>,
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

    /// A warning for the footer when the configured directory is missing.
    pub fn warning(&self) -> Option<String> {
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
        let (c, err) = parse("[agent]\nworking_directory = \"~/sites\"\n");
        assert_eq!(err, None);
        assert_eq!(c.agent.working_directory.as_deref(), Some("~/sites"));
        let (c, err) = parse("[agent]\nworking_dir = \"x\"\n");
        assert_eq!(c, Config::default());
        assert!(err.unwrap().contains("unknown field"));
        assert!(parse("[agent").1.is_some());
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
