//! Developer one-liners: `uuid`, `password 24`, `sha256 text`, `base64 text`, `b64d …`,
//! `urlencode …`, `urldecode …`, `epoch [n]`.
//!
//! Random answers (uuid, password) are derived from a per-launch seed plus the query, so
//! they hold still while the user types and change on the next launch.

use super::{Answer, Kind};
use sha2::{Digest, Sha256};

/// 32 random bytes from the OS (/dev/urandom), read once per launch by the caller.
pub fn session_seed() -> [u8; 32] {
    use std::io::Read;
    let mut seed = [0u8; 32];
    let read = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut seed));
    if read.is_err() {
        // Practically unreachable on Linux/macOS; fall back to something launch-unique.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let mut h = Sha256::new();
        h.update(nanos.to_le_bytes());
        h.update(std::process::id().to_le_bytes());
        h.update((&seed as *const _ as usize).to_le_bytes());
        seed.copy_from_slice(&h.finalize());
    }
    seed
}

pub fn answer(query: &str, seed: &[u8; 32]) -> Option<Answer> {
    let query = query.trim_start();
    let (keyword, arg) = match query.find(char::is_whitespace) {
        Some(i) => {
            let sep = query[i..].chars().next().map_or(1, char::len_utf8);
            (&query[..i], &query[i + sep..])
        }
        None => (query, ""),
    };
    if keyword.len() > 9 {
        return None;
    }
    let keyword = keyword.to_ascii_lowercase();
    let dev = |title: String, subtitle: String| Some(Answer::copy(Kind::Dev, title, subtitle));
    match keyword.as_str() {
        "uuid" => {
            if !arg.trim().is_empty() {
                return None;
            }
            dev(uuid_v4(seed), "UUID v4".into())
        }
        "password" | "pass" | "pwd" => {
            let arg = arg.trim();
            let len = if arg.is_empty() {
                20
            } else {
                if !arg.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                arg.parse::<usize>().unwrap_or(usize::MAX).clamp(4, 256)
            };
            dev(password(seed, len), format!("{len}-character password"))
        }
        "sha256" => {
            if arg.is_empty() {
                return None;
            }
            dev(hex(&Sha256::digest(arg.as_bytes())), format!("SHA-256 of \"{}\"", preview(arg)))
        }
        "base64" | "b64" => {
            if arg.is_empty() {
                return None;
            }
            dev(base64_encode(arg.as_bytes()), format!("Base64 of \"{}\"", preview(arg)))
        }
        "b64d" | "unbase64" => {
            let bytes = base64_decode(arg.trim())?;
            let text = String::from_utf8(bytes).ok()?;
            if text.is_empty() {
                return None;
            }
            dev(text, "Decoded from Base64".into())
        }
        "urlencode" => {
            if arg.is_empty() {
                return None;
            }
            dev(url_encode(arg), "URL-encoded".into())
        }
        "urldecode" => {
            if arg.is_empty() {
                return None;
            }
            dev(url_decode(arg)?, "URL-decoded".into())
        }
        "epoch" => {
            let arg = arg.trim();
            if arg.is_empty() {
                return dev(jiff::Timestamp::now().as_second().to_string(), "Unix seconds now".into());
            }
            let (title, subtitle) = format_epoch(arg, &jiff::tz::TimeZone::system())?;
            dev(title, subtitle)
        }
        _ => None,
    }
}

/// Formats a Unix timestamp (10-ish digits = seconds, 13 digits = milliseconds) in `tz`.
fn format_epoch(arg: &str, tz: &jiff::tz::TimeZone) -> Option<(String, String)> {
    if arg.is_empty() || arg.len() > 13 || !arg.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: i64 = arg.parse().ok()?;
    let (ts, unit) = if arg.len() == 13 {
        (jiff::Timestamp::from_millisecond(n).ok()?, "Unix ms")
    } else {
        (jiff::Timestamp::from_second(n).ok()?, "Unix")
    };
    let title = ts.to_zoned(tz.clone()).strftime("%a %-d %b %Y %H:%M:%S").to_string();
    let zone = tz.iana_name().unwrap_or("local time");
    Some((title, format!("{unit} {n} in {zone}")))
}

