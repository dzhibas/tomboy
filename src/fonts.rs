//! The fonts the window registers up front, so the first frame never asks the system for one.
//!
//! On Linux, any family Rune doesn't already hold (the `NERD_FONT` icons, `ui-monospace`) is
//! looked up by building a fontconfig collection of every installed font on the UI thread:
//! about 20–35 ms before the first frame (`TOMBOY_TRACE`: `text.begin` → `text.fonts`). So when
//! the JetBrainsMono Nerd Font file is where it is usually installed, tomboy reads it and
//! registers it with Inter (Rune's own copy, which `AppSpec::fonts` would otherwise replace),
//! and uses it as the mono family too: on Omarchy that is what `monospace` resolves to anyway.
//! Without the file nothing changes: Rune's Inter, the system's monospace, a lookup on first use.

#[cfg(target_os = "linux")]
const INTER: [&[u8]; 3] = [
    include_bytes!("../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
];

/// The file name the Nerd Fonts release and Arch's `ttf-jetbrains-mono-nerd` both use.
#[cfg(target_os = "linux")]
const NERD_FILE: &str = "JetBrainsMonoNerdFont-Regular.ttf";

/// `AppSpec::fonts` and `AppSpec::families`, read once per process.
pub fn spec() -> (&'static [&'static [u8]], (&'static str, &'static str)) {
    static SPEC: std::sync::OnceLock<(&'static [&'static [u8]], (&'static str, &'static str))> = std::sync::OnceLock::new();
    *SPEC.get_or_init(|| crate::trace::span("fonts: read Nerd Font", load))
}

#[cfg(target_os = "linux")]
fn load() -> (&'static [&'static [u8]], (&'static str, &'static str)) {
    let Some(nerd) = nerd_font_paths().into_iter().find_map(|p| std::fs::read(p).ok()) else { return fallback() };
    let mut fonts: Vec<&'static [u8]> = INTER.to_vec();
    fonts.push(Box::leak(nerd.into_boxed_slice()));
    (Box::leak(fonts.into_boxed_slice()), ("Inter", "JetBrainsMono Nerd Font"))
}

#[cfg(not(target_os = "linux"))]
fn load() -> (&'static [&'static [u8]], (&'static str, &'static str)) { fallback() }

/// Rune's built-in Inter and the system's monospace (fontconfig's `monospace`, SF Mono on macOS).
fn fallback() -> (&'static [&'static [u8]], (&'static str, &'static str)) { (&[], ("Inter", "ui-monospace")) }

/// Where the font usually is: the user's font folders first, then the system's (`TTF/` is
/// Arch's), each directly and in a `JetBrainsMono`/`NerdFonts` folder as installers make.
#[cfg(target_os = "linux")]
fn nerd_font_paths() -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| home.as_ref().map(|h| h.join(".local/share")));
    let mut dirs: Vec<PathBuf> = Vec::new();
    dirs.extend(data.map(|d| d.join("fonts")));
    dirs.extend(home.map(|h| h.join(".fonts")));
    dirs.extend(["/usr/share/fonts", "/usr/local/share/fonts"].map(PathBuf::from));
    dirs.iter()
        .flat_map(|d| ["", "TTF", "truetype", "JetBrainsMono", "NerdFonts", "nerd-fonts", "truetype/jetbrains-mono-nerd"].map(|sub| d.join(sub).join(NERD_FILE)))
        .collect()
}
