//! "What time is it in X?" answers: `time in tokyo`, `now in utc`, `time pst`, `london time`.
//!
//! Zones come from the system tz database via `jiff`. Places resolve, in order, through a
//! fixed abbreviation table (`pst`, `cet`, …), a few city aliases that aren't IANA names
//! (`nyc`, `sf`, `beijing`, …), city names taken from the IANA zone list (`new york` →
//! `America/New_York`), and finally full IANA names (`europe/vilnius`).

use std::collections::HashMap;
use std::sync::OnceLock;

use jiff::Timestamp;
use jiff::tz::{self, TimeZone};

use super::{Answer, Kind};

/// Abbreviations mapped to a representative zone.
const ABBREVIATIONS: &[(&str, &str)] = &[
    ("utc", "UTC"),
    ("gmt", "UTC"),
    ("cet", "Europe/Paris"),
    ("cest", "Europe/Paris"),
    ("eet", "Europe/Athens"),
    ("eest", "Europe/Athens"),
    ("wet", "Europe/Lisbon"),
    ("west", "Europe/Lisbon"),
    ("bst", "Europe/London"),
    ("est", "America/New_York"),
    ("edt", "America/New_York"),
    ("cst", "America/Chicago"),
    ("cdt", "America/Chicago"),
    ("mst", "America/Denver"),
    ("mdt", "America/Denver"),
    ("pst", "America/Los_Angeles"),
    ("pdt", "America/Los_Angeles"),
    ("ist", "Asia/Kolkata"),
    ("jst", "Asia/Tokyo"),
    ("kst", "Asia/Seoul"),
    ("aest", "Australia/Sydney"),
    ("aedt", "Australia/Sydney"),
    ("nzst", "Pacific/Auckland"),
    ("nzdt", "Pacific/Auckland"),
    ("msk", "Europe/Moscow"),
    ("hkt", "Asia/Hong_Kong"),
    ("sgt", "Asia/Singapore"),
];

/// Common names that aren't the last component of any IANA zone.
const ALIASES: &[(&str, &str)] = &[
    ("san francisco", "America/Los_Angeles"),
    ("sf", "America/Los_Angeles"),
    ("la", "America/Los_Angeles"),
    ("seattle", "America/Los_Angeles"),
    ("nyc", "America/New_York"),
    ("washington", "America/New_York"),
    ("boston", "America/New_York"),
    ("miami", "America/New_York"),
    ("beijing", "Asia/Shanghai"),
    ("delhi", "Asia/Kolkata"),
    ("new delhi", "Asia/Kolkata"),
    ("mumbai", "Asia/Kolkata"),
    ("bangalore", "Asia/Kolkata"),
    ("bengaluru", "Asia/Kolkata"),
    ("hong kong", "Asia/Hong_Kong"),
    ("munich", "Europe/Berlin"),
    ("milan", "Europe/Rome"),
    ("barcelona", "Europe/Madrid"),
    ("st petersburg", "Europe/Moscow"),
    ("saint petersburg", "Europe/Moscow"),
];

/// Regions whose zones win when two zone names share a city component, so a legacy
/// name like `US/Central` never shadows a real `<Region>/Central`-style zone.
const CANONICAL_REGIONS: &[&str] = &[
    "Africa",
    "America",
    "Antarctica",
    "Arctic",
    "Asia",
    "Atlantic",
    "Australia",
    "Europe",
    "Indian",
    "Pacific",
];

pub fn answer(query: &str) -> Option<Answer> {
    // Cheap rejection before touching the clock or the tz database.
    place_of(query)?;
    let local = TimeZone::system();
    answer_at(query, Timestamp::now(), &local)
}

fn answer_at(query: &str, now: Timestamp, local: &TimeZone) -> Option<Answer> {
    let place = place_of(query)?;
    let (name, zone) = resolve(&place)?;

    let there = now.to_zoned(zone);
    let here = now.to_zoned(local.clone());

    let mut title = there.strftime("%H:%M").to_string();
    if there.date() != here.date() {
        title.push(' ');
        title.push_str(&there.strftime("%a").to_string());
    }

    let diff = there.offset().seconds() - here.offset().seconds();
    Some(Answer::copy(
        Kind::Time,
        title,
        format!("{name} · {}", describe_diff(diff)),
    ))
}

