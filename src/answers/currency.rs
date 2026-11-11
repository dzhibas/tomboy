//! Currency conversion: `123 eur to usd`, `$120 in eur`, `50 euros -> dollars`.
//!
//! Rates come from open.er-api.com (EUR base, no key) and are cached in
//! `$XDG_CACHE_HOME/tomboy/exchange-rates.json`. Nothing touches the disk or the network
//! until the first conversion is typed; a stale cache (> 24 h) is refreshed on a
//! background thread, which calls `wake()` when it finishes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::{Action, Answer, Kind, Wake};

const URL: &str = "https://open.er-api.com/v6/latest/EUR";
/// Refresh the cache when it is older than this.
const MAX_AGE_SECS: u64 = 24 * 60 * 60;
/// After a failed fetch, wait this long before trying again (not on every keystroke).
const RETRY_SECS: u64 = 60;
const TIMEOUT: Duration = Duration::from_secs(5);

/// The currencies open.er-api.com quotes (ISO 4217 codes, upper case).
const CODES: &[&str] = &[
    "EUR", "AED", "AFN", "ALL", "AMD", "ANG", "AOA", "ARS", "AUD", "AWG", "AZN", "BAM", "BBD",
    "BDT", "BGN", "BHD", "BIF", "BMD", "BND", "BOB", "BRL", "BSD", "BTN", "BWP", "BYN", "BZD",
    "CAD", "CDF", "CHF", "CLF", "CLP", "CNH", "CNY", "COP", "CRC", "CUP", "CVE", "CZK", "DJF",
    "DKK", "DOP", "DZD", "EGP", "ERN", "ETB", "FJD", "FKP", "FOK", "GBP", "GEL", "GGP", "GHS",
    "GIP", "GMD", "GNF", "GTQ", "GYD", "HKD", "HNL", "HRK", "HTG", "HUF", "IDR", "ILS", "IMP",
    "INR", "IQD", "IRR", "ISK", "JEP", "JMD", "JOD", "JPY", "KES", "KGS", "KHR", "KID", "KMF",
    "KRW", "KWD", "KYD", "KZT", "LAK", "LBP", "LKR", "LRD", "LSL", "LYD", "MAD", "MDL", "MGA",
    "MKD", "MMK", "MNT", "MOP", "MRU", "MUR", "MVR", "MWK", "MXN", "MYR", "MZN", "NAD", "NGN",
    "NIO", "NOK", "NPR", "NZD", "OMR", "PAB", "PEN", "PGK", "PHP", "PKR", "PLN", "PYG", "QAR",
    "RON", "RSD", "RUB", "RWF", "SAR", "SBD", "SCR", "SDG", "SEK", "SGD", "SHP", "SLE", "SLL",
    "SOS", "SRD", "SSP", "STN", "SYP", "SZL", "THB", "TJS", "TMT", "TND", "TOP", "TRY", "TTD",
    "TVD", "TWD", "TZS", "UAH", "UGX", "USD", "UYU", "UZS", "VES", "VND", "VUV", "WST", "XAF",
    "XCD", "XCG", "XDR", "XOF", "XPF", "YER", "ZAR", "ZMW", "ZWG", "ZWL",
];

/// Symbols and names (lower case) for the common currencies.
const ALIASES: &[(&str, &str)] = &[
    ("$", "USD"), ("€", "EUR"), ("£", "GBP"), ("¥", "JPY"), ("₹", "INR"), ("₽", "RUB"),
    ("₴", "UAH"), ("zł", "PLN"), ("₩", "KRW"), ("₺", "TRY"), ("₪", "ILS"), ("r$", "BRL"),
    ("c$", "CAD"), ("a$", "AUD"), ("fr", "CHF"),
    ("euro", "EUR"), ("euros", "EUR"),
    ("dollar", "USD"), ("dollars", "USD"), ("buck", "USD"), ("bucks", "USD"),
    ("pound", "GBP"), ("pounds", "GBP"), ("quid", "GBP"),
    ("yen", "JPY"),
    ("yuan", "CNY"), ("rmb", "CNY"), ("renminbi", "CNY"),
    ("rupee", "INR"), ("rupees", "INR"),
    ("rouble", "RUB"), ("roubles", "RUB"), ("ruble", "RUB"), ("rubles", "RUB"),
    ("hryvnia", "UAH"), ("hryvnias", "UAH"), ("hryvnas", "UAH"),
    ("zloty", "PLN"), ("zlotys", "PLN"), ("złoty", "PLN"), ("złotys", "PLN"),
    ("franc", "CHF"), ("francs", "CHF"),
    ("krona", "SEK"), ("kronor", "SEK"),
    ("krone", "NOK"), ("kroner", "NOK"),
    ("won", "KRW"),
    ("real", "BRL"), ("reais", "BRL"), ("reals", "BRL"),
    ("lira", "TRY"), ("liras", "TRY"), ("lire", "TRY"),
    ("shekel", "ILS"), ("shekels", "ILS"),
    ("peso", "MXN"), ("pesos", "MXN"),
];

