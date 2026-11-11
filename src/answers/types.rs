//! What every answer provider returns. Providers live beside this file (`units.rs`,
//! `timezone.rs`, `devtools.rs`, `kill.rs`, `currency.rs`) and use these types as
//! `super::{Answer, Action, Kind, Wake}`.

use std::sync::Arc;

/// Wakes tomboy's window from another thread (Rune's `Waker` has this exact type), so a
/// provider that finishes work in the background (a network fetch) can ask for a redraw.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// Which feature produced an answer; the UI picks the row's icon and label from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Units,
    Time,
    Dev,
    Process,
    Currency,
}

/// What Enter does on an answer row.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Copy this text to the clipboard.
    Copy(String),
    /// Send SIGTERM to this process.
    Kill { pid: u32, name: String },
    /// Nothing to do (a "Fetching rates…" or error row).
    None,
}

/// One answer row: the result in large text, its context underneath.
#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    pub kind: Kind,
    /// The answer itself, e.g. `62.1371 mi`, `18:13`, `aGVsbG8gd29ybGQ=`.
    pub title: String,
    /// Context, e.g. `100 km = 62.1371 mi`, `Asia/Tokyo · 6 hours ahead`.
    pub subtitle: String,
    pub action: Action,
}

impl Answer {
    /// The usual answer: copy `title` on Enter.
    pub fn copy(kind: Kind, title: impl Into<String>, subtitle: impl Into<String>) -> Self {
        let title = title.into();
        Answer { kind, action: Action::Copy(title.clone()), title, subtitle: subtitle.into() }
    }
}
