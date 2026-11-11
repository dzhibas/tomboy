//! Answers: queries that come back with a result at the head of the list instead of only
//! filtering — unit and currency conversion, time zones, developer utilities and killing
//! processes. Each feature is one file with an `answer`/`answers` function returning
//! [`Answer`]s; the calculator stays in `crate::calc`.

mod types;
pub use types::*;

pub mod currency;
pub mod devtools;
pub mod kill;
pub mod timezone;
pub mod units;

/// Process rows shown for one `kill …` query.
const MAX_PROCESSES: usize = 10;

/// Every answer for `query`, in display order. Cheap for ordinary searches: each provider
/// bails out early on input that isn't its kind.
pub fn collect(query: &str, seed: &[u8; 32], wake: &Wake) -> Vec<Answer> {
    if query.trim().is_empty() { return Vec::new(); }
    let mut out = Vec::new();
    out.extend(devtools::answer(query, seed));
    out.extend(units::answer(query));
    out.extend(currency::answer(query, wake));
    out.extend(timezone::answer(query));
    out.extend(kill::answers(query, MAX_PROCESSES));
    out
}
