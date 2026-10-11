# Licht: leuchtende Objekte, richtbare Spots, Lichtmodus

Stand 11.10.2026 · Status: Entwurf zur Durchsicht

## Ziel

Lichtplanung mit realistischem Eindruck (keine Lux-Berechnung): indirektes Licht (Uplights), LED-Streifen,
beliebige Objekte als Leuchtkörper, damit auch der KI-Agent Lampen aus Grundformen bauen kann. Dazu ein
eigener Modus zum Platzieren und Ausrichten von Licht mit Tageszeit.

Rahmen: Leistung zählt (schwächere Rechner kamen schon an die Grenze). Live = Näherung, «Foto rendern» = exakt.

## Nicht im Umfang

- Gruppieren mehrerer Objekte zu einem (eine KI-Lampe aus 3 Quadern bleibt 3 Möbel).
- Schatten von Ersatzlichtern.
- Einzelne Materialien eines GLB leuchten lassen (GLB leuchtet ganz).
- Lux-/Lichtplanungs-Kennzahlen.

## 1 · Datenmodell (`plan-core`)

### `LightKind`

```rust
pub enum LightKind {
    Point,
    Spot {
        angle_deg: f32,
        /// 0 = straight down, 90 = horizontal, 180 = straight up.
        #[serde(default)] tilt_deg: f32,
        /// Plan direction the spot leans towards (deg, like furniture rotation: 0 = +x, turning towards +y).
        #[serde(default)] heading_deg: f32,
    },
    Area { w_mm: u32, h_mm: u32 },
    /// The furniture it is attached to glows (lamp shade, LED strip). Requires `attached_to`.
    Surface { face: GlowFace },
}

pub enum GlowFace { All, Down, Up, Front }
```

- Alte Projekte: Spot ohne `tilt_deg`/`heading_deg` → 0/0 = nach unten wie bisher.
- Spot mit `attached_to`: `heading_deg` relativ zur Möbel-Rotation (Neigung absolut, da Möbel nur um die Hochachse drehen).
- `Surface`: `pos_mm` wird ignoriert (Lage folgt dem Möbel). Ohne gültiges `attached_to` → Validierungsfehler
  in `validate.rs` («Leuchtfläche ohne Objekt»). `delete_furniture` löscht angehängte Lichter bereits.
- Helfer `light_dir(l, project) -> [f64; 3]` (Plan-Koordinaten, z hoch) für Spot-Richtung, in Rust und TS gespiegelt.

### Platzierung per Fläche

Neuer Befehl `commands::place_light(p, kind, hit: SurfaceHit) -> Vec<Op>`:
`SurfaceHit { pos_mm: [i32;3], normal: [f32;3], furniture: Option<Id> }`.
- Spot: Ausrichtung = Normale (Decke → tilt 0, Boden → tilt 180, Wand → tilt 90, heading = Normale im Plan).
- Treffer auf Möbel → `attached_to` = Möbel, `pos_mm` als lokaler Offset (wie bestehende Möbel-Lichter).
- Position 30 mm von der Fläche weg (Lichtquelle nicht in der Geometrie).

### Vorlagen

- «LED-Streifen»: Grundform `Plate` 12 mm hoch, 10 mm tief, Länge gezogen; dazu `Surface { Down }`,
  900 lm/m, 3000 K. Beim Setzen an Wand/Decke: Seite = zur Fläche weg (Wand → `Front`, Decke → `Down`,
  Unterseite Hängeschrank → `Down`, Boden → `Up`).
- «Uplight»: Spot, tilt 180, 60°, 600 lm.

## 2 · Darstellung (`web/src/view3d`)

### Live (`Lights.tsx`)

- Spot: Ziel-Objekt aus `light_dir` statt fest `[0,-1,0]`.
- Surface: das Möbel-Mesh bekommt `emissive` = Lichtfarbe, `emissiveIntensity` aus Lumen/Fläche (begrenzt,
  damit es nicht ausfrisst); bei `Down/Up/Front` nur die entsprechende Seite (Grundformen: Material pro Seite;
  GLB: ganzes Modell). Ersatzlicht ohne Schatten:
  - `Down/Up/Front` → `RectAreaLight` auf dieser Seite, Grösse der Seite, Intensität aus Lumen (wie `Area`).
  - `All` → `PointLight` in der Mitte, kein Schatten.
