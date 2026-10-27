//! Platform-independent recipe logic shared by the app and the server.

pub mod draft;
#[cfg(feature = "fetch")]
pub mod fetch;
pub mod import;
pub mod ingredient;
pub mod library;
pub mod model;
#[cfg(feature = "photos")]
pub mod photo;
pub mod share;
pub mod url;
pub mod usage;

pub use draft::RecipeDraft;
pub use import::{ImportError, parse_recipe_html};
#[cfg(feature = "fetch")]
pub use import::fetch_recipe;
pub use ingredient::{Ingredient, Quantity};
pub use model::{GroceryItem, Meal, MealPlanEntry, Note, Recipe};
pub use share::{ShareLink, SharedRecipe};
pub use usage::{Event, UsageReport};
