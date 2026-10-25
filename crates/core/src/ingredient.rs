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
            '⅕' => " 1/5", '⅛' => " 1/8", '⅜' => " 3/8", '⅝' => " 5/8", '⅞' => " 7/8",
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

/// Reads one number token ("2", "1.5", "1/2") from the start of `s`.
/// Returns the value, whether it was a fraction, and the remainder.
fn take_number(s: &str) -> Option<(f64, bool, &str)> {
    let s = s.trim_start();
    let end = s.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '/')).unwrap_or(s.len());
    let tok = &s[..end];
    let value = match tok.split_once('/') {
        Some((n, d)) => {
            let (n, d): (f64, f64) = (n.parse().ok()?, d.parse().ok()?);
            (d != 0.0).then(|| n / d)?
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
    let sep = ["-", "–", "to "].iter().find_map(|sep| after.strip_prefix(sep));
    if let Some((upper, rest)) = sep.and_then(take_mixed) {
        return (Some(Quantity { amount, upper: Some(upper) }), rest);
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
        let boundary = rest[unit.len()..].chars().next().is_none_or(|c| !c.is_alphanumeric());
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
    let whole = v.trunc();
    let frac = v - whole;
    if frac < 0.04 {
        return format!("{}", whole as i64);
    }
    if frac > 0.96 {
        return format!("{}", whole as i64 + 1);
    }
    if let Some((_, f)) = FRACTIONS.iter().find(|(x, _)| (frac - x).abs() < 0.04) {
        return if whole == 0.0 { f.to_string() } else { format!("{} {f}", whole as i64) };
    }
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
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
    }
}
