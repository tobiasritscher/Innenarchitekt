//! Furniture library: primitives, GLB uploads, IKEA imports.

use axum::extract::{Multipart, Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use bytes::Bytes;
use object_store::path::Path as ObjPath;
use object_store::PutPayload;
use plan_core::assets::{Asset, AssetKind};
use plan_core::primitives::PrimitiveShape;
use serde::Deserialize;
use sqlx::types::Json as SqlJson;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::{ApiError, ApiResult, AppJson};
use crate::glb;
use crate::state::AppState;

fn model_key(id: Uuid) -> String {
    format!("assets/{id}/model.glb")
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn kind_str(k: AssetKind) -> &'static str {
    match k {
        AssetKind::Ikea => "Ikea",
        AssetKind::Upload => "Upload",
        AssetKind::Primitive => "Primitive",
        AssetKind::Photo => "Photo",
    }
}

fn valid_dims(d: [u32; 3]) -> bool {
    d.iter().all(|v| (1..=50_000).contains(v))
}

pub async fn put_model(st: &AppState, id: Uuid, bytes: Vec<u8>) -> ApiResult<()> {
    st.store.put(&ObjPath::from(model_key(id)), PutPayload::from(Bytes::from(bytes))).await?;
    Ok(())
}

async fn insert(db: &PgPool, a: &Asset, ikea_article: Option<&str>) -> Result<bool, sqlx::Error> {
    let r = sqlx::query(
        "INSERT INTO assets (id, kind, name, data, ikea_article) VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (ikea_article) DO NOTHING",
    )
    .bind(a.id)
    .bind(kind_str(a.kind))
    .bind(&a.name)
    .bind(SqlJson(a))
    .bind(ikea_article)
    .execute(db)
    .await?;
    Ok(r.rows_affected() == 1)
}

pub async fn find_by_article(db: &PgPool, article: &str) -> Result<Option<Asset>, sqlx::Error> {
    let row = sqlx::query("SELECT data FROM assets WHERE ikea_article = $1").bind(article).fetch_optional(db).await?;
    Ok(match row {
        Some(r) => Some(r.try_get::<SqlJson<Asset>, _>("data")?.0),
        None => None,
    })
}

/// Insert an IKEA asset; on a concurrent duplicate return the existing one (and drop our model copy).
pub async fn insert_ikea(st: &AppState, a: Asset) -> ApiResult<Asset> {
    let article = a.ikea.as_ref().map(|m| m.article_no.clone());
    if insert(&st.db, &a, article.as_deref()).await? {
        return Ok(a);
    }
    if a.model_url.is_some() {
        let _ = st.store.delete(&ObjPath::from(model_key(a.id))).await;
    }
    match article {
        Some(art) => find_by_article(&st.db, &art).await?.ok_or_else(|| ApiError::internal("IKEA-Asset verschwunden")),
        None => Err(ApiError::internal("IKEA-Asset ohne Artikelnummer")),
    }
}

async fn load(db: &PgPool, id: Uuid) -> ApiResult<Asset> {
    let row = sqlx::query("SELECT data FROM assets WHERE id = $1")
        .bind(id)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| ApiError::not_found("Möbel nicht gefunden"))?;
    Ok(row.try_get::<SqlJson<Asset>, _>("data")?.0)
}

// ---------- Handlers ----------

pub async fn list(State(st): State<AppState>) -> ApiResult<Json<Vec<Asset>>> {
    let rows = sqlx::query("SELECT data FROM assets ORDER BY created_at DESC").fetch_all(&st.db).await?;
    let out = rows
        .iter()
        .map(|r| r.try_get::<SqlJson<Asset>, _>("data").map(|j| j.0))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(out))
}

pub async fn get(State(st): State<AppState>, Path(id): Path<Uuid>) -> ApiResult<Json<Asset>> {
    Ok(Json(load(&st.db, id).await?))
}

pub async fn delete(State(st): State<AppState>, Path(id): Path<Uuid>) -> ApiResult<StatusCode> {
    let r = sqlx::query("DELETE FROM assets WHERE id = $1").bind(id).execute(&st.db).await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found("Möbel nicht gefunden"));
    }
    if let Err(e) = st.store.delete(&ObjPath::from(model_key(id))).await {
        if !matches!(e, object_store::Error::NotFound { .. }) {
            tracing::warn!(%id, "Modell nicht löschbar: {e}");
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn model(State(st): State<AppState>, Path(id): Path<Uuid>) -> ApiResult<Response> {
    crate::files::stream_object(&st, &model_key(id), Some("model/gltf-binary")).await
}

#[derive(Deserialize)]
pub struct PrimitiveBody {
    shape: PrimitiveShape,
    name: Option<String>,
    dims_mm: Option<[u32; 3]>,
}

/// New (not yet stored) library asset for a primitive shape with default dims.
pub fn primitive_template(shape: PrimitiveShape) -> Asset {
    Asset {
        id: Uuid::new_v4(),
        kind: AssetKind::Primitive,
        name: shape.label_de().to_string(),
        dims_mm: shape.default_dims(),
        model_url: None,
        thumbnail_url: None,
        primitive: Some(shape),
        ikea: None,
        light: None,
        warnings: vec![],
        created_at: now(),
    }
}

pub async fn store_asset(st: &AppState, a: &Asset) -> ApiResult<()> {
    insert(&st.db, a, None).await?;
    Ok(())
}

pub async fn create_primitive(State(st): State<AppState>, AppJson(b): AppJson<PrimitiveBody>) -> ApiResult<Json<Asset>> {
    let dims = b.dims_mm.unwrap_or_else(|| b.shape.default_dims());
    if !valid_dims(dims) {
        return Err(ApiError::unprocessable("Masse müssen zwischen 1 und 50'000 mm liegen"));
    }
    let name = b.name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| b.shape.label_de().to_string());
    let a = Asset {
        id: Uuid::new_v4(),
        kind: AssetKind::Primitive,
        name,
        dims_mm: dims,
        model_url: None,
        thumbnail_url: None,
        primitive: Some(b.shape),
        ikea: None,
        light: None,
        warnings: vec![],
        created_at: now(),
    };
    insert(&st.db, &a, None).await?;
    Ok(Json(a))
}

pub fn parse_dim(s: &str, label: &str) -> ApiResult<Option<u32>> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    s.parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 1.0 && *v <= 50_000.0)
        .map(|v| Some(v.round() as u32))
        .ok_or_else(|| ApiError::unprocessable(format!("{label} ungültig (1–50'000 mm)")))
}

