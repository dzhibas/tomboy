# Hacker News

The Hacker News front page inside tomboy: type `hn`, pick a story, press Enter to open it.

- **Enter** opens the story (or its discussion, for Ask HN)
- **Ctrl/⌘+K** for more: Open Comments, Copy Link, Copy as Markdown, Refresh
- Type to filter the stories by title or site

Stories come from the [Hacker News Search API](https://hn.algolia.com/api) and are cached
for five minutes, so reopening is instant.

## Preferences

Pick the feed in `~/.config/tomboy/config.toml`:

```toml
[extensions.hackernews]
feed = "show_hn"   # front_page (default), newest, show_hn, ask_hn
```
