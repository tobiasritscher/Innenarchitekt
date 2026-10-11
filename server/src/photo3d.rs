//! «Möbel aus Foto» (Phase 2): photo + target dimensions → GLB via TRELLIS.2 (trellis-mac, native
//! Python on Apple Silicon/MPS) → library asset scaled to the dimensions.
//!
//! The Python side is an external checkout (`PHOTO3D_DIR`, see deploy/photo3d-setup.sh); we only run its
//! `generate.py` as a low-priority subprocess and translate its log lines into job progress.
//! Jobs run in their own queue so a 5-minute generation never blocks floor plan analyses.

use std::path::Path as FsPath;
use std::process::Stdio;
use std::time::Duration;

use axum::extract::{Multipart, State};
use axum::Json;
use object_store::path::Path as ObjPath;
use object_store::PutPayload;
use plan_core::assets::AssetKind;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::types::Json as SqlJson;
use tokio::io::{AsyncBufReadExt, BufReader};
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::jobs::{sniff_image, Job, KIND_PHOTO3D};
use crate::state::AppState;

const TIMEOUT: Duration = Duration::from_secs(45 * 60);
/// Target triangle count after simplification (Spec: about 50'000).
const TARGET_FACES: u32 = 50_000;

#[derive(Serialize, Deserialize)]
struct Payload {
    image_key: String,
    name: String,
    dims_mm: [u32; 3],
    seed: u32,
}

/// POST /api/assets/from-photo (multipart: photo, name, w_mm, d_mm, h_mm, seed?) → `{job_id}`.
/// The job result is the new asset.
pub async fn upload(State(st): State<AppState>, mut mp: Multipart) -> ApiResult<Json<Value>> {
    if st.cfg.photo3d_dir.is_none() {
        return Err(ApiError::new(axum::http::StatusCode::SERVICE_UNAVAILABLE, "«Möbel aus Foto» ist auf dem Server nicht eingerichtet (PHOTO3D_DIR)"));
    }
    let mut photo = None;
    let mut name = String::new();
    let mut dims: [Option<u32>; 3] = [None; 3];
    let mut seed = 42u32;
    let bad = |e: axum::extract::multipart::MultipartError| ApiError::bad_request(format!("Upload fehlerhaft: {e}"));
    while let Some(field) = mp.next_field().await.map_err(bad)? {
        match field.name().unwrap_or("") {
            "photo" => photo = Some(field.bytes().await.map_err(bad)?),
            "name" => name = field.text().await.map_err(bad)?.trim().to_string(),
            "w_mm" => dims[0] = crate::assets::parse_dim(&field.text().await.map_err(bad)?, "Breite")?,
            "d_mm" => dims[1] = crate::assets::parse_dim(&field.text().await.map_err(bad)?, "Tiefe")?,
            "h_mm" => dims[2] = crate::assets::parse_dim(&field.text().await.map_err(bad)?, "Höhe")?,
            "seed" => seed = field.text().await.map_err(bad)?.trim().parse().unwrap_or(42),
            _ => {}
        }
    }
    let photo = photo.ok_or_else(|| ApiError::unprocessable("Feld «photo» fehlt"))?;
    let ext = sniff_image(&photo).ok_or_else(|| ApiError::unprocessable("Foto bitte als JPG oder PNG"))?;
    // The model does not know the real size: dimensions are mandatory.
    let [Some(w), Some(d), Some(h)] = dims else {
        return Err(ApiError::unprocessable("Bitte Breite, Tiefe und Höhe angeben – das Modell kennt die echte Grösse nicht"));
    };
    let job_id = Uuid::new_v4();
    let image_key = format!("photos/{job_id}/photo.{ext}");
    st.store.put(&ObjPath::from(image_key.as_str()), PutPayload::from(photo)).await?;
    let payload = Payload { image_key, name: if name.is_empty() { "Möbel aus Foto".into() } else { name }, dims_mm: [w, d, h], seed };
    sqlx::query("INSERT INTO jobs (id, kind, status, progress, message, payload) VALUES ($1, $2, 'queued', 0, 'In Warteschlange', $3)")
        .bind(job_id)
        .bind(KIND_PHOTO3D)
        .bind(SqlJson(&payload))
        .execute(&st.db)
        .await?;
    st.job_notify.notify_waiters();
    Ok(Json(json!({ "job_id": job_id })))
}

/// Progress from generate.py's log lines (it prints stage headers).
fn stage(line: &str) -> Option<(f32, &'static str)> {
    let l = line.trim_start();
    Some(if l.starts_with("Loading pipeline") {
        (0.05, "KI-Modell laden (ca. 2 Min.)")
    } else if l.starts_with("Loaded in") || l.starts_with("Device:") {
        (0.3, "Foto vorbereiten")
    } else if l.starts_with("Generating 3D model") {
        (0.35, "3D-Form erzeugen")
    } else if l.starts_with("Mesh:") {
        (0.65, "Form fertig")
    } else if l.starts_with("Baking PBR textures") {
        (0.7, "Texturen backen")
    } else if l.starts_with("Simplifying mesh") {
        (0.75, "Modell vereinfachen")
    } else if l.contains("UV unwrap") {
        (0.8, "Texturen abwickeln")
    } else if l.starts_with("Saved:") && l.ends_with(".glb") {
        (0.95, "Modell speichern")
    } else {
        return None;
    })
}

fn send_progress(st: &AppState, job_id: Uuid, frac: f32, msg: &str) {
    let _ = st.jobs_tx.send(Job {
        id: job_id,
        kind: KIND_PHOTO3D.into(),
        status: "running".into(),
        progress: frac,
        message: msg.into(),
        result: None,
        error: None,
    });
    let db = st.db.clone();
    let msg = msg.to_string();
    tokio::spawn(async move {
        let _ = sqlx::query("UPDATE jobs SET progress = $2, message = $3, updated_at = now() WHERE id = $1 AND status = 'running'")
            .bind(job_id)
            .bind(frac)
            .bind(msg)
            .execute(&db)
            .await;
    });
}

