//! Parsing ingredient lines ("1 1/2 cups flour") into quantity, unit and name
//! so they can be scaled when the user changes the serving count.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ingredient {
    /// The line exactly as written in the source recipe.
    pub raw: String,
    pub quantity: Option<Quantity>,
    pub unit: Option<String>,
    pub name: String,
}

/// An amount, optionally a range like "2-3".
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Quantity {
    pub amount: f64,
    pub upper: Option<f64>,
}

const UNITS: &[&str] = &[
    "teaspoons", "teaspoon", "tsp", "tsps", "t",
    "tablespoons", "tablespoon", "tbsp", "tbsps", "tbs", "tbl", "T",
    "cups", "cup", "c",
    "fluid ounces", "fluid ounce", "fl oz", "fl. oz.",
    "ounces", "ounce", "oz",
    "pounds", "pound", "lbs", "lb",
    "grams", "gram", "g",
    "kilograms", "kilogram", "kg",
    "milliliters", "millilitres", "milliliter", "millilitre", "ml",
    "liters", "litres", "liter", "litre", "l",
    "pints", "pint", "pt", "quarts", "quart", "qt", "gallons", "gallon", "gal",
    "pinches", "pinch", "dashes", "dash",
    "cloves", "clove", "cans", "can", "packages", "package", "pkg",
    "sticks", "stick", "slices", "slice", "bunches", "bunch",
    "sprigs", "sprig", "heads", "head", "stalks", "stalk",
];

impl Ingredient {
    pub fn parse(line: &str) -> Self {
        let raw = line.trim().to_string();
        let normalized = normalize_fractions(&raw);
        let (quantity, rest) = parse_quantity(&normalized);
        let (unit, name) = match quantity {
            Some(_) => parse_unit(rest),
            None => (None, rest),
        };
        Self {
            raw,
            quantity,
            unit: unit.map(str::to_string),
            name: name.trim().trim_start_matches("of ").trim().to_string(),
        }
    }

    /// Render the ingredient with its quantity multiplied by `factor`.
    /// Falls back to the raw text when there is nothing to scale.
    pub fn display_scaled(&self, factor: f64) -> String {
        let Some(q) = self.quantity else {
            return self.raw.clone();
        };
        if (factor - 1.0).abs() < f64::EPSILON {
            return self.raw.clone();
        }
        let mut amount = format_amount(q.amount * factor);
        if let Some(upper) = q.upper {
            amount = format!("{amount}–{}", format_amount(upper * factor));
        }
        match &self.unit {
            Some(unit) => format!("{amount} {unit} {}", self.name),
            None => format!("{amount} {}", self.name),
        }
    }
}

fn normalize_fractions(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        let rep = match ch {
            '½' => " 1/2", '⅓' => " 1/3", '⅔' => " 2/3", '¼' => " 1/4", '¾' => " 3/4",
            '⅕' => " 1/5", '⅖' => " 2/5", '⅗' => " 3/5", '⅘' => " 4/5", '⅙' => " 1/6", '⅚' => " 5/6",
            '⅐' => " 1/7", '⅛' => " 1/8", '⅜' => " 3/8", '⅝' => " 5/8", '⅞' => " 7/8", '⅑' => " 1/9",
            '⅒' => " 1/10",
            '⁄' => "/",
            _ => {
                out.push(ch);
                continue;
            }
        };
        out.push_str(rep);
    }
    out.trim().to_string()
}

/// Reads one number token ("2", "1.5", "1,5", "1/2") from the start of `s`.
/// Returns the value, whether it was a fraction, and the remainder.
fn take_number(s: &str) -> Option<(f64, bool, &str)> {
    let s = s.trim_start();
    let mut end = s.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '/')).unwrap_or(s.len());
    let mut tok = s[..end].to_string();
    // A decimal comma, as in "1,5 kg". Not "1, 2" or "1,000".
    if let Some(after) = s[end..].strip_prefix(',') {
        let digits = after.find(|c: char| !c.is_ascii_digit()).unwrap_or(after.len());
        if (1..=2).contains(&digits) && !tok.is_empty() && tok.bytes().all(|b| b.is_ascii_digit()) {
            tok = format!("{tok}.{}", &after[..digits]);
            end += 1 + digits;
        }
    }
    let value = match tok.split_once('/') {
        Some((n, d)) => {
            let (n, d): (f64, f64) = (n.parse().ok()?, d.parse().ok()?);
            (d != 0.0).then(|| n / d).filter(|v| v.is_finite())?
        }
        None => tok.parse().ok().filter(|v: &f64| v.is_finite())?,
    };
    Some((value, tok.contains('/'), &s[end..]))
}

