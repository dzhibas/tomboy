//! Date math: `days until 25 dec`, `days since 1 jan 2020`, `weeks until christmas`,
//! `in 3 weeks`, `3 days ago`, `today + 10 days`, `monday in 3 weeks`, `next friday`.
//!
//! "today" is the system's local date. A date written without a year means its next
//! occurrence (`days until 1 jan`), or its last one after `since`. A weekday with an offset
//! (`monday in 3 weeks`) is that weekday in the week (Monday to Sunday) the offset lands in.

use jiff::civil::{Date, Weekday};
use jiff::{Span, Zoned};

use super::{Answer, Kind};

/// Which way a date without a year (or a bare weekday) is resolved.
#[derive(Clone, Copy, PartialEq)]
enum Dir {
    /// Today or later (`until`, offsets).
    Next,
    /// Today or earlier (`since`).
    Prev,
}

pub fn answer(query: &str) -> Option<Answer> {
    let q = normalize(query);
    // Cheap rejection before looking at the clock: every form has one of these words.
    const HINTS: &[&str] = &["day", "week", "month", "year", "ago", "next", "last", "today", "tomorrow", "yesterday", "in ", "-"];
    if !HINTS.iter().any(|h| q.contains(h)) { return None; }
    answer_at(&q, Zoned::now().date())
}

fn answer_at(q: &str, today: Date) -> Option<Answer> {
    count(q, today).or_else(|| offset(q, today)).or_else(|| lone_date(q, today))
}

/// Lowercase, single-spaced, without a trailing `?` or a leading "how many".
fn normalize(query: &str) -> String {
    let q = query.trim().trim_end_matches('?').to_lowercase().replace(',', " ");
    let q = q.split_whitespace().collect::<Vec<_>>().join(" ");
    q.strip_prefix("how many ").map(str::to_string).unwrap_or(q)
}

// ---- forms ---------------------------------------------------------------------------------

/// `days until <date>`, `days since <date>`, `weeks until <date>`.
fn count(q: &str, today: Date) -> Option<Answer> {
    let (unit, rest) = q.split_once(' ')?;
    let weeks = match unit { "days" | "day" => false, "weeks" | "week" => true, _ => return None };
    let (word, phrase) = rest.split_once(' ')?;
    let dir = match word {
        "until" | "till" | "til" | "to" | "before" => Dir::Next,
        "since" | "from" | "after" => Dir::Prev,
        _ => return None,
    };
    let date = parse_date(phrase, today, dir)?;
    let n = days_between(today, date)?;
    let amount = if weeks { weeks_and_days(n.abs()) } else { plural(n.abs(), "day") };
    let title = match (dir, n) {
        (_, 0) => "0 days".to_string(),
        (Dir::Next, n) if n < 0 => format!("{amount} ago"),
        (Dir::Prev, n) if n > 0 => format!("in {amount}"),
        _ => amount,
    };
    let sub = if n == 0 { format!("Today · {}", long(date)) } else { long(date) };
    Some(Answer::copy(Kind::Date, title, sub))
}

/// `in 3 weeks`, `3 days from now`, `2 months ago`, `today + 10 days`, `25 dec - 1 week`,
/// `2 weeks after christmas`, and with a weekday in front: `monday in 3 weeks`.
fn offset(q: &str, today: Date) -> Option<Answer> {
    // A weekday in front picks that day in the week the rest lands in.
    let (weekday, rest) = match q.split_once(' ') {
        Some((w, rest)) if weekday(w).is_some() => (weekday(w), rest),
        _ => (None, q),
    };
    let date = shifted(rest, today)?;
    let date = match weekday {
        Some(wd) => week_start(date).checked_add(Span::new().days(wd.to_monday_zero_offset() as i64)).ok()?,
        None => date,
    };
    Some(date_answer(date, today))
}

