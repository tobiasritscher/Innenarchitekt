//! Static-token auth: `Authorization: Bearer <token>` or cookie `ia_token`.

use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::error::{ApiError, AppJson};
use crate::state::AppState;

pub const COOKIE: &str = "ia_token";

/// Constant-time comparison (length leak is acceptable for a static token).
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn cookie_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == COOKIE)
        .map(|(_, v)| v.trim().to_string())
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = v.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| token.trim().to_string())
}

pub fn is_authorized(headers: &HeaderMap, token: &str) -> bool {
    [bearer_token(headers), cookie_token(headers)]
        .into_iter()
        .flatten()
        .any(|t| ct_eq(t.as_bytes(), token.as_bytes()))
}

pub async fn require_auth(State(st): State<AppState>, req: Request, next: Next) -> Response {
    if is_authorized(req.headers(), &st.cfg.auth_token) {
        next.run(req).await
    } else {
        ApiError::unauthorized().into_response()
    }
}

#[derive(Deserialize)]
pub struct LoginBody {
    token: String,
}

fn is_https(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(',').next().unwrap_or("").trim().eq_ignore_ascii_case("https"))
        .unwrap_or(false)
        || headers
            .get("forwarded")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.to_ascii_lowercase().contains("proto=https"))
            .unwrap_or(false)
}

/// POST /api/auth
pub async fn login(State(st): State<AppState>, headers: HeaderMap, AppJson(body): AppJson<LoginBody>) -> Response {
    if !ct_eq(body.token.trim().as_bytes(), st.cfg.auth_token.as_bytes()) {
        return ApiError::new(StatusCode::UNAUTHORIZED, "Token ungültig").into_response();
    }
    let mut cookie = format!("{COOKIE}={}; HttpOnly; SameSite=Strict; Path=/; Max-Age=31536000", st.cfg.auth_token);
    if is_https(&headers) {
        cookie.push_str("; Secure");
    }
    let mut res = StatusCode::NO_CONTENT.into_response();
    if let Ok(v) = HeaderValue::from_str(&cookie) {
        res.headers_mut().insert(header::SET_COOKIE, v);
    }
    res
}

/// GET /api/auth
pub async fn check(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if is_authorized(&headers, &st.cfg.auth_token) {
        Json(json!({ "ok": true })).into_response()
    } else {
        ApiError::unauthorized().into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"abcd"));
    }

    #[test]
    fn token_sources() {
        let tok = "s3cret-token-0123456789";
        let mut h = HeaderMap::new();
        assert!(!is_authorized(&h, tok));
        h.insert(header::AUTHORIZATION, HeaderValue::from_str(&format!("Bearer {tok}")).unwrap());
        assert!(is_authorized(&h, tok));
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, HeaderValue::from_str(&format!("foo=bar; ia_token={tok}; x=y")).unwrap());
        assert!(is_authorized(&h, tok));
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, HeaderValue::from_static("ia_token=wrong"));
        h.insert(header::AUTHORIZATION, HeaderValue::from_static("Basic abc"));
        assert!(!is_authorized(&h, tok));
    }

    #[test]
    fn https_detection() {
        let mut h = HeaderMap::new();
        assert!(!is_https(&h));
        h.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        assert!(is_https(&h));
    }
}