/// Reads a mixed number like "1", "1 1/2" or "1/2" from the start of `s`.
fn take_mixed(s: &str) -> Option<(f64, &str)> {
    let (whole, is_frac, rest) = take_number(s)?;
    if !is_frac
        && rest.starts_with(char::is_whitespace)
        && let Some((frac, true, rest)) = take_number(rest)
        && (whole + frac).is_finite()
    {
        return Some((whole + frac, rest));
    }
    Some((whole, rest))
}

fn parse_quantity(s: &str) -> (Option<Quantity>, &str) {
    let Some((amount, rest)) = take_mixed(s) else {
        return (None, s);
    };
    let after = rest.trim_start();
    let sep = ["-", "–", "to "].iter().find_map(|sep| after.strip_prefix(sep).map(|r| (*sep, r)));
    if let Some((sep, (upper, after))) = sep.and_then(|(sep, r)| Some((sep, take_mixed(r)?))) {
        if upper > amount {
            return (Some(Quantity { amount, upper: Some(upper) }), after);
        }
        // "1-1/2 cups" is one and a half, not a range down to a half.
        if sep == "-" && amount.fract() == 0.0 && upper < 1.0 {
            return (Some(Quantity { amount: amount + upper, upper: None }), after);
        }
    }
    (Some(Quantity { amount, upper: None }), rest)
}

fn parse_unit(rest: &str) -> (Option<&str>, &str) {
    let rest = rest.trim_start();
    // Longest match first so "fl oz" wins over "fl".
    let mut best: Option<&str> = None;
    for unit in UNITS {
        let case_sensitive = unit.len() == 1;
        let Some(head) = rest.get(..unit.len()) else { continue };
        let hit = if case_sensitive { head == *unit } else { head.eq_ignore_ascii_case(unit) };
        // One-letter units need a clear end, so "T-bone" isn't a tablespoon.
        let boundary = rest[unit.len()..].chars().next().is_none_or(|c| {
            if case_sensitive { c.is_whitespace() || c == '.' || c == ')' } else { !c.is_alphanumeric() }
        });
        if hit && boundary && best.is_none_or(|b| unit.len() > b.len()) {
            best = Some(unit);
        }
    }
    match best {
        Some(u) => {
            let after = rest[u.len()..].trim_start_matches('.');
            (Some(&rest[..u.len()]), after)
        }
        None => (None, rest),
    }
}

