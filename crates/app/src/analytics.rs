//! Opt-in usage statistics.
//!
//! Two switches, both off by default: `collect` keeps counters on the device,
//! and `send` (only possible while collecting) uploads them at most once a
//! day. Turning collection off wipes the counters and the install id. This
//! file is per-device and never synced.

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use hestia_core::usage::{Event, MAX_COUNT, UsageReport};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const FILE: &str = "usage.json";

/// Where reports go. Set at build time, e.g.
/// `HESTIA_EVENTS_URL=https://example.com/api/events dx bundle ...`;
/// builds without it never send anything.
pub const EVENTS_URL: Option<&str> = option_env!("HESTIA_EVENTS_URL");

const REPORT_EVERY: chrono::Duration = chrono::Duration::days(1);

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Usage {
    collect: bool,
    send: bool,
    install_id: Option<Uuid>,
    counts: BTreeMap<Event, u64>,
    last_attempt: Option<DateTime<Utc>>,
}

impl Usage {
    pub fn collecting(&self) -> bool {
        self.collect
    }

    pub fn sending(&self) -> bool {
        self.send
    }

    pub fn set_collect(&mut self, on: bool) {
        if on == self.collect {
            return;
        }
        *self = if on {
            // A fresh id each time, so separate opt-ins can't be linked.
            Usage { collect: true, install_id: Some(Uuid::new_v4()), ..Default::default() }
        } else {
            Usage::default()
        };
    }

    pub fn set_send(&mut self, on: bool) {
        self.send = on && self.collect;
    }

    pub fn record(&mut self, event: Event) {
        if self.collect {
            let n = self.counts.entry(event).or_default();
            *n = (*n + 1).min(MAX_COUNT);
        }
    }

    /// The report that would be sent right now, if there is anything to send.
    pub fn report(&self) -> Option<UsageReport> {
        let install_id = self.install_id.filter(|_| self.collect && !self.counts.is_empty())?;
        Some(UsageReport {
            install_id,
            app_version: env!("CARGO_PKG_VERSION").into(),
            platform: std::env::consts::OS.into(),
            counts: self.counts.clone(),
        })
    }

    fn due(&self, now: DateTime<Utc>) -> bool {
        self.collect
            && self.send
            && !self.counts.is_empty()
            && self.last_attempt.is_none_or(|t| now - t >= REPORT_EVERY)
    }

    /// Forget what `report` covered, keeping anything counted since it was built.
    fn delivered(&mut self, report: &UsageReport) {
        if self.install_id != Some(report.install_id) {
            return; // stats were switched off (and maybe on again) meanwhile
        }
        for (event, sent) in &report.counts {
            if let Some(n) = self.counts.get_mut(event) {
                *n = n.saturating_sub(*sent);
            }
        }
        self.counts.retain(|_, n| *n > 0);
    }
}

/// Send the counters if the user opted in and a day has passed since the last try.
pub async fn upload_if_due(mut usage: Signal<Usage>) {
    let Some(url) = EVENTS_URL else { return };
    let now = Utc::now();
    let report = {
        let u = usage.peek();
        if !u.due(now) {
            return;
        }
        u.report()
    };
    let Some(report) = report else { return };
    usage.write().last_attempt = Some(now);

    match post(url, &report).await {
        Ok(status) if status.is_success() => usage.write().delivered(&report),
        Ok(status) if status.is_client_error() && status != reqwest::StatusCode::TOO_MANY_REQUESTS => {
            // The server will never accept this report; don't retry it forever.
            eprintln!("usage report rejected: {status}");
            usage.write().delivered(&report);
        }
        // Offline or server trouble: keep the counts and try again tomorrow.
        Ok(status) => eprintln!("usage report failed: {status}"),
        Err(e) => eprintln!("usage report failed: {e}"),
    }
}

async fn post(url: &str, report: &UsageReport) -> reqwest::Result<reqwest::StatusCode> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(15)).build()?;
    let res = client
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(serde_json::to_vec(report).expect("report serializes"))
        .send()
        .await?;
    Ok(res.status())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opted_in() -> Usage {
        let mut u = Usage::default();
        u.set_collect(true);
        u
    }

    #[test]
    fn off_by_default_and_records_nothing() {
        let mut u = Usage::default();
        assert!(!u.collecting() && !u.sending());
        u.record(Event::CookStarted);
        u.set_send(true);
        assert!(!u.sending(), "can't send without collecting");
        assert_eq!(u.report(), None);
    }

    #[test]
    fn switching_off_wipes_everything() {
        let mut u = opted_in();
        u.set_send(true);
        u.record(Event::CookStarted);
        let first_id = u.report().unwrap().install_id;

        u.set_collect(false);
        assert_eq!(u, Usage::default());

        u.set_collect(true);
        u.record(Event::CookStarted);
        assert!(!u.sending());
        assert_ne!(u.report().unwrap().install_id, first_id);
    }

    #[test]
    fn report_is_valid() {
        let mut u = opted_in();
        u.record(Event::RecipeCreated);
        u.record(Event::RecipeCreated);
        let r = u.report().unwrap();
        assert_eq!(r.counts, BTreeMap::from([(Event::RecipeCreated, 2)]));
        assert_eq!(r.validate(), Ok(()));
    }

    #[test]
    fn reports_daily_only_when_sending() {
        let now = Utc::now();
        let mut u = opted_in();
        u.record(Event::AppLaunched);
        assert!(!u.due(now));
        u.set_send(true);
        assert!(u.due(now));
        u.last_attempt = Some(now - chrono::Duration::hours(3));
        assert!(!u.due(now));
        u.last_attempt = Some(now - REPORT_EVERY);
        assert!(u.due(now));
    }

    #[test]
    fn delivery_keeps_newer_counts() {
        let mut u = opted_in();
        u.record(Event::CookStarted);
        u.record(Event::AppLaunched);
        let report = u.report().unwrap();
        u.record(Event::CookStarted); // happened while the request was in flight
        u.delivered(&report);
        assert_eq!(u.counts, BTreeMap::from([(Event::CookStarted, 1)]));

        // A report from before a re-opt-in must not touch the new counters.
        u.set_collect(false);
        u.set_collect(true);
        u.record(Event::CookStarted);
        u.delivered(&report);
        assert_eq!(u.counts, BTreeMap::from([(Event::CookStarted, 1)]));
    }
}