- Ersatzlichter zählen nicht ins Schatten-Budget.

### Foto (`PathTrace.tsx`)

- Surface: echte emissive Fläche, Ersatzlicht aus (sonst doppelt hell).
- Spot: Richtung wie live.

## 3 · Lichtmodus «Licht» (`web/src/view3d/LightMode.tsx`, neu)

Tab in der Topbar zwischen «Einrichten» und «Begehen» (`mode === 'light'`).

- **Ansicht** wie `RoomMode`: Raum im Fokus, Orbit, Wände zur Kamera werden zu Stummeln, ◀ ▶ Raumwechsel.
  Beleuchtung wie `ViewMode` (Sonne, Himmel, Lampen, Belichtung, Qualitätsstufe). Möbel nicht auswählbar
  und nicht verschiebbar; nur Lichter (inkl. Surface über Klick aufs leuchtende Objekt).
- **Panel-Tabs**:
  1. *Setzen*: Deckenleuchte · Spot · Uplight · Leuchtfläche · LED-Streifen · «Objekt leuchten lassen».
     Klick auf beliebige Fläche (Decke, Wand, Boden, Möbel) → `place_light` mit Treffer-Normale.
     Shift = mehrere. LED-Streifen: ziehen (Start → Ende auf der Fläche). «Objekt leuchten lassen»:
     Möbel anklicken → Seite wählen → Surface-Licht.
  2. *Eigenschaften*: bestehendes `LightProps` erweitert um Neigung/Richtung (Spot) und Seite (Surface).
     Im 3D: Zielpunkt-Griff am gewählten Spot, ziehen = ausrichten (ein Undo-Schritt).
  3. *Stimmung*: Datum, Uhrzeit (Schnellwahl Morgen 8:00 / Mittag 12:00 / Abend 19:00 / Nacht 22:00),
     Himmel, Belichtung, Lichtszenen, «Foto rendern». Wiederverwendet aus `ViewMode` (Teile in eigene
     Komponenten herausziehen, damit beide Modi sie teilen).
- `ViewMode`-Lampenliste: Hinweis «Lampen setzt man im Lichtmodus» mit Link.
- `RoomMode` Lampen-Setzen bleibt (nur Decke).

## 4 · KI-Agent (`server/src/ai.rs`, `ai_agent.rs`)

- `add_primitive` / `set_furniture`: `glow: {face: all|down|up|front, lumen, kelvin}` erzeugt/ändert das
  angehängte Surface-Licht; `glow: null` entfernt es.
- `add_light` / `set_light`: `light_type` + `uplight`, `area`; neu `tilt_deg`, `heading_deg`, `angle_deg`,
  `attach_to` (Möbel-Id; dann x/y/z lokal).
- Raum-Überblick für den Agenten: Lampen mit `type`, `tilt`, `heading`, `attached_to`, Surface mit `face`.
- Systemprompt: kurzer Abschnitt Lichtplanung (LED ~900 lm/m, 2700 K Wohnen, Uplight für Indirektes,
  Lampe aus Grundformen: Zylinder-Sockel + Stab + Zylinder-Schirm mit `glow: all`).

## 5 · Tests

- `plan-core`: altes JSON (Spot ohne neue Felder) lädt; `light_dir` für Decke/Boden/Wand und mit gedrehtem
  Möbel; `place_light` setzt Ausrichtung und `attached_to`; Surface ohne Objekt → Validierungsfehler;
  `delete_furniture` löscht Surface-Licht.
- Server: Aktionen `glow`, `glow: null`, `uplight`, `attach_to` → erwartete Ops.
- Web: `tsc`, Build; Playwright auf Dev-Szene im Lichtmodus (Uplight, LED-Streifen, leuchtender Zylinder),
  Screenshot live + Foto.

## Reihenfolge

1. Datenmodell + Spot-Richtung + Surface-Darstellung (live + Foto).
2. Lichtmodus mit Setzen/Ausrichten/Stimmung.
3. KI-Agent.