/// Formats a number using kitchen-friendly fractions where close enough.
pub fn format_amount(v: f64) -> String {
    const FRACTIONS: &[(f64, &str)] = &[
        (0.125, "1/8"), (0.25, "1/4"), (1.0 / 3.0, "1/3"), (0.375, "3/8"), (0.5, "1/2"),
        (0.625, "5/8"), (2.0 / 3.0, "2/3"), (0.75, "3/4"), (0.875, "7/8"),
    ];
    let decimal = |places: usize| {
        let s = format!("{v:.places$}");
        if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
    };
    // Too small for a kitchen fraction: one significant digit, not "0".
    if v > 0.0 && v < 0.1 {
        return decimal((-v.log10()).ceil() as usize);
    }
    // Beyond whole-number precision (and i64), plain digits.
    if !v.is_finite() || v.abs() >= 1e15 {
        return decimal(0);
    }
    let whole = v.trunc();
    let frac = v - whole;
    if frac < 0.04 {
        return format!("{}", whole as i64);
    }
    if frac > 0.96 {
        return format!("{}", whole as i64 + 1);
    }
    let nearest = FRACTIONS.iter().min_by(|(a, _), (b, _)| (frac - a).abs().total_cmp(&(frac - b).abs()));
    if let Some((_, f)) = nearest.filter(|(x, _)| (frac - x).abs() < 0.04) {
        return if whole == 0.0 { f.to_string() } else { format!("{} {f}", whole as i64) };
    }
    decimal(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(amount: f64) -> Option<Quantity> {
        Some(Quantity { amount, upper: None })
    }

    #[test]
    fn parses_simple() {
        let i = Ingredient::parse("2 cups all-purpose flour");
        assert_eq!(i.quantity, q(2.0));
        assert_eq!(i.unit.as_deref(), Some("cups"));
        assert_eq!(i.name, "all-purpose flour");
    }

    #[test]
    fn parses_mixed_and_unicode_fractions() {
        assert_eq!(Ingredient::parse("1 1/2 tsp salt").quantity, q(1.5));
        let i = Ingredient::parse("1½ tbsp olive oil");
        assert_eq!(i.quantity, q(1.5));
        assert_eq!(i.unit.as_deref(), Some("tbsp"));
        assert_eq!(i.name, "olive oil");
    }

    #[test]
    fn parses_range() {
        let i = Ingredient::parse("2-3 cloves garlic, minced");
        assert_eq!(i.quantity, Some(Quantity { amount: 2.0, upper: Some(3.0) }));
        assert_eq!(i.unit.as_deref(), Some("cloves"));
        assert_eq!(i.name, "garlic, minced");
    }

    #[test]
    fn no_quantity() {
        let i = Ingredient::parse("Salt and pepper to taste");
        assert_eq!(i.quantity, None);
        assert_eq!(i.name, "Salt and pepper to taste");
        assert_eq!(i.display_scaled(2.0), "Salt and pepper to taste");
    }

    #[test]
    fn unit_needs_word_boundary() {
        let i = Ingredient::parse("3 garlic cloves");
        assert_eq!(i.unit, None);
        assert_eq!(i.name, "garlic cloves");
        let i = Ingredient::parse("4 large eggs");
        assert_eq!(i.unit, None, "'l' must not match 'large'");
    }

    #[test]
    fn scales() {
        let i = Ingredient::parse("3/4 cup sugar");
        assert_eq!(i.display_scaled(2.0), "1 1/2 cup sugar");
        assert_eq!(i.display_scaled(1.0), "3/4 cup sugar");
        let i = Ingredient::parse("2-3 cloves garlic");
        assert_eq!(i.display_scaled(0.5), "1–1 1/2 cloves garlic");
    }

    #[test]
    fn formats_amounts() {
        assert_eq!(format_amount(0.333), "1/3");
        assert_eq!(format_amount(2.0), "2");
        assert_eq!(format_amount(1.99), "2");
        assert_eq!(format_amount(1.2), "1.2");
        assert_eq!(format_amount(0.36), "3/8", "nearest fraction, not the first close one");
        assert_eq!(format_amount(0.03125), "0.03");
        assert_eq!(format_amount(0.001), "0.001");
        assert_eq!(format_amount(1e20), "100000000000000000000");
        assert_eq!(Ingredient::parse("1/4 tsp salt").display_scaled(0.125), "0.03 tsp salt");
    }

    #[test]
    fn hyphenated_mixed_number_is_not_a_range() {
        let i = Ingredient::parse("1-1/2 cups flour");
        assert_eq!(i.quantity, q(1.5));
        assert_eq!(i.unit.as_deref(), Some("cups"));
        assert_eq!(i.display_scaled(2.0), "3 cups flour");
        assert_eq!(Ingredient::parse("1/2-1 cup milk").quantity, Some(Quantity { amount: 0.5, upper: Some(1.0) }));
        let i = Ingredient::parse("3-2 eggs");
        assert_eq!((i.quantity, i.name.as_str()), (q(3.0), "-2 eggs"), "not a range downward");
    }

    #[test]
    fn one_letter_units_need_a_clear_end() {
        let i = Ingredient::parse("4 T-bone steaks");
        assert_eq!(i.unit, None);
        assert_eq!(i.display_scaled(2.0), "8 T-bone steaks");
        assert_eq!(Ingredient::parse("2 c. sugar").unit.as_deref(), Some("c"));
        assert_eq!(Ingredient::parse("1 T butter").unit.as_deref(), Some("T"));
        assert_eq!(Ingredient::parse("1 t").unit.as_deref(), Some("t"));
    }

    #[test]
    fn more_fractions_and_decimal_commas() {
        assert_eq!(Ingredient::parse("⅙ cup water").quantity, q(1.0 / 6.0));
        assert_eq!(Ingredient::parse("2⅘ oz").quantity, q(2.8));
        assert_eq!(Ingredient::parse("⅒ l milk").quantity, q(0.1));
        let i = Ingredient::parse("1,5 kg potatoes");
        assert_eq!(i.quantity, q(1.5));
        assert_eq!((i.unit.as_deref(), i.name.as_str()), (Some("kg"), "potatoes"));
        assert_eq!(Ingredient::parse("0,25 l cream").quantity, q(0.25));
        assert_eq!(Ingredient::parse("2, 3 eggs").quantity, q(2.0));
    }

    /// An infinite amount would serialize as JSON `null`, which can't be read
    /// back, so the whole recipe file would be unreadable.
    #[test]
    fn amounts_are_always_finite() {
        let huge = "1".repeat(309);
        for line in [
            format!("{huge}/0.001 cups flour"),
            format!("{huge} cups flour"),
            format!("1 {huge}/0.001 cups flour"),
            format!("{huge}0 {huge}0/1 cups flour"),
        ] {
            let i = Ingredient::parse(&line);
            assert!(i.quantity.is_none_or(|q| q.amount.is_finite()), "{line:.40}: {:?}", i.quantity);
            let json = serde_json::to_string(&i).unwrap();
            assert_eq!(serde_json::from_str::<Ingredient>(&json).unwrap(), i);
        }
        // A whole number that fits, plus a fraction, still works.
        assert_eq!(Ingredient::parse("1 1/2 cups").quantity, q(1.5));
    }
}
