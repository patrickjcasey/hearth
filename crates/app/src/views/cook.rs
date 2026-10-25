use dioxus::prelude::*;
use hestia_core::Event;
use uuid::Uuid;

use crate::{Route, use_data, use_usage, wake};

/// Full-screen, one-step-at-a-time view. The screen stays awake while it's open.
/// `servings` carries the scaler value from the detail page (0 = as written).
#[component]
pub fn CookMode(id: Uuid, servings: u32) -> Element {
    let mut data = use_data();
    let mut usage = use_usage();
    let mut step = use_signal(|| 0usize);
    let mut show_ingredients = use_signal(|| false);
    // Set once "Done" is pressed: offer a cooking log note before leaving.
    let mut finished = use_signal(|| false);
    let mut note_text = use_signal(String::new);

    use_hook(move || {
        wake::keep_awake(true);
        usage.write().record(Event::CookStarted);
    });
    use_drop(|| wake::keep_awake(false));

    let data_ref = data.read();
    let Some(recipe) = data_ref.recipe(id) else {
        return rsx! {
            p { class: "empty", "This recipe no longer exists." }
            Link { to: Route::Home {}, "Back to recipes" }
        };
    };
    let factor = match (recipe.servings, servings) {
        (Some(base), target) if target > 0 => target as f64 / base as f64,
        _ => 1.0,
    };
    let total = recipe.steps.len();
    let current = step().min(total.saturating_sub(1));
    let is_last = current + 1 >= total;
    let serving_label = match (servings, recipe.servings) {
        (0, Some(n)) | (n @ 1.., _) => format!(" for {n} servings"),
        _ => String::new(),
    };

    rsx! {
        div { class: "cook",
            header { class: "cook-bar",
                Link { to: Route::RecipeDetail { id }, class: "close", "aria-label": "Leave cook mode", "✕" }
                h1 { "{recipe.title}" }
                button {
                    class: if show_ingredients() { "toggle on" } else { "toggle" },
                    onclick: move |_| show_ingredients.toggle(),
                    "Ingredients"
                }
            }
            if show_ingredients() {
                section { class: "cook-ingredients",
                    p { class: "meta", "Ingredients{serving_label}" }
                    ul {
                        for (i, ing) in recipe.ingredients.iter().enumerate() {
                            li { key: "{i}", {ing.display_scaled(factor)} }
                        }
                    }
                }
            }
            if finished() {
                section { class: "cook-finish",
                    h2 { "How did it go?" }
                    p { class: "meta", "Add a note to the cooking log, or skip." }
                    textarea {
                        rows: "3",
                        placeholder: "Used 2 tsp salt; bake 5 min less next time",
                        "aria-label": "Cooking log note",
                        value: "{note_text}",
                        oninput: move |e| note_text.set(e.value()),
                    }
                    div { class: "cook-nav",
                        button { onclick: move |_| { navigator().replace(Route::RecipeDetail { id }); }, "Skip" }
                        button {
                            class: "primary",
                            disabled: note_text.read().trim().is_empty(),
                            onclick: move |_| {
                                let text = note_text();
                                if let Some(r) = data.write().recipes.iter_mut().find(|r| r.id == id) {
                                    r.add_note(&text);
                                }
                                navigator().replace(Route::RecipeDetail { id });
                            },
                            "Save note"
                        }
                    }
                }
            } else if total == 0 {
                p { class: "empty", "This recipe has no steps yet. Add some with Edit." }
            } else {
                div { class: "progress",
                    for i in 0..total {
                        button {
                            key: "{i}",
                            class: if i == current { "dot on" } else if i < current { "dot done" } else { "dot" },
                            aria_label: "Go to step {i + 1}",
                            onclick: move |_| step.set(i),
                        }
                    }
                }
                p { class: "meta", "Step {current + 1} of {total}" }
                p { class: "cook-step", "{recipe.steps[current]}" }
                div { class: "cook-nav",
                    button {
                        disabled: current == 0,
                        onclick: move |_| step.set(current.saturating_sub(1)),
                        "Back"
                    }
                    if is_last {
                        button {
                            class: "primary",
                            onclick: move |_| {
                                usage.write().record(Event::CookFinished);
                                finished.set(true);
                            },
                            "Done"
                        }
                    } else {
                        button { class: "primary", onclick: move |_| step.set(current + 1), "Next" }
                    }
                }
            }
        }
    }
}
