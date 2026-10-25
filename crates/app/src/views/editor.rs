use dioxus::prelude::*;
use hestia_core::{Event, RecipeDraft};
use uuid::Uuid;

use crate::{Route, use_data, use_usage};

#[component]
pub fn NewRecipe() -> Element {
    rsx! { RecipeEditor { id: None } }
}

#[component]
pub fn EditRecipe(id: Uuid) -> Element {
    rsx! { RecipeEditor { id: Some(id) } }
}

/// Form for creating a recipe (`id: None`) or editing an existing one.
#[component]
fn RecipeEditor(id: Option<Uuid>) -> Element {
    let mut data = use_data();
    let mut usage = use_usage();
    let existing = id.and_then(|id| data.peek().recipe(id).map(RecipeDraft::from_recipe));
    let missing = id.is_some() && existing.is_none();
    let mut draft = use_signal(|| existing.unwrap_or_default());

    if missing {
        return rsx! {
            p { class: "empty", "This recipe no longer exists." }
            Link { to: Route::Home {}, "Back to recipes" }
        };
    }

    let save = move |e: FormEvent| {
        e.prevent_default();
        let form = draft.read();
        if !form.is_valid() {
            return;
        }
        let saved_id = match id {
            Some(id) => {
                if let Some(r) = data.write().recipes.iter_mut().find(|r| r.id == id) {
                    form.apply_to(r);
                }
                id
            }
            None => {
                let recipe = form.into_recipe();
                let new_id = recipe.id;
                data.write().recipes.insert(0, recipe);
                usage.write().record(Event::RecipeCreated);
                new_id
            }
        };
        navigator().replace(Route::RecipeDetail { id: saved_id });
    };

    rsx! {
        form { class: "editor", onsubmit: save,
            div { class: "editor-bar",
                button { r#type: "button", class: "quiet", onclick: move |_| navigator().go_back(), "Cancel" }
                h1 { if id.is_some() { "Edit recipe" } else { "New recipe" } }
                button { r#type: "submit", disabled: !draft.read().is_valid(), "Save" }
            }
            Field {
                label: "Title",
                value: draft.read().title.clone(),
                oninput: move |v| draft.write().title = v,
                placeholder: "Grandma's lasagna",
            }
            Field {
                label: "Description",
                value: draft.read().description.clone(),
                oninput: move |v| draft.write().description = v,
                rows: 2,
            }
            div { class: "field-row",
                Field {
                    label: "Servings",
                    value: draft.read().servings.clone(),
                    oninput: move |v| draft.write().servings = v,
                    numeric: true,
                }
                Field {
                    label: "Prep (min)",
                    value: draft.read().prep_minutes.clone(),
                    oninput: move |v| draft.write().prep_minutes = v,
                    numeric: true,
                }
                Field {
                    label: "Cook (min)",
                    value: draft.read().cook_minutes.clone(),
                    oninput: move |v| draft.write().cook_minutes = v,
                    numeric: true,
                }
            }
            Field {
                label: "Ingredients",
                hint: "One per line, e.g. \"1 1/2 cups flour\". Amounts scale with servings.",
                value: draft.read().ingredients.clone(),
                oninput: move |v| draft.write().ingredients = v,
                rows: 8,
            }
            Field {
                label: "Steps",
                hint: "One step per line.",
                value: draft.read().steps.clone(),
                oninput: move |v| draft.write().steps = v,
                rows: 8,
            }
            Field {
                label: "Tags",
                hint: "Separate with commas.",
                value: draft.read().tags.clone(),
                oninput: move |v| draft.write().tags = v,
                placeholder: "dinner, italian",
            }
            Field {
                label: "Notes",
                hint: "Tips that always apply. Notes from each cook go in the cooking log on the recipe page.",
                value: draft.read().notes.clone(),
                oninput: move |v| draft.write().notes = v,
                rows: 3,
            }
            Field {
                label: "Author",
                value: draft.read().author.clone(),
                oninput: move |v| draft.write().author = v,
            }
            Field {
                label: "Photo URL",
                value: draft.read().image_url.clone(),
                oninput: move |v| draft.write().image_url = v,
                url: true,
            }
            Field {
                label: "Source URL",
                value: draft.read().source_url.clone(),
                oninput: move |v| draft.write().source_url = v,
                url: true,
            }
        }
    }
}

/// A labelled input. `rows > 0` renders a multi-line textarea.
#[component]
fn Field(
    label: &'static str,
    value: String,
    oninput: EventHandler<String>,
    #[props(default)] hint: &'static str,
    #[props(default)] placeholder: &'static str,
    #[props(default)] rows: u32,
    #[props(default)] numeric: bool,
    #[props(default)] url: bool,
) -> Element {
    let kind = if url { "url" } else { "text" };
    let inputmode = if numeric { "numeric" } else { "text" };
    rsx! {
        label { class: "field",
            span { class: "label", "{label}" }
            if rows > 0 {
                textarea {
                    rows: "{rows}",
                    placeholder,
                    value,
                    oninput: move |e| oninput.call(e.value()),
                }
            } else {
                input {
                    r#type: kind,
                    inputmode,
                    placeholder,
                    value,
                    oninput: move |e| oninput.call(e.value()),
                }
            }
            if !hint.is_empty() {
                span { class: "hint", "{hint}" }
            }
        }
    }
}
