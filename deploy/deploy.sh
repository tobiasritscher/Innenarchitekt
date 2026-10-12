#!/bin/bash
# Build locally (macOS arm64) and deploy to the server ("asgard") as a user LaunchAgent.
#
#   deploy/deploy.sh                 # build + deploy
#   deploy/deploy.sh --skip-build    # deploy existing artefacts
#
# Env overrides: DEPLOY_HOST (default asgard), REMOTE_BASE (default ~/innenarchitekt, i.e. $HOME of the
# remote user), PG_BIN (remote, default /opt/homebrew/opt/postgresql@16/bin).
# Never touches ~/innenarchitekt/data, ~/innenarchitekt/.env (after first run) or other services.
set -euo pipefail

HOST="${DEPLOY_HOST:-asgard}"
PG_BIN="${PG_BIN:-/opt/homebrew/opt/postgresql@16/bin}"
LABEL="ch.innenarchitekt.server"
BACKUP_LABEL="ch.innenarchitekt.backup"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SKIP_BUILD=0
for a in "$@"; do
  case "$a" in
    --skip-build) SKIP_BUILD=1 ;;
    -h|--help) sed -n '2,10p' "$0"; exit 0 ;;
    *) echo "Unbekannte Option: $a" >&2; exit 2 ;;
  esac
done

say() { printf '\033[1;33m▸ %s\033[0m\n' "$*"; }

# ---------- 1. Build ----------
if [ "$SKIP_BUILD" = 0 ]; then
  say "Backend bauen (release)"
  (cd "$ROOT" && cargo build --release -p innenarchitekt)
  say "Frontend bauen"
  (cd "$ROOT/web" && { [ -d node_modules ] || npm ci; } && npm run build)
fi
BIN="$ROOT/target/release/innenarchitekt"
[ -x "$BIN" ] || { echo "Binary fehlt: $BIN" >&2; exit 1; }
[ -f "$ROOT/web/dist/index.html" ] || { echo "web/dist fehlt – zuerst bauen" >&2; exit 1; }
[ "$(uname -m)" = "arm64" ] || { echo "Binary muss auf arm64 gebaut werden (Server ist Apple Silicon)" >&2; exit 1; }

# ---------- 2. Remote layout ----------
REMOTE_HOME="$(ssh -n "$HOST" 'printf %s "$HOME"')"
REMOTE_USER="$(ssh -n "$HOST" whoami)"
BASE="${REMOTE_BASE:-$REMOTE_HOME/innenarchitekt}"
say "Ziel: $HOST:$BASE"
ssh "$HOST" "mkdir -p '$BASE/bin' '$BASE/web' '$BASE/logs' '$BASE/data' '$BASE/backups' '$REMOTE_HOME/Library/LaunchAgents'"

# ---------- 3. Pre-deploy backup (DB + data), only if an installation exists ----------
if ssh "$HOST" "test -x '$BASE/bin/backup.sh' && test -f '$BASE/.env'"; then
  say "Backup vor Deploy"
  ssh "$HOST" "PG_BIN='$PG_BIN' IA_BASE='$BASE' '$BASE/bin/backup.sh'"
fi

# ---------- 4. Upload ----------
say "Dateien übertragen"
rsync -az "$BIN" "$HOST:$BASE/bin/innenarchitekt.new"
rsync -az "$ROOT/deploy/backup.sh" "$HOST:$BASE/bin/backup.sh"
rsync -az --delete "$ROOT/web/dist/" "$HOST:$BASE/web/"
ssh "$HOST" "chmod +x '$BASE/bin/innenarchitekt.new' '$BASE/bin/backup.sh' && mv -f '$BASE/bin/innenarchitekt.new' '$BASE/bin/innenarchitekt'"

# ---------- 5. First run: database + .env ----------
say "Datenbank prüfen"
ssh "$HOST" "PG_BIN='$PG_BIN' bash -s" <<'REMOTE'
set -euo pipefail
if ! "$PG_BIN/psql" -h 127.0.0.1 -d postgres -Atc "SELECT 1 FROM pg_database WHERE datname='innenarchitekt'" | grep -q 1; then
  "$PG_BIN/createdb" -h 127.0.0.1 innenarchitekt
  echo "Datenbank «innenarchitekt» angelegt"
