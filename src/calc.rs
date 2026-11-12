//! Tiny calculator for the launcher: a recursive-descent expression evaluator.
//!
//! Percentages work as on a phone calculator: `15% of 200` = 30, `15% off 200` = 170,
//! `200 + 15%` = 230, `200 - 15%` = 170, `200 * 15%` = 30. A `%` followed by a number, `(` or
//! a name is still modulo (`10 % 3` = 1).
//!
//! `eval` returns `None` for anything that is not clearly a math expression
//! (plain numbers, plain constants, unknown words, parse errors, NaN/inf).

use std::f64::consts::{E, PI, TAU};

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Plus,
    Minus,
    Mul,
    Div,
    Mod,
    /// Postfix `%`: divides by 100, and makes `+`/`-` work relative to the left side.
    Percent,
    /// `of` in `15% of 200`.
    Of,
    /// `off` in `15% off 200`.
    Off,
    Pow,
    LParen,
    RParen,
    Comma,
}

fn tokenize(s: &str) -> Option<Vec<Tok>> {
    let chars: Vec<char> = s.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            c if c.is_whitespace() => i += 1,
            '0'..='9' | '.' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == '_') {
                    i += 1;
                }
                let raw: String = chars[start..i].iter().collect();
                // `_` is only allowed as a single separator between digits.
                if raw.ends_with('_') || raw.contains("__") || raw.contains("_.") || raw.contains("._") {
                    return None;
                }
                let cleaned: String = raw.chars().filter(|&c| c != '_').collect();
                if !cleaned.chars().any(|c| c.is_ascii_digit()) {
                    return None;
                }
                toks.push(Tok::Num(cleaned.parse().ok()?));
            }
            c if c.is_ascii_alphabetic() => {
                let start = i;
                while i < chars.len() && chars[i].is_ascii_alphabetic() {
                    i += 1;
                }
                let mut word: String = chars[start..i].iter().collect::<String>().to_ascii_lowercase();
                if word == "log" && i < chars.len() && chars[i] == '2' {
                    word.push('2');
                    i += 1;
                }
                if word == "x" {
                    toks.push(Tok::Mul);
                } else if word == "of" {
                    toks.push(Tok::Of);
                } else if word == "off" {
                    toks.push(Tok::Off);
                } else {
                    if !is_known(&word) {
                        return None;
                    }
                    toks.push(Tok::Ident(word));
                }
            }
            '*' => {
                if chars.get(i + 1) == Some(&'*') {
                    toks.push(Tok::Pow);
                    i += 2;
                } else {
                    toks.push(Tok::Mul);
                    i += 1;
                }
            }
            _ => {
                let t = match c {
                    '+' => Tok::Plus,
                    '-' | '−' => Tok::Minus,
                    '×' | '·' => Tok::Mul,
                    '/' | '÷' => Tok::Div,
                    '%' if is_modulo(&chars[i + 1..]) => Tok::Mod,
                    '%' => Tok::Percent,
                    '^' => Tok::Pow,
                    '(' => Tok::LParen,
                    ')' => Tok::RParen,
                    ',' => Tok::Comma,
                    _ => return None,
                };
                toks.push(t);
                i += 1;
            }
        }
    }
    Some(toks)
}

/// Is the `%` before `rest` modulo? Yes when an operand follows it (`10 % 3`, `7 % (2+1)`,
/// `10 % pi`); no for `15%`, `15% of 200`, `15%)`, `200 + 15% + 1`.
fn is_modulo(rest: &[char]) -> bool {
    let rest: String = rest.iter().collect();
    let rest = rest.trim_start();
    let word: String = rest.chars().take_while(|c| c.is_ascii_alphabetic()).collect::<String>().to_ascii_lowercase();
    match rest.chars().next() {
        Some(c) if c.is_ascii_digit() || c == '.' || c == '(' => true,
        Some(c) if c.is_ascii_alphabetic() => word != "of" && word != "off",
        _ => false,
    }
}

const FUNCS: &[&str] = &[
    "sqrt", "cbrt", "abs", "ln", "log", "log2", "exp", "sin", "cos", "tan", "asin", "acos",
    "atan", "floor", "ceil", "round", "min", "max",
];

