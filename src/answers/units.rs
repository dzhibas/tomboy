//! Unit conversion answers: `100 km to miles` → `62.1371 mi`.
//!
//! Grammar: `<amount><unit> <sep> <unit>`, where the amount may be glued to the unit
//! (`100km`), may be negative or decimal, and `<sep>` is one of `to`, `in`, `as`, `->`, `=`.
//! Both units must be known and in the same category, otherwise there is no answer.

use super::{Answer, Kind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Category {
    Length,
    Mass,
    Volume,
    Temperature,
    Data,
    Speed,
    Time,
    Area,
}

/// One unit: every spelling it answers to (lowercase), its canonical symbol, its
/// category, and how many base units (m, kg, l, K, B, m/s, s, m²) one of it is.
/// Temperatures ignore `factor` and convert through [`to_kelvin`] / [`from_kelvin`].
struct Unit {
    names: &'static [&'static str],
    symbol: &'static str,
    cat: Category,
    factor: f64,
}

const fn u(names: &'static [&'static str], symbol: &'static str, cat: Category, factor: f64) -> Unit {
    Unit { names, symbol, cat, factor }
}

use Category::*;

const KIB: f64 = 1024.0;

static UNITS: &[Unit] = &[
    // Length (base: metre)
    u(&["mm", "millimetre", "millimetres", "millimeter", "millimeters"], "mm", Length, 0.001),
    u(&["cm", "centimetre", "centimetres", "centimeter", "centimeters"], "cm", Length, 0.01),
    u(&["m", "metre", "metres", "meter", "meters"], "m", Length, 1.0),
    u(&["km", "kms", "kilometre", "kilometres", "kilometer", "kilometers"], "km", Length, 1000.0),
    u(&["in", "inch", "inches", "\""], "in", Length, 0.0254),
    u(&["ft", "foot", "feet", "'"], "ft", Length, 0.3048),
    u(&["yd", "yds", "yard", "yards"], "yd", Length, 0.9144),
    u(&["mi", "mile", "miles"], "mi", Length, 1609.344),
    u(&["nmi", "nautical mile", "nautical miles"], "nmi", Length, 1852.0),
    // Mass (base: kilogram)
    u(&["mg", "milligram", "milligrams"], "mg", Mass, 1e-6),
    u(&["g", "gram", "grams", "gramme", "grammes"], "g", Mass, 0.001),
    u(&["kg", "kgs", "kilo", "kilos", "kilogram", "kilograms", "kilogramme", "kilogrammes"], "kg", Mass, 1.0),
    u(&["t", "tonne", "tonnes", "metric ton", "metric tons"], "t", Mass, 1000.0),
    u(&["lb", "lbs", "pound", "pounds"], "lb", Mass, 0.453_592_37),
    u(&["oz", "ounce", "ounces"], "oz", Mass, 0.028_349_523_125),
    u(&["st", "stone", "stones"], "st", Mass, 6.350_293_18),
    // Volume (base: litre; US customary)
    u(&["ml", "millilitre", "millilitres", "milliliter", "milliliters"], "ml", Volume, 0.001),
    u(&["cl", "centilitre", "centilitres", "centiliter", "centiliters"], "cl", Volume, 0.01),
    u(&["dl", "decilitre", "decilitres", "deciliter", "deciliters"], "dl", Volume, 0.1),
    u(&["l", "litre", "litres", "liter", "liters"], "l", Volume, 1.0),
    u(&["m³", "m3", "cubic metre", "cubic metres", "cubic meter", "cubic meters"], "m³", Volume, 1000.0),
    u(&["gal", "gallon", "gallons"], "gal", Volume, 3.785_411_784),
    u(&["qt", "quart", "quarts"], "qt", Volume, 0.946_352_946),
    u(&["pt", "pint", "pints"], "pt", Volume, 0.473_176_473),
    u(&["cup", "cups"], "cup", Volume, 0.236_588_236_5),
    u(&["fl oz", "floz", "fl. oz", "fl.oz", "fluid ounce", "fluid ounces"], "fl oz", Volume, 0.029_573_529_562_5),
    u(&["tbsp", "tablespoon", "tablespoons"], "tbsp", Volume, 0.014_786_764_781_25),
    u(&["tsp", "teaspoon", "teaspoons"], "tsp", Volume, 0.004_928_921_593_75),
    // Temperature (factor unused)
    u(&["c", "°c", "degc", "deg c", "celsius", "centigrade"], "°C", Temperature, 0.0),
    u(&["f", "°f", "degf", "deg f", "fahrenheit"], "°F", Temperature, 0.0),
    u(&["k", "kelvin", "kelvins"], "K", Temperature, 0.0),
    // Data (base: byte)
    u(&["b", "byte", "bytes"], "B", Data, 1.0),
    u(&["kb", "kilobyte", "kilobytes"], "KB", Data, 1e3),
    u(&["mb", "megabyte", "megabytes"], "MB", Data, 1e6),
    u(&["gb", "gigabyte", "gigabytes"], "GB", Data, 1e9),
    u(&["tb", "terabyte", "terabytes"], "TB", Data, 1e12),
    u(&["pb", "petabyte", "petabytes"], "PB", Data, 1e15),
    u(&["kib", "kibibyte", "kibibytes"], "KiB", Data, KIB),
    u(&["mib", "mebibyte", "mebibytes"], "MiB", Data, KIB * KIB),
    u(&["gib", "gibibyte", "gibibytes"], "GiB", Data, KIB * KIB * KIB),
    u(&["tib", "tebibyte", "tebibytes"], "TiB", Data, KIB * KIB * KIB * KIB),
    u(&["pib", "pebibyte", "pebibytes"], "PiB", Data, KIB * KIB * KIB * KIB * KIB),
    // Speed (base: metre per second)
    u(&["m/s", "mps", "metres per second", "meters per second"], "m/s", Speed, 1.0),
    u(&["km/h", "kph", "kmh", "kmph", "km/hr", "kilometres per hour", "kilometers per hour"], "km/h", Speed, 1.0 / 3.6),
    u(&["mph", "mi/h", "miles per hour"], "mph", Speed, 0.447_04),
    u(&["kn", "kt", "kts", "knot", "knots"], "kn", Speed, 1852.0 / 3600.0),
    u(&["ft/s", "fps", "feet per second"], "ft/s", Speed, 0.3048),
    // Time (base: second)
    u(&["ms", "msec", "millisecond", "milliseconds"], "ms", Time, 0.001),
    u(&["s", "sec", "secs", "second", "seconds"], "s", Time, 1.0),
    u(&["min", "mins", "minute", "minutes"], "min", Time, 60.0),
    u(&["h", "hr", "hrs", "hour", "hours"], "h", Time, 3600.0),
    u(&["d", "day", "days"], "d", Time, 86_400.0),
    u(&["wk", "wks", "week", "weeks"], "wk", Time, 604_800.0),
    u(&["yr", "yrs", "y", "year", "years"], "yr", Time, 31_557_600.0), // Julian year, 365.25 d
    // Area (base: square metre)
    u(&["cm²", "cm2", "sq cm", "sqcm", "square centimetre", "square centimetres", "square centimeter", "square centimeters"], "cm²", Area, 1e-4),
    u(&["m²", "m2", "sq m", "sqm", "square metre", "square metres", "square meter", "square meters"], "m²", Area, 1.0),
    u(&["km²", "km2", "sq km", "sqkm", "square kilometre", "square kilometres", "square kilometer", "square kilometers"], "km²", Area, 1e6),
    u(&["in²", "in2", "sq in", "sqin", "square inch", "square inches"], "in²", Area, 0.000_645_16),
    u(&["ft²", "ft2", "sq ft", "sqft", "square foot", "square feet"], "ft²", Area, 0.092_903_04),
    u(&["yd²", "yd2", "sq yd", "sqyd", "square yard", "square yards"], "yd²", Area, 0.836_127_36),
    u(&["mi²", "mi2", "sq mi", "sqmi", "square mile", "square miles"], "mi²", Area, 2_589_988.110_336),
    u(&["ac", "acre", "acres"], "ac", Area, 4_046.856_422_4),
    u(&["ha", "hectare", "hectares"], "ha", Area, 10_000.0),
];

