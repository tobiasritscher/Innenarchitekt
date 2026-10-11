# API-Vertrag (MVP)

Alle Pfade unter `/api`. JSON, Fehler als `{ "error": "<Text, deutsch>" }`. Typen = ts-rs-Typen aus `plan-core`
(`web/src/core/generated/*.ts`, Rust `plan_core::*`).

## Auth
Statisches Token (`AUTH_TOKEN`). Akzeptiert als `Authorization: Bearer <token>` **oder** Cookie `ia_token`
(damit `<img>`/GLB-Loader ohne Header funktionieren). Ohne Auth: `GET /api/health`, `POST /api/auth`, `GET /api/auth`,
statische Frontend-Dateien.

| Methode | Pfad | Body | Antwort |
|---|---|---|---|
| POST | `/api/auth` | `{token}` | 204 + `Set-Cookie: ia_token=…; HttpOnly; SameSite=Strict; Path=/; Max-Age=31536000` / 401 |
| GET | `/api/auth` | – | 200 `{ok:true}` / 401 |
| GET | `/api/health` | – | 200 `{ok:true}` |
| GET | `/api/config` | – | `{llm_enabled: bool, llm_model: string}` |

## Projekte
| Methode | Pfad | Body | Antwort |
|---|---|---|---|
| GET | `/api/projects` | – | `ProjectSummary[]` = `{id, name, version, updated_at (RFC3339), wall_count}` |
| POST | `/api/projects` | `{name, address?: string}` | `Project` (Adresse → Nominatim-Geocoding, bei Fehler `location: null`) |
| GET | `/api/projects/{id}` | – | `Project` (`version` = Server-Version) |
| DELETE | `/api/projects/{id}` | – | 204 |
| POST | `/api/projects/{id}/ops` | `{base_version: number, ops: Op[]}` | 200 `{version}` · 409 `{error, version, project}` wenn `base_version` ≠ aktuelle Version · 422 `{error, index}` wenn ein Op scheitert (Batch atomar) |
| POST | `/api/geocode` | `{address}` | `GeoLocation` / 404 |

Server wendet Ops mit `plan_core::apply_batch` an, `version += 1` pro Batch, speichert Snapshot (JSONB) + Op-Log.

## Grundriss-Import
| Methode | Pfad | Body | Antwort |
|---|---|---|---|
| POST | `/api/projects/{id}/floorplan` | multipart: `file` (PNG/JPG; PDF wird im Browser zu PNG gerendert), `corners` optional JSON `[[x,y],[x,y],[x,y],[x,y]]` in Bild-Pixeln (TL, TR, BR, BL) | `{job_id}` |
| GET | `/api/jobs/{id}` | – | `Job` |
| GET | `/api/jobs/{id}/events` | – | SSE, Event `job`, data = `Job`-JSON; endet nach `done`/`failed` |

`Job = {id, kind: "floorplan", status: "queued"|"running"|"done"|"failed", progress: 0..1, message: string, result: FloorplanProposal|null, error: string|null}`

`FloorplanProposal.ops` beziehen sich auf `project.floors[0].id` und enthalten nur `PutNode/PutWall/PutOpening/PutRoom`
(neue IDs). `image` = entzerrtes Bild als `RefImage` (`url` = `/api/files/<key>`), `origin_mm=[0,0]`, `mm_per_px` = geschätzter Massstab.
Plan-Koordinaten = Bildpixel × `mm_per_px` (Ursprung oben links). Die Ops werden **nicht** angewendet – das Frontend zeigt sie als Vorschlag.

## Möbel-Bibliothek
| Methode | Pfad | Body | Antwort |
|---|---|---|---|
| GET | `/api/assets` | – | `Asset[]` (neueste zuerst) |
| GET | `/api/assets/{id}` | – | `Asset` |
| DELETE | `/api/assets/{id}` | – | 204 |
| POST | `/api/assets/ikea` | `{url}` | `Asset` (gecacht nach Artikelnummer; GLB in Object-Store; `warnings` bei >3 % Abweichung BBox vs. Produktmasse oder fehlendem 3D-Modell) |
| POST | `/api/assets/upload` | multipart: `file` (GLB), `name`, `w_mm`, `d_mm`, `h_mm` | `Asset` |
| POST | `/api/assets/primitive` | `{shape: PrimitiveShape, name?, dims_mm?: [w,d,h]}` | `Asset` (kein Modell; Mesh kommt aus WASM `primitive()`) |
| GET | `/api/assets/{id}/model.glb` | – | GLB (`model/gltf-binary`) |
| GET | `/api/files/{key}` | – | Datei aus dem Object-Store (Referenzbilder, Thumbnails) |

`Asset.model_url` = `/api/assets/{id}/model.glb`, `thumbnail_url` = `/api/files/…` oder IKEA-Bild-URL.
