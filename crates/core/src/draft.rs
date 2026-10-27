//! Editable, text-only view of a recipe, as shown in a form.

use std::collections::HashSet;

use crate::{Ingredient, Recipe};

/// Every field is plain text so a UI can bind inputs to it directly.
/// Ingredients and steps are one per line; tags are comma-separated.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecipeDraft {
    pub title: String,
    pub description: String,
    pub author: String,
    pub source_url: String,
    pub image_url: String,
    pub servings: String,
    pub prep_minutes: String,
    pub cook_minutes: String,
    pub ingredients: String,
    pub steps: String,
    pub tags: String,
    pub notes: String,
}

impl RecipeDraft {
    pub fn from_recipe(r: &Recipe) -> Self {
        let opt = |v: &Option<String>| v.clone().unwrap_or_default();
        let num = |v: Option<u32>| v.map(|n| n.to_string()).unwrap_or_default();
        Self {
            title: r.title.clone(),
            description: opt(&r.description),
            author: opt(&r.author),
            source_url: opt(&r.source_url),
            image_url: opt(&r.image_url),
            servings: num(r.servings),
            prep_minutes: num(r.prep_minutes),
            cook_minutes: num(r.cook_minutes),
            ingredients: r.ingredients.iter().map(|i| i.raw.as_str()).collect::<Vec<_>>().join("\n"),
            steps: r.steps.join("\n"),
            tags: r.tags.join(", "),
            notes: opt(&r.notes),
        }
    }

    /// A recipe needs a title; everything else is optional.
    pub fn is_valid(&self) -> bool {
        !self.title.trim().is_empty()
    }

    /// Write the draft into `recipe`. The id, favorite flag, creation time and
    /// cooking log are kept. Total time is recomputed only when prep or cook time
    /// changed, so an imported total (which may include rising or resting time)
    /// survives an edit that doesn't touch either.
    pub fn apply_to(&self, recipe: &mut Recipe) {
        recipe.title = self.title.trim().to_string();
        recipe.description = text(&self.description);
        recipe.author = text(&self.author);
        recipe.source_url = text(&self.source_url);
        recipe.image_url = text(&self.image_url);

        let servings = number(&self.servings);
        if servings != recipe.servings {
            // The imported yield text describes the old serving count.
            recipe.yield_text = None;
        }
        recipe.servings = servings;
        let (prep, cook) = (number(&self.prep_minutes), number(&self.cook_minutes));
        if (prep, cook) != (recipe.prep_minutes, recipe.cook_minutes) {
            recipe.total_minutes = match (prep, cook) {
                (None, None) => None,
                _ => Some(prep.unwrap_or(0).saturating_add(cook.unwrap_or(0))),
            };
        }
        (recipe.prep_minutes, recipe.cook_minutes) = (prep, cook);

        // Re-parse only lines that changed, so unchanged ingredients keep
        // exactly what the importer produced.
        let old = std::mem::take(&mut recipe.ingredients);
        recipe.ingredients = lines(&self.ingredients)
            .map(|line| match old.iter().find(|i| i.raw == line) {
                Some(i) => i.clone(),
                None => Ingredient::parse(&line),
            })
            .collect();
        recipe.steps = lines(&self.steps).collect();

        // A tag may itself contain a comma ("Soups, Stews" from an import),
        // so the field is only re-split once the user has changed it.
        if self.tags != recipe.tags.join(", ") {
            let mut seen = HashSet::new();
            recipe.tags = self
                .tags
                .split(',')
                .map(str::trim)
                .filter(|t| !t.is_empty() && seen.insert(t.to_lowercase()))
                .map(String::from)
                .collect();
        }
        recipe.notes = text(&self.notes);
        recipe.touch();
    }

    pub fn into_recipe(&self) -> Recipe {
        let mut recipe = Recipe::new("");
        self.apply_to(&mut recipe);
        recipe
    }
}