/// Separators, tried in this order. ` in ` is last because `in` is also inches.
const SEPARATORS: &[&str] = &["->", "=", " to ", " as ", " in "];

/// Answers `<amount><unit> <sep> <unit>` queries; `None` for anything else.
pub fn answer(query: &str) -> Option<Answer> {
    let query = query.trim();
    let first = query.chars().next()?;
    if !(first.is_ascii_digit() || matches!(first, '-' | '+' | '.')) {
        return None;
    }
    let lower = query.to_lowercase();
    for sep in SEPARATORS {
        // Overlapping search, so `10 in in cm` also tries the second ` in `.
        let mut from = 0;
        while let Some(found) = lower[from..].find(sep) {
            let at = from + found;
            if let Some(answer) = convert(&lower[..at], &lower[at + sep.len()..]) {
                return Some(answer);
            }
            from = at + 1;
            while !lower.is_char_boundary(from) {
                from += 1;
            }
        }
    }
    None
}

fn convert(lhs: &str, rhs: &str) -> Option<Answer> {
    let (amount, from) = split_amount(lhs.trim())?;
    let from = lookup(from)?;
    let to = lookup(rhs)?;
    if from.cat != to.cat {
        return None;
    }
    let result = if from.cat == Temperature {
        from_kelvin(to.symbol, to_kelvin(from.symbol, amount))
    } else {
        amount * from.factor / to.factor
    };
    if !result.is_finite() {
        return None;
    }
    let title = format!("{} {}", format_number(result), to.symbol);
    let subtitle = format!("{} {} = {title}", format_number(amount), from.symbol);
    Some(Answer::copy(Kind::Units, title, subtitle))
}

