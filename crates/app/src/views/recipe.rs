use chrono::{DateTime, Local, Utc};
use dioxus::prelude::*;
use hestia_core::{Event, GroceryItem};
use uuid::Uuid;

use crate::photos::use_photo;
use crate::{Route, use_data, use_usage};

#[component]
pub fn RecipeDetail(id: Uuid) -> Element {
    let mut data = use_data();
    let mut usage = use_usage();
    let base_servings = data.read().recipe(id).and_then(|r| r.servings);
    let mut servings = use_signal(|| base_servings.unwrap_or(1));
    let mut added = use_signal(|| false);
    let image_url = data.read().recipe(id).and_then(|r| r.image_url.clone());
    let hero = use_photo(id, image_url.as_deref()).or(image_url);
    let mut hero_broken = use_signal(|| None::<String>);
    let mut note_text = use_signal(String::new);

    let data_ref = data.read();
    let Some(recipe) = data_ref.recipe(id) else {
        return rsx! {
            p { class: "empty", "This recipe no longer exists." }
            Link { to: Route::Home {}, class: "button", "Back to recipes" }
        };
    };
    let factor = base_servings.map_or(1.0, |b| servings() as f64 / b as f64);
    let scaled: Vec<String> = recipe.ingredients.iter().map(|i| i.display_scaled(factor)).collect();

    let toggle_favorite = move |_| {
        if let Some(r) = data.write().recipes.iter_mut().find(|r| r.id == id) {
            r.favorite = !r.favorite;
            r.touch();
        }
    };
    let add_to_groceries = {
        let scaled = scaled.clone();
        move |_| {
            let items = scaled.iter().map(|text| GroceryItem::new(text.clone(), Some(id)));
            data.write().groceries.extend(items);
            added.set(true);
            usage.write().record(Event::GroceriesAdded);
        }
    };
    let add_note = move |_| {
        let text = note_text();
        if text.trim().is_empty() {
            return;
        }
        if let Some(r) = data.write().recipes.iter_mut().find(|r| r.id == id) {
            r.add_note(&text);
        }
        note_text.set(String::new());
    };
    let delete = move |_| {
        data.write().recipes.retain(|r| r.id != id);
        navigator().replace(Route::Home {});
    };

    rsx! {
        article { class: "recipe",
            if let Some(img) = hero.filter(|img| hero_broken().as_ref() != Some(img)) {
                img { class: "hero", src: "{img}", alt: "", onerror: move |_| hero_broken.set(Some(img.clone())) }
            }
            h1 { "{recipe.title}" }
            div { class: "meta-row",
                if let Some(a) = &recipe.author { span { "by {a}" } }
                if let Some(m) = recipe.prep_minutes { span { "Prep {m} min" } }
                if let Some(m) = recipe.cook_minutes { span { "Cook {m} min" } }
                if let Some(m) = recipe.total_minutes { span { "Total {m} min" } }
            }
            if let Some(d) = &recipe.description {
                p { class: "description", "{d}" }
            }
            div { class: "actions",
                if !recipe.steps.is_empty() {
                    Link {
                        to: Route::CookMode { id, servings: if base_servings.is_some() { servings() } else { 0 } },
                        class: "button primary",
                        "Start cooking"
                    }
                }
                Link { to: Route::EditRecipe { id }, class: "button", "Edit" }
                button { onclick: toggle_favorite,
                    if recipe.favorite { "★ Favorited" } else { "☆ Favorite" }
                }
                button { onclick: add_to_groceries, disabled: added(),
                    if added() { "Added to groceries" } else { "Add to groceries" }
                }
                if let Some(src) = &recipe.source_url {
                    a { href: "{src}", target: "_blank", class: "button", "Original" }
                }
            }

            div { class: "recipe-body",
                aside { class: "ingredients-panel",
                    h2 { "Ingredients" }
                    if base_servings.is_some() {
                        div { class: "servings",
                            button { onclick: move |_| if servings() > 1 { servings -= 1 }, "aria-label": "Fewer servings", "−" }
                            span { "{servings} servings" }
                            button { onclick: move |_| servings += 1, "aria-label": "More servings", "+" }
                        }
                    }
                    ul { class: "ingredients",
                        for (i, line) in scaled.iter().enumerate() {
                            li { key: "{i}", "{line}" }
                        }
                    }
                }
                div {
                    h2 { "Steps" }
                    ol { class: "steps",
                        for (i, step) in recipe.steps.iter().enumerate() {
                            li { key: "{i}", "{step}" }
                        }
                    }
                    if let Some(notes) = &recipe.notes {
                        h2 { "Notes" }
                        p { class: "notes", "{notes}" }
                    }
                }
            }

            section { class: "cooking-log",
                h2 { "Cooking log" }
                p { class: "meta", "Dated notes from each time you cook this." }
                div { class: "log-add",
                    textarea {
                        rows: "2",
                        placeholder: "Used 2 tsp salt; bake 5 min less next time",
                        "aria-label": "New cooking log note",
                        value: "{note_text}",
                        oninput: move |e| note_text.set(e.value()),
                    }
                    button { class: "primary", disabled: note_text.read().trim().is_empty(), onclick: add_note, "Add note" }
                }
                ul { class: "log",
                    for note in recipe.cooking_log.iter().rev() {
                        li { key: "{note.id}",
                            div {
                                time { class: "meta", datetime: "{note.created_at.to_rfc3339()}", {friendly_date(note.created_at)} }
                                p { class: "notes", "{note.text}" }
                            }
                            button {
                                class: "quiet",
                                "aria-label": "Delete note",
                                onclick: {
                                    let note_id = note.id;
                                    move |_| {
                                        if let Some(r) = data.write().recipes.iter_mut().find(|r| r.id == id) {
                                            r.remove_note(note_id);
                                        }
                                    }
                                },
                                "✕"
                            }
                        }
                    }
                }
            }

            if !recipe.tags.is_empty() {
                div { class: "tags",
                    for tag in recipe.tags.iter() {
                        span { key: "{tag}", class: "tag", "{tag}" }
                    }
                }
            }
            button { class: "danger", onclick: delete, "Delete recipe" }
        }
    }
}

/// "24 Oct 2026", in the device's time zone.
fn friendly_date(at: DateTime<Utc>) -> String {
    at.with_timezone(&Local).format("%-d %b %Y").to_string()
}
