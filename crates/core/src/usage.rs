//! Opt-in usage statistics: the only data the app sends home on its own.
//! (The other upload is a recipe the user explicitly shares; see `share`.)
//!
//! A report is a handful of counters plus an anonymous install id, app
//! version and platform. There is deliberately no field that could carry
//! recipe content, URLs, or anything typed by the user.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Something worth counting. Serialized as a snake_case string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    AppLaunched,
    RecipeCreated,
    RecipeImported,
    ImportFailed,
    CookStarted,
    CookFinished,
    GroceriesAdded,
    RecipeShared,
}

impl Event {
    pub const ALL: [Event; 8] = [
        Event::AppLaunched,
        Event::RecipeCreated,
        Event::RecipeImported,
        Event::ImportFailed,
        Event::CookStarted,
        Event::CookFinished,
        Event::GroceriesAdded,
        Event::RecipeShared,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Event::AppLaunched => "app_launched",
            Event::RecipeCreated => "recipe_created",
            Event::RecipeImported => "recipe_imported",
            Event::ImportFailed => "import_failed",
            Event::CookStarted => "cook_started",
            Event::CookFinished => "cook_finished",
            Event::GroceriesAdded => "groceries_added",
            Event::RecipeShared => "recipe_shared",
        }
    }
}

pub const PLATFORMS: [&str; 5] = ["ios", "macos", "android", "linux", "windows"];
/// Upper bound for one counter in one report; anything above is not a real device.
pub const MAX_COUNT: u64 = 10_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageReport {
    /// Random per install, regenerated every time stats are switched on.
    pub install_id: Uuid,
    pub app_version: String,
    pub platform: String,
    /// Occurrences since the last successful report.
    pub counts: BTreeMap<Event, u64>,
}

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum ReportError {
    #[error("report has no counts")]
    Empty,
    #[error("unknown platform")]
    Platform,
    #[error("invalid app version")]
    Version,
    #[error("count out of range")]
    Count,
}

impl UsageReport {
    pub fn validate(&self) -> Result<(), ReportError> {
        if self.counts.is_empty() {
            return Err(ReportError::Empty);
        }
        if !PLATFORMS.contains(&self.platform.as_str()) {
            return Err(ReportError::Platform);
        }
        let v = &self.app_version;
        if v.is_empty() || v.len() > 32 || !v.chars().all(|c| c.is_ascii_alphanumeric() || ".-+".contains(c)) {
            return Err(ReportError::Version);
        }
        if self.counts.values().any(|&n| n == 0 || n > MAX_COUNT) {
            return Err(ReportError::Count);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> UsageReport {
        UsageReport {
            install_id: Uuid::new_v4(),
            app_version: "0.1.0".into(),
            platform: "ios".into(),
            counts: BTreeMap::from([(Event::CookStarted, 3), (Event::RecipeCreated, 1)]),
        }
    }

    #[test]
    fn wire_format_is_plain_strings() {
        let json = serde_json::to_value(report()).unwrap();
        assert_eq!(json["counts"]["cook_started"], 3);
        for e in Event::ALL {
            assert_eq!(serde_json::to_value(e).unwrap(), e.as_str());
        }
    }

    #[test]
    fn rejects_unknown_fields_and_events() {
        let mut json = serde_json::to_value(report()).unwrap();
        json["email"] = "a@b.c".into();
        assert!(serde_json::from_value::<UsageReport>(json).is_err());

        let mut json = serde_json::to_value(report()).unwrap();
        json["counts"]["recipe_title"] = 1.into();
        assert!(serde_json::from_value::<UsageReport>(json).is_err());
    }

    #[test]
    fn validates_values() {
        assert_eq!(report().validate(), Ok(()));
        let bad = |f: fn(&mut UsageReport)| {
            let mut r = report();
            f(&mut r);
            r.validate()
        };
        assert_eq!(bad(|r| r.counts.clear()), Err(ReportError::Empty));
        assert_eq!(bad(|r| r.platform = "toaster".into()), Err(ReportError::Platform));
        assert_eq!(bad(|r| r.app_version = "<script>".into()), Err(ReportError::Version));
        assert_eq!(bad(|r| { r.counts.insert(Event::AppLaunched, MAX_COUNT + 1); }), Err(ReportError::Count));
    }
}