fn constant(name: &str) -> Option<f64> {
    match name {
        "pi" => Some(PI),
        "e" => Some(E),
        "tau" => Some(TAU),
        _ => None,
    }
}

fn is_known(word: &str) -> bool {
    constant(word).is_some() || FUNCS.contains(&word)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
    /// Set when the input contains an actual operation (binary op, function
    /// call, implicit multiplication) rather than a bare value.
    did_op: bool,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == Some(t) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// Did the operand just parsed end in a postfix `%`? (`15%` yes, `(50%)` no.)
    fn ended_in_percent(&self) -> bool {
        self.pos > 0 && self.toks.get(self.pos - 1) == Some(&Tok::Percent)
    }

    // expr := term (('+'|'-') term)*      a percent term adds/subtracts that share of the left
    fn expr(&mut self) -> Option<f64> {
        let (mut v, _) = self.term()?;
        loop {
            if self.eat(&Tok::Plus) {
                let (b, pct) = self.term()?;
                v = if pct { v * (1.0 + b) } else { v + b };
            } else if self.eat(&Tok::Minus) {
                let (b, pct) = self.term()?;
                v = if pct { v * (1.0 - b) } else { v - b };
            } else {
                break;
            }
            self.did_op = true;
        }
        Some(v)
    }

    // term := unary (('*'|'/'|'%') unary | <implicit> power | ('of'|'off') unary)*
    // Returns the value and whether it is a lone percentage (`15%`), for `expr`.
    fn term(&mut self) -> Option<(f64, bool)> {
        let mut v = self.unary()?;
        let mut pct = self.ended_in_percent();
        loop {
            match self.peek() {
                Some(Tok::Of) if pct => {
                    self.pos += 1;
                    v *= self.unary()?;
                }
                Some(Tok::Off) if pct => {
                    self.pos += 1;
                    v = self.unary()? * (1.0 - v);
                }
                Some(Tok::Mul) => {
                    self.pos += 1;
                    v *= self.unary()?;
                }
                Some(Tok::Div) => {
                    self.pos += 1;
                    v /= self.unary()?;
                }
                Some(Tok::Mod) => {
                    self.pos += 1;
                    v %= self.unary()?;
                }
                Some(Tok::Ident(_)) | Some(Tok::LParen) => {
                    v *= self.power()?;
                }
                _ => break,
            }
            self.did_op = true;
            pct = false;
        }
        Some((v, pct))
    }

    // unary := ('-'|'+') unary | power
    fn unary(&mut self) -> Option<f64> {
        if self.eat(&Tok::Minus) {
            Some(-self.unary()?)
        } else if self.eat(&Tok::Plus) {
            self.unary()
        } else {
            self.power()
        }
    }

    // power := primary ('^' unary)? '%'?   (right-associative; -2^2 == -4)
    fn power(&mut self) -> Option<f64> {
        let base = self.primary()?;
        let v = if self.eat(&Tok::Pow) {
            self.did_op = true;
            let exp = self.unary()?;
            base.powf(exp)
        } else {
            base
        };
        if self.eat(&Tok::Percent) {
            self.did_op = true;
            return Some(v / 100.0);
        }
        Some(v)
    }

    fn primary(&mut self) -> Option<f64> {
        match self.next()? {
            Tok::Num(n) => Some(n),
            Tok::LParen => {
                let v = self.expr()?;
                self.eat(&Tok::RParen).then_some(v)
            }
            Tok::Ident(name) => {
                if let Some(c) = constant(&name) {
                    return Some(c);
                }
                if !self.eat(&Tok::LParen) {
                    return None;
                }
                let mut args = vec![self.expr()?];
                while self.eat(&Tok::Comma) {
                    args.push(self.expr()?);
                }
                if !self.eat(&Tok::RParen) {
                    return None;
                }
                self.did_op = true;
                call(&name, &args)
            }
            _ => None,
        }
    }
}