/// Splits `-12.5km` / `100 km` into the number and the (trimmed) unit text after it.
fn split_amount(s: &str) -> Option<(f64, &str)> {
    let bytes = s.as_bytes();
    let mut end = 0;
    if matches!(bytes.first(), Some(b'-' | b'+')) {
        end = 1;
    }
    let mut digits = 0;
    let mut dots = 0;
    while let Some(&b) = bytes.get(end) {
        match b {
            b'0'..=b'9' => digits += 1,
            b'.' if dots == 0 => dots += 1,
            _ => break,
        }
        end += 1;
    }
    if digits == 0 {
        return None;
    }
    let amount: f64 = s[..end].parse().ok()?;
    let unit = s[end..].trim();
    (!unit.is_empty() && amount.is_finite()).then_some((amount, unit))
}

/// Finds a unit by any of its spellings; internal whitespace runs count as one space.
fn lookup(name: &str) -> Option<&'static Unit> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let normalized;
    let name = if name.contains(char::is_whitespace) {
        normalized = name.split_whitespace().collect::<Vec<_>>().join(" ");
        normalized.as_str()
    } else {
        name
    };
    UNITS.iter().find(|unit| unit.names.contains(&name))
}

fn to_kelvin(symbol: &str, v: f64) -> f64 {
    match symbol {
        "°C" => v + 273.15,
        "°F" => (v - 32.0) * 5.0 / 9.0 + 273.15,
        _ => v,
    }
}

fn from_kelvin(symbol: &str, k: f64) -> f64 {
    match symbol {
        "°C" => k - 273.15,
        "°F" => (k - 273.15) * 9.0 / 5.0 + 32.0,
        _ => k,
    }
}

/// Up to 6 significant digits, trailing zeros trimmed; values of a million or more
/// are rounded to whole numbers instead (all digits kept) and exponent form is used
/// outside `[1e-4, 1e15)`.
fn format_number(v: f64) -> String {
    const SIG: i32 = 6;
    if v == 0.0 {
        return "0".to_string();
    }
    let abs = v.abs();
    if !(1e-4..1e15).contains(&abs) {
        let s = format!("{:.*e}", (SIG - 1) as usize, v);
        return match s.split_once('e') {
            Some((mantissa, exp)) => format!("{}e{exp}", trim_zeros(mantissa)),
            None => s,
        };
    }
    let magnitude = abs.log10().floor() as i32;
    let decimals = (SIG - 1 - magnitude).max(0) as usize;
    let s = format!("{v:.decimals$}");
    let s = trim_zeros(&s);
    if s == "-0" { "0".to_string() } else { s.to_string() }
}

