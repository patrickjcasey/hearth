//! The Hestia website: a landing page, the privacy and support pages the App
//! Store requires, and an endpoint that receives opt-in usage reports.
//! Recipes never reach the server; they live on-device and in iCloud.

use std::str::FromStr;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use chrono::{Duration, Utc};
use hestia_core::usage::{ReportError, UsageReport};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};
use tower_http::trace::TraceLayer;

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
}

/// Reports are a few hundred bytes; refuse anything much larger.
const MAX_REPORT_BYTES: usize = 4 * 1024;
/// The app reports at most daily, so more often than this is a retry loop or abuse.
const MIN_REPORT_INTERVAL: Duration = Duration::hours(1);

/// Opens the database and applies migrations. WAL lets readers run beside
/// the writer, and writers queue for up to the busy timeout.
pub async fn connect(url: &str) -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::from_str(url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(std::time::Duration::from_secs(10));
    let db = SqlitePool::connect_with(options).await?;
    sqlx::migrate!().run(&db).await?;
    Ok(db)
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(|| page("Hestia: a free recipe box", include_str!("../site/index.html"))))
        .route("/privacy", get(|| page("Privacy policy · Hestia", include_str!("../site/privacy.html"))))
        .route("/support", get(|| page("Support · Hestia", include_str!("../site/support.html"))))
        .route("/style.css", get(stylesheet))
        .route("/health", get(|| async { "ok" }))
        .route("/api/events", post(receive_report).layer(DefaultBodyLimit::max(MAX_REPORT_BYTES)))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn page(title: &str, body: &str) -> Html<String> {
    Html(format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<link rel="stylesheet" href="/style.css">
</head>
<body>
<div class="wrap">
<header class="site">
  <a class="brand" href="/">Hestia</a>
  <nav><a href="/support">Support</a><a href="/privacy">Privacy</a></nav>
</header>
{body}
<footer class="site">
  <a href="/privacy">Privacy</a><a href="/support">Support</a>
  <span>Free and open source (MIT).</span>
</footer>
</div>
</body>
</html>
"#
    ))
}

async fn stylesheet() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], include_str!("../site/style.css"))
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error(transparent)]
    Invalid(#[from] ReportError),
    #[error("too many reports")]
    TooSoon,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self {
            ApiError::Invalid(_) => StatusCode::UNPROCESSABLE_ENTITY,
            ApiError::TooSoon => StatusCode::TOO_MANY_REQUESTS,
            ApiError::Db(_) => {
                tracing::error!(error = %self, "internal error");
                return (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response();
            }
        };
        (status, self.to_string()).into_response()
    }
}

