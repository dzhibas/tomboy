//! macOS-only pieces: the native pasteboard, pasting into the front app, the LaunchAgent that
//! keeps `tomboy daemon` running, and the system commands shown in the launcher.

pub mod launch_agent;
pub mod menu;
pub mod paste;
pub mod pasteboard;
