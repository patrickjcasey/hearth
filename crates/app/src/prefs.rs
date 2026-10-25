//! Per-device preferences. Stored next to usage.json and never synced.

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

pub const FILE: &str = "settings.json";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Keep the screen on while a recipe or cook mode is open.
    pub keep_awake: bool,
}

pub fn use_prefs() -> Signal<Prefs> {
    use_context()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_by_default_and_tolerates_other_files() {
        assert!(!Prefs::default().keep_awake);
        let empty: Prefs = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, Prefs::default());
        let on: Prefs = serde_json::from_str(r#"{"keep_awake":true,"later":1}"#).unwrap();
        assert!(on.keep_awake);
    }
}
