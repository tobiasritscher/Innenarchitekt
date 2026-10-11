//! HTTP-level tests against a real Postgres. Skipped unless `DATABASE_URL` is set
//! (e.g. `DATABASE_URL=postgres://postgres:ia@127.0.0.1:5544/innenarchitekt cargo test -p innenarchitekt`).

use std::time::Duration;

use reqwest::StatusCode;
use serde_json::{json, Value};
use tokio::sync::watch;
use uuid::Uuid;

use crate::config::Config;

const TOKEN: &str = "test-token-0123456789abcdef";

struct TestServer {
    base: String,
    http: reqwest::Client,
    _shutdown: watch::Sender<bool>,
    _dir: std::path::PathBuf,
}

impl TestServer {
    fn url(&self, p: &str) -> String {
        format!("{}{}", self.base, p)
    }
    fn get(&self, p: &str) -> reqwest::RequestBuilder {
        self.http.get(self.url(p)).bearer_auth(TOKEN)
    }
    fn post(&self, p: &str) -> reqwest::RequestBuilder {
        self.http.post(self.url(p)).bearer_auth(TOKEN)
    }
}

async fn spawn() -> Option<TestServer> {
    let Ok(db_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL nicht gesetzt – HTTP-Tests übersprungen");
        return None;
    };
    let db = sqlx::postgres::PgPoolOptions::new().max_connections(5).connect(&db_url).await.expect("db");
    sqlx::migrate!("./migrations").run(&db).await.expect("migrate");
    let dir = std::env::temp_dir().join(format!("ia-test-{}", Uuid::new_v4()));
    let cfg = Config {
        database_url: db_url,
        bind: "127.0.0.1:0".into(),
        auth_token: TOKEN.into(),
        data_dir: dir.clone(),
        web_dir: dir.join("web"),
        ikea_country: "ch".into(),
        ikea_lang: "de".into(),
        ikea_rotera_client_id: crate::config::DEFAULT_ROTERA_CLIENT_ID.into(),
        photo3d_dir: None,
    };
    std::fs::create_dir_all(dir.join("web/assets")).unwrap();
    std::fs::write(dir.join("web/index.html"), "<!doctype html><title>IA</title>").unwrap();
    std::fs::write(dir.join("web/assets/app-123.js"), "console.log(1)").unwrap();
    let (tx, rx) = watch::channel(false);
    let st = crate::build_state(cfg, db, rx).unwrap();
    tokio::spawn(crate::jobs::worker(st.clone(), &[crate::jobs::KIND_FLOORPLAN]));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, crate::router(st)).await });
    Some(TestServer {
        base: format!("http://{addr}"),
        http: reqwest::Client::builder().timeout(Duration::from_secs(60)).build().unwrap(),
        _shutdown: tx,
        _dir: dir,
    })
}

