use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ingredient::Ingredient;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recipe {
    pub id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub source_url: Option<String>,
    pub image_url: Option<String>,
    pub author: Option<String>,
    /// Number of servings the ingredient quantities are written for.
    pub servings: Option<u32>,
    /// Original yield text, e.g. "1 loaf" or "24 cookies".
    pub yield_text: Option<String>,
    pub prep_minutes: Option<u32>,
    pub cook_minutes: Option<u32>,
    pub total_minutes: Option<u32>,
    pub ingredients: Vec<Ingredient>,
    pub steps: Vec<String>,
    pub tags: Vec<String>,
    /// Standing notes that always apply, edited with the rest of the recipe.
    pub notes: Option<String>,
    /// Dated notes added after cooking, oldest first. Missing in files saved
    /// before the log existed.
    #[serde(default)]
    pub cooking_log: Vec<Note>,
    pub favorite: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Recipe {
    pub fn new(title: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            title: title.into(),
            description: None,
            source_url: None,
            image_url: None,
            author: None,
            servings: None,
            yield_text: None,
            prep_minutes: None,
            cook_minutes: None,
            total_minutes: None,
            ingredients: Vec::new(),
            steps: Vec::new(),
            tags: Vec::new(),
            notes: None,
            cooking_log: Vec::new(),
            favorite: false,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn touch(&mut self) {
        self.updated_at = Utc::now();
    }

    /// Add a dated note to the cooking log. Blank text is ignored; returns
    /// whether a note was added.
    pub fn add_note(&mut self, text: &str) -> bool {
        let text = text.trim();
        if text.is_empty() {
            return false;
        }
        self.cooking_log.push(Note::new(text));
        self.touch();
        true
    }

    /// Remove a cooking log note; returns whether it was there.
    pub fn remove_note(&mut self, id: Uuid) -> bool {
        let before = self.cooking_log.len();
        self.cooking_log.retain(|n| n.id != id);
        let removed = self.cooking_log.len() != before;
        if removed {
            self.touch();
        }
        removed
    }

    /// Case-insensitive match against title, tags, ingredient names and
    /// cooking log notes.
    pub fn matches(&self, query: &str) -> bool {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return true;
        }
        self.title.to_lowercase().contains(&q)
            || self.tags.iter().any(|t| t.to_lowercase().contains(&q))
            || self.ingredients.iter().any(|i| i.name.to_lowercase().contains(&q))
            || self.cooking_log.iter().any(|n| n.text.to_lowercase().contains(&q))
    }
}

/// A dated entry in a recipe's cooking log, e.g. "bake 5 min less".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub id: Uuid,
    pub text: String,
    pub created_at: DateTime<Utc>,
}

impl Note {
    pub fn new(text: impl Into<String>) -> Self {
        Self { id: Uuid::new_v4(), text: text.into(), created_at: Utc::now() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroceryItem {
    pub id: Uuid,
    pub text: String,
    pub checked: bool,
    /// Recipe this item was added from, if any.
    pub recipe_id: Option<Uuid>,
}

impl GroceryItem {
    pub fn new(text: impl Into<String>, recipe_id: Option<Uuid>) -> Self {
        Self { id: Uuid::new_v4(), text: text.into(), checked: false, recipe_id }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Meal {
    Breakfast,
    Lunch,
    Dinner,
    Snack,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MealPlanEntry {
    pub id: Uuid,
    pub date: NaiveDate,
    pub meal: Meal,
    pub recipe_id: Uuid,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_recipe_saved_before_cooking_log() {
        let json = r#"{
            "id": "6f1c2a3e-8b4d-4c6e-9f00-1a2b3c4d5e6f",
            "title": "Bread", "description": null, "source_url": null,
            "image_url": null, "author": null, "servings": null,
            "yield_text": null, "prep_minutes": null, "cook_minutes": null,
            "total_minutes": null, "ingredients": [], "steps": [], "tags": [],
            "notes": "Use bread flour.", "favorite": false,
            "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
        }"#;
        let r: Recipe = serde_json::from_str(json).unwrap();
        assert_eq!(r.notes.as_deref(), Some("Use bread flour."));
        assert!(r.cooking_log.is_empty());
    }

    #[test]
    fn add_and_remove_notes() {
        let mut r = Recipe::new("Bread");
        let before = r.updated_at;
        assert!(!r.add_note("   "));
        assert_eq!(r.updated_at, before);

        assert!(r.add_note("  Bake 5 min less.\n"));
        assert_eq!(r.cooking_log.len(), 1);
        assert_eq!(r.cooking_log[0].text, "Bake 5 min less.");
        assert!(r.updated_at >= before);

        let id = r.cooking_log[0].id;
        assert!(r.remove_note(id));
        assert!(!r.remove_note(id));
        assert!(r.cooking_log.is_empty());
    }

    #[test]
    fn search_covers_cooking_log() {
        let mut r = Recipe::new("Bread");
        assert!(!r.matches("dry"));
        r.add_note("Came out DRY");
        assert!(r.matches("dry"));
    }

    #[test]
    fn cooking_log_round_trips() {
        let mut r = Recipe::new("Bread");
        r.add_note("More salt");
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<Recipe>(&json).unwrap(), r);
    }
}