/// Characters that may form a currency token next to an amount (letters plus symbols).
fn is_token_char(c: char) -> bool {
    c.is_alphabetic() || "$€£¥₹₽₴₩₺₪".contains(c)
}

/// The ISO code for a code, symbol or name (case-insensitive).
fn currency(token: &str) -> Option<&'static str> {
    let t = token.trim().to_lowercase();
    if t.is_empty() {
        return None;
    }
    if let Some(&(_, code)) = ALIASES.iter().find(|(a, _)| *a == t) {
        return Some(code);
    }
    let up = t.to_ascii_uppercase();
    CODES.iter().copied().find(|&c| c == up)
}

/// `<amount> <cur>`, `<cur><amount>`, `<amount><cur>`, or a bare `<cur>` (amount 1).
fn parse_side(s: &str) -> Option<(f64, &'static str)> {
    let s = s.trim();
    if let Some(code) = currency(s) {
        return Some((1.0, code));
    }
    // Currency after the amount.
    let start = s
        .char_indices()
        .rev()
        .take_while(|&(_, c)| is_token_char(c))
        .last()
        .map(|(i, _)| i);
    if let Some(i) = start
        && i > 0
        && let Some(code) = currency(&s[i..])
        && let Some(amount) = crate::calc::eval_amount(&s[..i])
    {
        return Some((amount, code));
    }
    // Currency before the amount.
    let end = s
        .char_indices()
        .find(|&(_, c)| !is_token_char(c))
        .map(|(i, _)| i)?;
    if end > 0
        && let Some(code) = currency(&s[..end])
        && let Some(amount) = crate::calc::eval_amount(&s[end..])
    {
        return Some((amount, code));
    }
    None
}

/// Split `query` into `(amount, from, to)`, or None if it isn't a currency conversion.
pub fn parse(query: &str) -> Option<(f64, &'static str, &'static str)> {
    let q = query.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    let seps = [" to ", " in ", " as ", "->", "="];
    for sep in seps {
        for (i, _) in q.match_indices(sep) {
            let (left, right) = (&q[..i], &q[i + sep.len()..]);
            if let Some(to) = currency(right)
                && let Some((amount, from)) = parse_side(left)
            {
                return Some((amount, from, to));
            }
        }
    }
    None
}

/// Parsed rates (base EUR).
#[derive(Clone, Debug, PartialEq)]
pub struct Rates {
    /// Units of each currency per 1 EUR.
    pub rates: HashMap<String, f64>,
    /// When the provider last updated the rates (unix seconds).
    pub updated: i64,
}

/// Parse an open.er-api.com response.
pub fn parse_rates(json: &serde_json::Value) -> Result<Rates, String> {
    if json.get("result").and_then(|r| r.as_str()) != Some("success") {
        let kind = json.get("error-type").and_then(|e| e.as_str()).unwrap_or("unexpected response");
        return Err(format!("rates API error: {kind}"));
    }
    let updated = json
        .get("time_last_update_unix")
        .and_then(|t| t.as_i64())
        .ok_or("rates API: missing time_last_update_unix")?;
    let map = json.get("rates").and_then(|r| r.as_object()).ok_or("rates API: missing rates")?;
    let mut rates: HashMap<String, f64> = map
        .iter()
        .filter_map(|(k, v)| Some((k.to_ascii_uppercase(), v.as_f64().filter(|r| *r > 0.0)?)))
        .collect();
    rates.entry("EUR".into()).or_insert(1.0);
    Ok(Rates { rates, updated })
}