fn trim_zeros(s: &str) -> &str {
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.') } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn title(q: &str) -> String {
        answer(q).unwrap_or_else(|| panic!("no answer for {q:?}")).title
    }

    #[test]
    fn spec_table() {
        assert_eq!(title("100 km to miles"), "62.1371 mi");
        assert_eq!(title("20 c to f"), "68 °F");
        assert_eq!(title("5 gb to mb"), "5000 MB");
        assert_eq!(title("1 tb to gib"), "931.323 GiB");
        assert_eq!(title("70 kg to lb"), "154.324 lb");
        assert_eq!(title("2 cup to ml"), "473.176 ml");
    }

    #[test]
    fn subtitle_and_action() {
        let a = answer("100 km to miles").unwrap();
        assert_eq!(a.kind, Kind::Units);
        assert_eq!(a.subtitle, "100 km = 62.1371 mi");
        assert_eq!(a.action, super::super::Action::Copy("62.1371 mi".into()));
    }

    #[test]
    fn separators_and_gluing() {
        assert_eq!(title("100km to mi"), "62.1371 mi");
        assert_eq!(title("100 km in mi"), "62.1371 mi");
        assert_eq!(title("100 km as mi"), "62.1371 mi");
        assert_eq!(title("100km->mi"), "62.1371 mi");
        assert_eq!(title("100 km = mi"), "62.1371 mi");
        assert_eq!(title("100 KM TO MILES"), "62.1371 mi");
        assert_eq!(title("  100   km   to   mi  "), "62.1371 mi");
    }

    #[test]
    fn inches_vs_in_separator() {
        assert_eq!(title("10 in to cm"), "25.4 cm");
        assert_eq!(title("10 in in cm"), "25.4 cm");
        assert_eq!(title("1 ft in in"), "12 in");
    }

    #[test]
    fn temperatures() {
        assert_eq!(title("-40 c to f"), "-40 °F");
        assert_eq!(title("212 fahrenheit to celsius"), "100 °C");
        assert_eq!(title("0 k to c"), "-273.15 °C");
        assert_eq!(title("37.5 °c to kelvin"), "310.65 K");
        assert_eq!(title("32 f to c"), "0 °C");
        assert_eq!(answer("-40 c to f").unwrap().subtitle, "-40 °C = -40 °F");
    }

    #[test]
    fn data_ladders() {
        assert_eq!(title("1 gib to mib"), "1024 MiB");
        assert_eq!(title("1 mib to bytes"), "1048576 B");
        assert_eq!(title("1 pb to tb"), "1000 TB");
        assert_eq!(title("1 tb to b"), "1000000000000 B");
        assert_eq!(title("1 kib to kb"), "1.024 KB");
    }

    #[test]
    fn other_categories() {
        assert_eq!(title("1 mile to feet"), "5280 ft");
        assert_eq!(title("1 stone to pounds"), "14 lb");
        assert_eq!(title("1 gallon to litres"), "3.78541 l");
        assert_eq!(title("1 fl oz to ml"), "29.5735 ml");
        assert_eq!(title("1 floz to ml"), "29.5735 ml");
        assert_eq!(title("3 teaspoons to tablespoons"), "1 tbsp");
        assert_eq!(title("2 pints to quarts"), "1 qt");
        assert_eq!(title("100 km/h to mph"), "62.1371 mph");
        assert_eq!(title("10 m/s to kph"), "36 km/h");
        assert_eq!(title("1 knot to km/h"), "1.852 km/h");
        assert_eq!(title("2 hours to minutes"), "120 min");
        assert_eq!(title("1 week to days"), "7 d");
        assert_eq!(title("1500 ms to s"), "1.5 s");
        assert_eq!(title("1 year to days"), "365.25 d");
        assert_eq!(title("1 ha to m²"), "10000 m²");
        assert_eq!(title("1 acre to sq ft"), "43560 ft²");
        assert_eq!(title("1 km² to hectares"), "100 ha");
        assert_eq!(title("100 sqm to sq ft"), "1076.39 ft²");
        assert_eq!(title("1 kilometre to metres"), "1000 m");
        assert_eq!(title("1 kilometer to meters"), "1000 m");
        assert_eq!(title("1 yard to inches"), "36 in");
        assert_eq!(title("1 kg to grams"), "1000 g");
        assert_eq!(title("1 lb to ounces"), "16 oz");
        assert_eq!(title(".5 l to ml"), "500 ml");
    }

    #[test]
    fn formatting() {
        assert_eq!(format_number(62.137_119_2), "62.1371");
        assert_eq!(format_number(5000.0), "5000");
        assert_eq!(format_number(123_456_789.4), "123456789");
        assert_eq!(format_number(999_999.7), "1000000");
        assert_eq!(format_number(-0.000_000_1), "-1e-7");
        assert_eq!(format_number(2.5e20), "2.5e20");
        assert_eq!(format_number(0.000_123_456_7), "0.000123457");
        assert_eq!(format_number(-0.0), "0");
    }

    #[test]
    fn rejections() {
        for q in [
            "5 kg to km",
            "firefox",
            "2+2",
            "",
            "   ",
            "100",
            "100 km",
            "100 km to",
            "km to mi",
            "100 furlongs to m",
            "100 km to parsecs",
            "1 mb to mph",
            "- km to mi",
            "1.2.3 km to mi",
            "20 c to kg",
        ] {
            assert_eq!(answer(q), None, "{q:?}");
        }
    }
}
