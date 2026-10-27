use dioxus::core::spawn_forever;
use dioxus::prelude::*;

use hestia_core::Event;
use hestia_core::url::is_public_web_url;
use uuid::Uuid;

use crate::photos::use_photo;
use crate::{Route, use_data, use_usage};

#[component]
pub fn Home() -> Element {
    let mut data = use_data();
    let mut usage = use_usage();
    let mut query = use_signal(String::new);
    let mut url = use_signal(String::new);
    let mut importing = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);

    let import = move |e: FormEvent| {
        e.prevent_default();
        let target = url.read().trim().to_string();
        if target.is_empty() || importing() {
            return;
        }
        importing.set(true);
        error.set(None);
        let nav = navigator();
        // Not tied to this screen, so switching tabs doesn't cancel the import.
        spawn_forever(async move {
            let result = hestia_core::fetch_recipe(&target).await;
            // This screen's signals are gone if the user left; the library isn't.
            let still_here = importing.try_write().map(|mut i| *i = false).is_ok();
            match result {
                Ok(recipe) => {
                    usage.write().record(Event::RecipeImported);
                    let id = recipe.id;
                    data.write().recipes.insert(0, recipe);
                    if still_here {
                        url.set(String::new());
                        nav.push(Route::RecipeDetail { id });
                    }
                }
                Err(e) => {
                    usage.write().record(Event::ImportFailed);
                    if let Ok(mut error) = error.try_write() {
                        *error = Some(e.to_string());
                    }
                }
            }
        });
    };

    let data_ref = data.read();
    let mut recipes: Vec<_> = data_ref.recipes.iter().filter(|r| r.matches(&query.read())).collect();
    recipes.sort_by_key(|r| !r.favorite);

    rsx! {
        div { class: "title-row",
            h1 { "Recipes" }
            Link { to: Route::NewRecipe {}, class: "button primary", "+ New" }
        }
        form { class: "import", onsubmit: import,
            input {
                r#type: "url",
                placeholder: "Paste a recipe link…",
                value: "{url}",
                oninput: move |e| url.set(e.value()),
            }
            button { r#type: "submit", disabled: importing(),
                if importing() { "Importing…" } else { "Import" }
            }
        }
        if let Some(err) = error() {
            p { class: "error", "{err}" }
        }
        input {
            class: "search",
            r#type: "search",
            placeholder: "Search recipes, tags, ingredients",
            value: "{query}",
            oninput: move |e| query.set(e.value()),
        }
        if recipes.is_empty() {
            p { class: "empty",
                if data_ref.recipes.is_empty() {
                    "Your recipe box is empty. Paste a link from any recipe site, or tap + New to write your own."
                } else {
                    "No recipes match your search."
                }
            }
        }
        ul { class: "recipe-list",
            for r in recipes {
                li { key: "{r.id}",
                    RecipeCard {
                        id: r.id,
                        title: r.title.clone(),
                        image_url: r.image_url.clone(),
                        favorite: r.favorite,
                        total_minutes: r.total_minutes,
                    }
                }
            }
        }
    }
}

#[component]
fn RecipeCard(
    id: Uuid,
    title: String,
    image_url: Option<String>,
    favorite: bool,
    total_minutes: Option<u32>,
) -> Element {
    // The stored copy when there is one, else the link while it downloads.
    // Only public web links reach the page; a library file could hold anything.
    let src = use_photo(id, image_url.as_deref()).or(image_url.filter(|u| is_public_web_url(u)));
    // A photo that fails to load (offline, dead link) falls back to the
    // initial, until a different source comes along.
    let mut broken = use_signal(|| None::<String>);
    rsx! {
        Link { to: Route::RecipeDetail { id }, class: "recipe-card",
            div { class: "thumb",
                match src {
                    Some(img) if broken().as_ref() != Some(&img) => rsx! {
                        img { src: "{img}", alt: "", onerror: move |_| broken.set(Some(img.clone())) }
                    },
                    _ => rsx! { div { class: "initial", "{initial(&title)}" } },
                }
                if favorite {
                    span { class: "fav", title: "Favorite", "★" }
                }
            }
            div { class: "card-body",
                h2 { class: "card-title", "{title}" }
                if let Some(mins) = total_minutes {
                    span { class: "meta", "{mins} min" }
                }
            }
        }
    }
}

/// The first letter of a title, shown when a recipe has no photo.
fn initial(title: &str) -> String {
    title.trim().chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default()
}
