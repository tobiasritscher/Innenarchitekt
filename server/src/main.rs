#![recursion_limit = "256"]
//! Innenarchitekt backend: axum + Postgres + object store. See docs/api.md.

mod ai;
mod assets;
mod auth;
mod config;
mod error;
mod files;
mod floorplan;
mod glb;
mod ikea;
mod jobs;
mod llm;
mod photo3d;
mod projects;
mod state;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::extract::{DefaultBodyLimit, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use tokio::sync::{broadcast, watch, Notify};
use tower_http::compression::predicate::{NotForContentType, Predicate};
use tower_http::compression::{CompressionLayer, DefaultPredicate};
use tower_http::trace::TraceLayer;

use crate::config::Config;
use crate::error::ApiError;
use crate::llm::LlmClient;
use crate::state::AppState;

const BODY_LIMIT: usize = 60 * 1024 * 1024;

async fn health() -> Json<Value> {
    Json(json!({ "ok": true }))
}

async fn app_config(State(st): State<AppState>) -> Json<Value> {
    Json(json!({
        "llm_enabled": st.llm.is_some(),
        "llm_model": st.llm.as_ref().map(|l| l.model.clone()).unwrap_or_default(),
        "photo3d_enabled": st.cfg.photo3d_dir.is_some(),
    }))
}

async fn api_not_found() -> ApiError {
    ApiError::not_found("Unbekannter API-Pfad")
}

pub fn router(st: AppState) -> Router {
    let protected = Router::new()
        .route("/api/config", get(app_config))
        .route("/api/projects", get(projects::list).post(projects::create))
        .route("/api/projects/{id}", get(projects::get).delete(projects::delete))
        .route("/api/projects/{id}/ops", post(projects::push_ops))
        .route("/api/projects/{id}/floorplan", post(jobs::upload_floorplan))
        .route("/api/projects/{id}/ai", post(ai::command))
        .route("/api/geocode", post(projects::geocode))
        .route("/api/jobs/{id}", get(jobs::get))
        .route("/api/jobs/{id}/events", get(jobs::events))
        .route("/api/assets", get(assets::list))
        .route("/api/assets/ikea", post(assets::import_ikea))
        .route("/api/assets/upload", post(assets::upload))
        .route("/api/assets/primitive", post(assets::create_primitive))
        .route("/api/assets/from-photo", post(photo3d::upload))
        .route("/api/assets/{id}", get(assets::get).delete(assets::delete))
        .route("/api/assets/{id}/model.glb", get(assets::model))
        .route("/api/files/{*key}", get(files::get_file))
        .route_layer(axum::middleware::from_fn_with_state(st.clone(), auth::require_auth));
    let public = Router::new()
        .route("/api/health", get(health))
        .route("/api/auth", post(auth::login).get(auth::check))
        .route("/api", axum::routing::any(api_not_found))
        .route("/api/{*rest}", axum::routing::any(api_not_found));
    Router::new()
        .merge(protected)
        .merge(public)
        .fallback_service(files::web_router(&st.cfg.web_dir))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        // GLBs are already Draco/WebP-compressed; images and SSE are excluded by the default predicate.
        .layer(CompressionLayer::new().compress_when(DefaultPredicate::new().and(NotForContentType::const_new("model/gltf-binary"))))
        .layer(TraceLayer::new_for_http())
        .with_state(st)
}

/// The LLM module is owned by another component; never let its construction take the server down.
fn llm_from_env() -> Option<Arc<LlmClient>> {
    match std::panic::catch_unwind(LlmClient::from_env) {
        Ok(c) => c.map(Arc::new),
        Err(_) => {
            tracing::warn!("LLM-Client nicht verfügbar (Initialisierung fehlgeschlagen) – KI-Funktionen deaktiviert");
            None
        }
    }
}

pub fn general_http() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("Innenarchitekt/0.1 (private)")
        .timeout(Duration::from_secs(20))
        .build()
        .expect("http client")
}

pub fn build_state(cfg: Config, db: sqlx::PgPool, shutdown: watch::Receiver<bool>) -> anyhow::Result<AppState> {
    let store = files::local_store(&cfg.data_dir).context("Object-Store (DATA_DIR) nicht nutzbar")?;
    let (jobs_tx, _) = broadcast::channel(256);
    Ok(AppState {
        db,
        store,
        llm: llm_from_env(),
        cfg: Arc::new(cfg),
        http: general_http(),
        ikea_http: ikea::http_client(),
        jobs_tx,
        job_notify: Arc::new(Notify::new()),
        shutdown,
    })
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "innenarchitekt=info,tower_http=info,sqlx=warn".into()),
        )
        .init();

    let cfg = Config::from_env()?;
    let db = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&cfg.database_url)
        .await
        .context("Datenbank nicht erreichbar (DATABASE_URL)")?;
    sqlx::migrate!("./migrations").run(&db).await.context("Migrationen fehlgeschlagen")?;

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let bind = cfg.bind.clone();
    let st = build_state(cfg, db.clone(), shutdown_rx)?;
    tracing::info!(
        llm = st.llm.as_ref().map(|l| l.model.as_str()).unwrap_or("aus"),
        data_dir = %st.cfg.data_dir.display(),
        web_dir = %st.cfg.web_dir.display(),
        "Konfiguration geladen"
    );

    let worker = tokio::spawn(jobs::worker(st.clone(), &[jobs::KIND_FLOORPLAN]));
    let photo_worker = tokio::spawn(jobs::worker(st.clone(), &[jobs::KIND_PHOTO3D]));
    let app = router(st);
    let listener = tokio::net::TcpListener::bind(&bind).await.with_context(|| format!("BIND {bind} nicht möglich"))?;
    tracing::info!("Innenarchitekt läuft auf http://{bind}");

    let mut srx = shutdown_tx.subscribe();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = srx.wait_for(|v| *v).await;
            })
            .await
    });

    shutdown_signal().await;
    tracing::info!("Beende … (offene Verbindungen max. 10 s)");
    let _ = shutdown_tx.send(true);
    if tokio::time::timeout(Duration::from_secs(10), server).await.is_err() {
        tracing::warn!("Verbindungen nicht rechtzeitig geschlossen – erzwinge Ende");
    }
    let _ = tokio::time::timeout(Duration::from_secs(5), worker).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), photo_worker).await;
    db.close().await;
    Ok(())
}
