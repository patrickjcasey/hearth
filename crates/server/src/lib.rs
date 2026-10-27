//! The Hestia website: a landing page, the privacy and support pages the App
//! Store requires, an endpoint that receives opt-in usage reports, and share
//! links. Recipes live on-device and in iCloud; the only recipes that reach the
//! server are ones a user explicitly shares as a link, and those expire.

use std::str::FromStr;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use hestia_core::share::{SHARE_DAYS, ShareError, ShareLink, SharedRecipe};
use hestia_core::usage::{ReportError, UsageReport};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};
use tower_http::trace::TraceLayer;

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    /// Public origin used to build share links, e.g. `https://hestia.example.com`.
    pub public_url: String,
}

/// Reports are a few hundred bytes; refuse anything much larger.
const MAX_REPORT_BYTES: usize = 4 * 1024;
/// The app reports at most daily, so more often than this is a retry loop or abuse.
const MIN_REPORT_INTERVAL: Duration = Duration::hours(1);
/// A long recipe is a few KB of text; photos are links, never uploaded.
const MAX_SHARE_BYTES: usize = 64 * 1024;
/// How long a share link works.
pub const SHARE_LIFETIME: Duration = Duration::days(SHARE_DAYS);
/// Caps how fast the shares table can grow, whoever is posting.
const MAX_SHARES_PER_HOUR: i64 = 600;
const SHARE_ID_LEN: usize = 10;

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
        .route("/api/share", post(create_share).layer(DefaultBodyLimit::max(MAX_SHARE_BYTES)))
        .route("/r/{id}", get(show_share))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn page(title: &str, body: &str) -> Html<String> {
    Html(layout(title, "", body))
}

fn layout(title: &str, head: &str, body: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<link rel="stylesheet" href="/style.css">
{head}</head>
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
    )
}

