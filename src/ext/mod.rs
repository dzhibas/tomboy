//! Extensions: sandboxed Luau scripts that add commands to tomboy, installed from the
//! `extensions/` folder of tomboy's repo (see `docs/extensions.md`).
//!
//! - `manifest`: `tomboy.toml`, permissions, preferences
//! - `index`: `extensions/index.json`, generated from the folder and read by the store
//! - `install`: download, verify, install, update, uninstall; `installed.json`
//! - `runtime`: one Luau VM per running extension, with `@tomboy/api` and `@tomboy/ui`
//! - `view`: the lists and details extensions return, as plain data tomboy draws
//! - `cli`: `tomboy ext …`

pub mod cli;
pub mod index;
pub mod install;
pub mod manifest;
pub mod runtime;
pub mod view;

use std::path::PathBuf;

/// `~/.local/share/tomboy/extensions` (`$XDG_DATA_HOME` honoured): installed extensions,
/// one folder each, and `installed.json`.
pub fn data_dir() -> Option<PathBuf> {
    xdg("XDG_DATA_HOME", ".local/share").map(|d| d.join("tomboy/extensions"))
}

/// `~/.local/state/tomboy/ext/<id>`: an extension's `api.storage`.
pub fn state_dir(id: &str) -> Option<PathBuf> {
    xdg("XDG_STATE_HOME", ".local/state").map(|d| d.join("tomboy/ext").join(id))
}

/// `~/.cache/tomboy/ext/<id>`: an extension's `api.cache`.
pub fn cache_dir(id: &str) -> Option<PathBuf> {
    cache_root().map(|d| d.join("ext").join(id))
}

/// `~/.cache/tomboy`.
pub fn cache_root() -> Option<PathBuf> {
    xdg("XDG_CACHE_HOME", ".cache").map(|d| d.join("tomboy"))
}

fn xdg(var: &str, fallback: &str) -> Option<PathBuf> {
    std::env::var_os(var).filter(|d| !d.is_empty()).map(PathBuf::from)
        .or_else(|| crate::sys::home_dir().map(|h| h.join(fallback)))
}
