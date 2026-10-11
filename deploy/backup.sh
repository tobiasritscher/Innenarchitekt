#!/bin/bash
# Nightly backup on the server: pg_dump (custom format) + data dir (hard-link incremental).
# Keeps 14 days. Runs via LaunchAgent ch.innenarchitekt.backup. Restore: see README.md.
set -euo pipefail

BASE="${IA_BASE:-$HOME/innenarchitekt}"
PG_BIN="${PG_BIN:-/opt/homebrew/opt/postgresql@16/bin}"
DB_NAME="${IA_DB:-innenarchitekt}"
KEEP_DAYS="${IA_KEEP_DAYS:-14}"
DEST="$BASE/backups"
TS="$(date +%Y-%m-%d_%H%M%S)"
TARGET="$DEST/$TS"

log() { echo "$(date '+%d.%m.%Y %H:%M:%S') backup: $*"; }

mkdir -p "$TARGET"
log "Start → $TARGET"

"$PG_BIN/pg_dump" -h 127.0.0.1 -Fc -d "$DB_NAME" -f "$TARGET/$DB_NAME.dump.tmp"
mv "$TARGET/$DB_NAME.dump.tmp" "$TARGET/$DB_NAME.dump"

if [ -d "$BASE/data" ]; then
  if [ -d "$DEST/latest/data" ]; then
    rsync -a --delete --link-dest="$DEST/latest/data" "$BASE/data/" "$TARGET/data/"
  else
    rsync -a --delete "$BASE/data/" "$TARGET/data/"
  fi
fi
[ -f "$BASE/.env" ] && cp -p "$BASE/.env" "$TARGET/env.backup" && chmod 600 "$TARGET/env.backup"

ln -sfn "$TS" "$DEST/latest"

# Rotation: remove backup folders older than KEEP_DAYS (never the one just written).
find "$DEST" -mindepth 1 -maxdepth 1 -type d -name '20??-??-??_*' -mtime +"$KEEP_DAYS" ! -name "$TS" -print -exec rm -rf {} +

log "Fertig ($(du -sh "$TARGET" | cut -f1))"