async fn stylesheet() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], include_str!("../site/style.css"))
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error(transparent)]
    Invalid(#[from] ReportError),
    #[error(transparent)]
    InvalidShare(#[from] ShareError),
    #[error("too many requests")]
    TooSoon,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self {
            ApiError::Invalid(_) | ApiError::InvalidShare(_) => StatusCode::UNPROCESSABLE_ENTITY,
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

/// Timestamps in `shares` use one fixed format so they compare as text.
fn db_time(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// A random, URL-safe id with 60 bits of entropy.
fn share_id() -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let bytes = uuid::Uuid::new_v4().into_bytes();
    // Bytes 6 and 8 carry the UUID version and variant bits; skip them.
    const RANDOM_BYTES: [usize; SHARE_ID_LEN] = [0, 1, 2, 3, 4, 5, 9, 10, 11, 12];
    RANDOM_BYTES
        .iter()
        .map(|&i| ALPHABET[(bytes[i] & 63) as usize] as char)
        .collect()
}

async fn create_share(
    State(s): State<AppState>,
    Json(recipe): Json<SharedRecipe>,
) -> Result<(StatusCode, Json<ShareLink>), ApiError> {
    recipe.validate()?;
    let now = Utc::now();
    let expires_at = now + SHARE_LIFETIME;
    let json = serde_json::to_string(&recipe).expect("recipe serializes");

    let mut tx = s.db.begin().await?;
    sqlx::query("DELETE FROM shares WHERE expires_at <= ?").bind(db_time(now)).execute(&mut *tx).await?;
    let (recent,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM shares WHERE created_at > ?")
        .bind(db_time(now - Duration::hours(1)))
        .fetch_one(&mut *tx)
        .await?;
    if recent >= MAX_SHARES_PER_HOUR {
        return Err(ApiError::TooSoon);
    }
    let mut attempt = 0;
    let id = loop {
        let id = share_id();
        let inserted = sqlx::query("INSERT INTO shares (id, recipe, created_at, expires_at) VALUES (?, ?, ?, ?)")
            .bind(&id)
            .bind(&json)
            .bind(db_time(now))
            .bind(db_time(expires_at))
            .execute(&mut *tx)
            .await;
        match inserted {
            Ok(_) => break id,
            // A collision is astronomically unlikely, but cheap to survive.
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() && attempt < 3 => attempt += 1,
            Err(e) => return Err(e.into()),
        }
    };
    tx.commit().await?;

    let url = format!("{}/r/{id}", s.public_url.trim_end_matches('/'));
    Ok((StatusCode::CREATED, Json(ShareLink { url, expires_at })))
}

async fn show_share(State(s): State<AppState>, Path(id): Path<String>) -> Result<Response, ApiError> {
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT recipe, expires_at FROM shares WHERE id = ? AND expires_at > ?")
            .bind(&id)
            .bind(db_time(Utc::now()))
            .fetch_optional(&s.db)
            .await?;
    let noindex = [(header::HeaderName::from_static("x-robots-tag"), "noindex")];
    let found = row.and_then(|(json, expires)| {
        Some((serde_json::from_str::<SharedRecipe>(&json).ok()?, DateTime::parse_from_rfc3339(&expires).ok()?))
    });
    let Some((recipe, expires_at)) = found else {
        let body = format!(
            "<article class=\"prose\"><h1>This link has expired</h1>\n\
             <p>Shared recipes are kept for {} days. Ask whoever sent it to share it again.</p></article>",
            SHARE_LIFETIME.num_days()
        );
        return Ok((StatusCode::NOT_FOUND, noindex, Html(layout("Link expired · Hestia", "", &body)))
            .into_response());
    };
    let title = format!("{} · Hestia", esc(&recipe.title));
    let html = layout(&title, &share_head(&recipe), &share_body(&recipe, expires_at.with_timezone(&Utc)));
    Ok((noindex, Html(html)).into_response())
}

/// Link-preview tags for messaging apps, and a hint to keep shares out of search.
fn share_head(r: &SharedRecipe) -> String {
    let mut head = String::from("<meta name=\"robots\" content=\"noindex\">\n");
    head += &format!("<meta property=\"og:type\" content=\"article\">\n<meta property=\"og:title\" content=\"{}\">\n", esc(&r.title));
    if let Some(d) = &r.description {
        head += &format!("<meta property=\"og:description\" content=\"{}\">\n", esc(d));
    }
    if let Some(img) = &r.image_url {
        head += &format!("<meta property=\"og:image\" content=\"{}\">\n", esc(img));
    }
    head
}

fn share_body(r: &SharedRecipe, expires_at: DateTime<Utc>) -> String {
    let mut meta = Vec::new();
    if let Some(a) = &r.author {
        meta.push(format!("by {}", esc(a)));
    }
    match (&r.yield_text, r.servings) {
        (Some(y), _) => meta.push(format!("Makes {}", esc(y))),
        (None, Some(n)) => meta.push(format!("Serves {n}")),
        _ => {}
    }
    for (label, m) in [("Prep", r.prep_minutes), ("Cook", r.cook_minutes), ("Total", r.total_minutes)] {
        if let Some(m) = m {
            meta.push(format!("{label} {m} min"));
        }
    }
    let list = |items: &[String]| items.iter().map(|i| format!("  <li>{}</li>\n", esc(i))).collect::<String>();

    let mut b = format!("<article class=\"prose shared-recipe\">\n<h1>{}</h1>\n", esc(&r.title));
    if !meta.is_empty() {
        b += &format!("<p class=\"meta\">{}</p>\n", meta.join(" · "));
    }
    if let Some(img) = &r.image_url {
        b += &format!("<img class=\"photo\" src=\"{}\" alt=\"\">\n", esc(img));
    }
    if let Some(d) = &r.description {
        b += &format!("<p>{}</p>\n", esc(d));
    }
    if !r.ingredients.is_empty() {
        b += &format!("<h2>Ingredients</h2>\n<ul>\n{}</ul>\n", list(&r.ingredients));
    }
    if !r.steps.is_empty() {
        b += &format!("<h2>Steps</h2>\n<ol>\n{}</ol>\n", list(&r.steps));
    }
    if !r.tags.is_empty() {
        let tags: Vec<_> = r.tags.iter().map(|t| esc(t)).collect();
        b += &format!("<p class=\"meta\">{}</p>\n", tags.join(" · "));
    }
    if let Some(src) = &r.source_url {
        b += &format!(
            "<p class=\"meta\">Originally from <a href=\"{0}\" rel=\"nofollow noopener\">{0}</a></p>\n",
            esc(src)
        );
    }
    b += &format!(
        "<div class=\"feature\"><p>To save this recipe in Hestia, copy this page's link and paste it into \
         the import box. Most recipe apps that import from a link can read it too. \
         This link works until {}.</p></div>\n",
        expires_at.format("%B %-d, %Y")
    );
    b += &format!("<script type=\"application/ld+json\">{}</script>\n</article>", r.json_ld_script());
    b
}

/// Escapes text for HTML element content and quoted attribute values.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use hestia_core::{Event, Ingredient, Recipe, parse_recipe_html};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    use uuid::Uuid;

    async fn app() -> (Router, SqlitePool) {
        let db = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!().run(&db).await.unwrap();
        (router(AppState { db: db.clone(), public_url: "https://hestia.test/".into() }), db)
    }

    fn post_to(path: &str, body: impl Into<Body>) -> Request<Body> {
        Request::post(path)
            .header("content-type", "application/json")
            .body(body.into())
            .unwrap()
    }

    fn post_json(body: impl Into<Body>) -> Request<Body> {
        post_to("/api/events", body)
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

    async fn get_text(app: &Router, path: &str) -> (StatusCode, String) {
        let res = app.clone().oneshot(Request::get(path).body(Body::empty()).unwrap()).await.unwrap();
        let status = res.status();
        let body = res.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn serves_pages() {
        let (app, _) = app().await;
        for (path, needle) in [("/", "Free, forever"), ("/privacy", "off by default"), ("/support", "MIT")] {
            let (status, html) = get_text(&app, path).await;
            assert_eq!(status, StatusCode::OK, "{path}");
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
        let app = router(AppState { db: db.clone(), public_url: "https://hestia.test/".into() });
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

    fn recipe() -> Recipe {
        let mut r = Recipe::new("Weeknight Dal");
        r.description = Some("Comforting <and> quick.".into());
        r.author = Some("Priya".into());
        r.source_url = Some("https://example.com/dal".into());
        r.servings = Some(4);
        r.prep_minutes = Some(10);
        r.cook_minutes = Some(30);
        r.ingredients = ["1 cup red lentils", "2 1/2 cups water", "1 tsp turmeric"].map(Ingredient::parse).to_vec();
        r.steps = vec!["Rinse the lentils.".into(), "Simmer everything for 25 minutes.".into()];
        r.tags = vec!["dinner".into(), "vegan".into()];
        r.notes = Some("Double the garlic next time".into());
        r
    }

    async fn share(app: &Router, r: &SharedRecipe) -> ShareLink {
        let res = app.clone().oneshot(post_to("/api/share", serde_json::to_vec(r).unwrap())).await.unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let body = res.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn shared_link_shows_and_imports_recipe() {
        let (app, _) = app().await;
        let original = recipe();
        let link = share(&app, &SharedRecipe::from(&original)).await;
        let id = link.url.strip_prefix("https://hestia.test/r/").expect("link on public url");
        assert_eq!(id.len(), SHARE_ID_LEN);
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        assert!(link.expires_at > Utc::now() + SHARE_LIFETIME - Duration::minutes(1));

        let (status, html) = get_text(&app, &format!("/r/{id}")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(html.contains("<h1>Weeknight Dal</h1>"));
        assert!(html.contains("Comforting &lt;and&gt; quick."), "text is escaped");
        assert!(html.contains("<li>2 1/2 cups water</li>"));
        assert!(html.contains("noindex"));
        assert!(!html.contains("garlic"), "notes are never shared");

        // The page is importable by the app's own importer.
        let back = parse_recipe_html(&html, Some(&link.url)).unwrap();
        assert_eq!(back.title, original.title);
        assert_eq!(back.ingredients, original.ingredients);
        assert_eq!(back.steps, original.steps);
        assert_eq!(back.servings, Some(4));
        assert_eq!((back.prep_minutes, back.cook_minutes), (Some(10), Some(30)));
    }

    #[tokio::test]
    async fn rejects_bad_shares() {
        let (app, db) = app().await;
        let mut untitled = SharedRecipe::from(&recipe());
        untitled.title = " ".into();
        let req = post_to("/api/share", serde_json::to_vec(&untitled).unwrap());
        assert_eq!(status(&app, req).await, StatusCode::UNPROCESSABLE_ENTITY);

        // A whole local Recipe (with id, notes, favorite) is not a SharedRecipe.
        let req = post_to("/api/share", serde_json::to_vec(&recipe()).unwrap());
        assert_eq!(status(&app, req).await, StatusCode::UNPROCESSABLE_ENTITY);

        let huge = vec![b' '; MAX_SHARE_BYTES + 1];
        assert_eq!(status(&app, post_to("/api/share", huge)).await, StatusCode::PAYLOAD_TOO_LARGE);

        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM shares").fetch_one(&db).await.unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn expired_shares_are_gone() {
        let (app, db) = app().await;
        let past = db_time(Utc::now() - Duration::days(1));
        let json = serde_json::to_string(&SharedRecipe::from(&recipe())).unwrap();
        sqlx::query("INSERT INTO shares (id, recipe, created_at, expires_at) VALUES ('old', ?, ?, ?)")
            .bind(&json)
            .bind(&past)
            .bind(&past)
            .execute(&db)
            .await
            .unwrap();

        let (status, html) = get_text(&app, "/r/old").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(html.contains("expired"));
        assert_eq!(get_text(&app, "/r/nope").await.0, StatusCode::NOT_FOUND);

        // Creating a new share sweeps out expired ones.
        share(&app, &SharedRecipe::from(&recipe())).await;
        let ids: Vec<(String,)> = sqlx::query_as("SELECT id FROM shares").fetch_all(&db).await.unwrap();
        assert_eq!(ids.len(), 1);
        assert_ne!(ids[0].0, "old");
    }

    #[tokio::test]
    async fn caps_shares_per_hour() {
        let (app, db) = app().await;
        let now = db_time(Utc::now());
        let later = db_time(Utc::now() + SHARE_LIFETIME);
        for i in 0..MAX_SHARES_PER_HOUR {
            sqlx::query("INSERT INTO shares (id, recipe, created_at, expires_at) VALUES (?, '{}', ?, ?)")
                .bind(i.to_string())
                .bind(&now)
                .bind(&later)
                .execute(&db)
                .await
                .unwrap();
        }
        let req = post_to("/api/share", serde_json::to_vec(&SharedRecipe::from(&recipe())).unwrap());
        assert_eq!(status(&app, req).await, StatusCode::TOO_MANY_REQUESTS);
    }

    #[test]
    fn escapes_html() {
        assert_eq!(esc(r#"<a href="x">Tom & Jerry's</a>"#), "&lt;a href=&quot;x&quot;&gt;Tom &amp; Jerry&#39;s&lt;/a&gt;");
    }
}
