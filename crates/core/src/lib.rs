//! Platform-independent recipe logic shared by the app and the server.

pub mod draft;
pub mod import;
pub mod ingredient;
pub mod library;
pub mod model;
#[cfg(feature = "photos")]
pub mod photo;
pub mod share;
pub mod usage;

pub use draft::RecipeDraft;
pub use import::{ImportError, parse_recipe_html};
#[cfg(feature = "fetch")]
pub use import::fetch_recipe;
pub use ingredient::{Ingredient, Quantity};
pub use model::{GroceryItem, Meal, MealPlanEntry, Recipe};
pub use share::{ShareLink, SharedRecipe};
pub use usage::{Event, UsageReport};