pub async fn run(st: &AppState, job_id: Uuid, payload: Value) -> Result<Value, String> {
    let p: Payload = serde_json::from_value(payload).map_err(|e| format!("Job-Daten ungültig: {e}"))?;
    let dir = st.cfg.photo3d_dir.clone().ok_or("«Möbel aus Foto» ist nicht eingerichtet")?;
    let photo = st
        .store
        .get(&ObjPath::from(p.image_key.as_str()))
        .await
        .map_err(|e| format!("Foto nicht lesbar: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("Foto nicht lesbar: {e}"))?;
    let work = st.cfg.data_dir.join("tmp").join(format!("photo3d-{job_id}"));
    tokio::fs::create_dir_all(&work).await.map_err(|e| format!("Arbeitsordner: {e}"))?;
    let ext = p.image_key.rsplit('.').next().unwrap_or("png");
    let input = work.join(format!("input.{ext}"));
    tokio::fs::write(&input, &photo).await.map_err(|e| format!("Arbeitsordner: {e}"))?;
    let out_base = work.join("model");
    let result = generate(st, job_id, &dir, &input, &out_base, p.seed).await;
    let glb = match result {
        Ok(()) => tokio::fs::read(out_base.with_extension("glb")).await.map_err(|e| format!("Kein Modell erzeugt: {e}")),
        Err(e) => Err(e),
    };
    let _ = tokio::fs::remove_dir_all(&work).await;
    let glb = glb?;
    send_progress(st, job_id, 0.97, "In die Bibliothek übernehmen");
    let thumb = Some(format!("/api/files/{}", p.image_key));
    let asset = crate::assets::create_glb_asset(st, glb, p.name, p.dims_mm.map(Some), AssetKind::Photo, thumb)
        .await
        .map_err(|e| format!("Modell nicht übernehmbar: {}", e.body["error"].as_str().unwrap_or("unbekannter Fehler")))?;
    serde_json::to_value(&asset).map_err(|e| e.to_string())
}

async fn generate(st: &AppState, job_id: Uuid, dir: &FsPath, input: &FsPath, out_base: &FsPath, seed: u32) -> Result<(), String> {
    send_progress(st, job_id, 0.02, "Startet …");
    let mut child = tokio::process::Command::new("nice")
        .args(["-n", "10"])
        .arg(dir.join(".venv/bin/python"))
        .arg("-u")
        .arg(dir.join("generate.py"))
        .arg(input)
        .arg("--output")
        .arg(out_base)
        .args(["--pipeline-type", "512", "--texture-size", "1024", "--seed", &seed.to_string()])
        .current_dir(dir)
        .env("IA_TARGET_FACES", TARGET_FACES.to_string())
        .env("PYTORCH_ENABLE_MPS_FALLBACK", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("TRELLIS konnte nicht gestartet werden: {e}"))?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let st2 = st.clone();
    let out_task = tokio::spawn(async move {
        let mut tail: Vec<String> = Vec::new();
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some((f, m)) = stage(&line) {
                send_progress(&st2, job_id, f, m);
            }
            tracing::debug!(target: "photo3d", "{line}");
            tail.push(line);
            if tail.len() > 30 {
                tail.remove(0);
            }
        }
        tail
    });
    let err_task = tokio::spawn(async move {
        let mut tail: Vec<String> = Vec::new();
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            // tqdm progress bars go to stderr; keep only the last lines for error messages
            tail.push(line);
            if tail.len() > 30 {
                tail.remove(0);
            }
        }
        tail
    });
    let status = match tokio::time::timeout(TIMEOUT, child.wait()).await {
        Ok(s) => s.map_err(|e| format!("TRELLIS: {e}"))?,
        Err(_) => {
            let _ = child.kill().await;
            return Err("Zeitüberschreitung bei der 3D-Erzeugung (45 Min.)".into());
        }
    };
    let out_tail = out_task.await.unwrap_or_default();
    let err_tail = err_task.await.unwrap_or_default();
    if !status.success() {
        let all: Vec<&String> = out_tail.iter().chain(err_tail.iter()).collect();
        tracing::warn!(job = %job_id, "TRELLIS fehlgeschlagen: {}", all.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" | "));
        let hint = if all.iter().any(|l| l.contains("gated") || l.contains("401") || l.contains("403")) {
            "Kein Zugriff auf die KI-Modelle bei Hugging Face (DINOv3/RMBG-2.0 freischalten)."
        } else if all.iter().any(|l| l.contains("empty mesh")) {
            "Aus dem Foto liess sich keine Form erzeugen – anderes Foto (Möbel freigestellt, ganz im Bild) versuchen."
        } else {
            "Details im Server-Log."
        };
        return Err(format!("3D-Erzeugung fehlgeschlagen ({status}). {hint}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_are_monotonic() {
        let log = ["Loading pipeline...", "Loaded in 103s", "Generating 3D model (pipeline=512, seed=42)...", "Mesh: 812,345 vertices, 1,600,000 triangles", "Baking PBR textures via KDTree (1024x1024)...", "  Simplifying mesh: 1,600,000 -> ~50,000 faces", "  UV unwrapping with xatlas...", "  Saved: /x/model.glb"];
        let fr: Vec<f32> = log.iter().filter_map(|l| stage(l)).map(|s| s.0).collect();
        assert_eq!(fr.len(), log.len());
        assert!(fr.windows(2).all(|w| w[0] < w[1]), "{fr:?}");
        assert!(stage("random noise").is_none());
    }
}