/// `(converted amount, rate from→to)`, or None if either currency isn't quoted.
pub fn convert(rates: &Rates, amount: f64, from: &str, to: &str) -> Option<(f64, f64)> {
    let f = *rates.rates.get(from)?;
    let t = *rates.rates.get(to)?;
    let rate = t / f;
    let value = amount * rate;
    value.is_finite().then_some((value, rate))
}

/// `decimals` places, or ~4 significant digits for tiny non-zero values.
fn fmt_num(v: f64, decimals: usize, tiny: f64) -> String {
    let a = v.abs();
    let s = if a != 0.0 && a < tiny {
        let d = (-a.log10()).ceil() as usize + 3;
        let s = format!("{v:.d$}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        format!("{v:.decimals$}")
    };
    if s.trim_start_matches('-').chars().all(|c| c == '0' || c == '.') {
        s.trim_start_matches('-').to_string()
    } else {
        s
    }
}

/// `28 Aug` for a unix time (UTC).
fn fmt_day(unix: i64) -> String {
    // Howard Hinnant's civil_from_days.
    let z = unix.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    const MONTHS: [&str; 12] =
        ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{d} {}", MONTHS[(m - 1) as usize])
}

/// The answer row for a conversion. `offline_since` is the fetch time of a stale cache
/// being served after a failed refresh.
pub fn format_answer(
    rates: &Rates,
    amount: f64,
    from: &str,
    to: &str,
    offline_since: Option<i64>,
) -> Option<Answer> {
    let (value, rate) = convert(rates, amount, from, to)?;
    let number = fmt_num(value, 2, 0.01);
    let mut subtitle = format!(
        "{} {from} at {} · {}",
        crate::calc::format(amount),
        fmt_num(rate, 4, 0.001),
        fmt_day(rates.updated)
    );
    if let Some(t) = offline_since {
        subtitle.push_str(&format!(" · offline, rates from {}", fmt_day(t)));
    }
    Some(Answer {
        kind: Kind::Currency,
        title: format!("{number} {to}"),
        subtitle,
        action: Action::Copy(number),
    })
}

// ---- shared state, cache and fetching ----

struct Cached {
    rates: Rates,
    /// When we fetched it (unix seconds).
    fetched_at: u64,
}

#[derive(Default)]
struct State {
    loaded: bool,
    cache: Option<Cached>,
    fetching: bool,
    /// The last fetch's error, cleared on success.
    error: Option<String>,
    /// When the last fetch was started (unix seconds).
    last_attempt: u64,
}

static STATE: Mutex<State> = Mutex::new(State {
    loaded: false,
    cache: None,
    fetching: false,
    error: None,
    last_attempt: 0,
});

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn cache_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("tomboy").join("exchange-rates.json"))
}

fn load_cache() -> Option<Cached> {
    let text = std::fs::read_to_string(cache_path()?).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let fetched_at = json.get("fetched_at")?.as_u64()?;
    let rates = parse_rates(json.get("response")?).ok()?;
    Some(Cached { rates, fetched_at })
}

fn save_cache(response: serde_json::Value, fetched_at: u64) {
    let Some(path) = cache_path() else { return };
    let Some(dir) = path.parent() else { return };
    let json = serde_json::json!({ "fetched_at": fetched_at, "response": response });
    let tmp = path.with_extension("json.tmp");
    let ok = std::fs::create_dir_all(dir).is_ok()
        && std::fs::write(&tmp, json.to_string()).is_ok()
        && std::fs::rename(&tmp, &path).is_ok();
    if !ok {
        let _ = std::fs::remove_file(&tmp);
    }
}

fn fetch() -> Result<serde_json::Value, String> {
    let agent: ureq::Agent =
        ureq::Agent::config_builder().timeout_global(Some(TIMEOUT)).build().into();
    let body = agent
        .get(URL)
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&body).map_err(|e| format!("bad JSON from rates API: {e}"))
}

