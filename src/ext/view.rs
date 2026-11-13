//! What an extension shows: a list or a Markdown detail, as plain data decoded from what
//! `tomboy.luau` serialized. tomboy draws it with its own rows, so every extension looks and
//! behaves like the rest of the launcher.

use serde::{Deserialize, Deserializer};

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum View {
    List(List),
    Detail(Detail),
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct List {
    pub title: Option<String>,
    #[serde(default, deserialize_with = "seq")]
    pub items: Vec<ListItem>,
    #[serde(default)]
    pub loading: bool,
    pub empty_text: Option<String>,
    pub search_placeholder: Option<String>,
    /// tomboy filters the items by what's typed; false when the extension searches itself.
    #[serde(default)]
    pub filtering: bool,
    /// The callback that gets the typed text (`onSearch`), instead of filtering.
    pub search: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct ListItem {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub icon: Option<String>,
    #[serde(default, deserialize_with = "seq")]
    pub accessories: Vec<String>,
    #[serde(default, deserialize_with = "seq")]
    pub keywords: Vec<String>,
    /// Markdown for the detail pane beside the list.
    pub detail: Option<String>,
    #[serde(default, deserialize_with = "seq")]
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Detail {
    pub title: Option<String>,
    #[serde(default)]
    pub markdown: String,
    #[serde(default, deserialize_with = "seq")]
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Action {
    Open { title: String, url: String },
    Copy { title: String, text: String },
    /// `ui.push` or `ui.run`: tomboy fires the callback and the script does the rest.
    Callback { title: String, id: i64 },
}

impl Action {
    pub fn title(&self) -> &str {
        match self {
            Action::Open { title, .. } | Action::Copy { title, .. } | Action::Callback { title, .. } => title,
        }
    }
}

impl View {
    pub fn actions_of<'a>(&'a self, item: Option<&'a ListItem>) -> &'a [Action] {
        match (self, item) {
            (View::List(_), Some(item)) => &item.actions,
            (View::List(_), None) => &[],
            (View::Detail(d), _) => &d.actions,
        }
    }
}

impl List {
    /// The items matching `query`: every word in the title, subtitle or keywords. With
    /// filtering off (the extension searches itself) every item.
    pub fn matches(&self, query: &str) -> Vec<usize> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        if !self.filtering || words.is_empty() {
            return (0..self.items.len()).collect();
        }
        self.items.iter().enumerate().filter(|(_, it)| {
            let hay = format!("{} {} {}", it.title, it.subtitle.as_deref().unwrap_or(""), it.keywords.join(" "))
                .to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        }).map(|(i, _)| i).collect()
    }
}

/// A Luau array, or an empty table (which crosses as an empty map, not a list).
fn seq<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Vec<T>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Seq<T> {
        List(Vec<T>),
        Empty(#[allow(dead_code)] std::collections::BTreeMap<String, serde::de::IgnoredAny>),
    }
    Ok(match Seq::deserialize(d)? {
        Seq::List(v) => v,
        Seq::Empty(_) => Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_lists_and_empty_tables() {
        let v: View = serde_json::from_value(serde_json::json!({
            "type": "list", "items": [
                { "id": "1", "title": "Rust 2026", "subtitle": "blog.rust-lang.org", "accessories": {},
                  "keywords": [], "actions": [{ "kind": "open", "title": "Open", "url": "https://x" },
                                              { "kind": "callback", "title": "Refresh", "id": 3 }] }
            ], "loading": false, "filtering": true
        })).unwrap();
        let View::List(l) = v else { panic!() };
        assert_eq!(l.items[0].actions[1], Action::Callback { title: "Refresh".into(), id: 3 });
        assert!(l.items[0].accessories.is_empty());
        assert_eq!(l.matches("rust blog"), vec![0]);
        assert!(l.matches("python").is_empty());
    }

    #[test]
    fn decodes_detail() {
        let v: View = serde_json::from_value(serde_json::json!({ "type": "detail", "markdown": "# Hi", "actions": {} })).unwrap();
        assert_eq!(v, View::Detail(Detail { title: None, markdown: "# Hi".into(), actions: vec![] }));
    }
}