fi
REMOTE

NEW_TOKEN=""
if ! ssh "$HOST" "test -f '$BASE/.env'"; then
  say ".env anlegen (erster Deploy)"
  NEW_TOKEN="$(openssl rand -hex 32)"
  ssh "$HOST" "umask 077 && cat > '$BASE/.env'" <<EOF
# Innenarchitekt – Konfiguration (vom ersten Deploy erzeugt, wird danach nie überschrieben)
DATABASE_URL=postgres://$REMOTE_USER@127.0.0.1:5432/innenarchitekt
BIND=127.0.0.1:8090
AUTH_TOKEN=$NEW_TOKEN
DATA_DIR=$BASE/data
WEB_DIR=$BASE/web
# KI über LiteLLM (OpenAI-kompatibel). Leer lassen = KI-Funktionen aus.
LLM_BASE_URL=http://127.0.0.1:4001/v1
LLM_API_KEY=
LLM_MODEL=claude-opus-5-5
IKEA_COUNTRY=ch
IKEA_LANG=de
RUST_LOG=innenarchitekt=info,tower_http=info,sqlx=warn
EOF
fi

# ---------- 6. LaunchAgents ----------
say "LaunchAgents installieren/neu laden"
for L in "$LABEL" "$BACKUP_LABEL"; do
  sed "s#__BASE__#$BASE#g" "$ROOT/deploy/$L.plist" | ssh "$HOST" "cat > '$REMOTE_HOME/Library/LaunchAgents/$L.plist'"
done
ssh "$HOST" "LABEL='$LABEL' BACKUP_LABEL='$BACKUP_LABEL' bash -s" <<'REMOTE'
set -euo pipefail
DOMAIN="gui/$(id -u)"
for L in "$LABEL" "$BACKUP_LABEL"; do
  PLIST="$HOME/Library/LaunchAgents/$L.plist"
  plutil -lint "$PLIST" >/dev/null
  if launchctl print "$DOMAIN/$L" >/dev/null 2>&1; then
    launchctl bootout "$DOMAIN/$L" 2>/dev/null || true
    for i in 1 2 3 4 5 6 7 8 9 10; do launchctl print "$DOMAIN/$L" >/dev/null 2>&1 || break; sleep 1; done
  fi
  if ! launchctl bootstrap "$DOMAIN" "$PLIST"; then
    echo "bootstrap fehlgeschlagen – Fallback auf launchctl load" >&2
    launchctl unload "$PLIST" 2>/dev/null || true
    launchctl load -w "$PLIST"
  fi
done
REMOTE

# ---------- 7. Health check ----------
say "Health-Check"
BIND_ADDR="$(ssh "$HOST" "grep -E '^BIND=' '$BASE/.env' | cut -d= -f2" || true)"
BIND_ADDR="${BIND_ADDR:-127.0.0.1:8090}"
if ssh "$HOST" "for i in \$(seq 1 20); do curl -fsS 'http://$BIND_ADDR/api/health' >/dev/null 2>&1 && exit 0; sleep 1; done; exit 1"; then
  echo "✓ Server läuft auf $HOST ($BIND_ADDR)"
else
  echo "✗ Server antwortet nicht – letzte Log-Zeilen:" >&2
  ssh "$HOST" "tail -n 40 '$BASE/logs/server.log'" >&2 || true
  exit 1
fi

if [ -n "$NEW_TOKEN" ]; then
  echo
  echo "════════════════════════════════════════════════════════════════════"
  echo " AUTH_TOKEN (Login im Browser): $NEW_TOKEN"
  echo " Gespeichert in $HOST:$BASE/.env"
  echo " LLM_API_KEY ist leer → KI aus. Bei Bedarf in .env setzen und erneut deployen."
  echo "════════════════════════════════════════════════════════════════════"
fi
