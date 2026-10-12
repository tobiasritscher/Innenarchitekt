//! Static-token auth: `Authorization: Bearer <token>` or cookie `ia_token`.
//! Wrong tokens are counted per client IP; too many lock that IP out for a while (brute-force brake).

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, Extensions, HeaderMap, HeaderValue, StatusCode};
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

fn presents_token(headers: &HeaderMap) -> bool {
    bearer_token(headers).is_some() || cookie_token(headers).is_some()
}

const MAX_FAILS: u32 = 10;
const LOCKOUT: Duration = Duration::from_secs(15 * 60);

/// Failed attempts per IP within a fixed window of `LOCKOUT` from the first failure.
#[derive(Default)]
pub struct FailGuard(Mutex<HashMap<IpAddr, (u32, Instant)>>);

impl FailGuard {
    fn blocked(&self, ip: IpAddr, now: Instant) -> bool {
        let map = self.0.lock().unwrap();
        map.get(&ip).is_some_and(|&(n, since)| n >= MAX_FAILS && now.duration_since(since) < LOCKOUT)
    }

    fn fail(&self, ip: IpAddr, now: Instant) {
        let mut map = self.0.lock().unwrap();
        map.retain(|_, (_, since)| now.duration_since(*since) < LOCKOUT);
        map.entry(ip).or_insert((0, now)).0 += 1;
    }

    fn clear(&self, ip: IpAddr) {
        self.0.lock().unwrap().remove(&ip);
    }
}

/// Client IP: last `X-Forwarded-For` hop (set by Caddy, not spoofable through it), else the TCP peer.
fn client_ip(headers: &HeaderMap, peer: Option<SocketAddr>) -> Option<IpAddr> {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit(',').next())
        .and_then(|v| v.trim().parse().ok())
        .or(peer.map(|p| p.ip()))
}

fn peer(ext: &Extensions) -> Option<SocketAddr> {
    ext.get::<ConnectInfo<SocketAddr>>().map(|c| c.0)
}

fn too_many() -> Response {
    ApiError::new(StatusCode::TOO_MANY_REQUESTS, "Zu viele Fehlversuche – bitte später erneut versuchen").into_response()
}

/// Checks the token and books the attempt. `None` = OK, `Some(resp)` = reject with that response.
fn guard(st: &AppState, ip: Option<IpAddr>, ok: bool, counts: bool) -> Option<Response> {
    let now = Instant::now();
    if let Some(ip) = ip {
        if st.auth_fails.blocked(ip, now) {
            return Some(too_many());
        }
        if ok {
            st.auth_fails.clear(ip);
        } else if counts {
            st.auth_fails.fail(ip, now);
        }
    }
    (!ok).then(|| ApiError::unauthorized().into_response())
}

pub async fn require_auth(State(st): State<AppState>, req: Request, next: Next) -> Response {
    let headers = req.headers();
    let ip = client_ip(headers, peer(req.extensions()));
    let ok = is_authorized(headers, &st.cfg.auth_token);
    match guard(&st, ip, ok, presents_token(headers)) {
        None => next.run(req).await,
        Some(r) => r,
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
pub async fn login(State(st): State<AppState>, ext: Extensions, headers: HeaderMap, AppJson(body): AppJson<LoginBody>) -> Response {
    let ok = ct_eq(body.token.trim().as_bytes(), st.cfg.auth_token.as_bytes());
    if let Some(r) = guard(&st, client_ip(&headers, peer(&ext)), ok, true) {
        if r.status() == StatusCode::TOO_MANY_REQUESTS {
            return r;
        }
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
pub async fn check(State(st): State<AppState>, ext: Extensions, headers: HeaderMap) -> Response {
    let ok = is_authorized(&headers, &st.cfg.auth_token);
    guard(&st, client_ip(&headers, peer(&ext)), ok, presents_token(&headers))
        .unwrap_or_else(|| Json(json!({ "ok": true })).into_response())
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

    #[test]
    fn lockout_after_max_fails_and_expiry() {
        let g = FailGuard::default();
        let ip: IpAddr = "203.0.113.7".parse().unwrap();
        let other: IpAddr = "203.0.113.8".parse().unwrap();
        let t0 = Instant::now();
        for _ in 0..MAX_FAILS - 1 {
            g.fail(ip, t0);
        }
        assert!(!g.blocked(ip, t0));
        g.fail(ip, t0);
        assert!(g.blocked(ip, t0));
        assert!(!g.blocked(other, t0));
        assert!(!g.blocked(ip, t0 + LOCKOUT));
    }

    #[test]
    fn client_ip_prefers_last_forwarded_hop() {
        let peer: SocketAddr = "192.168.1.5:5000".parse().unwrap();
        let mut h = HeaderMap::new();
        assert_eq!(client_ip(&h, Some(peer)), Some(peer.ip()));
        h.insert("x-forwarded-for", HeaderValue::from_static("1.1.1.1, 198.51.100.4"));
        assert_eq!(client_ip(&h, Some(peer)), Some("198.51.100.4".parse().unwrap()));
    }
}
