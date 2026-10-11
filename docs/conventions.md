# Konventionen (für alle Beteiligten)

Produkt-Spec: `Innenarchitektur-App – Specs.md` (Repo-Root). API-Vertrag: `docs/api.md`.

## Repo-Layout
- `crates/plan-core` – Rust-Kern: Datenmodell (`model.rs`), Operationen + Inverse (`ops.rs`), Befehle → Op-Batches (`commands.rs`),
  Wände/Gehrungen (`walls.rs`), abgeleitete Räume (`rooms.rs`), Meshes (`mesh.rs`), Basisformen (`primitives.rs`), Sonnenstand NOAA (`sun.rs`),
  Möbel-Snapping/Abstände/Überschneidung/Kollision (`furniture.rs`), Validierung (`validate.rs`), Asset-/Import-Typen.
- `crates/plan-wasm` – wasm-bindgen-Wrapper (`PlanEngine`). Gebaut nach `web/src/core/wasm/` (`npm run wasm` in `web/`).
- `server` – axum-Backend (Binary `innenarchitekt`).
- `web` – React + TS + Vite. TS-Typen aus Rust: `web/src/core/generated/*.ts` (ts-rs, nie von Hand ändern).

Änderungen an `plan-core` → danach `cd web && npm run wasm` (baut WASM + TS-Typen neu) und `cargo test -p plan-core`.

## Koordinaten & Einheiten
- Plan: Millimeter, ganzzahlig. x nach rechts, **y nach unten** (wie das Bild). Plan-Rotation in Grad dreht +x Richtung +y (auf dem Bildschirm im Uhrzeigersinn).
- Wand a→b: «links» = Seite mit Normalen `(d.y, −d.x)` (von oben gesehen links beim Gehen von a nach b). `finish_left/right` entsprechend.
- Öffnung: `offset_mm` = Abstand von Knoten a bis zur nahen Kante. `Hinge::Start` = Band an der Kante näher bei a. `Swing::Left` = Tür schlägt zur linken Wandseite auf.
- Möbel: `pos_mm` = Mitte der Grundfläche (x, y) + Höhe Unterkante (z). Lokal +y = Vorderseite, −y = Rückseite. `dims_mm` = [Breite (lokal x), Tiefe (lokal y), Höhe].
- 3D (three.js): Meter, Y oben. Plan (x, y) mm → (X = x/1000, Z = y/1000). Höhe → Y.
  Möbel-Rotation: `object.rotation.y = −rotation_deg·π/180`. Lokales three +Z = Möbel-Vorderseite.
  Primitive/GLB: Ursprung unten Mitte, Breite X, Tiefe Z, Höhe Y.
- Nordausrichtung `north_angle_deg` N: Kompass-Azimut A zeigt in Plan-Richtung `(sin(N+A), −cos(N+A))`.
  Sonnenrichtung in 3D (zur Sonne hin): `X = cos(el)·sin(N+A)`, `Y = sin(el)`, `Z = −cos(el)·cos(N+A)`.
- Licht an Möbel gekoppelt (`attached_to`): `pos_mm` ist Offset im lokalen Möbel-Frame (x, y Plan-lokal, z Höhe).

## UI
- Sprache Deutsch (Schweiz): immer «ss» statt «ß», echte Umlaute. Zahlen mit Apostroph als Tausendertrenner (`fmtNumber`), Datum TT.MM.JJJJ. Helfer: `web/src/core/format.ts`.
- Farben/Abstände nur über CSS-Variablen aus `web/src/ui/styles.css` (Light + Dark).
- Jede Modelländerung läuft über `useProject.getState().commit(ops, label)` (ein Undo-Schritt) bzw. beim Ziehen `setTransient(ops)` + `endTransient(true|false)`.
  Komplexe Edits über `engine.cmd_*` (liefern Ops, wenden nicht an), dann `commit`.
- KI ändert nie still etwas: KI-Ergebnisse sind Vorschläge (Ops), die der Nutzer annimmt/anpasst/verwirft.
