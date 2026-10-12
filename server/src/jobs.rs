//! Background jobs: Postgres table as queue (`FOR UPDATE SKIP LOCKED`), one worker loop,
//! live progress via `tokio::sync::broadcast` → SSE.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Multipart, Path, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::Json;
use bytes::Bytes;
use futures::Stream;
use object_store::path::Path as ObjPath;
use object_store::PutPayload;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::types::Json as SqlJson;
use sqlx::{PgPool, Row};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::floorplan::{self, AnalyzeInput};
use crate::state::AppState;

pub const KIND_FLOORPLAN: &str = "floorplan";
const JOB_TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Serialize, Clone, Debug)]
pub struct Job {
    pub id: Uuid,
    pub kind: String,
    pub status: String,
    pub progress: f32,
    pub message: String,
    pub result: Option<Value>,
    pub error: Option<String>,
}

impl Job {
    pub fn is_terminal(&self) -> bool {
        self.status == "done" || self.status == "failed"
    }
}

const JOB_COLS: &str = "id, kind, status, progress, message, result, error";

fn job_from_row(r: &sqlx::postgres::PgRow) -> Result<Job, sqlx::Error> {
    Ok(Job {
        id: r.try_get("id")?,
        kind: r.try_get("kind")?,
        status: r.try_get("status")?,
        progress: r.try_get("progress")?,
        message: r.try_get("message")?,
        result: r.try_get::<Option<SqlJson<Value>>, _>("result")?.map(|j| j.0),
        error: r.try_get("error")?,
    })
}

pub async fn load_job(db: &PgPool, id: Uuid) -> Result<Option<Job>, sqlx::Error> {
    sqlx::query(&format!("SELECT {JOB_COLS} FROM jobs WHERE id = $1"))
        .bind(id)
        .fetch_optional(db)
        .await?
        .map(|r| job_from_row(&r))
        .transpose()
}

#[derive(Serialize, Deserialize, Debug)]
struct FloorplanPayload {
    project_id: Uuid,
    floor_id: Uuid,
    image_key: String,
    corners: Option<[[f64; 2]; 4]>,
    #[serde(default)]
    paper_mm_per_px: Option<f64>,
}

// ---------- HTTP ----------

pub async fn get(State(st): State<AppState>, Path(id): Path<Uuid>) -> ApiResult<Json<Job>> {
    load_job(&st.db, id).await?.map(Json).ok_or_else(|| ApiError::not_found("Job nicht gefunden"))
}

fn sse_event(job: &Job) -> Result<Event, Infallible> {
    Ok(Event::default().event("job").json_data(job).unwrap_or_else(|_| Event::default().event("job").data("{}")))
}

struct SseState {
    st: AppState,
    id: Uuid,
    rx: tokio::sync::broadcast::Receiver<Job>,
    first: Option<Job>,
    last: Option<(String, f32, String)>,
    done: bool,
}

