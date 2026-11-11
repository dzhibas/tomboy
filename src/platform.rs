//! Which desktop tomboy runs on, so it can expose that platform's own commands.
//! The OS-level commands (opening URLs, the clipboard, launching) live in `sys` and
//! `apps::{linux, macos}`; this picks the desktop flavour on top of the OS.

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub enum Platform {
    /// Omarchy (omarchy.org): Arch + Hyprland with its own menu of system actions.
    Omarchy { root: PathBuf },
    /// Any other Linux desktop: apps, calculator and URLs only.
    Linux,
    /// macOS: `.app` bundles, `open`, `pbcopy`.
    MacOS,
}

impl Platform {
    pub fn name(&self) -> &'static str {
        match self {
            Platform::Omarchy { .. } => "Omarchy",
            Platform::Linux => "Linux",
            Platform::MacOS => "macOS",
        }
    }
}

/// `TOMBOY_PLATFORM=generic` skips desktop detection (plain Linux or macOS).
pub fn detect() -> Platform {
    if cfg!(target_os = "macos") {
        return Platform::MacOS;
    }
    if std::env::var("TOMBOY_PLATFORM").is_ok_and(|p| p.eq_ignore_ascii_case("generic")) {
        return Platform::Linux;
    }
    let candidates = std::env::var_os("OMARCHY_PATH").map(PathBuf::from).into_iter()
        .chain([PathBuf::from("/usr/share/omarchy")]);
    candidates.into_iter()
        .find(|root| root.join("default/omarchy/omarchy-menu.jsonc").is_file())
        .map_or(Platform::Linux, |root| Platform::Omarchy { root })
}
