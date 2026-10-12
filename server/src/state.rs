use std::sync::Arc;

use object_store::ObjectStore;
use sqlx::PgPool;
use tokio::sync::{broadcast, watch, Notify};

use crate::config::Config;
use crate::jobs::Job;
use crate::llm::LlmClient;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub store: Arc<dyn ObjectStore>,
    pub llm: Option<Arc<LlmClient>>,
    pub cfg: Arc<Config>,
    /// Generic outbound HTTP (Nominatim).
    pub http: reqwest::Client,
    /// Browser-like client for IKEA (redirects restricted to ikea.com).
    pub ikea_http: reqwest::Client,
    /// Live job updates for SSE.
    pub jobs_tx: broadcast::Sender<Job>,
    /// Wakes the job worker when a job is enqueued.
    pub job_notify: Arc<Notify>,
    /// Failed logins per client IP (brute-force brake).
    pub auth_fails: Arc<crate::auth::FailGuard>,
    /// Flips to `true` on shutdown (ends SSE streams and the worker).
    pub shutdown: watch::Receiver<bool>,
}