/// GET /api/jobs/{id}/events — current state first, then live updates; ends after done/failed.
pub async fn events(State(st): State<AppState>, Path(id): Path<Uuid>) -> ApiResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    // Subscribe before reading the DB so no update between the two is lost.
    let rx = st.jobs_tx.subscribe();
    let job = load_job(&st.db, id).await?.ok_or_else(|| ApiError::not_found("Job nicht gefunden"))?;
    let init = SseState { st, id, rx, first: Some(job), last: None, done: false };
    let stream = futures::stream::unfold(init, |mut s| async move {
        if s.done {
            return None;
        }
        let emit = |s: &mut SseState, j: Job| {
            s.done = j.is_terminal();
            s.last = Some((j.status.clone(), j.progress, j.message.clone()));
            sse_event(&j)
        };
        if let Some(j) = s.first.take() {
            let ev = emit(&mut s, j);
            return Some((ev, s));
        }
        let mut shutdown = s.st.shutdown.clone();
        loop {
            tokio::select! {
                r = s.rx.recv() => match r {
                    Ok(j) if j.id == s.id => { let ev = emit(&mut s, j); return Some((ev, s)); }
                    Ok(_) => continue,
                    Err(RecvError::Lagged(_)) => {
                        if let Ok(Some(j)) = load_job(&s.st.db, s.id).await { let ev = emit(&mut s, j); return Some((ev, s)); }
                    }
                    Err(RecvError::Closed) => return None,
                },
                // Safety net: re-read the DB periodically (e.g. missed in-process notification).
                _ = tokio::time::sleep(Duration::from_secs(10)) => {
                    match load_job(&s.st.db, s.id).await {
                        Ok(Some(j)) => {
                            let key = (j.status.clone(), j.progress, j.message.clone());
                            if s.last.as_ref() != Some(&key) { let ev = emit(&mut s, j); return Some((ev, s)); }
                        }
                        Ok(None) => return None,
                        Err(_) => {}
                    }
                }
                _ = shutdown.changed() => return None,
            }
        }
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

pub fn sniff_image(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("png")
    } else if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else {
        None
    }
}

/// POST /api/projects/{id}/floorplan (multipart: file, corners?, paper_mm_per_px?)
pub async fn upload_floorplan(State(st): State<AppState>, Path(project_id): Path<Uuid>, mut mp: Multipart) -> ApiResult<Json<Value>> {
    let project = crate::projects::load(&st.db, project_id).await?;
    let floor_id = project.floors.first().map(|f| f.id).ok_or_else(|| ApiError::unprocessable("Projekt hat kein Stockwerk"))?;
    let mut file: Option<Bytes> = None;
    let mut corners: Option<[[f64; 2]; 4]> = None;
    let mut paper_mm_per_px: Option<f64> = None;
    while let Some(field) = mp.next_field().await.map_err(|e| ApiError::bad_request(format!("Upload fehlerhaft: {e}")))? {
        match field.name() {
            Some("file") => file = Some(field.bytes().await.map_err(|e| ApiError::bad_request(format!("Upload fehlerhaft: {e}")))?),
            Some("corners") => {
                let t = field.text().await.map_err(|e| ApiError::bad_request(format!("Upload fehlerhaft: {e}")))?;
                if !t.trim().is_empty() && t.trim() != "null" {
                    let c: [[f64; 2]; 4] = serde_json::from_str(&t)
                        .map_err(|_| ApiError::unprocessable("«corners» muss [[x,y],[x,y],[x,y],[x,y]] sein (TL, TR, BR, BL)"))?;
                    if c.iter().flatten().any(|v| !v.is_finite()) {
                        return Err(ApiError::unprocessable("«corners» enthält ungültige Zahlen"));
                    }
                    corners = Some(c);
                }
            }
            Some("paper_mm_per_px") => {
                let t = field.text().await.map_err(|e| ApiError::bad_request(format!("Upload fehlerhaft: {e}")))?;
                paper_mm_per_px = t.trim().parse::<f64>().ok().filter(|v| v.is_finite() && *v > 0.0 && *v < 10.0);
            }
            _ => {}
        }
    }
    let file = file.ok_or_else(|| ApiError::unprocessable("Feld «file» fehlt"))?;
    let ext = sniff_image(&file).ok_or_else(|| ApiError::unprocessable("Nur PNG oder JPG werden unterstützt (PDF bitte im Browser umwandeln)"))?;
    let job_id = Uuid::new_v4();
    let image_key = format!("floorplans/{job_id}/original.{ext}");
    st.store.put(&ObjPath::from(image_key.as_str()), PutPayload::from(file)).await?;
    let payload = FloorplanPayload { project_id, floor_id, image_key, corners, paper_mm_per_px };
    sqlx::query("INSERT INTO jobs (id, kind, status, progress, message, payload) VALUES ($1, $2, 'queued', 0, 'In Warteschlange', $3)")
        .bind(job_id)
        .bind(KIND_FLOORPLAN)
        .bind(SqlJson(&payload))
        .execute(&st.db)
        .await?;
    st.job_notify.notify_waiters();
    Ok(Json(json!({ "job_id": job_id })))
}

// ---------- Worker ----------

pub async fn recover_stuck(db: &PgPool, kinds: &[&str]) -> Result<u64, sqlx::Error> {
    let kinds: Vec<String> = kinds.iter().map(|k| k.to_string()).collect();
    let r = sqlx::query("UPDATE jobs SET status = 'queued', progress = 0, message = 'Nach Neustart erneut eingereiht', updated_at = now() WHERE status = 'running' AND kind = ANY($1)")
        .bind(&kinds)
        .execute(db)
        .await?;
    Ok(r.rows_affected())
}

async fn claim(db: &PgPool, kinds: &[&str]) -> Result<Option<(Job, Value)>, sqlx::Error> {
    let kinds: Vec<String> = kinds.iter().map(|k| k.to_string()).collect();
    let row = sqlx::query(&format!(
        "UPDATE jobs SET status = 'running', progress = 0, message = 'Startet …', updated_at = now()
         WHERE id = (SELECT id FROM jobs WHERE status = 'queued' AND kind = ANY($1) ORDER BY created_at FOR UPDATE SKIP LOCKED LIMIT 1)
         RETURNING {JOB_COLS}, payload"
    ))
    .bind(&kinds)
    .fetch_optional(db)
    .await?;
    match row {
        Some(r) => {
            let SqlJson(payload): SqlJson<Value> = r.try_get("payload")?;
            Ok(Some((job_from_row(&r)?, payload)))
        }
        None => Ok(None),
    }
}

/// One worker loop per queue: long photo jobs must not block floor plan analyses.
pub async fn worker(st: AppState, kinds: &'static [&'static str]) {
    match recover_stuck(&st.db, kinds).await {
        Ok(n) if n > 0 => tracing::info!("{n} hängende Jobs erneut eingereiht"),
        Ok(_) => {}
        Err(e) => tracing::error!("Job-Recovery fehlgeschlagen: {e}"),
    }
    let mut shutdown = st.shutdown.clone();
    loop {
        if *shutdown.borrow() {
            break;
        }
        match claim(&st.db, kinds).await {
            Ok(Some((job, payload))) => {
                let _ = st.jobs_tx.send(job.clone());
                tracing::info!(job = %job.id, kind = %job.kind, "Job gestartet");
                let final_job = run_job(&st, job, payload).await;
                tracing::info!(job = %final_job.id, status = %final_job.status, "Job beendet");
                let _ = st.jobs_tx.send(final_job);
            }
            Ok(None) => {
                tokio::select! {
                    _ = st.job_notify.notified() => {}
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                    _ = shutdown.changed() => {}
                }
            }
            Err(e) => {
                tracing::error!("Job-Queue nicht lesbar: {e}");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
    tracing::info!("Job-Worker beendet");
}

/// Runs a claimed job to completion and persists the final state. Never panics.
async fn run_job(st: &AppState, job: Job, payload: Value) -> Job {
    let outcome = match job.kind.as_str() {
        KIND_FLOORPLAN => run_floorplan(st, job.id, payload).await,
        other => Err(format!("Unbekannter Job-Typ «{other}»")),
    };
    let mut fin = job.clone();
    match outcome {
        Ok(result) => {
            fin.status = "done".into();
            fin.progress = 1.0;
            fin.message = "Fertig".into();
            fin.result = Some(result);
            fin.error = None;
        }
        Err(err) => {
            tracing::warn!(job = %job.id, "Job fehlgeschlagen: {err}");
            fin.status = "failed".into();
            fin.message = "Fehlgeschlagen".into();
            fin.error = Some(err);
        }
    }
    let r = sqlx::query("UPDATE jobs SET status = $2, progress = $3, message = $4, result = $5, error = $6, updated_at = now() WHERE id = $1")
        .bind(fin.id)
        .bind(&fin.status)
        .bind(fin.progress)
        .bind(&fin.message)
        .bind(fin.result.as_ref().map(SqlJson))
        .bind(&fin.error)
        .execute(&st.db)
        .await;
    if let Err(e) = r {
        tracing::error!(job = %fin.id, "Job-Ergebnis nicht speicherbar: {e}");
    }
    fin
}

fn panic_message(e: tokio::task::JoinError) -> String {
    if e.is_cancelled() {
        return "abgebrochen".into();
    }
    let p = e.into_panic();
    if let Some(s) = p.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "unbekannter Fehler".into()
    }
}

async fn run_floorplan(st: &AppState, job_id: Uuid, payload: Value) -> Result<Value, String> {
    let p: FloorplanPayload = serde_json::from_value(payload).map_err(|e| format!("Job-Daten ungültig: {e}"))?;
    let image = st
        .store
        .get(&ObjPath::from(p.image_key.as_str()))
        .await
        .map_err(|e| format!("Grundriss-Bild nicht lesbar: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("Grundriss-Bild nicht lesbar: {e}"))?
        .to_vec();

    // Progress: broadcast immediately (SSE), persist via a coalescing writer task.
    let (ptx, mut prx) = mpsc::unbounded_channel::<(f32, String)>();
    let db = st.db.clone();
    let writer = tokio::spawn(async move {
        while let Some(mut latest) = prx.recv().await {
            while let Ok(next) = prx.try_recv() {
                latest = next;
            }
            let _ = sqlx::query("UPDATE jobs SET progress = $2, message = $3, updated_at = now() WHERE id = $1 AND status = 'running'")
                .bind(job_id)
                .bind(latest.0)
                .bind(&latest.1)
                .execute(&db)
                .await;
        }
    });

    let jobs_tx = st.jobs_tx.clone();
    let llm = st.llm.clone();
    let handle = tokio::runtime::Handle::current();
    let input = AnalyzeInput { image, corners: p.corners, floor_id: p.floor_id, paper_mm_per_px: p.paper_mm_per_px };
    // Run on a blocking thread: CPU-heavy CV doesn't stall the runtime, the future need not be
    // `Send`, and a panic (e.g. unimplemented stage) becomes a JoinError instead of killing us.
    let task = tokio::task::spawn_blocking(move || {
        handle.block_on(async move {
            let progress = move |frac: f32, msg: &str| {
                let frac = if frac.is_finite() { frac.clamp(0.0, 1.0) } else { 0.0 };
                let _ = jobs_tx.send(Job {
                    id: job_id,
                    kind: KIND_FLOORPLAN.into(),
                    status: "running".into(),
                    progress: frac,
                    message: msg.to_string(),
                    result: None,
                    error: None,
                });
                let _ = ptx.send((frac, msg.to_string()));
            };
            floorplan::analyze(input, llm.as_deref(), &progress).await
        })
    });
    let joined = tokio::time::timeout(JOB_TIMEOUT, task).await;
    if joined.is_err() {
        writer.abort(); // the blocking task (and its sender) may still be alive
    }
    let _ = writer.await;
    let out = match joined {
        Err(_) => return Err("Zeitüberschreitung bei der Grundriss-Analyse".into()),
        Ok(Err(join)) => return Err(format!("Grundriss-Analyse abgestürzt (interner Fehler: {})", panic_message(join))),
        Ok(Ok(Err(e))) => return Err(format!("Grundriss-Analyse fehlgeschlagen: {e:#}")),
        Ok(Ok(Ok(out))) => out,
    };
    let key = format!("floorplans/{job_id}/rectified.png");
    st.store
        .put(&ObjPath::from(key.as_str()), PutPayload::from(out.rectified_png))
        .await
        .map_err(|e| format!("Entzerrtes Bild nicht speicherbar: {e}"))?;
    let mut proposal = out.proposal;
    proposal.image.url = format!("/api/files/{key}");
    serde_json::to_value(&proposal).map_err(|e| format!("Ergebnis nicht serialisierbar: {e}"))
}