/// The place part of a query, lowercased, or `None` when no trigger matches.
fn place_of(query: &str) -> Option<String> {
    let q = query.trim().to_lowercase();
    let place = q
        .strip_prefix("time in ")
        .or_else(|| q.strip_prefix("now in "))
        .or_else(|| q.strip_prefix("time "))
        .or_else(|| q.strip_suffix(" time"))?;
    let place = place.split_whitespace().collect::<Vec<_>>().join(" ");
    (!place.is_empty() && place != "in").then_some(place)
}

/// Finds the zone for a (lowercased) place; returns the name to display and the zone.
fn resolve(place: &str) -> Option<(String, TimeZone)> {
    let folded = fold(place);
    let iana = ABBREVIATIONS
        .iter()
        .chain(ALIASES)
        .find(|(k, _)| *k == folded)
        .map(|(_, v)| (*v).to_string())
        .or_else(|| cities().get(&folded).cloned());

    let zone = match &iana {
        Some(name) => tz::db().get(name).ok()?,
        // Full IANA names; the lookup is case-insensitive. Restrict to names that look
        // like zone names so arbitrary words don't hit legacy entries.
        None if place.contains('/') => tz::db().get(place).ok()?,
        None => return None,
    };
    let name = zone.iana_name().map(str::to_string).or(iana)?;
    Some((name, zone))
}

/// City name (lowercased, `_` → space, accents folded) → IANA zone name.
fn cities() -> &'static HashMap<String, String> {
    static CITIES: OnceLock<HashMap<String, String>> = OnceLock::new();
    CITIES.get_or_init(|| {
        let mut map: HashMap<String, String> = HashMap::new();
        for name in tz::db().available() {
            let name = name.as_str();
            let Some((region, city)) = name.rsplit_once('/') else {
                continue;
            };
            let region_root = region.split('/').next().unwrap_or(region);
            if matches!(region_root, "posix" | "right" | "Etc" | "SystemV") {
                continue;
            }
            let key = fold(&city.replace('_', " ").to_lowercase());
            let canonical = CANONICAL_REGIONS.contains(&region_root);
            match map.get(&key) {
                Some(existing) if !canonical || is_canonical(existing) => {}
                _ => {
                    map.insert(key, name.to_string());
                }
            }
        }
        map
    })
}

fn is_canonical(name: &str) -> bool {
    let root = name.split('/').next().unwrap_or(name);
    CANONICAL_REGIONS.contains(&root)
}

/// Strips common diacritics so `são paulo` matches `Sao_Paulo`.
fn fold(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' | 'ø' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            'ý' | 'ÿ' => 'y',
            _ => c,
        })
        .collect()
}

