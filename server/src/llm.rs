//! OpenAI-compatible LLM client (LiteLLM). The model always answers via a forced tool call
//! whose arguments follow a JSON schema — never free text, never direct changes.
//! OWNER: floorplan-pipeline agent. Signatures are a contract used by the backend.

use std::time::Duration;

use anyhow::{anyhow, Context};
use base64::Engine;
use serde_json::{json, Value};

pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
const TIMEOUT: Duration = Duration::from_secs(120);
const MAX_TOKENS: u32 = 16_000;

#[derive(Clone)]
pub struct LlmClient {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub http: reqwest::Client,
}

// Manual Debug: never print the API key.
impl std::fmt::Debug for LlmClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmClient")
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .field("model", &self.model)
            .finish()
    }
}

impl LlmClient {
    /// From env: LLM_BASE_URL (e.g. http://127.0.0.1:4001/v1), LLM_API_KEY, LLM_MODEL (default "claude-opus-5-5").
    /// Returns None if LLM_BASE_URL or LLM_API_KEY is missing/empty (= LLM features disabled).
    pub fn from_env() -> Option<Self> {
        let get = |k: &str| std::env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        let base_url = get("LLM_BASE_URL")?;
        let api_key = get("LLM_API_KEY")?;
        let model = get("LLM_MODEL").unwrap_or_else(|| DEFAULT_MODEL.to_string());
        Some(Self::new(base_url, api_key, model))
    }

    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>, model: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .connect_timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        LlmClient { base_url: base_url.into().trim_end_matches('/').to_string(), api_key: api_key.into(), model: model.into(), http }
    }

    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    /// Ask for structured output. `image_png` is attached as a data URL (vision).
    /// Returns the parsed tool-call arguments (JSON object matching `schema`).
    pub async fn structured(
        &self,
        system: &str,
        user_text: &str,
        image_png: Option<&[u8]>,
        tool_name: &str,
        schema: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let images: Vec<(&str, &[u8])> = image_png.map(|p| ("image/png", p)).into_iter().collect();
        self.structured_images(system, user_text, &images, tool_name, schema).await
    }

    /// Like [`structured`](Self::structured) with any number of images `(mime, bytes)` (PNG/JPEG).
    pub async fn structured_images(
        &self,
        system: &str,
        user_text: &str,
        images: &[(&str, &[u8])],
        tool_name: &str,
        schema: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let body = build_request_images(&self.model, system, user_text, images, tool_name, schema);
        let mut last_err = None;
        for attempt in 0..3 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            let res = self.http.post(self.endpoint()).bearer_auth(&self.api_key).json(&body).send().await;
            match res {
                Ok(resp) => {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    if status.is_success() {
                        let v: Value = serde_json::from_str(&text).context("LLM-Antwort ist kein JSON")?;
                        match parse_response(&v) {
                            Ok(out) => return Ok(out),
                            Err(e) => {
                                // Occasionally the model answers in prose or gets cut off: retry once.
                                let finish = v.pointer("/choices/0/finish_reason").and_then(|f| f.as_str()).unwrap_or("?");
                                tracing::warn!(attempt, finish_reason = finish, "LLM-Antwort unbrauchbar: {e}");
                                last_err = Some(e);
                                continue;
                            }
                        }
                    }
                    let err = anyhow!("LLM-Fehler {}: {}", status.as_u16(), truncate(&text, 400));
                    if status.is_server_error() || status.as_u16() == 429 {
                        last_err = Some(err);
                        continue;
                    }
                    return Err(err);
                }
                Err(e) => {
                    // never include the request (headers) in the message
                    let retry = e.is_timeout() || e.is_connect() || e.is_request();
                    let err = anyhow!("LLM nicht erreichbar: {}", e.without_url());
                    if retry {
                        last_err = Some(err);
                        continue;
                    }
                    return Err(err);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow!("LLM-Aufruf fehlgeschlagen")))
    }
}

