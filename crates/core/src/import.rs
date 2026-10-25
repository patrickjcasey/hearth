//! Extracting recipes from web pages via schema.org `Recipe` JSON-LD, which
//! nearly every recipe site embeds for search engines.

use scraper::{Html, Selector};
use serde_json::Value;

use crate::ingredient::Ingredient;
use crate::model::Recipe;

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("no recipe data found on this page")]
    NotFound,
    #[cfg(feature = "fetch")]
    #[error("could not fetch page: {0}")]
    Fetch(#[from] reqwest::Error),
}

/// Downloads `url` and extracts the recipe from it.
#[cfg(feature = "fetch")]
pub async fn fetch_recipe(url: &str) -> Result<Recipe, ImportError> {
    let html = reqwest::Client::builder()
        // Some sites refuse requests without a browser-like user agent.
        .user_agent("Mozilla/5.0 (compatible; Hestia/0.1; +https://github.com/)")
        .build()?
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse_recipe_html(&html, Some(url))
}

pub fn parse_recipe_html(html: &str, source_url: Option<&str>) -> Result<Recipe, ImportError> {
    let doc = Html::parse_document(html);
    let selector = Selector::parse(r#"script[type="application/ld+json"]"#).expect("valid selector");
    let node = doc
        .select(&selector)
        .filter_map(|el| serde_json::from_str::<Value>(&el.text().collect::<String>()).ok())
        .find_map(|v| find_recipe_node(&v).cloned())
        .ok_or(ImportError::NotFound)?;

    let mut recipe = recipe_from_json_ld(&node);
    recipe.source_url = source_url.map(str::to_string);
    Ok(recipe)
}

/// Searches a JSON-LD value (object, array, or `@graph`) for a Recipe node.
fn find_recipe_node(v: &Value) -> Option<&Value> {
    match v {
        Value::Array(items) => items.iter().find_map(find_recipe_node),
        Value::Object(obj) => {
            let is_recipe = match obj.get("@type") {
                Some(Value::String(t)) => t == "Recipe",
                Some(Value::Array(ts)) => ts.iter().any(|t| t == "Recipe"),
                _ => false,
            };
            if is_recipe {
                return Some(v);
            }
            obj.get("@graph").and_then(find_recipe_node)
        }
        _ => None,
    }
}

fn recipe_from_json_ld(node: &Value) -> Recipe {
    let mut r = Recipe::new(text(node.get("name")).unwrap_or_else(|| "Untitled recipe".into()));
    r.description = text(node.get("description"));
    r.image_url = image(node.get("image"));
    r.author = author(node.get("author"));
    r.yield_text = recipe_yield(node.get("recipeYield"));
    r.servings = r.yield_text.as_deref().and_then(leading_number);
    r.prep_minutes = node.get("prepTime").and_then(Value::as_str).and_then(parse_iso_duration);
    r.cook_minutes = node.get("cookTime").and_then(Value::as_str).and_then(parse_iso_duration);
    r.total_minutes = node.get("totalTime").and_then(Value::as_str).and_then(parse_iso_duration);
    r.ingredients = string_list(node.get("recipeIngredient"))
        .iter()
        .map(|s| Ingredient::parse(s))
        .collect();
    r.steps = instructions(node.get("recipeInstructions"));

    let mut tags = Vec::new();
    for key in ["recipeCategory", "recipeCuisine"] {
        tags.extend(string_list(node.get(key)));
    }
    if let Some(kw) = node.get("keywords") {
        match kw {
            Value::String(s) => tags.extend(s.split(',').map(|t| t.trim().to_string())),
            other => tags.extend(string_list(Some(other))),
        }
    }
    let mut seen = std::collections::HashSet::new();
    tags.retain(|t| !t.is_empty() && seen.insert(t.to_lowercase()));
    r.tags = tags;
    r
}

/// Cleans text from JSON-LD, which is often HTML-escaped and may contain tags.
fn clean(s: &str) -> String {
    let decoded = html_escape::decode_html_entities(s);
    let stripped = if decoded.contains('<') {
        Html::parse_fragment(&decoded).root_element().text().collect::<String>()
    } else {
        decoded.into_owned()
    };
    stripped.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn text(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(clean(s)).filter(|s| !s.is_empty()),
        Value::Array(items) => items.iter().find_map(|i| text(Some(i))),
        _ => None,
    }
}

fn string_list(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) => vec![clean(s)],
        Some(Value::Array(items)) => items.iter().filter_map(|i| text(Some(i))).collect(),
        _ => Vec::new(),
    }
}

fn image(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => items.iter().find_map(|i| image(Some(i))),
        Value::Object(obj) => obj.get("url").and_then(Value::as_str).map(str::to_string),
        _ => None,
    }
}

fn author(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(clean(s)),
        Value::Array(items) => items.iter().find_map(|i| author(Some(i))),
        Value::Object(obj) => text(obj.get("name")),
        _ => None,
    }
}

