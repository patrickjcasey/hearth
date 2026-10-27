use chrono::{DateTime, Local, Utc};
use dioxus::prelude::*;
use hestia_core::share::SHARE_DAYS;
use hestia_core::{Event, GroceryItem, ShareLink, SharedRecipe};
use uuid::Uuid;

use crate::photos::use_photo;
use crate::share;
use crate::{Route, use_data, use_usage, wake};

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
    // "Keep screen on" lasts until it's switched off or the recipe is closed.
    let mut awake = use_signal(|| false);
    use_drop(|| wake::keep_awake(false));
    let mut note_text = use_signal(String::new);
    let mut share_open = use_signal(|| false);

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
    let set_awake = move |e: FormEvent| {
        let on = e.checked();
        awake.set(on);
        wake::keep_awake(on);
        if on {
            usage.write().record(Event::CookStarted);
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
                label { class: "awake",
                    input { r#type: "checkbox", role: "switch", class: "switch", checked: awake(), onchange: set_awake }
                    "Keep screen on"
                }
                Link { to: Route::EditRecipe { id }, class: "button", "Edit" }
                button { onclick: toggle_favorite,
                    if recipe.favorite { "★ Favorited" } else { "☆ Favorite" }
                }
                button { onclick: add_to_groceries, disabled: added(),
                    if added() { "Added to groceries" } else { "Add to groceries" }
                }
                if share::SHARE_URL.is_some() {
                    button { onclick: move |_| share_open.toggle(), disabled: share_open(), "Share" }
                }
                if let Some(src) = &recipe.source_url {
                    a { href: "{src}", target: "_blank", class: "button", "Original" }
                }
            }
            if share_open() {
                SharePanel { id, onclose: move |_| share_open.set(false) }
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

#[derive(Clone, PartialEq)]
enum ShareState {
    Confirm,
    Sending,
    Done { link: ShareLink, copied: bool },
    Failed(String),
}

/// Asks before uploading anything, then shows the link and copies it.
#[component]
fn SharePanel(id: Uuid, onclose: EventHandler) -> Element {
    let data = use_data();
    let mut usage = use_usage();
    let mut state = use_signal(|| ShareState::Confirm);
    let host = share::host().unwrap_or("the Hestia server");

    let create = move |_| {
        let Some(recipe) = data.peek().recipe(id).map(SharedRecipe::from) else { return };
        state.set(ShareState::Sending);
        spawn(async move {
            match share::create_link(&recipe).await {
                Ok(link) => {
                    usage.write().record(Event::RecipeShared);
                    let copied = share::copy_to_clipboard(&link.url);
                    state.set(ShareState::Done { link, copied });
                }
                Err(e) => state.set(ShareState::Failed(e)),
            }
        });
    };
    let copy = move |_| {
        let ShareState::Done { link, .. } = state() else { return };
        let ok = share::copy_to_clipboard(&link.url);
        if let ShareState::Done { copied, .. } = &mut *state.write() {
            *copied = ok;
        }
    };
    let close = move |_| onclose.call(());

    rsx! {
        div { class: "share-panel",
            match state() {
                ShareState::Confirm => rsx! {
                    p { "Create a link anyone can open, even without Hestia?" }
                    p { class: "meta",
                        "This uploads the title, description, author, servings, times, ingredients, steps, tags, "
                        "and source and photo links to {host}. Your notes and photos stay on this device. "
                        "The link shows the recipe as it is now and stops working after {SHARE_DAYS} days."
                    }
                    div { class: "actions",
                        button { class: "primary", onclick: create, "Create link" }
                        button { class: "quiet", onclick: close, "Cancel" }
                    }
                },
                ShareState::Sending => rsx! { p { class: "meta", "Creating link…" } },
                ShareState::Done { link, copied } => {
                    let until = link.expires_at.with_timezone(&chrono::Local).format("%B %-d, %Y").to_string();
                    rsx! {
                        div { class: "share-link",
                            input { readonly: true, value: "{link.url}", "aria-label": "Share link" }
                            button { onclick: copy, if copied { "Copied" } else { "Copy" } }
                        }
                        p { class: "meta",
                            if copied { "Link copied. " }
                            "Anyone with it can view and import this recipe until {until}."
                        }
                        button { class: "quiet", onclick: close, "Done" }
                    }
                }
                ShareState::Failed(err) => rsx! {
                    p { class: "error", "{err}" }
                    div { class: "actions",
                        button { onclick: create, "Try again" }
                        button { class: "quiet", onclick: close, "Cancel" }
                    }
                },
            }
        }
    }
}