fn text(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

fn number(s: &str) -> Option<u32> {
    s.trim().parse().ok().filter(|&n| n > 0)
}

fn lines(s: &str) -> impl Iterator<Item = String> + '_ {
    s.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_recipe_from_form_text() {
        let draft = RecipeDraft {
            title: "  Pancakes ".into(),
            servings: "4".into(),
            prep_minutes: "10".into(),
            cook_minutes: "15".into(),
            ingredients: "1 1/2 cups flour\n\n2 eggs\n".into(),
            steps: "Mix.\n  \nCook on a griddle.".into(),
            tags: "breakfast, Sweet, breakfast,  ".into(),
            ..Default::default()
        };
        let r = draft.into_recipe();
        assert_eq!(r.title, "Pancakes");
        assert_eq!(r.servings, Some(4));
        assert_eq!(r.total_minutes, Some(25));
        assert_eq!(r.ingredients.len(), 2);
        assert_eq!(r.ingredients[0].unit.as_deref(), Some("cups"));
        assert_eq!(r.steps, ["Mix.", "Cook on a griddle."]);
        assert_eq!(r.tags, ["breakfast", "Sweet"]);
        assert_eq!(r.description, None);
    }

    #[test]
    fn editing_keeps_identity_and_imported_data() {
        let mut r = Recipe::new("Soup");
        r.favorite = true;
        r.total_minutes = Some(90);
        r.servings = Some(4);
        r.yield_text = Some("4 bowls".into());
        r.ingredients = vec![Ingredient::parse("1 onion")];
        r.add_note("Needed more salt");
        let (id, created) = (r.id, r.created_at);

        let mut draft = RecipeDraft::from_recipe(&r);
        assert_eq!(draft.ingredients, "1 onion");
        draft.title = "Onion soup".into();
        draft.ingredients.push_str("\n2 cups stock");
        draft.apply_to(&mut r);

        assert_eq!((r.id, r.created_at, r.favorite), (id, created, true));
        assert_eq!(r.title, "Onion soup");
        assert_eq!(r.total_minutes, Some(90), "no prep/cook given, keep imported total");
        assert_eq!(r.yield_text.as_deref(), Some("4 bowls"), "servings unchanged");
        assert_eq!(r.ingredients.len(), 2);
        assert_eq!(r.cooking_log.len(), 1, "editing keeps the cooking log");

        draft.servings = "6".into();
        draft.apply_to(&mut r);
        assert_eq!(r.yield_text, None);
    }

    #[test]
    fn total_follows_prep_and_cook_only_when_they_change() {
        let mut r = Recipe::new("Bread");
        (r.prep_minutes, r.cook_minutes, r.total_minutes) = (Some(20), Some(30), Some(180));
        let mut draft = RecipeDraft::from_recipe(&r);
        draft.title = "Rye bread".into();
        draft.apply_to(&mut r);
        assert_eq!(r.total_minutes, Some(180), "rising time kept");

        draft.cook_minutes = "40".into();
        draft.apply_to(&mut r);
        assert_eq!(r.total_minutes, Some(60));

        (draft.prep_minutes, draft.cook_minutes) = (String::new(), String::new());
        draft.apply_to(&mut r);
        assert_eq!((r.prep_minutes, r.cook_minutes, r.total_minutes), (None, None, None));

        (draft.prep_minutes, draft.cook_minutes) = ("4000000000".into(), "4000000000".into());
        draft.apply_to(&mut r);
        assert_eq!(r.total_minutes, Some(u32::MAX));
    }

    #[test]
    fn untouched_tags_survive_an_edit() {
        let mut r = Recipe::new("Stew");
        r.tags = vec!["Soups, Stews".into(), "winter".into()];
        let mut draft = RecipeDraft::from_recipe(&r);
        assert_eq!(draft.tags, "Soups, Stews, winter");
        draft.title = "Beef stew".into();
        draft.apply_to(&mut r);
        assert_eq!(r.tags, ["Soups, Stews", "winter"]);

        // Once edited, the field is split on commas as usual.
        draft.tags.push_str(", Beef");
        draft.apply_to(&mut r);
        assert_eq!(r.tags, ["Soups", "Stews", "winter", "Beef"]);
    }

    #[test]
    fn requires_title() {
        assert!(!RecipeDraft { title: "  ".into(), ..Default::default() }.is_valid());
    }
}
