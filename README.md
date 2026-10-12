# Innenarchitekt

Self-hosted Web-App: Grundriss-Foto rein → editierbares 3D-Modell der Wohnung → Möbel platzieren (IKEA-Link, Basisform, GLB) → bei Tages- und Kunstlicht begehen. MVP für genau einen Nutzer.

- Produkt-Spec: [`Innenarchitektur-App – Specs.md`](./Innenarchitektur-App%20–%20Specs.md)
- API-Vertrag: [`docs/api.md`](docs/api.md) · Konventionen: [`docs/conventions.md`](docs/conventions.md)

## Aufbau

| Pfad | Inhalt |
|---|---|
| `crates/plan-core` | Rust-Kern: Datenmodell, Operationen, Geometrie, Validierung (nativ + WASM) |
| `crates/plan-wasm` | WASM-Wrapper für das Frontend |
| `server` | Backend (axum, sqlx/Postgres, `object_store`), Binary `innenarchitekt` |
| `web` | Frontend (React, TypeScript, Vite, three.js) |
| `deploy` | Deploy- und Backup-Skripte (macOS, launchd) |

Backend-Eckpunkte: Projekte als JSONB-Snapshot + Op-Log mit Versionsnummer (409 bei Konflikt), Jobs als Postgres-Queue (`FOR UPDATE SKIP LOCKED`) mit SSE-Fortschritt, Dateien im Object-Store (`DATA_DIR/objects`, lokal; S3/MinIO später nur Konfiguration), Migrationen sind ins Binary eingebettet.

## Entwicklung

Voraussetzungen: Rust (stable), Node 20+, Docker (für Postgres), optional `wasm-pack`.

```bash
# Postgres für Entwicklung/Tests (Port 5544)
docker run -d --name ia-postgres -e POSTGRES_PASSWORD=ia -e POSTGRES_DB=innenarchitekt \
  -p 127.0.0.1:5544:5432 postgres:16

# .env im Repo-Root
cat > .env <<'EOF'
DATABASE_URL=postgres://postgres:ia@127.0.0.1:5544/innenarchitekt
AUTH_TOKEN=dev-token-bitte-aendern-0123
EOF

cargo run -p innenarchitekt          # Backend auf http://127.0.0.1:8090
cd web && npm install && npm run wasm && npm run dev   # Frontend auf :5173, /api → :8090
```

Tests:

```bash
cargo test -p plan-core
DATABASE_URL=postgres://postgres:ia@127.0.0.1:5544/innenarchitekt cargo test -p innenarchitekt
```

Ohne `DATABASE_URL` laufen nur die Unit-Tests (GLB-BBox, IKEA-Parser gegen Fixtures in `server/tests/fixtures/`, Auth); die HTTP-Tests werden übersprungen.

## Konfiguration (Umgebungsvariablen)

Gelesen aus der Umgebung bzw. aus `.env` im Arbeitsverzeichnis.

| Variable | Default | Bedeutung |
|---|---|---|
| `DATABASE_URL` | – (Pflicht) | Postgres-Verbindung |
| `AUTH_TOKEN` | – (Pflicht, ≥ 12 Zeichen) | Statisches Login-Token (Header `Authorization: Bearer …` oder Cookie `ia_token`) |
| `BIND` | `127.0.0.1:8090` | Listen-Adresse |
| `DATA_DIR` | `./data` | Object-Store (Uploads, entzerrte Grundrisse, GLB-Modelle) unter `DATA_DIR/objects` |
| `WEB_DIR` | `./web/dist` | Gebautes Frontend (SPA-Fallback auf `index.html`) |
| `LLM_BASE_URL` | – | OpenAI-kompatibler Endpunkt (LiteLLM), z. B. `http://127.0.0.1:4001/v1` |
| `LLM_API_KEY` | – | Key für den Endpunkt; fehlt `LLM_BASE_URL` oder `LLM_API_KEY` → KI aus |
| `LLM_MODEL` | `claude-opus-5-5` | Modellname |
| `IKEA_COUNTRY` / `IKEA_LANG` | `ch` / `de` | Markt für IKEA-Import (bei Links wird der Markt aus der URL genommen) |
| `IKEA_ROTERA_CLIENT_ID` | IKEA-Web-Default | Client-ID der IKEA-3D-Viewer-API, nur falls IKEA sie ändert |
| `RUST_LOG` | `innenarchitekt=info,tower_http=info,sqlx=warn` | Log-Level |