/// POST /api/assets/upload (multipart: file, name, w_mm, d_mm, h_mm)
pub async fn upload(State(st): State<AppState>, mut mp: Multipart) -> ApiResult<Json<Asset>> {
    let (mut file, mut name, mut fname) = (None::<Bytes>, None::<String>, None::<String>);
    let mut dims: [Option<u32>; 3] = [None; 3];
    let bad = |e: axum::extract::multipart::MultipartError| ApiError::bad_request(format!("Upload fehlerhaft: {e}"));
    while let Some(field) = mp.next_field().await.map_err(bad)? {
        match field.name().unwrap_or("") {
            "file" => {
                fname = field.file_name().map(str::to_string);
                file = Some(field.bytes().await.map_err(bad)?);
            }
            "name" => name = Some(field.text().await.map_err(bad)?),
            "w_mm" => dims[0] = parse_dim(&field.text().await.map_err(bad)?, "Breite")?,
            "d_mm" => dims[1] = parse_dim(&field.text().await.map_err(bad)?, "Tiefe")?,
            "h_mm" => dims[2] = parse_dim(&field.text().await.map_err(bad)?, "Höhe")?,
            _ => {}
        }
    }
    let file = file.ok_or_else(|| ApiError::unprocessable("Feld «file» fehlt"))?;
    let name = name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .or_else(|| fname.map(|f| f.trim_end_matches(".glb").trim_end_matches(".GLB").to_string()))
        .unwrap_or_else(|| "Eigenes Modell".into());
    Ok(Json(create_glb_asset(&st, file.to_vec(), name, dims, AssetKind::Upload, None).await?))
}

/// Store a GLB as a library asset, scaled to `dims` (missing dims from the model's bounding box).
pub async fn create_glb_asset(st: &AppState, file: Vec<u8>, name: String, dims: [Option<u32>; 3], kind: AssetKind, thumbnail_url: Option<String>) -> ApiResult<Asset> {
    if !glb::is_glb(&file) {
        return Err(ApiError::unprocessable("Keine GLB-Datei (binäres glTF 2.0 erwartet)"));
    }
    let bbox = glb::bbox(&file).map_err(|e| ApiError::unprocessable(format!("GLB nicht lesbar: {e:#}")))?;
    let model = bbox.wdh_mm();
    let mut warnings = vec![];
    let mut final_dims = [0u32; 3];
    for i in 0..3 {
        final_dims[i] = dims[i].or((model[i] > 0).then_some(model[i])).unwrap_or(1).max(1);
    }
    if dims.iter().any(Option::is_none) {
        warnings.push("Fehlende Masse aus dem Modell übernommen (Annahme: Modell in Metern)".into());
    }
    // Non-uniform scaling distorts the model: warn if proportions differ by > 3 %.
    let ratios: Vec<f64> = (0..3).filter(|i| model[*i] > 0).map(|i| final_dims[i] as f64 / model[i] as f64).collect();
    if let (Some(mn), Some(mx)) = (ratios.iter().cloned().reduce(f64::min), ratios.iter().cloned().reduce(f64::max)) {
        if mn > 0.0 && mx / mn > 1.03 {
            warnings.push(format!(
                "Proportionen weichen vom Modell ab (Modell {}×{}×{} mm) – es wird verzerrt dargestellt",
                model[0], model[1], model[2]
            ));
        }
    }
    let id = Uuid::new_v4();
    put_model(st, id, file).await?;
    let a = Asset {
        id,
        kind,
        name,
        dims_mm: final_dims,
        model_url: Some(format!("/api/assets/{id}/model.glb")),
        thumbnail_url,
        primitive: None,
        ikea: None,
        light: None,
        warnings,
        created_at: now(),
    };
    insert(&st.db, &a, None).await?;
    Ok(a)
}

#[derive(Deserialize)]
pub struct IkeaBody {
    url: String,
}

pub async fn import_ikea(State(st): State<AppState>, AppJson(b): AppJson<IkeaBody>) -> ApiResult<Json<Asset>> {
    if b.url.trim().is_empty() {
        return Err(ApiError::unprocessable("IKEA-Link fehlt"));
    }
    Ok(Json(crate::ikea::import(&st, &b.url).await?))
}