fn call(name: &str, args: &[f64]) -> Option<f64> {
    match name {
        "min" => return args.iter().copied().reduce(f64::min),
        "max" => return args.iter().copied().reduce(f64::max),
        _ => {}
    }
    let [x] = *args else { return None };
    Some(match name {
        "sqrt" => x.sqrt(),
        "cbrt" => x.cbrt(),
        "abs" => x.abs(),
        "ln" => x.ln(),
        "log" => x.log10(),
        "log2" => x.log2(),
        "exp" => x.exp(),
        "sin" => x.sin(),
        "cos" => x.cos(),
        "tan" => x.tan(),
        "asin" => x.asin(),
        "acos" => x.acos(),
        "atan" => x.atan(),
        "floor" => x.floor(),
        "ceil" => x.ceil(),
        "round" => x.round(),
        _ => return None,
    })
}

/// Evaluate a math expression. Returns `None` if the input isn't a math
/// expression (bare number/constant, unknown words), fails to parse, or the
/// result is not finite.
pub fn eval(input: &str) -> Option<f64> {
    let s = input.trim();
    let s = s.strip_suffix('=').unwrap_or(s).trim_end();
    if s.is_empty() {
        return None;
    }
    let toks = tokenize(s)?;
    let mut p = Parser { toks, pos: 0, did_op: false };
    let v = p.expr()?;
    if p.pos != p.toks.len() || !p.did_op || !v.is_finite() {
        return None;
    }
    Some(v)
}

/// Evaluate an amount: like `eval`, but a bare number or constant (`123`, `1.5`,
/// `(20+5)`, `pi`) is accepted too. Returns `None` on parse errors or a non-finite result.
pub fn eval_amount(input: &str) -> Option<f64> {
    let s = input.trim();
    let s = s.strip_suffix('=').unwrap_or(s).trim_end();
    if s.is_empty() {
        return None;
    }
    let toks = tokenize(s)?;
    let mut p = Parser { toks, pos: 0, did_op: false };
    let v = p.expr()?;
    if p.pos != p.toks.len() || !v.is_finite() {
        return None;
    }
    Some(v)
}

/// Format a result for display: plain digits, no thousands separators.
pub fn format(v: f64) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "∞".into() } else { "-∞".into() };
    }
    if v == 0.0 {
        return "0".into();
    }
    let a = v.abs();
    if a < 1e15 && v.fract() == 0.0 {
        return format!("{}", v as i64);
    }
    if a >= 1e15 || a < 1e-4 {
        let s = format!("{:.9e}", v);
        let (mant, exp) = s.split_once('e').unwrap_or((&s, "0"));
        let mant = trim_zeros(mant);
        return format!("{mant}e{exp}");
    }
    // Keep ~15 significant digits total, at most 10 decimals.
    let int_digits = (a.log10().floor() as i32 + 1).max(1);
    let decimals = (15 - int_digits).clamp(0, 10) as usize;
    let s = trim_zeros(&format!("{:.*}", decimals, v)).to_string();
    if s == "-0" { "0".into() } else { s }
}