/// Shortens user text for a subtitle.
fn preview(text: &str) -> String {
    const MAX: usize = 40;
    if text.chars().count() <= MAX {
        text.to_string()
    } else {
        let mut s: String = text.chars().take(MAX - 1).collect();
        s.push('…');
        s
    }
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 15) as usize] as char);
    }
    s
}

// ---------------------------------------------------------------------------------------
// Deterministic randomness: SHA-256(seed ‖ domain ‖ len(param) ‖ param ‖ counter) blocks.

struct KeyStream {
    seed: [u8; 32],
    domain: &'static [u8],
    param: u64,
    counter: u64,
    block: [u8; 32],
    pos: usize,
}

impl KeyStream {
    fn new(seed: &[u8; 32], domain: &'static [u8], param: u64) -> Self {
        KeyStream { seed: *seed, domain, param, counter: 0, block: [0; 32], pos: 32 }
    }

    fn byte(&mut self) -> u8 {
        if self.pos == 32 {
            let mut h = Sha256::new();
            h.update(self.seed);
            h.update((self.domain.len() as u64).to_le_bytes());
            h.update(self.domain);
            h.update(self.param.to_le_bytes());
            h.update(self.counter.to_le_bytes());
            self.block.copy_from_slice(&h.finalize());
            self.counter += 1;
            self.pos = 0;
        }
        let b = self.block[self.pos];
        self.pos += 1;
        b
    }

    fn u32(&mut self) -> u32 {
        u32::from_le_bytes([self.byte(), self.byte(), self.byte(), self.byte()])
    }

    /// Uniform in `0..n` (n > 0) via rejection sampling.
    fn below(&mut self, n: u32) -> u32 {
        let zone = u32::MAX - (u32::MAX - n + 1) % n; // largest multiple of n, minus 1
        loop {
            let v = self.u32();
            if v <= zone {
                return v % n;
            }
        }
    }
}