impl LlmClient {
    /// One turn of a tool-using conversation (OpenAI format). The model must call one of `tools`
    /// (`tool_choice: required`). Returns the assistant message (with `tool_calls`).
    pub async fn chat(&self, messages: &[Value], tools: &[Value]) -> anyhow::Result<Value> {
        let body = json!({
            "model": self.model,
            "max_tokens": MAX_TOKENS,
            "messages": messages,
            "tools": tools,
            "tool_choice": "required",
        });
        let mut last_err = None;
        for attempt in 0..3 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            match self.http.post(self.endpoint()).bearer_auth(&self.api_key).json(&body).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    if status.is_success() {
                        let v: Value = serde_json::from_str(&text).context("LLM-Antwort ist kein JSON")?;
                        if let Some(m) = v.pointer("/choices/0/message") {
                            return Ok(m.clone());
                        }
                        last_err = Some(anyhow!("LLM-Antwort ohne choices[0].message"));
                        continue;
                    }
                    let err = anyhow!("LLM-Fehler {}: {}", status.as_u16(), truncate(&text, 400));
                    if status.is_server_error() || status.as_u16() == 429 {
                        last_err = Some(err);
                        continue;
                    }
                    return Err(err);
                }
                Err(e) => {
                    let retry = e.is_timeout() || e.is_connect() || e.is_request();
                    let err = anyhow!("LLM nicht erreichbar: {}", e.without_url());
                    if retry {
                        last_err = Some(err);
                        continue;
                    }
                    return Err(err);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow!("LLM-Aufruf fehlgeschlagen")))
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

/// Chat-completions request body with exactly one forced tool.
#[cfg_attr(not(test), allow(dead_code))]
pub fn build_request(model: &str, system: &str, user_text: &str, image_png: Option<&[u8]>, tool_name: &str, schema: Value) -> Value {
    let images: Vec<(&str, &[u8])> = image_png.map(|p| ("image/png", p)).into_iter().collect();
    build_request_images(model, system, user_text, &images, tool_name, schema)
}

pub fn build_request_images(model: &str, system: &str, user_text: &str, images: &[(&str, &[u8])], tool_name: &str, schema: Value) -> Value {
    let mut content = vec![json!({"type": "text", "text": user_text})];
    for (mime, bytes) in images {
        let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
        content.push(json!({"type": "image_url", "image_url": {"url": format!("data:{mime};base64,{b64}")}}));
    }
    json!({
        "model": model,
        "max_tokens": MAX_TOKENS,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": content}
        ],
        "tools": [{
            "type": "function",
            "function": {
                "name": tool_name,
                "description": format!("Return the result by calling {tool_name}."),
                "parameters": schema
            }
        }],
        "tool_choice": {"type": "function", "function": {"name": tool_name}}
    })
}

/// Extract the tool-call arguments; fall back to JSON in `content`.
pub fn parse_response(v: &Value) -> anyhow::Result<Value> {
    let msg = v
        .pointer("/choices/0/message")
        .ok_or_else(|| anyhow!("LLM-Antwort ohne choices[0].message"))?;
    if let Some(args) = msg.pointer("/tool_calls/0/function/arguments") {
        match args {
            Value::String(s) => {
                if let Ok(parsed) = serde_json::from_str::<Value>(s) {
                    return Ok(parsed);
                }
                if let Some(parsed) = json_from_text(s) {
                    return Ok(parsed);
                }
            }
            Value::Object(_) => return Ok(args.clone()),
            _ => {}
        }
    }
    let text = match msg.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    };
    json_from_text(&text).ok_or_else(|| anyhow!("LLM lieferte weder Tool-Aufruf noch JSON"))
}

