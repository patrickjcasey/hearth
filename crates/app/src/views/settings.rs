use dioxus::prelude::*;

use crate::store::{self, AppData};
use crate::{analytics, use_data, use_library, use_usage};

#[component]
pub fn Settings() -> Element {
    let mut usage = use_usage();
    let u = usage.read();
    let preview = u.report().map(|r| serde_json::to_string_pretty(&r).expect("report serializes"));

    rsx! {
        div { class: "settings",
        h1 { "Settings" }

        section { class: "settings-group",
            h2 { "Usage statistics" }
            p { class: "meta",
                "Help improve Hestia by counting how often features are used, such as imports "
                "and keeping the screen on. Counts never include recipes, links, or anything you type."
            }
            Toggle {
                label: "Collect usage statistics",
                detail: "Counted only on this device.",
                on: u.collecting(),
                onchange: move |on| usage.write().set_collect(on),
            }
            Toggle {
                label: "Send statistics to Hestia",
                detail: "At most once a day, anonymously.",
                on: u.sending(),
                disabled: !u.collecting(),
                onchange: move |on| usage.write().set_send(on),
            }
            if u.collecting() {
                h3 { "What would be sent" }
                if let Some(json) = preview {
                    pre { class: "report-preview", "{json}" }
                } else {
                    p { class: "meta", "Nothing yet." }
                }
                if analytics::EVENTS_URL.is_none() {
                    p { class: "meta", "This build has no statistics server configured, so nothing is sent." }
                }
            }
        }

        Backups {}

        section { class: "settings-group",
            h2 { "About" }
            p { class: "meta", "Hestia {env!(\"CARGO_PKG_VERSION\")}, free and open source under the MIT license." }
            // Which servers this build talks to, so a tester can confirm it.
            p { class: "meta",
                "Statistics server: {analytics::EVENTS_URL.unwrap_or(\"none in this build\")}. "
                "Share links: {crate::share::host().unwrap_or(\"none in this build\")}."
            }
        }
        }
    }
}

#[component]
fn Toggle(
    label: &'static str,
    detail: &'static str,
    on: bool,
    #[props(default)] disabled: bool,
    onchange: EventHandler<bool>,
) -> Element {
    rsx! {
        label { class: if disabled { "toggle-row disabled" } else { "toggle-row" },
            span {
                span { class: "label", "{label}" }
                span { class: "hint", "{detail}" }
            }
            input {
                r#type: "checkbox",
                role: "switch",
                class: "switch",
                checked: on,
                disabled,
                onchange: move |e| onchange.call(e.checked()),
            }
        }
    }
}

#[component]
fn Backups() -> Element {
    let mut data = use_data();
    let library = use_library();
    let mut status = use_signal(String::new);

    let export = move |_| {
        let today = chrono::Local::now().date_naive();
        status.set(match store::export(&data.read(), &store::export_dir(), today) {
            Ok(path) => format!("Saved to {}", path.display()),
            Err(e) => format!("Export failed: {e}"),
        });
    };

    let import = move |e: FormEvent| {
        let library = library.clone();
        async move {
            let Some(file) = e.files().into_iter().next() else { return };
            let text = match file.read_string().await {
                Ok(text) => text,
                Err(e) => return status.set(format!("Couldn't read {}: {e}", file.name())),
            };
            let incoming: AppData = match serde_json::from_str(&text) {
                Ok(incoming) => incoming,
                Err(_) => return status.set(format!("{} isn't a Hestia export.", file.name())),
            };
            let (merged, summary) = library.borrow().import(&data.read(), incoming);
            data.set(merged);
            status.set(format!("Added {} recipes and updated {}.", summary.added, summary.updated));
        }
    };

    rsx! {
        section { class: "settings-group",
            h2 { "Backups" }
            p { class: "meta",
                "Hestia keeps a copy of your library each day you open it, for the last "
                "{store::KEEP_SNAPSHOTS} days with changes, in the backups folder beside your recipes. "
                "Export a copy to keep somewhere else too."
            }
            div { class: "backup-actions",
                button { onclick: export, "Export library" }
                label { class: "button",
                    "Import export…"
                    input { r#type: "file", accept: ".json,application/json", hidden: true, onchange: import }
                }
            }
            p { class: "meta",
                "Importing adds missing recipes and keeps whichever copy of a recipe was changed "
                "last. Recipes deleted since the export stay deleted."
            }
            if !status.read().is_empty() {
                p { class: "meta", "{status}" }
            }
        }
    }
}
