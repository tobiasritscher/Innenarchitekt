//! Object store access (`GET /api/files/{*key}`) and static frontend serving.

use std::path::Path as FsPath;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Router;
use futures::TryStreamExt;
use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjPath;
use object_store::ObjectStore;
use tower_http::services::{ServeDir, ServeFile};

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub fn local_store(data_dir: &FsPath) -> anyhow::Result<Arc<dyn ObjectStore>> {
    let root = data_dir.join("objects");
    std::fs::create_dir_all(&root)?;
    Ok(Arc::new(LocalFileSystem::new_with_prefix(&root)?))
}

pub fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 512
        && key.split('/').all(|seg| !seg.is_empty() && seg != "." && seg != "..")
        && key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '.'))
}

/// Stream an object with content type and immutable caching (keys never get overwritten with other content).
pub async fn stream_object(st: &AppState, key: &str, content_type: Option<&str>) -> ApiResult<Response> {
    if !valid_key(key) {
        return Err(ApiError::not_found("Datei nicht gefunden"));
    }
    let res = st.store.get(&ObjPath::from(key)).await?;
    let size = res.meta.size;
    let ct = content_type
        .map(str::to_string)
        .unwrap_or_else(|| mime_guess::from_path(key).first_or_octet_stream().essence_str().to_string());
    let body = Body::from_stream(res.into_stream().map_err(std::io::Error::other));
    Ok((
        [
            (header::CONTENT_TYPE, ct),
            (header::CONTENT_LENGTH, size.to_string()),
            (header::CACHE_CONTROL, "private, max-age=31536000, immutable".to_string()),
        ],
        body,
    )
        .into_response())
}

pub async fn get_file(State(st): State<AppState>, Path(key): Path<String>) -> ApiResult<Response> {
    stream_object(&st, &key, None).await
}

/// Cache headers for the SPA: hashed `assets/*` immutable, everything else revalidated.
async fn static_cache(req: Request, next: Next) -> Response {
    let immutable = req.uri().path().starts_with("/assets/");
    let mut res = next.run(req).await;
    if res.status() == StatusCode::OK || res.status() == StatusCode::NOT_MODIFIED {
        let v = if immutable { "public, max-age=31536000, immutable" } else { "no-cache" };
        res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static(v));
    }
    res
}

/// Static frontend from `WEB_DIR` with SPA fallback to `index.html`.
pub fn web_router(web_dir: &FsPath) -> Router {
    if !web_dir.join("index.html").exists() {
        tracing::warn!("WEB_DIR {} enthält keine index.html – Frontend wird nicht ausgeliefert", web_dir.display());
    }
    let serve = ServeDir::new(web_dir)
        .precompressed_gzip()
        .fallback(ServeFile::new(web_dir.join("index.html")));
    Router::new().fallback_service(serve).layer(axum::middleware::from_fn(static_cache))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        assert!(valid_key("floorplans/abc/rectified.png"));
        assert!(!valid_key("../etc/passwd"));
        assert!(!valid_key("a//b"));
        assert!(!valid_key("a/./b"));
        assert!(!valid_key("a b"));
        assert!(!valid_key(""));
    }

    #[test]
    fn wasm_mime() {
        assert_eq!(mime_guess::from_path("x.wasm").first_or_octet_stream().essence_str(), "application/wasm");
    }
}
