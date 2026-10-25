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
    pub notes: Option<String>,
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
            favorite: false,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn touch(&mut self) {
        self.updated_at = Utc::now();
    }

    /// Case-insensitive match against title, tags and ingredient names.
    pub fn matches(&self, query: &str) -> bool {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return true;
        }
        self.title.to_lowercase().contains(&q)
            || self.tags.iter().any(|t| t.to_lowercase().contains(&q))
            || self.ingredients.iter().any(|i| i.name.to_lowercase().contains(&q))
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