/// Parse JSON from free text: strips ``` fences, else takes the outermost {...}.
fn json_from_text(s: &str) -> Option<Value> {
    let t = s.trim();
    let t = if let Some(start) = t.find("```") {
        let rest = &t[start + 3..];
        let rest = rest.strip_prefix("json").unwrap_or(rest);
        match rest.find("```") {
            Some(end) => &rest[..end],
            None => rest,
        }
    } else {
        t
    };
    if let Ok(v) = serde_json::from_str::<Value>(t.trim()) {
        if v.is_object() {
            return Some(v);
        }
    }
    let (a, b) = (t.find('{')?, t.rfind('}')?);
    if b <= a {
        return None;
    }
    serde_json::from_str::<Value>(&t[a..=b]).ok().filter(|v| v.is_object())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_has_forced_tool_and_image() {
        let body = build_request("m", "sys", "hello", Some(&[1, 2, 3]), "report", json!({"type": "object"}));
        assert_eq!(body["model"], "m");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["content"][0]["text"], "hello");
        let url = body["messages"][1]["content"][1]["image_url"]["url"].as_str().unwrap();
        assert_eq!(url, "data:image/png;base64,AQID");
        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
        assert_eq!(body["tools"][0]["function"]["name"], "report");
        assert_eq!(body["tool_choice"]["function"]["name"], "report");
        let no_img = build_request("m", "s", "u", None, "t", json!({}));
        assert_eq!(no_img["messages"][1]["content"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn parses_tool_call_arguments() {
        let v = json!({"choices": [{"message": {"content": null, "tool_calls": [{"type": "function",
            "function": {"name": "report", "arguments": "{\"a\": 1}"}}]}}]});
        assert_eq!(parse_response(&v).unwrap(), json!({"a": 1}));
        let v = json!({"choices": [{"message": {"tool_calls": [{"function": {"arguments": {"b": 2}}}]}}]});
        assert_eq!(parse_response(&v).unwrap(), json!({"b": 2}));
    }

    #[test]
    fn falls_back_to_content_json() {
        let v = json!({"choices": [{"message": {"content": "Here you go:\n```json\n{\"rooms\": []}\n```"}}]});
        assert_eq!(parse_response(&v).unwrap(), json!({"rooms": []}));
        let v = json!({"choices": [{"message": {"content": "Result {\"x\": {\"y\": 2}} done"}}]});
        assert_eq!(parse_response(&v).unwrap(), json!({"x": {"y": 2}}));
        let v = json!({"choices": [{"message": {"content": "no json"}}]});
        assert!(parse_response(&v).is_err());
    }

    #[test]
    fn debug_hides_key() {
        let c = LlmClient::new("http://h/v1/", "secret-key", "m");
        assert!(!format!("{c:?}").contains("secret-key"));
        assert_eq!(c.endpoint(), "http://h/v1/chat/completions");
    }

    /// Against a local mock server: first call 503 (retried), second returns a tool call.
    #[tokio::test]
    async fn structured_retries_and_parses() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let hits = Arc::new(AtomicUsize::new(0));
        let h2 = hits.clone();
        let app = axum::Router::new().route(
            "/v1/chat/completions",
            axum::routing::post(move |headers: axum::http::HeaderMap, axum::Json(body): axum::Json<Value>| {
                let h = h2.clone();
                async move {
                    assert_eq!(headers.get("authorization").unwrap(), "Bearer k");
                    assert_eq!(body["tool_choice"]["function"]["name"], "t");
                    if h.fetch_add(1, Ordering::SeqCst) == 0 {
                        return (axum::http::StatusCode::SERVICE_UNAVAILABLE, axum::Json(json!({"error": "busy"})));
                    }
                    (axum::http::StatusCode::OK, axum::Json(json!({"choices": [{"message": {"tool_calls": [{"function": {"name": "t", "arguments": "{\"ok\": true}"}}]}}]})))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let c = LlmClient::new(format!("http://{addr}/v1"), "k", "m");
        let v = c.structured("s", "u", Some(&[0u8; 4]), "t", json!({"type": "object"})).await.unwrap();
        assert_eq!(v, json!({"ok": true}));
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }
}