/// `6 hours ahead`, `1 hour behind`, `5.5 hours ahead`, `5:45 hours ahead`, `same time`.
fn describe_diff(seconds: i32) -> String {
    if seconds == 0 {
        return "same time".to_string();
    }
    let dir = if seconds > 0 { "ahead" } else { "behind" };
    let minutes = seconds.unsigned_abs() / 60;
    let (h, m) = (minutes / 60, minutes % 60);
    match m {
        0 if h == 1 => format!("1 hour {dir}"),
        0 => format!("{h} hours {dir}"),
        30 => format!("{h}.5 hours {dir}"),
        _ => format!("{h}:{m:02} hours {dir}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2025-07-10 09:13 UTC — summer, so Vilnius is UTC+3 and Los Angeles UTC-7.
    fn now() -> Timestamp {
        "2025-07-10T09:13:00Z".parse().unwrap()
    }

    fn zone(name: &str) -> TimeZone {
        tz::db().get(name).unwrap()
    }

    fn at(query: &str, local: &str) -> Option<(String, String)> {
        answer_at(query, now(), &zone(local)).map(|a| (a.title, a.subtitle))
    }

    fn ok(query: &str, local: &str) -> (String, String) {
        at(query, local).unwrap_or_else(|| panic!("no answer for {query:?}"))
    }

    #[test]
    fn table_rows() {
        assert_eq!(
            ok("time in tokyo", "Europe/Vilnius"),
            ("18:13".into(), "Asia/Tokyo · 6 hours ahead".into())
        );
        assert_eq!(
            ok("now in utc", "Europe/Vilnius"),
            ("09:13".into(), "UTC · 3 hours behind".into())
        );
        assert_eq!(
            ok("time in pst", "Europe/Vilnius"),
            (
                "02:13".into(),
                "America/Los_Angeles · 10 hours behind".into()
            )
        );
    }

    #[test]
    fn copies_title() {
        let a = answer_at("time in tokyo", now(), &zone("Europe/Vilnius")).unwrap();
        assert_eq!(a.kind, Kind::Time);
        assert_eq!(a.action, super::super::Action::Copy("18:13".into()));
    }

    #[test]
    fn trigger_forms() {
        for q in [
            "time in tokyo",
            "now in tokyo",
            "time tokyo",
            "tokyo time",
            "  Time In TOKYO  ",
        ] {
            assert_eq!(
                ok(q, "Europe/Vilnius").1,
                "Asia/Tokyo · 6 hours ahead",
                "{q}"
            );
        }
    }

    #[test]
    fn abbreviations() {
        let cases = [
            ("gmt", "UTC"),
            ("cet", "Europe/Paris"),
            ("est", "America/New_York"),
            ("cst", "America/Chicago"),
            ("mst", "America/Denver"),
            ("pdt", "America/Los_Angeles"),
            ("ist", "Asia/Kolkata"),
            ("jst", "Asia/Tokyo"),
            ("aest", "Australia/Sydney"),
            ("msk", "Europe/Moscow"),
            ("sgt", "Asia/Singapore"),
        ];
        for (abbr, name) in cases {
            let (_, sub) = ok(&format!("time in {abbr}"), "UTC");
            assert!(sub.starts_with(&format!("{name} · ")), "{abbr}: {sub}");
        }
        assert_eq!(ok("time in jst", "UTC").1, "Asia/Tokyo · 9 hours ahead");
    }

    #[test]
    fn cities_and_aliases() {
        assert_eq!(
            ok("time in new york", "UTC"),
            ("05:13".into(), "America/New_York · 4 hours behind".into())
        );
        assert_eq!(
            ok("time in sao paulo", "UTC").1,
            "America/Sao_Paulo · 3 hours behind"
        );
        assert_eq!(
            ok("time in são paulo", "UTC").1,
            "America/Sao_Paulo · 3 hours behind"
        );
        assert_eq!(ok("london time", "UTC").1, "Europe/London · 1 hour ahead");
        assert_eq!(
            ok("time in nyc", "UTC").1,
            "America/New_York · 4 hours behind"
        );
        assert_eq!(
            ok("time in san francisco", "UTC").1,
            "America/Los_Angeles · 7 hours behind"
        );
        assert_eq!(
            ok("time in beijing", "UTC").1,
            "Asia/Shanghai · 8 hours ahead"
        );
        assert_eq!(
            ok("time in mumbai", "UTC").1,
            "Asia/Kolkata · 5.5 hours ahead"
        );
        assert_eq!(
            ok("time in europe/vilnius", "UTC").1,
            "Europe/Vilnius · 3 hours ahead"
        );
        assert_eq!(
            ok("time in vilnius", "Europe/Vilnius").1,
            "Europe/Vilnius · same time"
        );
    }

    #[test]
    fn fractional_offsets() {
        assert_eq!(
            ok("time in kolkata", "UTC"),
            ("14:43".into(), "Asia/Kolkata · 5.5 hours ahead".into())
        );
        assert_eq!(
            ok("time in kathmandu", "UTC"),
            ("14:58".into(), "Asia/Kathmandu · 5:45 hours ahead".into())
        );
        assert_eq!(
            ok("time in utc", "Asia/Kolkata").1,
            "UTC · 5.5 hours behind"
        );
        assert_eq!(
            ok("time in kathmandu", "Europe/Vilnius").1,
            "Asia/Kathmandu · 2:45 hours ahead"
        );
    }

    #[test]
    fn day_suffix() {
        // 22:00 UTC: 01:00 Friday in Vilnius, 15:00 Thursday in Los Angeles.
        let late: Timestamp = "2025-07-10T22:00:00Z".parse().unwrap();
        let a = answer_at("time in pst", late, &zone("Europe/Vilnius")).unwrap();
        assert_eq!(a.title, "15:00 Thu");
        let a = answer_at("time in tokyo", late, &zone("Europe/Vilnius")).unwrap();
        assert_eq!(a.title, "07:00");
    }

    #[test]
    fn rejections() {
        for q in [
            "firefox",
            "time in nowhereland",
            "time",
            "time in",
            "tokyo",
            "firefox time",
            "",
            "timer 5",
        ] {
            assert_eq!(at(q, "UTC"), None, "{q:?}");
        }
    }
}
