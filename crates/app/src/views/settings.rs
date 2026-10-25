use dioxus::prelude::*;

use crate::{analytics, use_usage};

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
                "and cook mode. Counts never include recipes, links, or anything you type."
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

        section { class: "settings-group",
            h2 { "About" }
            p { class: "meta", "Hestia {env!(\"CARGO_PKG_VERSION\")}, free and open source under the MIT license." }
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