/// The date `rest` describes as an offset, or `None` when it isn't one.
fn shifted(rest: &str, today: Date) -> Option<Date> {
    if let Some(span) = rest.strip_prefix("in ").and_then(span) { return today.checked_add(span).ok(); }
    for suffix in [" from now", " from today", " later"] {
        if let Some(span) = rest.strip_suffix(suffix).and_then(span) { return today.checked_add(span).ok(); }
    }
    if let Some(span) = rest.strip_suffix(" ago").and_then(span) { return today.checked_sub(span).ok(); }
    // `<span> after|from|before <date>`
    for (word, sign) in [(" after ", 1), (" from ", 1), (" before ", -1)] {
        if let Some((a, b)) = rest.split_once(word) {
            if let (Some(span), Some(base)) = (span(a), parse_date(b, today, Dir::Next)) {
                return if sign > 0 { base.checked_add(span).ok() } else { base.checked_sub(span).ok() };
            }
        }
    }
    // `<date> + <span>`, `<date> - <span>`
    for (op, sign) in [" + ", " - "].into_iter().zip([1, -1]) {
        if let Some((a, b)) = rest.rsplit_once(op) {
            if let (Some(base), Some(span)) = (parse_date(a, today, Dir::Next), span(b)) {
                return if sign > 0 { base.checked_add(span).ok() } else { base.checked_sub(span).ok() };
            }
        }
    }
    None
}

/// A query that is only a date: `next friday`, `last monday`, `tomorrow`, `2027-01-01`.
/// (A bare weekday or month would shadow app search, so those need `next`/`last`.)
fn lone_date(q: &str, today: Date) -> Option<Answer> {
    let explicit = ["today", "tomorrow", "yesterday"].contains(&q)
        || q.starts_with("next ") || q.starts_with("last ") || iso(q).is_some();
    if !explicit { return None; }
    let dir = if q.starts_with("last ") || q == "yesterday" { Dir::Prev } else { Dir::Next };
    Some(date_answer(parse_date(q, today, dir)?, today))
}

fn date_answer(date: Date, today: Date) -> Answer {
    let rel = days_between(today, date).map(relative).unwrap_or_default();
    Answer::copy(Kind::Date, long(date), format!("{date} · {rel}"))
}

// ---- pieces --------------------------------------------------------------------------------

/// `3 weeks`, `1 day`, `a month`, `10 days`.
fn span(s: &str) -> Option<Span> {
    let (n, unit) = s.trim().split_once(' ')?;
    let n: i64 = match n { "a" | "an" | "one" => 1, n => n.parse().ok().filter(|n: &i64| n.abs() <= 100_000)? };
    Some(match unit {
        "day" | "days" => Span::new().days(n),
        "week" | "weeks" | "wk" | "wks" => Span::new().weeks(n),
        "month" | "months" => Span::new().months(n),
        "year" | "years" | "yr" | "yrs" => Span::new().years(n),
        _ => return None,
    })
}

/// A date phrase: `today`, `tomorrow`, `friday`, `next friday`, `25 dec`, `dec 25th 2027`,
/// `25th of december`, `2027-01-01`, `christmas`, `new year`.
fn parse_date(s: &str, today: Date, dir: Dir) -> Option<Date> {
    let s = s.trim().trim_start_matches("the ");
    match s {
        "today" | "now" => return Some(today),
        "tomorrow" => return today.tomorrow().ok(),
        "yesterday" => return today.yesterday().ok(),
        _ => {}
    }
    if let Some(d) = iso(s) { return Some(d); }
    if let Some(wd) = weekday(s) { return nth_weekday(today, wd, dir); }
    if let Some(wd) = s.strip_prefix("next ").and_then(weekday) { return nth_weekday(today, wd, Dir::Next); }
    if let Some(wd) = s.strip_prefix("last ").and_then(weekday) { return nth_weekday(today, wd, Dir::Prev); }
    if let Some(wd) = s.strip_prefix("this ").and_then(weekday) { return nth_weekday(today, wd, dir); }
    let (month, day, year) = holiday(s).map(|(m, d)| (m, d, None)).or_else(|| day_month(s))?;
    match year {
        Some(y) => Date::new(y, month, day).ok(),
        None => {
            let this = Date::new(today.year(), month, day).ok()?;
            let year = match dir {
                Dir::Next if this < today => today.year() + 1,
                Dir::Prev if this > today => today.year() - 1,
                _ => today.year(),
            };
            Date::new(year, month, day).ok()
        }
    }
}

