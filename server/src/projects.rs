//! Projects: snapshot + versioned op batches.

use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, SecondsFormat, Utc};
use plan_core::{apply_batch, GeoLocation, Op, Project};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::types::Json as SqlJson;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::{ApiError, ApiResult, AppJson};
use crate::state::AppState;

#[derive(Serialize)]
pub struct ProjectSummary {
    id: Uuid,
    name: String,
    version: i64,
    updated_at: String,
    wall_count: i64,
}

pub async fn list(State(st): State<AppState>) -> ApiResult<Json<Vec<ProjectSummary>>> {
    let rows = sqlx::query(
        "SELECT id, name, version, updated_at,
                COALESCE((SELECT SUM(jsonb_array_length(COALESCE(f->'walls', '[]'::jsonb)))
                          FROM jsonb_array_elements(snapshot->'floors') f), 0)::bigint AS wall_count
         FROM projects ORDER BY updated_at DESC",
    )
    .fetch_all(&st.db)
    .await?;
    let out = rows
        .into_iter()
        .map(|r| -> Result<ProjectSummary, sqlx::Error> {
            let updated: DateTime<Utc> = r.try_get("updated_at")?;
            Ok(ProjectSummary {
                id: r.try_get("id")?,
                name: r.try_get("name")?,
                version: r.try_get("version")?,
                updated_at: updated.to_rfc3339_opts(SecondsFormat::Secs, true),
                wall_count: r.try_get("wall_count")?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(out))
}

#[derive(Deserialize)]
pub struct CreateBody {
    name: String,
    address: Option<String>,
}

pub async fn create(State(st): State<AppState>, AppJson(body): AppJson<CreateBody>) -> ApiResult<Json<Project>> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(ApiError::unprocessable("Projektname fehlt"));
    }
    let mut p = Project::new(Uuid::new_v4(), name);
    if let Some(addr) = body.address.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
        p.location = geocode_address(&st.http, addr).await;
    }
    sqlx::query("INSERT INTO projects (id, name, snapshot, version) VALUES ($1, $2, $3, 0)")
        .bind(p.id)
        .bind(&p.name)
        .bind(SqlJson(&p))
        .execute(&st.db)
        .await?;
    Ok(Json(p))
}

pub async fn load(db: &PgPool, id: Uuid) -> ApiResult<Project> {
    let row = sqlx::query("SELECT snapshot, version FROM projects WHERE id = $1")
        .bind(id)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| ApiError::not_found("Projekt nicht gefunden"))?;
    snapshot_from_row(&row)
}

fn snapshot_from_row(row: &sqlx::postgres::PgRow) -> ApiResult<Project> {
    let SqlJson(mut p): SqlJson<Project> = row.try_get("snapshot")?;
    let v: i64 = row.try_get("version")?;
    p.version = v as u64;
    Ok(p)
}

pub async fn get(State(st): State<AppState>, Path(id): Path<Uuid>) -> ApiResult<Json<Project>> {
    Ok(Json(load(&st.db, id).await?))
}

pub async fn delete(State(st): State<AppState>, Path(id): Path<Uuid>) -> ApiResult<StatusCode> {
    let r = sqlx::query("DELETE FROM projects WHERE id = $1").bind(id).execute(&st.db).await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found("Projekt nicht gefunden"));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct OpsBody {
    base_version: i64,
    ops: Vec<Op>,
}

pub async fn push_ops(State(st): State<AppState>, Path(id): Path<Uuid>, AppJson(body): AppJson<OpsBody>) -> ApiResult<Response> {
    let mut tx = st.db.begin().await?;
    let row = sqlx::query("SELECT snapshot, version FROM projects WHERE id = $1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| ApiError::not_found("Projekt nicht gefunden"))?;
    let mut p = snapshot_from_row(&row)?;
    let version = p.version as i64;
    if body.base_version != version {
        return Err(ApiError::with_body(
            StatusCode::CONFLICT,
            json!({
                "error": format!("Versionskonflikt: Server hat Version {version}, Änderung basiert auf {}", body.base_version),
                "version": version,
                "project": p,
            }),
        ));
    }
    if body.ops.is_empty() {
        return Ok(Json(json!({ "version": version })).into_response());
    }
    if let Err((index, e)) = apply_batch(&mut p, &body.ops) {
        return Err(ApiError::with_body(
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "error": format!("Operation {} fehlgeschlagen: {e}", index + 1), "index": index }),
        ));
    }
    let new_version = version + 1;
    p.version = new_version as u64;
    sqlx::query("UPDATE projects SET snapshot = $2, version = $3, name = $4, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(SqlJson(&p))
        .bind(new_version)
        .bind(&p.name)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO project_ops (project_id, version, ops) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(new_version)
        .bind(SqlJson(&body.ops))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(json!({ "version": new_version })).into_response())
}

// ---------- Geocoding (Nominatim) ----------

#[derive(Deserialize)]
struct NominatimHit {
    lat: String,
    lon: String,
}

/// Geocode via Nominatim. Never fails: errors/timeouts → None.
pub async fn geocode_address(http: &reqwest::Client, address: &str) -> Option<GeoLocation> {
    let res = http
        .get("https://nominatim.openstreetmap.org/search")
        .query(&[("format", "json"), ("limit", "1"), ("accept-language", "de"), ("q", address)])
        .timeout(Duration::from_secs(8))
        .send()
        .await;
    let hits: Vec<NominatimHit> = match res {
        Ok(r) if r.status().is_success() => match r.json().await {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!("Nominatim-Antwort unlesbar: {e}");
                return None;
            }
        },
        Ok(r) => {
            tracing::warn!("Nominatim HTTP {}", r.status());
            return None;
        }
        Err(e) => {
            tracing::warn!("Nominatim nicht erreichbar: {e}");
            return None;
        }
    };
    let hit = hits.into_iter().next()?;
    Some(GeoLocation { lat: hit.lat.parse().ok()?, lon: hit.lon.parse().ok()?, address: Some(address.to_string()) })
}

#[derive(Deserialize)]
pub struct GeocodeBody {
    address: String,
}

pub async fn geocode(State(st): State<AppState>, AppJson(body): AppJson<GeocodeBody>) -> ApiResult<Json<GeoLocation>> {
    let addr = body.address.trim();
    if addr.is_empty() {
        return Err(ApiError::unprocessable("Adresse fehlt"));
    }
    geocode_address(&st.http, addr)
        .await
        .map(Json)
        .ok_or_else(|| ApiError::not_found("Adresse nicht gefunden"))
}
