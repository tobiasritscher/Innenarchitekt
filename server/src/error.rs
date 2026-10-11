//! API error type (`{ "error": "<deutsch>" }`) and a JSON extractor with German rejections.

use axum::extract::{FromRequest, Request};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub body: Value,
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    pub fn new(status: StatusCode, msg: impl Into<String>) -> Self {
        ApiError { status, body: json!({ "error": msg.into() }) }
    }
    pub fn with_body(status: StatusCode, body: Value) -> Self {
        ApiError { status, body }
    }
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, msg)
    }
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, msg)
    }
    pub fn unprocessable(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, msg)
    }
    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "Nicht angemeldet")
    }
    pub fn internal(e: impl std::fmt::Display) -> Self {
        tracing::error!("interner Fehler: {e}");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "Interner Serverfehler")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        ApiError::internal(format!("Datenbank: {e}"))
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError::internal(format!("{e:#}"))
    }
}

impl From<object_store::Error> for ApiError {
    fn from(e: object_store::Error) -> Self {
        match e {
            object_store::Error::NotFound { .. } => ApiError::not_found("Datei nicht gefunden"),
            e => ApiError::internal(format!("Object-Store: {e}")),
        }
    }
}

/// `Json<T>` with `{error}` rejections.
pub struct AppJson<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequest<S> for AppJson<T> {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(v)) => Ok(AppJson(v)),
            Err(rej) => Err(ApiError::new(rej.status(), format!("Ungültige Anfrage: {}", rej.body_text()))),
        }
    }
}