/// The next (or last) `wd` strictly after (or before) today.
fn nth_weekday(today: Date, wd: Weekday, dir: Dir) -> Option<Date> {
    match dir {
        Dir::Next => today.nth_weekday(1, wd).ok(),
        Dir::Prev => today.nth_weekday(-1, wd).ok(),
    }
}

fn iso(s: &str) -> Option<Date> {
    let b = s.as_bytes();
    (b.len() == 10 && b[4] == b'-' && b[7] == b'-').then(|| s.parse().ok()).flatten()
}

/// `25 dec`, `dec 25`, `25th of december 2027`, `december 25th`.
fn day_month(s: &str) -> Option<(i8, i8, Option<i16>)> {
    let words: Vec<&str> = s.split(' ').filter(|w| *w != "of").collect();
    let (day, mon, rest) = match words.as_slice() {
        [a, b, rest @ ..] if month(a).is_some() => (day_number(b)?, month(a)?, rest),
        [a, b, rest @ ..] => (day_number(a)?, month(b)?, rest),
        _ => return None,
    };
    let year = match rest {
        [] => None,
        [y] => Some(y.parse::<i16>().ok().filter(|y| (1..=9999).contains(y))?),
        _ => return None,
    };
    Some((mon, day, year))
}

fn day_number(s: &str) -> Option<i8> {
    let digits = s.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let suffix = &s[digits.len()..];
    if !matches!(suffix, "" | "st" | "nd" | "rd" | "th") { return None; }
    digits.parse().ok().filter(|d| (1..=31).contains(d))
}

fn month(s: &str) -> Option<i8> {
    const MONTHS: [&str; 12] = ["january", "february", "march", "april", "may", "june", "july",
        "august", "september", "october", "november", "december"];
    let s = s.trim_end_matches('.');
    if s.len() < 3 { return None; }
    if s == "sept" { return Some(9); }
    MONTHS.iter().position(|m| *m == s || (s.len() == 3 && m.starts_with(s))).map(|i| i as i8 + 1)
}

fn weekday(s: &str) -> Option<Weekday> {
    use Weekday::*;
    Some(match s {
        "monday" | "mon" => Monday,
        "tuesday" | "tue" | "tues" => Tuesday,
        "wednesday" | "wed" => Wednesday,
        "thursday" | "thu" | "thur" | "thurs" => Thursday,
        "friday" | "fri" => Friday,
        "saturday" | "sat" => Saturday,
        "sunday" | "sun" => Sunday,
        _ => return None,
    })
}

/// Fixed-date holidays, as (month, day).
fn holiday(s: &str) -> Option<(i8, i8)> {
    Some(match s.trim_end_matches(" day") {
        "christmas" | "xmas" => (12, 25),
        "christmas eve" | "xmas eve" => (12, 24),
        "new year" | "new years" | "new year's" => (1, 1),
        "new year's eve" | "new years eve" | "nye" => (12, 31),
        "halloween" => (10, 31),
        "valentine's" | "valentines" => (2, 14),
        _ => return None,
    })
}

/// The Monday of `date`'s week.
fn week_start(date: Date) -> Date {
    date.checked_sub(Span::new().days(date.weekday().to_monday_zero_offset() as i64)).unwrap_or(date)
}

fn days_between(from: Date, to: Date) -> Option<i64> {
    Some(from.until(to).ok()?.get_days() as i64)
}

/// `Friday, 25 December 2026`.
fn long(date: Date) -> String {
    date.strftime("%A, %-d %B %Y").to_string()
}

fn relative(days: i64) -> String {
    match days {
        0 => "today".into(),
        1 => "tomorrow".into(),
        -1 => "yesterday".into(),
        n if n > 0 => format!("in {}", plural(n, "day")),
        n => format!("{} ago", plural(-n, "day")),
    }
}

fn plural(n: i64, word: &str) -> String {
    if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") }
}