fn spawn_fetch(state: &mut State, wake: &Wake) {
    state.fetching = true;
    state.last_attempt = now();
    let wake = wake.clone();
    let spawned = std::thread::Builder::new().name("currency-rates".into()).spawn(move || {
        let result = fetch().and_then(|json| parse_rates(&json).map(|rates| (json, rates)));
        let fetched_at = now();
        let mut st = STATE.lock().unwrap_or_else(|e| e.into_inner());
        st.fetching = false;
        match result {
            Ok((json, rates)) => {
                st.cache = Some(Cached { rates, fetched_at });
                st.error = None;
                drop(st);
                save_cache(json, fetched_at);
            }
            Err(e) => {
                st.error = Some(e);
                drop(st);
            }
        }
        wake();
    });
    if let Err(e) = spawned {
        state.fetching = false;
        state.error = Some(e.to_string());
    }
}

fn status_row(title: &str, subtitle: impl Into<String>) -> Answer {
    Answer { kind: Kind::Currency, title: title.into(), subtitle: subtitle.into(), action: Action::None }
}

/// The conversion for `query`, or None if it isn't one. Never blocks: the first
/// conversion typed starts a background fetch and returns a "Fetching rates…" row
/// (Action::None); `wake()` is called when the fetch finishes so the UI asks again.
pub fn answer(query: &str, wake: &Wake) -> Option<Answer> {
    let (amount, from, to) = parse(query)?;
    let mut st = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if !st.loaded {
        st.loaded = true;
        st.cache = load_cache();
    }
    let t = now();
    let stale = st.cache.as_ref().is_none_or(|c| t.saturating_sub(c.fetched_at) > MAX_AGE_SECS);
    let may_retry = st.error.is_none() || t.saturating_sub(st.last_attempt) >= RETRY_SECS;
    if stale && !st.fetching && may_retry {
        spawn_fetch(&mut st, wake);
    }
    match &st.cache {
        Some(c) => {
            let offline = (stale && st.error.is_some()).then_some(c.fetched_at as i64);
            format_answer(&c.rates, amount, from, to, offline).or_else(|| {
                Some(status_row("Rates unavailable", format!("No rate for {from} → {to}")))
            })
        }
        None if st.fetching => Some(status_row("Fetching rates…", format!("{from} → {to}"))),
        None => {
            let err = st.error.as_deref().unwrap_or("unknown error");
            Some(status_row("Rates unavailable", format!("Offline — {err}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
        "result": "success",
        "time_last_update_unix": 1756339351,
        "base_code": "EUR",
        "rates": {"EUR": 1, "USD": 1.16512, "GBP": 0.8634, "JPY": 171.5, "IRR": 49000.0,
                  "CHF": 0.9365, "PLN": 4.2601, "BRL": 6.3255, "CAD": 1.6071}
    }"#;

    fn rates() -> Rates {
        parse_rates(&serde_json::from_str(FIXTURE).unwrap()).unwrap()
    }

    fn conv(q: &str) -> Answer {
        let (a, f, t) = parse(q).unwrap_or_else(|| panic!("no parse for {q:?}"));
        format_answer(&rates(), a, f, t, None).unwrap()
    }

    #[test]
    fn parses() {
        assert_eq!(parse("123 eur to usd"), Some((123.0, "EUR", "USD")));
        assert_eq!(parse("123USD to eur"), Some((123.0, "USD", "EUR")));
        assert_eq!(parse("$120 to eur"), Some((120.0, "USD", "EUR")));
        assert_eq!(parse("120$ in eur"), Some((120.0, "USD", "EUR")));
        assert_eq!(parse("50 euros to dollars"), Some((50.0, "EUR", "USD")));
        assert_eq!(parse("(20+5) eur to usd"), Some((25.0, "EUR", "USD")));
        assert_eq!(parse("eur to usd"), Some((1.0, "EUR", "USD")));
        assert_eq!(parse("10 GBP -> JPY"), Some((10.0, "GBP", "JPY")));
        assert_eq!(parse("10gbp->jpy"), Some((10.0, "GBP", "JPY")));
        assert_eq!(parse("5 quid = bucks"), Some((5.0, "GBP", "USD")));
        assert_eq!(parse("1.5 chf as eur"), Some((1.5, "CHF", "EUR")));
        assert_eq!(parse("100 zł to €"), Some((100.0, "PLN", "EUR")));
        assert_eq!(parse("R$10 to usd"), Some((10.0, "BRL", "USD")));
        assert_eq!(parse("C$ 10 to usd"), Some((10.0, "CAD", "USD")));
        assert_eq!(parse("10 fr to eur"), Some((10.0, "CHF", "EUR")));
        assert_eq!(parse("usd 100 in eur"), Some((100.0, "USD", "EUR")));
        assert_eq!(parse("100 inr to usd"), Some((100.0, "INR", "USD")));
        assert_eq!(parse("2*50 rubles to hryvnia"), Some((100.0, "RUB", "UAH")));
    }

    #[test]
    fn rejects() {
        for q in [
            "100 km to miles", "2+2", "1,000 eur to usd", "100 eur", "eur to", "100 eur to",
            "firefox", "100 foo to usd", "100 usd to bar", "kph to mph", "100 kph to mph",
            "1/0 eur to usd", "convert to pdf", "",
        ] {
            assert_eq!(parse(q), None, "{q:?}");
        }
    }

    #[test]
    fn converts_and_formats() {
        let a = conv("123 eur to usd");
        assert_eq!(a.title, "143.31 USD");
        assert_eq!(a.subtitle, "123 EUR at 1.1651 · 28 Aug");
        assert_eq!(a.action, Action::Copy("143.31".into()));
        assert_eq!(a.kind, Kind::Currency);
        assert_eq!(conv("123USD to eur").title, "105.57 EUR");
        assert_eq!(conv("$120 to eur").title, "102.99 EUR");
        assert_eq!(conv("50 euros to dollars").title, "58.26 USD");
        assert_eq!(conv("(20+5) eur to usd").title, "29.13 USD");
        let one = conv("eur to usd");
        assert_eq!(one.title, "1.17 USD");
        assert_eq!(one.subtitle, "1 EUR at 1.1651 · 28 Aug");
        assert_eq!(conv("usd to eur").subtitle, "1 USD at 0.8583 · 28 Aug");
    }

    #[test]
    fn small_values_keep_digits() {
        let a = conv("1 irr to eur");
        assert_eq!(a.title, "0.00002041 EUR");
        assert_eq!(a.subtitle, "1 IRR at 0.00002041 · 28 Aug");
        assert_eq!(conv("0 eur to usd").title, "0.00 USD");
        assert_eq!(conv("-5 eur to usd").title, "-5.83 USD");
    }

    #[test]
    fn offline_suffix() {
        let a = format_answer(&rates(), 123.0, "EUR", "USD", Some(1756166400)).unwrap();
        assert_eq!(a.subtitle, "123 EUR at 1.1651 · 28 Aug · offline, rates from 26 Aug");
    }

    #[test]
    fn missing_currency_in_rates() {
        assert_eq!(convert(&rates(), 1.0, "EUR", "SEK"), None);
    }

    #[test]
    fn bad_responses() {
        let err = serde_json::json!({"result": "error", "error-type": "unsupported-code"});
        assert!(parse_rates(&err).unwrap_err().contains("unsupported-code"));
        assert!(parse_rates(&serde_json::json!({"result": "success"})).is_err());
    }

    #[test]
    fn days() {
        assert_eq!(fmt_day(0), "1 Jan");
        assert_eq!(fmt_day(1756339351), "28 Aug");
        assert_eq!(fmt_day(1794355352), "11 Nov");
        assert_eq!(fmt_day(951_782_400), "29 Feb");
    }

    /// Hits the network: `cargo test -- --ignored live`.
    #[test]
    #[ignore]
    fn live() {
        let dir = std::env::temp_dir().join(format!("tomboy-currency-{}", std::process::id()));
        // SAFETY: this ignored test runs alone, before anything else reads the environment.
        unsafe { std::env::set_var("XDG_CACHE_HOME", &dir) };
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = Mutex::new(tx);
        let wake: Wake = std::sync::Arc::new(move || {
            let _ = tx.lock().unwrap().send(());
        });
        let first = answer("100 eur to usd", &wake).unwrap();
        assert_eq!(first.title, "Fetching rates…");
        assert_eq!(first.action, Action::None);
        rx.recv_timeout(Duration::from_secs(10)).expect("wake not called");
        let a = answer("100 eur to usd", &wake).unwrap();
        println!("{} · {}", a.title, a.subtitle);
        assert!(a.title.ends_with(" USD"), "{a:?}");
        assert!(matches!(a.action, Action::Copy(_)));
        assert!(dir.join("tomboy/exchange-rates.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