fn trim_zeros(s: &str) -> &str {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(s: &str) -> f64 {
        eval(s).unwrap_or_else(|| panic!("expected value for {s:?}"))
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn basics() {
        assert_eq!(ev("2+2"), 4.0);
        assert_eq!(ev("2 + 3 * 4"), 14.0);
        assert_eq!(ev("(2 + 3) * 4"), 20.0);
        assert_eq!(ev("10/4"), 2.5);
        assert_eq!(ev("10 - 4 - 3"), 3.0);
        assert_eq!(ev("7 % 3"), 1.0);
        assert_eq!(ev("2+2="), 4.0);
        assert_eq!(ev(" 1_000 * 2 "), 2000.0);
        assert_eq!(ev(".5 + .5"), 1.0);
    }

    #[test]
    fn power() {
        assert_eq!(ev("2^3^2"), 512.0);
        assert_eq!(ev("2**3"), 8.0);
        assert_eq!(ev("-2^2"), -4.0);
        assert_eq!(ev("2^-1"), 0.5);
        assert_eq!(ev("(-2)^2"), 4.0);
    }

    #[test]
    fn unary() {
        assert_eq!(ev("-3 + 5"), 2.0);
        assert_eq!(ev("--3 * 1"), 3.0);
        assert_eq!(ev("+3 * 2"), 6.0);
    }

    #[test]
    fn implicit_and_mult_symbols() {
        assert_eq!(ev("2(3+4)"), 14.0);
        assert!(close(ev("2pi"), 2.0 * PI));
        assert_eq!(ev("2 x 3"), 6.0);
        assert_eq!(ev("2x3"), 6.0);
        assert_eq!(ev("2 × 3"), 6.0);
        assert_eq!(ev("6 ÷ 3"), 2.0);
        assert_eq!(ev("(1+1)(2+2)"), 8.0);
    }

    #[test]
    fn functions_and_constants() {
        assert_eq!(eval("sqrt(16)"), Some(4.0));
        assert_eq!(ev("cbrt(27)"), 3.0);
        assert_eq!(ev("abs(-5)"), 5.0);
        assert!(close(ev("ln(e)"), 1.0));
        assert_eq!(ev("log(1000)"), 3.0);
        assert_eq!(ev("log2(8)"), 3.0);
        assert!(close(ev("sin(pi/2)"), 1.0));
        assert!(close(ev("cos(0)"), 1.0));
        assert_eq!(ev("floor(2.7) + ceil(2.1)"), 5.0);
        assert_eq!(ev("round(2.5)"), 3.0);
        assert_eq!(ev("max(1, 5, 3)"), 5.0);
        assert_eq!(ev("min(4,2)"), 2.0);
        assert!(close(ev("tau / 2"), PI));
        assert!(close(ev("exp(1)"), E));
    }

    #[test]
    fn percentages() {
        assert!(close(ev("15% of 200"), 30.0));
        assert!(close(ev("15 % OF 200"), 30.0));
        assert!(close(ev("15% off 200"), 170.0));
        assert!(close(ev("200 + 15%"), 230.0));
        assert!(close(ev("200 - 15%"), 170.0));
        assert!(close(ev("200 * 15%"), 30.0));
        assert!(close(ev("200 + 10% + 10%"), 242.0));
        assert!(close(ev("(50%) * 4"), 2.0));
        assert!(close(ev("15%"), 0.15));
        assert!(close(ev("(10+5)% of (100+100)"), 30.0));
        // `%` before an operand is still modulo.
        assert!(close(ev("10 % 3"), 1.0));
        assert!(close(ev("10%3"), 1.0));
        assert!(close(ev("7 % (2+1)"), 1.0));
    }

    #[test]
    fn rejects() {
        for s in [
            "", "2", "-2", "pi", "-pi", "(2)", "firefox", "term", "1,000", "2 +", "(2+3",
            "1/0", "0/0", "5 % 0", "sqrt(-1)", "2 3", "foo(2)", "sqrt 4", "1,2", "1__0+1",
            "1_+1", "hello world", "x", "%", "of", "15 of 200", "200 off", "15% of", "of 200",
        ] {
            assert_eq!(eval(s), None, "{s:?}");
        }
    }

    #[test]
    fn amounts() {
        assert_eq!(eval_amount("123"), Some(123.0));
        assert_eq!(eval_amount(" 1.5 "), Some(1.5));
        assert_eq!(eval_amount("(20+5)"), Some(25.0));
        assert_eq!(eval_amount("-2"), Some(-2.0));
        assert_eq!(eval_amount("2*3"), Some(6.0));
        assert_eq!(eval_amount("1_000"), Some(1000.0));
        for s in ["", "1,000", "1/0", "0/0", "km", "2 +", "(2", "2 3"] {
            assert_eq!(eval_amount(s), None, "{s:?}");
        }
        // `eval` still rejects bare values.
        assert_eq!(eval("123"), None);
    }

    #[test]
    fn formatting() {
        assert_eq!(format(ev("0.1+0.2")), "0.3");
        assert_eq!(format(4.0), "4");
        assert_eq!(format(-0.0), "0");
        assert_eq!(format(2.5), "2.5");
        assert_eq!(format(1.0 / 3.0), "0.3333333333");
        assert_eq!(format(-1.0 / 3.0), "-0.3333333333");
        assert_eq!(format(1.23e20), "1.23e20");
        assert_eq!(format(1.5e-7), "1.5e-7");
        assert_eq!(format(123456789.0), "123456789");
        assert_eq!(format(-42.0), "-42");
    }
}
