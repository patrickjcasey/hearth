use dioxus::prelude::*;
use hestia_core::GroceryItem;

use crate::use_data;

#[component]
pub fn Groceries() -> Element {
    let mut data = use_data();
    let mut draft = use_signal(String::new);

    let add = move |e: FormEvent| {
        e.prevent_default();
        let text = draft.read().trim().to_string();
        if !text.is_empty() {
            data.write().groceries.push(GroceryItem::new(text, None));
            draft.set(String::new());
        }
    };
    let clear_checked = move |_| data.write().groceries.retain(|g| !g.checked);

    let items = data.read().groceries.clone();
    let any_checked = items.iter().any(|g| g.checked);

    rsx! {
        h1 { "Groceries" }
        form { class: "import", onsubmit: add,
            input {
                placeholder: "Add an item…",
                value: "{draft}",
                oninput: move |e| draft.set(e.value()),
            }
            button { r#type: "submit", "Add" }
        }
        if items.is_empty() {
            p { class: "empty", "Your list is empty. Add ingredients from any recipe." }
        }
        ul { class: "groceries",
            for item in items {
                li { key: "{item.id}", class: if item.checked { "checked" },
                    label {
                        input {
                            r#type: "checkbox",
                            checked: item.checked,
                            onchange: move |_| {
                                if let Some(g) = data.write().groceries.iter_mut().find(|g| g.id == item.id) {
                                    g.checked = !g.checked;
                                }
                            },
                        }
                        "{item.text}"
                    }
                }
            }
        }
        if any_checked {
            button { onclick: clear_checked, "Clear checked items" }
        }
    }
}
