// rsx! expands "{string}" text nodes to format!(), which trips this lint.
#![allow(clippy::useless_format)]

mod analytics;
mod photos;
mod store;
mod views;
mod wake;

use std::cell::RefCell;
use std::rc::Rc;

use dioxus::prelude::*;
use uuid::Uuid;

use analytics::Usage;
use hestia_core::Event;
use store::{AppData, Library};
use views::{EditRecipe, Groceries, Home, NewRecipe, RecipeDetail, Settings};

const CSS: &str = include_str!("../assets/main.css");

#[derive(Routable, Clone, PartialEq, Debug)]
#[rustfmt::skip]
enum Route {
    #[layout(Shell)]
        #[route("/")]
        Home {},
        #[route("/recipe/new")]
        NewRecipe {},
        #[route("/recipe/:id")]
        RecipeDetail { id: Uuid },
        #[route("/recipe/:id/edit")]
        EditRecipe { id: Uuid },
        #[route("/groceries")]
        Groceries {},
        #[route("/settings")]
        Settings {},
}

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    let (library, data) = use_hook(|| {
        let dir = store::app_dir();
        // Before opening, so the copy predates any tidying up open does.
        let today = chrono::Local::now().date_naive();
        if let Err(e) = store::snapshot(&dir, &dir.join("backups"), today) {
            eprintln!("failed to back up the library: {e}");
        }
        let (library, data) = Library::open(dir);
        (Rc::new(RefCell::new(library)), data)
    });
    let data = use_context_provider(|| Signal::new(data));
    use_context_provider(|| library.clone());
    // Persist whenever the library changes.
    let saver = library.clone();
    use_effect(move || saver.borrow_mut().save(&data.read()));
    photos::use_photo_cache(library, data);

    let mut usage = use_context_provider(|| Signal::new(store::read_json::<Usage>(analytics::FILE)));
    use_effect(move || store::write_json(analytics::FILE, &*usage.read()));
    use_hook(move || {
        spawn(async move {
            usage.write().record(Event::AppLaunched);
            analytics::upload_if_due(usage).await;
        })
    });

    rsx! {
        style { {CSS} }
        Router::<Route> {}
    }
}

#[component]
fn Shell() -> Element {
    rsx! {
        div { class: "shell",
            nav { class: "sidebar",
                div { class: "brand", FlameIcon {} "Hestia" }
                Link { to: Route::Home {}, class: "nav-link", active_class: "active", BookIcon {} "Recipes" }
                Link { to: Route::Groceries {}, class: "nav-link", active_class: "active", BasketIcon {} "Groceries" }
                Link { to: Route::Settings {}, class: "nav-link", active_class: "active", GearIcon {} "Settings" }
            }
            main { class: "content", Outlet::<Route> {} }
        }
    }
}

// Icons are drawn inline so the app has no asset files to ship.

#[component]
fn FlameIcon() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "currentColor", "aria-hidden": "true",
            path { d: "M12 2c1 4 5 6 5 11a5 5 0 0 1-10 0c0-2 1-3 2-4 0 2 1 3 2 3 0-3 1-6 1-10z" }
        }
    }
}

#[component]
fn BookIcon() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor",
            stroke_width: "1.8", stroke_linecap: "round", stroke_linejoin: "round", "aria-hidden": "true",
            path { d: "M4 4.5A2.5 2.5 0 0 1 6.5 2H20v16H6.5A2.5 2.5 0 0 0 4 20.5z" }
            path { d: "M4 20.5V4.5M6.5 18H20v4H6.5A2.5 2.5 0 0 1 4 20.5" }
        }
    }
}

#[component]
fn BasketIcon() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor",
            stroke_width: "1.8", stroke_linecap: "round", stroke_linejoin: "round", "aria-hidden": "true",
            path { d: "M3 10h18l-1.5 10h-15z" }
            path { d: "M8 10l3-6M16 10l-3-6M9 14v3M15 14v3" }
        }
    }
}

#[component]
fn GearIcon() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor",
            stroke_width: "1.8", stroke_linecap: "round", stroke_linejoin: "round", "aria-hidden": "true",
            circle { cx: "12", cy: "12", r: "3" }
            path { d: "M12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M4.9 19.1L7 17M17 7l2.1-2.1" }
        }
    }
}

pub fn use_data() -> Signal<AppData> {
    use_context()
}

pub fn use_usage() -> Signal<Usage> {
    use_context()
}

pub fn use_library() -> Rc<RefCell<Library>> {
    use_context()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_round_trip() {
        let id = Uuid::new_v4();
        for route in [
            Route::Home {},
            Route::NewRecipe {},
            Route::RecipeDetail { id },
            Route::EditRecipe { id },
            Route::Settings {},
        ] {
            let url = route.to_string();
            assert_eq!(url.parse::<Route>().ok(), Some(route), "{url}");
        }
    }
}