/// `6 weeks 1 day`, `2 weeks`, `3 days`.
fn weeks_and_days(n: i64) -> String {
    match (n / 7, n % 7) {
        (0, d) => plural(d, "day"),
        (w, 0) => plural(w, "week"),
        (w, d) => format!("{} {}", plural(w, "week"), plural(d, "day")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Thursday, 12 November 2026.
    fn today() -> Date { Date::new(2026, 11, 12).unwrap() }

    fn ask(q: &str) -> Option<(String, String)> {
        answer_at(&normalize(q), today()).map(|a| (a.title, a.subtitle))
    }

    fn title(q: &str) -> String { ask(q).unwrap_or_else(|| panic!("no answer for {q:?}")).0 }

    #[test]
    fn counts_days() {
        assert_eq!(ask("days until 25 dec").unwrap(), ("43 days".into(), "Friday, 25 December 2026".into()));
        assert_eq!(title("Days until Christmas?"), "43 days");
        assert_eq!(title("how many days till dec 25th"), "43 days");
        assert_eq!(title("days until 1 jan"), "50 days");
        assert_eq!(title("days since 1 jan"), "315 days");
        assert_eq!(title("days since 1 jan 2020"), "2507 days");
        assert_eq!(title("days until 2027-01-01"), "50 days");
        assert_eq!(title("days until friday"), "1 day");
        assert_eq!(title("weeks until christmas"), "6 weeks 1 day");
        assert_eq!(title("weeks until 26 nov"), "2 weeks");
        assert_eq!(ask("days until today").unwrap(), ("0 days".into(), "Today · Thursday, 12 November 2026".into()));
        assert_eq!(title("days until 1 jan 2026"), "315 days ago");
        assert_eq!(title("days since 25 december 2026"), "in 43 days");
        // A date that is today counts as the next occurrence, not next year's.
        assert_eq!(title("days until 12 nov"), "0 days");
    }

    #[test]
    fn offsets() {
        let date = |q: &str| ask(q).unwrap_or_else(|| panic!("{q:?}")).1[..10].to_string();
        assert_eq!(ask("in 3 weeks").unwrap(), ("Thursday, 3 December 2026".into(), "2026-12-03 · in 21 days".into()));
        assert_eq!(date("3 weeks from now"), "2026-12-03");
        assert_eq!(date("3 days ago"), "2026-11-09");
        assert_eq!(date("a week ago"), "2026-11-05");
        assert_eq!(date("in 1 month"), "2026-12-12");
        assert_eq!(date("in 2 years"), "2028-11-12");
        assert_eq!(date("today + 10 days"), "2026-11-22");
        assert_eq!(date("tomorrow - 2 days"), "2026-11-11");
        assert_eq!(date("25 dec - 1 week"), "2026-12-18");
        assert_eq!(date("2 weeks after christmas"), "2027-01-08");
        assert_eq!(date("monday in 3 weeks"), "2026-11-30");
        assert_eq!(date("friday in 2 weeks"), "2026-11-27");
        assert_eq!(date("sunday in 1 week"), "2026-11-22");
    }

    #[test]
    fn lone_dates() {
        assert_eq!(ask("next friday").unwrap(), ("Friday, 13 November 2026".into(), "2026-11-13 · tomorrow".into()));
        assert_eq!(ask("last monday").unwrap().1, "2026-11-09 · 3 days ago");
        assert_eq!(ask("next thursday").unwrap().1, "2026-11-19 · in 7 days");
        assert_eq!(ask("yesterday").unwrap().1, "2026-11-11 · yesterday");
        assert_eq!(ask("today").unwrap().1, "2026-11-12 · today");
        assert_eq!(ask("2027-01-01").unwrap().0, "Friday, 1 January 2027");
    }

    #[test]
    fn rejects() {
        for q in ["days", "days until", "days until pizza", "monday", "december", "in 3 apples", "firefox",
            "in", "3 days", "days until 31 feb", "next", "today + 3", "2+2", "100 km to miles", "time in tokyo",
            "kill chromium", "2027-13-01", "in 999999999 days"] {
            assert_eq!(ask(q), None, "{q:?}");
        }
    }
}
