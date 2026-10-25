use hestia_server::{AppState, connect, router};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tower_http=debug".into()))
        .init();

    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://hestia.db".into());
    // Loopback by default; put a TLS-terminating reverse proxy in front in production.
    let addr = std::env::var("HESTIA_ADDR").unwrap_or_else(|_| "127.0.0.1:8000".into());

    let db = connect(&db_url).await?;

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on http://{addr}");
    axum::serve(listener, router(AppState { db })).await?;
    Ok(())
}
