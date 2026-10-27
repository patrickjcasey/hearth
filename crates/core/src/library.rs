//! Rules for keeping a recipe library in sync across devices.
//!
//! Each recipe is stored on its own as an [`Entry`]: the recipe itself, or a
//! [`Tombstone`] once it is deleted. Keeping the tombstone means a device that
//! was offline during the delete learns about it instead of bringing the
//! recipe back. When two versions of the same recipe meet, the one changed
//! most recently wins.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::Recipe;

/// Marks a recipe as deleted. Unknown fields are refused so a recipe that
/// happens to carry a `deleted_at` key can't be mistaken for one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tombstone {
    pub id: Uuid,
    pub deleted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
// Libraries hold a few hundred entries at most; boxing isn't worth the noise.
#[allow(clippy::large_enum_variant)]
pub enum Entry {
    // Tried first: a recipe never has `deleted_at`, and a tombstone has no title.
    Deleted(Tombstone),
    Recipe(Recipe),
}

impl Entry {
    pub fn deleted(id: Uuid) -> Self {
        Entry::Deleted(Tombstone { id, deleted_at: Utc::now() })
    }

    pub fn id(&self) -> Uuid {
        match self {
            Entry::Deleted(t) => t.id,
            Entry::Recipe(r) => r.id,
        }
    }

    pub fn changed_at(&self) -> DateTime<Utc> {
        match self {
            Entry::Deleted(t) => t.deleted_at,
            Entry::Recipe(r) => r.updated_at,
        }
    }

    pub fn recipe(&self) -> Option<&Recipe> {
        match self {
            Entry::Recipe(r) => Some(r),
            Entry::Deleted(_) => None,
        }
    }

    /// Whether `self` should replace `other`, which has the same id. On a
    /// tie the delete wins, so a deleted recipe can't come back by accident.
    /// Two different recipes with the same time are ordered by their JSON, so
    /// every device picks the same one whatever order they arrive in.
    pub fn supersedes(&self, other: &Entry) -> bool {
        match self.changed_at().cmp(&other.changed_at()) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Less => false,
            std::cmp::Ordering::Equal => match (self, other) {
                (Entry::Deleted(_), Entry::Recipe(_)) => true,
                (Entry::Recipe(a), Entry::Recipe(b)) => {
                    serde_json::to_string(a).ok() > serde_json::to_string(b).ok()
                }
                _ => false,
            },
        }
    }
}

/// Keep the winning version of every recipe, in whatever order entries arrive.
pub fn merge(entries: impl IntoIterator<Item = Entry>) -> HashMap<Uuid, Entry> {
    let mut library: HashMap<Uuid, Entry> = HashMap::new();
    for entry in entries {
        match library.get(&entry.id()) {
            Some(current) if !entry.supersedes(current) => {}
            _ => {
                library.insert(entry.id(), entry);
            }
        }
    }
    library
}

/// The recipes still alive after a merge, newest first.
pub fn live_recipes(library: &HashMap<Uuid, Entry>) -> Vec<Recipe> {
    let mut recipes: Vec<Recipe> = library.values().filter_map(Entry::recipe).cloned().collect();
    recipes.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
    recipes
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    fn edited(recipe: &Recipe, title: &str, minutes_later: i64) -> Recipe {
        let mut r = recipe.clone();
        r.title = title.into();
        r.updated_at = recipe.updated_at + Duration::minutes(minutes_later);
        r
    }

    fn tombstone(recipe: &Recipe, minutes_later: i64) -> Entry {
        Entry::Deleted(Tombstone {
            id: recipe.id,
            deleted_at: recipe.updated_at + Duration::minutes(minutes_later),
        })
    }

    #[test]
    fn newest_edit_wins_in_any_order() {
        let original = Recipe::new("Soup");
        let mac = edited(&original, "Soup (Mac)", 5);
        let phone = edited(&original, "Soup (phone)", 10);

        for order in [[&mac, &phone], [&phone, &mac]] {
            let library = merge(order.map(|r| Entry::Recipe(r.clone())));
            assert_eq!(live_recipes(&library), vec![phone.clone()]);
        }
    }

    #[test]
    fn delete_beats_older_edit_but_not_newer() {
        let original = Recipe::new("Soup");

        let library = merge([tombstone(&original, 10), Entry::Recipe(edited(&original, "x", 5))]);
        assert!(live_recipes(&library).is_empty());
        assert!(matches!(library[&original.id], Entry::Deleted(_)));

        // Edited on another device after the delete: keep the edit.
        let revived = edited(&original, "Soup again", 15);
        let library = merge([tombstone(&original, 10), Entry::Recipe(revived.clone())]);
        assert_eq!(live_recipes(&library), vec![revived]);
    }

    #[test]
    fn delete_wins_a_tie() {
        let original = Recipe::new("Soup");
        let gone = tombstone(&original, 0);
        assert!(gone.supersedes(&Entry::Recipe(original.clone())));
        assert!(!Entry::Recipe(original.clone()).supersedes(&gone));
    }

    #[test]
    fn same_time_edits_pick_the_same_winner_in_any_order() {
        let original = Recipe::new("Soup");
        let (a, b) = (edited(&original, "Soup A", 5), edited(&original, "Soup B", 5));
        let ab = merge([Entry::Recipe(a.clone()), Entry::Recipe(b.clone())]);
        let ba = merge([Entry::Recipe(b), Entry::Recipe(a)]);
        assert_eq!(ab, ba);
    }

    #[test]
    fn entries_round_trip_through_json() {
        let recipe = Recipe::new("Bread");
        for entry in [Entry::Recipe(recipe.clone()), tombstone(&recipe, 1)] {
            let json = serde_json::to_string(&entry).unwrap();
            assert_eq!(serde_json::from_str::<Entry>(&json).unwrap(), entry);
        }
    }

    #[test]
    fn a_recipe_with_a_deleted_at_key_is_not_a_tombstone() {
        let recipe = Recipe::new("Bread");
        let mut json = serde_json::to_value(&recipe).unwrap();
        json["deleted_at"] = serde_json::json!("2026-01-01T00:00:00Z");
        let entry: Entry = serde_json::from_value(json).unwrap();
        assert_eq!(entry, Entry::Recipe(recipe.clone()), "the extra key is ignored");

        // Too little for a recipe, too much for a tombstone: an error, never a delete.
        let json = serde_json::json!({
            "id": recipe.id, "deleted_at": "2026-01-01T00:00:00Z", "title": "Bread"
        });
        match serde_json::from_value::<Entry>(json) {
            Ok(Entry::Deleted(_)) => panic!("read as a tombstone"),
            Ok(Entry::Recipe(_)) | Err(_) => {}
        }

        let plain = tombstone(&recipe, 1);
        let json = serde_json::to_string(&plain).unwrap();
        assert_eq!(serde_json::from_str::<Entry>(&json).unwrap(), plain);
    }

    #[test]
    fn live_recipes_are_newest_first() {
        let old = Recipe::new("Old");
        let mut new = Recipe::new("New");
        new.created_at = old.created_at + Duration::days(1);
        let library = merge([Entry::Recipe(old.clone()), Entry::Recipe(new.clone())]);
        assert_eq!(live_recipes(&library), vec![new, old]);
    }
}