fn recipe_yield(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) => Some(clean(s)),
        // Often ["4", "4 servings"]; prefer the more descriptive entry.
        Value::Array(items) => items
            .iter()
            .filter_map(|i| recipe_yield(Some(i)))
            .max_by_key(String::len),
        _ => None,
    }
}

fn instructions(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) => {
            // A single blob: split on line breaks.
            let decoded = html_escape::decode_html_entities(s).replace("<br>", "\n");
            decoded.lines().map(clean).filter(|l| !l.is_empty()).collect()
        }
        Some(Value::Array(items)) => items.iter().flat_map(|i| instructions(Some(i))).collect(),
        Some(Value::Object(obj)) => {
            // HowToSection nests steps in itemListElement; HowToStep has text.
            if let Some(items) = obj.get("itemListElement") {
                instructions(Some(items))
            } else {
                text(obj.get("text").or(obj.get("name"))).into_iter().collect()
            }
        }
        _ => Vec::new(),
    }
}

fn leading_number(s: &str) -> Option<u32> {
    let digits: String = s.trim().chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok().filter(|n| *n > 0)
}

/// Parses ISO 8601 durations such as "PT1H30M" or "P0DT45M" into minutes.
pub fn parse_iso_duration(s: &str) -> Option<u32> {
    let s = s.trim().strip_prefix('P')?;
    let mut minutes = 0.0;
    let mut num = String::new();
    let mut in_time = false;
    for ch in s.chars() {
        match ch {
            'T' => in_time = true,
            '0'..='9' | '.' => num.push(ch),
            _ => {
                let n: f64 = num.parse().ok()?;
                num.clear();
                minutes += match (ch, in_time) {
                    ('D', false) => n * 24.0 * 60.0,
                    ('H', true) => n * 60.0,
                    ('M', true) => n,
                    ('S', true) => n / 60.0,
                    _ => return None,
                };
            }
        }
    }
    Some(minutes.round() as u32).filter(|m| *m > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"
    <html><head>
    <script type="application/ld+json">{"@context":"https://schema.org","@type":"WebSite","name":"x"}</script>
    <script type="application/ld+json">
    {"@context":"https://schema.org","@graph":[
      {"@type":"WebPage","name":"page"},
      {"@type":["Recipe","NewsArticle"],
       "name":"Mac &amp; Cheese",
       "description":"<p>Creamy and <b>easy</b>.</p>",
       "image":[{"@type":"ImageObject","url":"https://example.com/mac.jpg"}],
       "author":[{"@type":"Person","name":"Sam Cook"}],
       "recipeYield":["4","4 servings"],
       "prepTime":"PT10M","cookTime":"PT1H5M","totalTime":"P0DT1H15M",
       "recipeIngredient":["8 oz elbow macaroni","2 cups shredded cheddar","Salt to taste"],
       "recipeInstructions":[
         {"@type":"HowToSection","name":"Pasta","itemListElement":[
           {"@type":"HowToStep","text":"Boil the pasta."}]},
         {"@type":"HowToStep","text":"Stir in cheese."}],
       "recipeCategory":"Dinner",
       "keywords":"pasta, comfort food, dinner"}
    ]}
    </script></head><body></body></html>"#;

    #[test]
    fn extracts_recipe_from_graph() {
        let r = parse_recipe_html(PAGE, Some("https://example.com/mac")).unwrap();
        assert_eq!(r.title, "Mac & Cheese");
        assert_eq!(r.description.as_deref(), Some("Creamy and easy."));
        assert_eq!(r.image_url.as_deref(), Some("https://example.com/mac.jpg"));
        assert_eq!(r.author.as_deref(), Some("Sam Cook"));
        assert_eq!(r.yield_text.as_deref(), Some("4 servings"));
        assert_eq!(r.servings, Some(4));
        assert_eq!((r.prep_minutes, r.cook_minutes, r.total_minutes), (Some(10), Some(65), Some(75)));
        assert_eq!(r.ingredients.len(), 3);
        assert_eq!(r.ingredients[0].unit.as_deref(), Some("oz"));
        assert_eq!(r.steps, vec!["Boil the pasta.", "Stir in cheese."]);
        assert_eq!(r.tags, vec!["Dinner", "pasta", "comfort food"]);
        assert_eq!(r.source_url.as_deref(), Some("https://example.com/mac"));
    }

    #[test]
    fn missing_recipe_is_error() {
        let err = parse_recipe_html("<html><body>hi</body></html>", None).unwrap_err();
        assert!(matches!(err, ImportError::NotFound));
    }

    #[test]
    fn string_instructions_split_on_lines() {
        let v: Value = serde_json::json!("Mix.\nBake.<br>Cool.");
        assert_eq!(instructions(Some(&v)), vec!["Mix.", "Bake.", "Cool."]);
    }

    #[test]
    fn durations() {
        assert_eq!(parse_iso_duration("PT45M"), Some(45));
        assert_eq!(parse_iso_duration("PT2H"), Some(120));
        assert_eq!(parse_iso_duration("P1D"), Some(1440));
        assert_eq!(parse_iso_duration("garbage"), None);
    }
}