#[tokio::test]
async fn auth_and_static() {
    let Some(s) = spawn().await else { return };
    // public
    assert_eq!(s.http.get(s.url("/api/health")).send().await.unwrap().status(), StatusCode::OK);
    // protected without token
    let r = s.http.get(s.url("/api/projects")).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    assert!(r.json::<Value>().await.unwrap()["error"].is_string());
    assert_eq!(s.http.get(s.url("/api/auth")).send().await.unwrap().status(), StatusCode::UNAUTHORIZED);
    // wrong token
    let r = s.http.post(s.url("/api/auth")).json(&json!({"token": "nope"})).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    // login → cookie
    let r = s.http.post(s.url("/api/auth")).json(&json!({"token": TOKEN})).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    let cookie = r.headers()["set-cookie"].to_str().unwrap().to_string();
    assert!(cookie.starts_with(&format!("ia_token={TOKEN};")));
    assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict") && cookie.contains("Max-Age=31536000"));
    assert!(!cookie.contains("Secure"));
    let r = s.http.post(s.url("/api/auth")).header("x-forwarded-proto", "https").json(&json!({"token": TOKEN})).send().await.unwrap();
    assert!(r.headers()["set-cookie"].to_str().unwrap().contains("Secure"));
    let r = s.http.get(s.url("/api/projects")).header("cookie", format!("ia_token={TOKEN}")).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(s.get("/api/auth").send().await.unwrap().status(), StatusCode::OK);
    // config
    let c: Value = s.get("/api/config").send().await.unwrap().json().await.unwrap();
    assert!(c["llm_enabled"].is_boolean());
    // static + SPA fallback (public)
    let r = s.http.get(s.url("/some/spa/route")).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(r.text().await.unwrap().contains("<title>IA</title>"));
    let r = s.http.get(s.url("/assets/app-123.js")).send().await.unwrap();
    assert!(r.headers()["cache-control"].to_str().unwrap().contains("immutable"));
    // unknown API path → JSON 404, not SPA
    let r = s.get("/api/nope").send().await.unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn projects_and_ops() {
    let Some(s) = spawn().await else { return };
    let r = s.post("/api/projects").json(&json!({"name": "Testwohnung"})).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let p: Value = r.json().await.unwrap();
    let id = p["id"].as_str().unwrap().to_string();
    let floor = p["floors"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(p["floors"][0]["name"], "EG");
    assert_eq!(p["version"], 0);
    assert!(p["location"].is_null());

    let n1 = Uuid::new_v4();
    let n2 = Uuid::new_v4();
    let w = Uuid::new_v4();
    let ops = json!([
        {"op": "PutNode", "floor": floor, "node": {"id": n1, "x_mm": 0, "y_mm": 0}},
        {"op": "PutNode", "floor": floor, "node": {"id": n2, "x_mm": 4000, "y_mm": 0}},
        {"op": "PutWall", "floor": floor, "wall": {"id": w, "a": n1, "b": n2, "thickness_mm": 120, "kind": "Interior",
            "finish_left": "paint:white", "finish_right": "paint:white"}}
    ]);
    let r = s.post(&format!("/api/projects/{id}/ops")).json(&json!({"base_version": 0, "ops": ops})).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.json::<Value>().await.unwrap()["version"], 1);

    // conflict: stale base_version
    let r = s.post(&format!("/api/projects/{id}/ops")).json(&json!({"base_version": 0, "ops": []})).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::CONFLICT);
    let b: Value = r.json().await.unwrap();
    assert_eq!(b["version"], 1);
    assert_eq!(b["project"]["floors"][0]["walls"].as_array().unwrap().len(), 1);
    assert!(b["error"].is_string());

    // 422: second op fails (wall with unknown node) → atomic, index 1
    let bad = json!([
        {"op": "PutNode", "floor": floor, "node": {"id": Uuid::new_v4(), "x_mm": 1, "y_mm": 1}},
        {"op": "PutWall", "floor": floor, "wall": {"id": Uuid::new_v4(), "a": n1, "b": Uuid::new_v4(), "thickness_mm": 100, "kind": "Interior",
            "finish_left": "paint:white", "finish_right": "paint:white"}}
    ]);
    let r = s.post(&format!("/api/projects/{id}/ops")).json(&json!({"base_version": 1, "ops": bad})).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let b: Value = r.json().await.unwrap();
    assert_eq!(b["index"], 1);
    let p: Value = s.get(&format!("/api/projects/{id}")).send().await.unwrap().json().await.unwrap();
    assert_eq!(p["version"], 1);
    assert_eq!(p["floors"][0]["nodes"].as_array().unwrap().len(), 2);

    // malformed op → JSON error
    let r = s.post(&format!("/api/projects/{id}/ops")).json(&json!({"base_version": 1, "ops": [{"op": "Nope"}]})).send().await.unwrap();
    assert!(r.status().is_client_error());
    assert!(r.json::<Value>().await.unwrap()["error"].is_string());

    // list
    let list: Value = s.get("/api/projects").send().await.unwrap().json().await.unwrap();
    let mine = list.as_array().unwrap().iter().find(|x| x["id"] == id.as_str()).unwrap();
    assert_eq!(mine["wall_count"], 1);
    assert_eq!(mine["version"], 1);
    assert!(mine["updated_at"].as_str().unwrap().ends_with('Z'));

    // delete
    assert_eq!(s.http.delete(s.url(&format!("/api/projects/{id}"))).bearer_auth(TOKEN).send().await.unwrap().status(), StatusCode::NO_CONTENT);
    assert_eq!(s.get(&format!("/api/projects/{id}")).send().await.unwrap().status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn assets_primitive_and_upload() {
    let Some(s) = spawn().await else { return };
    let a: Value = s.post("/api/assets/primitive").json(&json!({"shape": "Sofa"})).send().await.unwrap().json().await.unwrap();
    assert_eq!(a["kind"], "Primitive");
    assert_eq!(a["name"], "Sofa");
    assert_eq!(a["dims_mm"], json!([2200, 950, 850]));
    assert_eq!(a["primitive"], "Sofa");

    let glb = crate::glb::build_glb(&json!({
        "asset": {"version": "2.0"},
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
        "accessors": [{"componentType": 5126, "count": 3, "type": "VEC3", "min": [-0.5, 0, -0.25], "max": [0.5, 0.75, 0.25]}]
    }));
    let form = reqwest::multipart::Form::new()
        .part("file", reqwest::multipart::Part::bytes(glb.clone()).file_name("tisch.glb"))
        .text("name", "Mein Tisch")
        .text("w_mm", "1000")
        .text("d_mm", "500")
        .text("h_mm", "750");
    let r = s.post("/api/assets/upload").multipart(form).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let a: Value = r.json().await.unwrap();
    assert_eq!(a["kind"], "Upload");
    assert_eq!(a["warnings"], json!([]));
    let id = a["id"].as_str().unwrap();
    assert_eq!(a["model_url"], format!("/api/assets/{id}/model.glb"));
    let r = s.get(&format!("/api/assets/{id}/model.glb")).send().await.unwrap();
    assert_eq!(r.headers()["content-type"], "model/gltf-binary");
    assert_eq!(r.bytes().await.unwrap().to_vec(), glb);

    // not a GLB
    let form = reqwest::multipart::Form::new().part("file", reqwest::multipart::Part::bytes(b"hello world!".to_vec()));
    assert_eq!(s.post("/api/assets/upload").multipart(form).send().await.unwrap().status(), StatusCode::UNPROCESSABLE_ENTITY);

    let list: Value = s.get("/api/assets").send().await.unwrap().json().await.unwrap();
    assert!(list.as_array().unwrap().iter().any(|x| x["id"] == id));
    assert_eq!(s.http.delete(s.url(&format!("/api/assets/{id}"))).bearer_auth(TOKEN).send().await.unwrap().status(), StatusCode::NO_CONTENT);
    assert_eq!(s.get(&format!("/api/assets/{id}/model.glb")).send().await.unwrap().status(), StatusCode::NOT_FOUND);

    // IKEA: invalid links are rejected with a helpful 422 (no network needed)
    let r = s.post("/api/assets/ikea").json(&json!({"url": "https://example.com/p/x-00263850/"})).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn floorplan_job_reaches_terminal_state() {
    let Some(s) = spawn().await else { return };
    let p: Value = s.post("/api/projects").json(&json!({"name": "Plan"})).send().await.unwrap().json().await.unwrap();
    let id = p["id"].as_str().unwrap();
    let img = image::RgbImage::from_pixel(64, 48, image::Rgb([255, 255, 255]));
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let form = reqwest::multipart::Form::new()
        .part("file", reqwest::multipart::Part::bytes(png.into_inner()).file_name("grundriss.png"))
        .text("corners", "[[0,0],[63,0],[63,47],[0,47]]");
    let r = s.post(&format!("/api/projects/{id}/floorplan")).multipart(form).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let job_id = r.json::<Value>().await.unwrap()["job_id"].as_str().unwrap().to_string();

    // SSE: read events until terminal
    let mut res = s.get(&format!("/api/jobs/{job_id}/events")).send().await.unwrap();
    assert_eq!(res.headers()["content-type"], "text/event-stream");
    let mut buf = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    while let Ok(Some(chunk)) = tokio::time::timeout_at(deadline, res.chunk()).await.unwrap_or(Ok(None)) {
        buf.push_str(&String::from_utf8_lossy(&chunk));
    }
    assert!(buf.contains("event: job"), "{buf}");
    let last: Value = buf
        .lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .last()
        .map(|d| serde_json::from_str(d).unwrap())
        .unwrap();
    let status = last["status"].as_str().unwrap();
    assert!(status == "done" || status == "failed", "{last}");
    if status == "failed" {
        assert!(last["error"].as_str().unwrap().len() > 3);
    }
    let j: Value = s.get(&format!("/api/jobs/{job_id}")).send().await.unwrap().json().await.unwrap();
    assert_eq!(j["status"], status);
    // server survived
    assert_eq!(s.http.get(s.url("/api/health")).send().await.unwrap().status(), StatusCode::OK);
    // bad upload
    let form = reqwest::multipart::Form::new().part("file", reqwest::multipart::Part::bytes(b"%PDF-1.4".to_vec()));
    assert_eq!(s.post(&format!("/api/projects/{id}/floorplan")).multipart(form).send().await.unwrap().status(), StatusCode::UNPROCESSABLE_ENTITY);
    s.http.delete(s.url(&format!("/api/projects/{id}"))).bearer_auth(TOKEN).send().await.unwrap();
}
