--!strict
-- Hacker News for tomboy: the front page (or newest, Show HN, Ask HN) as a list, with the
-- picked story's details beside it. One request to the Algolia HN API, cached five minutes.
local api = require("@tomboy/api")
local ui = require("@tomboy/ui")

local FRESH = 5 * 60
local ITEM = "https://news.ycombinator.com/item?id="

local function feedUrl(feed: string): string
	if feed == "newest" then
		return "https://hn.algolia.com/api/v1/search_by_date?tags=story&hitsPerPage=50"
	end
	return `https://hn.algolia.com/api/v1/search?tags={feed}&hitsPerPage=50`
end

local function site(url: string?): string
	if not url then
		return "news.ycombinator.com"
	end
	local host = string.match(url, "^https?://([^/:?#]+)") or url
	return (string.gsub(host, "^www%.", ""))
end

-- Ask HN text arrives as HTML.
local function plain(html: string): string
	local s = string.gsub(html, "<p>", "\n\n")
	s = string.gsub(s, "<[^>]+>", "")
	s = string.gsub(s, "&#x2F;", "/")
	s = string.gsub(s, "&#x27;", "'")
	s = string.gsub(s, "&quot;", '"')
	s = string.gsub(s, "&gt;", ">")
	s = string.gsub(s, "&lt;", "<")
	s = string.gsub(s, "&amp;", "&")
	return s
end

local function row(hit: any, refresh: any)
	local comments = ITEM .. tostring(hit.objectID)
	local link = hit.url or comments
	local title = hit.title or "(untitled)"
	local points = hit.points or 0
	local count = hit.num_comments or 0
	local age = api.time.ago(hit.created_at_i or api.time.now())
	local detail = `## {title}\n\n**{points}** points · **{count}** comments · {age} ago · by {hit.author or "?"}\n\n[{site(hit.url)}]({link})`
	if hit.story_text then
		detail ..= "\n\n" .. plain(hit.story_text)
	end
	return {
		id = hit.objectID,
		title = title,
		subtitle = site(hit.url),
		accessories = { `▲ {points}`, `💬 {count}`, age },
		detail = detail,
		actions = {
			ui.open(link, "Open Story"),
			ui.open(comments, "Open Comments"),
			ui.copy(link, "Copy Link"),
			ui.copy(`[{title}]({link})`, "Copy as Markdown"),
			refresh,
		},
	}
end

local function fetch(feed: string): string
	local res = api.http.get(feedUrl(feed))
	if res.status ~= 200 then
		error(`Hacker News returned HTTP {res.status}`, 0)
	end
	api.cache.set("feed:" .. feed, res.body)
	return res.body
end

local load -- defined below; Refresh calls it

-- The stories in `body` (an API response, or nil while there is none) as a list.
local function list(feed: string, body: string?, loading: boolean, note: string?)
	local refresh = ui.run("Refresh", function()
		ui.update(list(feed, body, true))
		load(feed, body)
	end)
	local items = {}
	local ok, data = pcall(api.json.decode, body or "{}")
	for _, hit in if ok and data.hits then data.hits else {} do
		table.insert(items, row(hit, refresh))
	end
	return ui.list({
		title = if feed == "front_page" then "Hacker News" else `Hacker News · {feed}`,
		items = items,
		loading = loading,
		emptyText = note or "No stories",
		searchPlaceholder = "Filter stories…",
	})
end

-- Fetches the feed after the current view is up, then replaces the list with it.
load = function(feed: string, previous: string?)
	task.defer(function()
		local ok, result = pcall(fetch, feed)
		if ok then
			ui.update(list(feed, result, false))
		else
			local message = `Couldn't load Hacker News: {result}`
			ui.update(list(feed, previous, false, message))
			ui.toast(message)
		end
	end)
end

return {
	["front-page"] = function(ctx)
		local feed = tostring(ctx.preferences.feed or "front_page")
		local fresh = api.cache.get("feed:" .. feed, FRESH)
		if fresh then
			return list(feed, fresh, false)
		end
		-- Show what we had (any age) while the new list loads.
		local stale = api.cache.get("feed:" .. feed)
		load(feed, stale)
		return list(feed, stale, true)
	end,
}