fn uuid_v4(seed: &[u8; 32]) -> String {
    let mut ks = KeyStream::new(seed, b"uuid", 0);
    let mut b = [0u8; 16];
    b.iter_mut().for_each(|x| *x = ks.byte());
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex(&b);
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

const UPPER: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const LOWER: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
const DIGITS: &[u8] = b"0123456789";
const SYMBOLS: &[u8] = b"!@#$%^&*-_=+?";

fn password(seed: &[u8; 32], len: usize) -> String {
    let mut ks = KeyStream::new(seed, b"password", len as u64);
    let all: Vec<u8> = [UPPER, LOWER, DIGITS, SYMBOLS].concat();
    let pick = |ks: &mut KeyStream, set: &[u8]| set[ks.below(set.len() as u32) as usize];
    let mut out: Vec<u8> = Vec::with_capacity(len);
    if len >= 4 {
        for class in [UPPER, LOWER, DIGITS, SYMBOLS] {
            out.push(pick(&mut ks, class));
        }
    }
    while out.len() < len {
        out.push(pick(&mut ks, &all));
    }
    // Fisher-Yates so the guaranteed characters land anywhere.
    for i in (1..out.len()).rev() {
        let j = ks.below(i as u32 + 1) as usize;
        out.swap(i, j);
    }
    out.into_iter().map(char::from).collect()
}

// ---------------------------------------------------------------------------------------
// Base64 and URL encoding.

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                s.push(B64[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

/// Decodes standard or URL-safe Base64, padded or not. `None` on any invalid input.
fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let body = text.strip_suffix("==").or_else(|| text.strip_suffix('=')).unwrap_or(text);
    if body.is_empty() || body.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(body.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0;
    for c in body.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

fn url_encode(text: &str) -> String {
    let mut s = String::with_capacity(text.len() * 3);
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

fn url_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => {
                let hi = (*bytes.get(i + 1)? as char).to_digit(16)?;
                let lo = (*bytes.get(i + 2)? as char).to_digit(16)?;
                out.push((hi * 16 + lo) as u8);
                i += 2;
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: [u8; 32] = [7; 32];

    fn title(q: &str) -> Option<String> {
        answer(q, &SEED).map(|a| a.title)
    }

    #[test]
    fn uuid_format() {
        let a = answer("uuid", &SEED).unwrap();
        assert_eq!(a.kind, Kind::Dev);
        assert_eq!(a.subtitle, "UUID v4");
        let u = a.title;
        assert_eq!(u.len(), 36);
        let parts: Vec<&str> = u.split('-').collect();
        assert_eq!(parts.iter().map(|p| p.len()).collect::<Vec<_>>(), [8, 4, 4, 4, 12]);
        assert!(u.chars().all(|c| c == '-' || c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_eq!(&u[14..15], "4");
        assert!(matches!(&u[19..20], "8" | "9" | "a" | "b"));
        assert_eq!(title("UUID"), Some(u.clone()));
        assert_ne!(answer("uuid", &[8; 32]).unwrap().title, u);
        assert_eq!(title("uuid foo"), None);
    }

    fn classes_ok(p: &str) -> bool {
        let has = |set: &[u8]| p.bytes().any(|b| set.contains(&b));
        has(UPPER) && has(LOWER) && has(DIGITS) && has(SYMBOLS)
            && p.bytes().all(|b| [UPPER, LOWER, DIGITS, SYMBOLS].iter().any(|s| s.contains(&b)))
    }

    #[test]
    fn password_lengths_and_classes() {
        let a = answer("password 24", &SEED).unwrap();
        assert_eq!(a.title.len(), 24);
        assert_eq!(a.subtitle, "24-character password");
        assert!(classes_ok(&a.title));
        assert_eq!(title("password").unwrap().len(), 20);
        assert_eq!(title("pass 30").unwrap().len(), 30);
        assert_eq!(title("PWD 8").unwrap().len(), 8);
        assert_eq!(title("password 1").unwrap().len(), 4);
        assert_eq!(title("password 9999").unwrap().len(), 256);
        assert_eq!(title("password 99999999999999999999999").unwrap().len(), 256);
        for len in 4..=64 {
            for s in 0..8u8 {
                let p = password(&[s; 32], len);
                assert_eq!(p.len(), len);
                assert!(classes_ok(&p), "{p}");
            }
        }
        assert_eq!(title("password abc"), None);
    }

    #[test]
    fn password_determinism() {
        assert_eq!(title("password 24"), title("password 24"));
        assert_ne!(answer("password 24", &[1; 32]), answer("password 24", &[2; 32]));
        assert_ne!(title("password 24"), title("password 25").map(|p| p[..24].to_string()));
    }

    #[test]
    fn sha256_vector() {
        let a = answer("sha256 abc", &SEED).unwrap();
        assert_eq!(a.title, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(a.subtitle, "SHA-256 of \"abc\"");
        assert_eq!(answer("sha256 omarchy", &SEED).unwrap().subtitle, "SHA-256 of \"omarchy\"");
        assert_eq!(title("sha256"), None);
        assert_eq!(title("sha256 "), None);
    }

    #[test]
    fn base64_rows() {
        let a = answer("base64 hello world", &SEED).unwrap();
        assert_eq!(a.title, "aGVsbG8gd29ybGQ=");
        assert_eq!(a.subtitle, "Base64 of \"hello world\"");
        assert_eq!(title("b64 hello world").unwrap(), "aGVsbG8gd29ybGQ=");
        assert_eq!(title("B64 a  b").unwrap(), base64_encode(b"a  b"));
        let d = answer("b64d aGVsbG8=", &SEED).unwrap();
        assert_eq!(d.title, "hello");
        assert_eq!(d.subtitle, "Decoded from Base64");
        assert_eq!(title("unbase64 aGVsbG8").unwrap(), "hello");
        assert_eq!(title("b64d aGVsbG8gd29ybGQ").unwrap(), "hello world");
        assert_eq!(title("b64d"), None);
        assert_eq!(title("b64d a"), None);
        assert_eq!(title("b64d !!!!"), None);
        assert_eq!(title("b64d //8="), None); // 0xff 0xff: not UTF-8
    }

    #[test]
    fn base64_round_trips() {
        for len in 0..40usize {
            let data: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            let enc = base64_encode(&data);
            assert_eq!(enc.len() % 4, 0);
            if len > 0 {
                assert_eq!(base64_decode(&enc).unwrap(), data);
                let unpadded = enc.trim_end_matches('=');
                assert_eq!(base64_decode(unpadded).unwrap(), data);
                let url = unpadded.replace('+', "-").replace('/', "_");
                assert_eq!(base64_decode(&url).unwrap(), data);
            }
        }
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_decode("-_8").unwrap(), [0xfb, 0xff]);
    }

    #[test]
    fn url_rows() {
        let a = answer("urlencode a b&c", &SEED).unwrap();
        assert_eq!(a.title, "a%20b%26c");
        assert_eq!(a.subtitle, "URL-encoded");
        assert_eq!(url_encode("Az09-_.~ é/"), "Az09-_.~%20%C3%A9%2F");
        let d = answer("urldecode a%20b", &SEED).unwrap();
        assert_eq!(d.title, "a b");
        assert_eq!(d.subtitle, "URL-decoded");
        assert_eq!(title("urldecode a+b%2Bc").unwrap(), "a b+c");
        assert_eq!(title("urldecode %C3%a9").unwrap(), "é");
        assert_eq!(title("urldecode %zz"), None);
        assert_eq!(title("urldecode %4"), None);
        assert_eq!(title("urldecode %FF"), None);
        assert_eq!(title("urlencode"), None);
        assert_eq!(url_decode(&url_encode("x y&z=ü?")).unwrap(), "x y&z=ü?");
    }

    #[test]
    fn epoch_rows() {
        let now = answer("epoch", &SEED).unwrap();
        assert_eq!(now.subtitle, "Unix seconds now");
        assert!(now.title.parse::<i64>().unwrap() > 1_700_000_000);

        let vilnius = jiff::tz::TimeZone::fixed(jiff::tz::offset(3));
        let (t, _) = format_epoch("1787875351", &vilnius).unwrap();
        assert_eq!(t, "Fri 28 Aug 2026 03:02:31");
        let (t, s) = format_epoch("1787875351", &jiff::tz::TimeZone::UTC).unwrap();
        assert_eq!(t, "Fri 28 Aug 2026 00:02:31");
        assert_eq!(s, "Unix 1787875351 in UTC");
        let (t, s) = format_epoch("1787875351000", &jiff::tz::TimeZone::UTC).unwrap();
        assert_eq!(t, "Fri 28 Aug 2026 00:02:31");
        assert_eq!(s, "Unix ms 1787875351000 in UTC");
        assert_eq!(format_epoch("0", &jiff::tz::TimeZone::UTC).unwrap().0, "Thu 1 Jan 1970 00:00:00");

        let a = answer("epoch 1787875351", &SEED).unwrap();
        assert!(a.subtitle.starts_with("Unix 1787875351 in "));
        assert!(a.title.contains("Aug 2026"));
        assert_eq!(title("epoch abc"), None);
        assert_eq!(title("epoch -5"), None);
        assert_eq!(title("epoch 99999999999999"), None);
    }

    #[test]
    fn rejections() {
        for q in ["", "  ", "hello", "uuidx", "sha", "passwords 3", "epochs", "firefox", "base64x hi"] {
            assert_eq!(answer(q, &SEED), None, "{q:?}");
        }
    }

    #[test]
    fn seed_is_random() {
        assert_ne!(session_seed(), session_seed());
    }
}