## Deployment (Mac mini «asgard»)

Das Binary wird lokal gebaut (Apple Silicon, gleiche Architektur) und per SSH kopiert; auf dem Server braucht es kein Rust.

```bash
deploy/deploy.sh               # baut Backend (release) + Frontend, deployt, startet neu
deploy/deploy.sh --skip-build  # nur übertragen
```

Das Skript
- legt `~/innenarchitekt/{bin,web,logs,data,backups}` an (`data/` und `.env` werden nie überschrieben),
- erstellt beim ersten Lauf die Datenbank `innenarchitekt` (Homebrew-Postgres 16) und eine `.env` mit generiertem `AUTH_TOKEN` (wird einmalig ausgegeben),
- installiert die LaunchAgents `ch.innenarchitekt.server` (KeepAlive, Logs in `~/innenarchitekt/logs/server.log`) und `ch.innenarchitekt.backup` und lädt sie neu,
- prüft danach `GET /api/health`.

Produktiv läuft die App unter **https://innenarchitekt.ritscher.ch** (bis 12.10.2026 auf «jarvis», nur per SSH-Tunnel):

- **Caddy** (Docker/OrbStack auf asgard, `~/docker/caddy/Caddyfile`) terminiert TLS (DNS-01 über Cloudflare) und
  leitet auf `192.168.1.92:8090` weiter. Caddy setzt `X-Forwarded-Proto` selbst, das Login-Cookie wird damit `Secure`.
- **`BIND=0.0.0.0:8090`** in der `.env` auf asgard: Der Caddy-Container erreicht den `localhost` des Macs nicht,
  nur die LAN-IP. Die App ist damit auch im Heimnetz direkt erreichbar, das `AUTH_TOKEN` gilt dort genauso.
- **DNS:** CNAME `innenarchitekt` → `rehalp.skynetmedia.de` (nur DNS, nicht proxied) in der Cloudflare-Zone
  `ritscher.ch`. Ein eigener Eintrag ist nötig, weil `*.ritscher.ch` woandershin zeigt.

Das Skript ändert weder Caddy noch DNS. `.env` nach Änderungen (z. B. `AUTH_TOKEN`, `LLM_API_KEY`) übernehmen:
`launchctl kickstart -k gui/$(id -u)/ch.innenarchitekt.server`.

### Backup

`deploy/backup.sh` läuft nächtlich um 03:30 (LaunchAgent `ch.innenarchitekt.backup`): `pg_dump` (Custom-Format) plus inkrementelle Kopie von `data/` (Hardlinks) nach `~/innenarchitekt/backups/<Datum>/`, Aufbewahrung 14 Tage. Manuell: `bash ~/innenarchitekt/bin/backup.sh`.

Wiederherstellen:

```bash
launchctl bootout gui/$(id -u)/ch.innenarchitekt.server
/opt/homebrew/opt/postgresql@16/bin/pg_restore -h 127.0.0.1 --clean --if-exists -d innenarchitekt \
  ~/innenarchitekt/backups/<Datum>/innenarchitekt.dump
rsync -a --delete ~/innenarchitekt/backups/<Datum>/data/ ~/innenarchitekt/data/
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/ch.innenarchitekt.server.plist
```

## IKEA-Import

Der Parser (`server/src/ikea.rs`) nutzt die öffentliche Produktseite (JSON-LD + eingebettete Daten) und die Daten-API des IKEA-3D-Viewers («In 3D ansehen») für das GLB. Das ist keine offizielle Schnittstelle und kann jederzeit brechen; dann bleibt der GLB-Upload. Die Modelle sind geistiges Eigentum von IKEA – für private Nutzung gedacht.
