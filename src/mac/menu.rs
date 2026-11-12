//! macOS system commands, searchable in the launcher as Omarchy's menu is on Omarchy: lock,
//! sleep, restart, empty the Trash, dark mode… Each is a shell command (`pmset`, AppleScript)
//! in the same `MenuItem` shape, so search, ranking and frecency treat them alike.

use crate::omarchy::MenuItem;

/// The row that quits the resident launcher (`tomboy daemon`); `main` handles it itself.
pub const QUIT_ID: &str = "tomboy.quit";

/// `(id, label, Nerd Font icon, command, aliases)`.
const COMMANDS: &[(&str, &str, &str, &str, &[&str])] = &[
    ("macos.lock", "Lock Screen", "\u{f033e}",
     "pmset displaysleepnow", &["lock"]),
    ("macos.sleep", "Sleep", "\u{f1019}", "pmset sleepnow", &["suspend"]),
    ("macos.restart", "Restart…", "\u{f0709}",
     "osascript -e 'tell application \"loginwindow\" to «event aevtrrst»'", &["reboot"]),
    ("macos.shutdown", "Shut Down…", "\u{f0425}",
     "osascript -e 'tell application \"loginwindow\" to «event aevtrsdn»'", &["power off", "turn off"]),
    ("macos.logout", "Log Out…", "\u{f0343}",
     "osascript -e 'tell application \"loginwindow\" to «event aevtrlgo»'", &["sign out"]),
    ("macos.trash", "Empty Trash", "\u{f0a7a}",
     "osascript -e 'tell application \"Finder\" to empty trash'", &["bin"]),
    ("macos.dark", "Toggle Dark Mode", "\u{f050e}",
     "osascript -e 'tell application \"System Events\" to tell appearance preferences to set dark mode to not dark mode'",
     &["appearance", "light mode", "theme"]),
    ("macos.mute", "Toggle Mute", "\u{f0581}",
     "osascript -e 'set volume output muted (not (output muted of (get volume settings)))'", &["sound", "volume"]),
    ("macos.screensaver", "Start Screen Saver", "\u{f0379}", "open -a ScreenSaverEngine", &[]),
    ("macos.eject", "Eject All Disks", "\u{f01ea}",
     "osascript -e 'tell application \"Finder\" to eject (every disk whose ejectable is true)'", &["unmount"]),
];

/// The system commands, plus "Quit tomboy" when tomboy runs resident.
pub fn items(resident: bool) -> Vec<MenuItem> {
    let mut items: Vec<MenuItem> = COMMANDS.iter().map(|(id, label, icon, cmd, aliases)| MenuItem {
        id: id.to_string(),
        label: label.to_string(),
        icon: Some(icon.to_string()),
        action: Some(cmd.to_string()),
        aliases: aliases.iter().map(|a| a.to_string()).collect(),
        ..Default::default()
    }).collect();
    if resident {
        items.push(MenuItem {
            id: QUIT_ID.into(), label: "Quit tomboy".into(), icon: Some("\u{f0159}".into()),
            action: Some(String::new()), aliases: vec!["exit tomboy".into()],
            description: Some("Stops the background launcher until the next login".into()),
            ..Default::default()
        });
    }
    items
}

/// Run a command row detached, from $HOME.
pub fn run(item: &MenuItem) -> std::io::Result<()> {
    let Some(action) = item.action.as_deref().filter(|a| !a.is_empty()) else { return Ok(()) };
    let mut cmd = std::process::Command::new("/bin/sh");
    cmd.arg("-c").arg(action);
    if let Some(home) = crate::sys::home_dir() { cmd.current_dir(home); }
    crate::sys::spawn_detached(&mut cmd)
}