async fn receive_report(
    State(s): State<AppState>,
    Json(report): Json<UsageReport>,
) -> Result<StatusCode, ApiError> {
    report.validate()?;
    let install_id = report.install_id.to_string();
    // SQLite's own datetime format, so `datetime('now', ...)` compares correctly.
    let fmt = |t: chrono::DateTime<Utc>| t.format("%Y-%m-%d %H:%M:%S").to_string();
    let now = Utc::now();

    // IMMEDIATE takes the write lock up front. A deferred transaction would
    // read, then fail with SQLITE_BUSY when another request also wants to write.
    let mut tx = s.db.begin_with("BEGIN IMMEDIATE").await?;
    let recent: Option<(i64,)> =
        sqlx::query_as("SELECT 1 FROM usage_events WHERE install_id = ? AND received_at > ? LIMIT 1")
            .bind(&install_id)
            .bind(fmt(now - MIN_REPORT_INTERVAL))
            .fetch_optional(&mut *tx)
            .await?;
    if recent.is_some() {
        return Err(ApiError::TooSoon);
    }
    for (event, count) in &report.counts {
        sqlx::query(
            "INSERT INTO usage_events (install_id, app_version, platform, event, count, received_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&install_id)
        .bind(&report.app_version)
        .bind(&report.platform)
        .bind(event.as_str())
        .bind(*count as i64)
        .bind(fmt(now))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use hestia_core::Event;
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    use uuid::Uuid;

    async fn app() -> (Router, SqlitePool) {
        let db = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!().run(&db).await.unwrap();
        (router(AppState { db: db.clone() }), db)
    }

    fn post_json(body: impl Into<Body>) -> Request<Body> {
        Request::post("/api/events")
            .header("content-type", "application/json")
            .body(body.into())
            .unwrap()
    }

    fn report() -> UsageReport {
        UsageReport {
            install_id: Uuid::new_v4(),
            app_version: "0.1.0".into(),
            platform: "ios".into(),
            counts: BTreeMap::from([(Event::CookStarted, 2), (Event::RecipeImported, 5)]),
        }
    }

    async fn status(app: &Router, req: Request<Body>) -> StatusCode {
        app.clone().oneshot(req).await.unwrap().status()
    }

    #[tokio::test]
    async fn serves_pages() {
        let (app, _) = app().await;
        for (path, needle) in [("/", "Free, forever"), ("/privacy", "off by default"), ("/support", "MIT")] {
            let res = app.clone().oneshot(Request::get(path).body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(res.status(), StatusCode::OK, "{path}");
            let body = res.into_body().collect().await.unwrap().to_bytes();
            let html = String::from_utf8(body.to_vec()).unwrap();
            assert!(html.contains(needle), "{path} missing {needle:?}");
            assert!(html.contains(r#"href="/style.css""#));
        }
        let css = Request::get("/style.css").body(Body::empty()).unwrap();
        assert_eq!(status(&app, css).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn stores_report_once_per_interval() {
        let (app, db) = app().await;
        let r = report();
        let body = serde_json::to_vec(&r).unwrap();

        assert_eq!(status(&app, post_json(body.clone())).await, StatusCode::NO_CONTENT);
        let totals: Vec<(String, i64)> =
            sqlx::query_as("SELECT event, count FROM usage_events ORDER BY event").fetch_all(&db).await.unwrap();
        assert_eq!(totals, [("cook_started".into(), 2), ("recipe_imported".into(), 5)]);
        // Stored in SQLite's own format, so the README's datetime() query works.
        let (sqlite_format,): (bool,) =
            sqlx::query_as("SELECT received_at = datetime(received_at) FROM usage_events").fetch_one(&db).await.unwrap();
        assert!(sqlite_format);

        assert_eq!(status(&app, post_json(body)).await, StatusCode::TOO_MANY_REQUESTS);
        // A different install is unaffected.
        let other = serde_json::to_vec(&report()).unwrap();
        assert_eq!(status(&app, post_json(other)).await, StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn migration_converts_rfc3339_timestamps() {
        let (_, db) = app().await;
        sqlx::query(
            "INSERT INTO usage_events (install_id, app_version, platform, event, count, received_at)
             VALUES ('x', '0.1.0', 'ios', 'app_launched', 1, '2026-01-02T23:04:05.123456789-02:00')",
        )
        .execute(&db)
        .await
        .unwrap();
        sqlx::raw_sql(include_str!("../migrations/0002_sqlite_timestamps.sql")).execute(&db).await.unwrap();
        let (at,): (String,) = sqlx::query_as("SELECT received_at FROM usage_events").fetch_one(&db).await.unwrap();
        assert_eq!(at, "2026-01-03 01:04:05");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_reports_all_succeed() {
        // A file, not :memory:, so connections really contend for the lock.
        let path = std::env::temp_dir().join(format!("hestia-test-{}.db", Uuid::new_v4()));
        let db = connect(&format!("sqlite://{}", path.display())).await.unwrap();
        let app = router(AppState { db: db.clone() });
        let tasks: Vec<_> = (0..30)
            .map(|_| {
                let (app, body) = (app.clone(), serde_json::to_vec(&report()).unwrap());
                tokio::spawn(async move { status(&app, post_json(body)).await })
            })
            .collect();
        for t in tasks {
            assert_eq!(t.await.unwrap(), StatusCode::NO_CONTENT);
        }
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM usage_events").fetch_one(&db).await.unwrap();
        assert_eq!(n, 60);
        db.close().await;
        for ext in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
        }
    }

    #[tokio::test]
    async fn rejects_bad_reports() {
        let (app, db) = app().await;
        let mut empty = report();
        empty.counts.clear();
        assert_eq!(status(&app, post_json(serde_json::to_vec(&empty).unwrap())).await, StatusCode::UNPROCESSABLE_ENTITY);

        let mut extra = serde_json::to_value(report()).unwrap();
        extra["recipe"] = "secret family lasagna".into();
        assert_eq!(status(&app, post_json(extra.to_string())).await, StatusCode::UNPROCESSABLE_ENTITY);

        let huge = vec![b' '; MAX_REPORT_BYTES + 1];
        assert_eq!(status(&app, post_json(huge)).await, StatusCode::PAYLOAD_TOO_LARGE);

        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM usage_events").fetch_one(&db).await.unwrap();
        assert_eq!(n, 0);
    }
}
