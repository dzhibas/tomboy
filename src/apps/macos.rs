//! macOS application discovery and launching: `.app` bundles in the standard folders,
//! opened with `open`. Icons come from Rune's `ImageCache::app_icon` (Finder's icon for
//! the bundle path).

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::AppEntry;

fn app_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = [
        "/Applications",
        "/Applications/Utilities",
        "/System/Applications",
        "/System/Applications/Utilities",
        "/System/Library/CoreServices/Applications",
    ].into_iter().map(PathBuf::from).collect();
    if let Some(home) = crate::sys::home_dir() { dirs.insert(0, home.join("Applications")); }
    dirs
}

/// Bundles directly in `dir`, plus one level of plain folders (`/Applications/Setapp`).
fn collect(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(read) = fs::read_dir(dir) else { return };
    for entry in read.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "app") {
            out.push(path);
        } else if depth > 0 && entry.file_type().is_ok_and(|t| t.is_dir()) {
            collect(&path, depth - 1, out);
        }
    }
}

pub fn load_apps() -> Vec<AppEntry> {
    let mut found = Vec::new();
    for dir in app_dirs() { collect(&dir, 1, &mut found); }
    found.push(PathBuf::from("/System/Library/CoreServices/Finder.app"));
    let mut seen = HashSet::new();
    let mut apps: Vec<AppEntry> = found.into_iter()
        .filter(|p| p.is_dir())
        .filter_map(|path| {
            let id = path.file_name()?.to_string_lossy().into_owned();
            if !seen.insert(id.clone()) { return None; }
            let name = id.trim_end_matches(".app").to_string();
            Some(AppEntry {
                id, name, generic_name: None, comment: None,
                exec: path.to_string_lossy().into_owned(),
                icon: Some(String::new()), keywords: Vec::new(), terminal: false, path,
            })
        })
        .collect();
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

/// The bundle name, lowercased, for search (`/Applications/Visual Studio Code.app` → `visual studio code`).
pub fn program_name(exec: &str) -> String {
    Path::new(exec).file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub fn launch(app: &AppEntry) -> io::Result<()> {
    crate::sys::spawn_detached(Command::new("open").arg(&app.path))
}
