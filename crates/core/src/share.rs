//! Share links: the recipe fields that leave the device when the user taps
//! Share, and their schema.org JSON-LD form so any recipe app (including
//! Hestia's own importer) can read a shared page.
//!
//! Only the fields below are ever uploaded. Notes, the favorite flag, ids,
//! timestamps and stored photos stay on the device.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::model::Recipe;
use crate::url::is_web_url;

/// What the app uploads to create a link. The server rejects unknown fields.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedRecipe {
    pub title: String,
    pub description: Option<String>,
    pub author: Option<String>,
    /// Where the recipe originally came from, kept for attribution.
    pub source_url: Option<String>,
    /// A web link to a photo; local photo files are never uploaded.
    pub image_url: Option<String>,
    pub servings: Option<u32>,
    pub yield_text: Option<String>,
    pub prep_minutes: Option<u32>,
    pub cook_minutes: Option<u32>,
    pub total_minutes: Option<u32>,
    /// Ingredient lines as written.
    pub ingredients: Vec<String>,
    pub steps: Vec<String>,
    pub tags: Vec<String>,
}

/// The server's reply to a successful share.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShareLink {
    pub url: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum ShareError {
    #[error("a shared recipe needs a title")]
    Title,
    #[error("{0} is too long")]
    TooLong(&'static str),
    #[error("too many {0}")]
    TooMany(&'static str),
    #[error("{0} must be an http(s) link")]
    Url(&'static str),
    #[error("{0} out of range")]
    Range(&'static str),
}

/// How long a share link works before the server deletes it.
pub const SHARE_DAYS: i64 = 90;

const MAX_TITLE: usize = 200;
const MAX_SHORT: usize = 200;
const MAX_TEXT: usize = 5_000;
const MAX_URL: usize = 2_000;
const MAX_LINES: usize = 200;
const MAX_TAGS: usize = 50;
/// A week; anything longer is a typo.
const MAX_MINUTES: u32 = 7 * 24 * 60;
const MAX_SERVINGS: u32 = 1_000;

impl From<&Recipe> for SharedRecipe {
    fn from(r: &Recipe) -> Self {
        let web = |u: &Option<String>| u.clone().filter(|u| is_web_url(u));
        Self {
            title: r.title.clone(),
            description: r.description.clone(),
            author: r.author.clone(),
            source_url: web(&r.source_url),
            image_url: web(&r.image_url),
            servings: r.servings,
            yield_text: r.yield_text.clone(),
            prep_minutes: r.prep_minutes,
            cook_minutes: r.cook_minutes,
            total_minutes: r.total_minutes,
            ingredients: r.ingredients.iter().map(|i| i.raw.clone()).collect(),
            steps: r.steps.clone(),
            tags: r.tags.clone(),
        }
    }
}

impl SharedRecipe {
    pub fn validate(&self) -> Result<(), ShareError> {
        if self.title.trim().is_empty() {
            return Err(ShareError::Title);
        }
        let len = |s: &str, max, what| if s.chars().count() > max { Err(ShareError::TooLong(what)) } else { Ok(()) };
        len(&self.title, MAX_TITLE, "title")?;
        for (v, max, what) in [
            (&self.description, MAX_TEXT, "description"),
            (&self.author, MAX_SHORT, "author"),
            (&self.yield_text, MAX_SHORT, "yield"),
        ] {
            len(v.as_deref().unwrap_or_default(), max, what)?;
        }
        for (v, what) in [(&self.source_url, "source link"), (&self.image_url, "photo link")] {
            if let Some(u) = v {
                len(u, MAX_URL, what)?;
                if !is_web_url(u) {
                    return Err(ShareError::Url(what));
                }
            }
        }
        for (list, max_items, max_len, what) in [
            (&self.ingredients, MAX_LINES, MAX_TEXT, "ingredients"),
            (&self.steps, MAX_LINES, MAX_TEXT, "steps"),
            (&self.tags, MAX_TAGS, MAX_SHORT, "tags"),
        ] {
            if list.len() > max_items {
                return Err(ShareError::TooMany(what));
            }
            list.iter().try_for_each(|s| len(s, max_len, what))?;
        }
        for (v, what) in [
            (self.prep_minutes, "prep time"),
            (self.cook_minutes, "cook time"),
            (self.total_minutes, "total time"),
        ] {
            if v.is_some_and(|m| m > MAX_MINUTES) {
                return Err(ShareError::Range(what));
            }
        }
        if self.servings.is_some_and(|n| n == 0 || n > MAX_SERVINGS) {
            return Err(ShareError::Range("servings"));
        }
        Ok(())
    }

    /// The schema.org `Recipe` node for this recipe. `parse_recipe_html`
    /// reads it back with the same title, ingredients, steps, servings and times.
    /// (The importer strips HTML tags, so text like "<b>" does not survive.)
    pub fn to_json_ld(&self) -> Value {
        let mut node = json!({
            "@context": "https://schema.org",
            "@type": "Recipe",
            "name": self.title,
            "recipeIngredient": self.ingredients,
            "recipeInstructions": self.steps.iter()
                .map(|s| json!({ "@type": "HowToStep", "text": s }))
                .collect::<Vec<_>>(),
        });
        let mut set = |key: &str, v: Option<Value>| {
            if let Some(v) = v {
                node[key] = v;
            }
        };
        set("description", self.description.clone().map(Value::from));
        set("author", self.author.as_ref().map(|a| json!({ "@type": "Person", "name": a })));
        set("image", self.image_url.clone().map(Value::from));
        set("isBasedOn", self.source_url.clone().map(Value::from));
        set("recipeYield", self.recipe_yield().map(Value::from));
        let duration = |m: Option<u32>| m.filter(|m| *m > 0).map(|m| Value::from(format!("PT{m}M")));
        set("prepTime", duration(self.prep_minutes));
        set("cookTime", duration(self.cook_minutes));
        set("totalTime", duration(self.total_minutes));
        set("keywords", (!self.tags.is_empty()).then(|| Value::from(self.tags.clone())));
        node
    }

    /// The JSON-LD as text that is safe to place inside a `<script>` element.
    pub fn json_ld_script(&self) -> String {
        // `<`, `>` and `&` are escaped so the text can't close the element.
        self.to_json_ld()
            .to_string()
            .replace('<', "\\u003c")
            .replace('>', "\\u003e")
            .replace('&', "\\u0026")
    }

    /// The importer reads servings from the leading number of the yield, so
    /// fall back to "N servings" when the yield text says something else.
    fn recipe_yield(&self) -> Option<String> {
        let leading = |s: &str| s.trim().split(|c: char| !c.is_ascii_digit()).next()?.parse::<u32>().ok();
        match (self.servings, &self.yield_text) {
            (Some(n), Some(y)) if leading(y) == Some(n) => Some(y.clone()),
            (Some(n), _) => Some(format!("{n} servings")),
            (None, y) => y.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Ingredient, parse_recipe_html};

    fn recipe() -> Recipe {
        let mut r = Recipe::new("Mac & Cheese");
        r.description = Some("Creamy and easy.".into());
        r.author = Some("Sam Cook".into());
        r.source_url = Some("https://example.com/mac".into());
        r.image_url = Some("https://example.com/mac.jpg".into());
        r.servings = Some(2);
        r.yield_text = Some("1 large pan".into());
        r.prep_minutes = Some(10);
        r.cook_minutes = Some(65);
        r.total_minutes = Some(75);
        r.ingredients = ["8 oz elbow macaroni", "1 1/2 cups shredded cheddar", "Salt to taste"]
            .map(Ingredient::parse)
            .to_vec();
        r.steps = vec!["Boil the pasta.".into(), "Stir in cheese & serve.".into()];
        r.tags = vec!["dinner".into(), "comfort food, fast".into()];
        r.notes = Some("Aunt Sue's secret: extra butter".into());
        r.favorite = true;
        r
    }

    #[test]
    fn leaves_private_fields_out() {
        let shared = SharedRecipe::from(&recipe());
        let json = serde_json::to_string(&shared).unwrap();
        assert!(!json.contains("Aunt Sue"));
        assert!(!json.contains("favorite") && !json.contains("\"id\""));

        let mut local = recipe();
        local.image_url = Some("file:///Users/me/photo.jpg".into());
        assert_eq!(SharedRecipe::from(&local).image_url, None);
    }

    #[test]
    fn json_ld_round_trips_through_importer() {
        let original = recipe();
        let shared = SharedRecipe::from(&original);
        let page = format!(
            r#"<html><head><script type="application/ld+json">{}</script></head></html>"#,
            shared.json_ld_script()
        );
        let back = parse_recipe_html(&page, Some("https://hestia.example.com/r/abc")).unwrap();
        assert_eq!(back.title, original.title);
        assert_eq!(back.description, original.description);
        assert_eq!(back.author, original.author);
        assert_eq!(back.image_url, original.image_url);
        assert_eq!(back.ingredients, original.ingredients);
        assert_eq!(back.steps, original.steps);
        assert_eq!(back.servings, original.servings);
        assert_eq!(
            (back.prep_minutes, back.cook_minutes, back.total_minutes),
            (original.prep_minutes, original.cook_minutes, original.total_minutes)
        );
        assert_eq!(back.tags, original.tags);
        assert_eq!(back.notes, None);
    }

    #[test]
    fn script_text_cannot_close_the_element() {
        let mut r = recipe();
        r.steps = vec!["Stir in cheese </script><script>alert(1)</script> & serve.".into()];
        let s = SharedRecipe::from(&r).json_ld_script();
        assert!(!s.contains('<') && !s.contains('>'));
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["recipeInstructions"][0]["text"], r.steps[0]);
    }

    #[test]
    fn yield_keeps_servings() {
        let mut s = SharedRecipe { title: "x".into(), ..Default::default() };
        assert_eq!(s.recipe_yield(), None);
        s.yield_text = Some("24 cookies".into());
        assert_eq!(s.recipe_yield().as_deref(), Some("24 cookies"));
        s.servings = Some(24);
        assert_eq!(s.recipe_yield().as_deref(), Some("24 cookies"));
        s.servings = Some(12);
        assert_eq!(s.recipe_yield().as_deref(), Some("12 servings"));
    }

    #[test]
    fn validates() {
        let ok = SharedRecipe::from(&recipe());
        assert_eq!(ok.validate(), Ok(()));
        let bad = |f: fn(&mut SharedRecipe)| {
            let mut s = ok.clone();
            f(&mut s);
            s.validate()
        };
        assert_eq!(bad(|s| s.title = "  ".into()), Err(ShareError::Title));
        assert_eq!(bad(|s| s.title = "x".repeat(MAX_TITLE + 1)), Err(ShareError::TooLong("title")));
        assert_eq!(bad(|s| s.steps = vec!["x".into(); MAX_LINES + 1]), Err(ShareError::TooMany("steps")));
        assert_eq!(bad(|s| s.image_url = Some("javascript:alert(1)".into())), Err(ShareError::Url("photo link")));
        assert_eq!(bad(|s| s.cook_minutes = Some(MAX_MINUTES + 1)), Err(ShareError::Range("cook time")));
        assert_eq!(bad(|s| s.servings = Some(0)), Err(ShareError::Range("servings")));

        let mut json = serde_json::to_value(&ok).unwrap();
        json["notes"] = "secret".into();
        assert!(serde_json::from_value::<SharedRecipe>(json).is_err());
    }
}
