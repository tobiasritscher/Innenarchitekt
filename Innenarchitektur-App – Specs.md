# Innenarchitektur-App – Specs

Oct 11, 2026 · @Tobias Ritscher

## Vision und Prinzipien

Eine Web-App, die aus einem Grundriss-Foto in wenigen Minuten ein editierbares 3D-Modell einer Wohnung macht. Darin platziert man Möbel und begeht die Wohnung bei Tageslicht und Kunstlicht. Zielgruppe sind Privatpersonen; der MVP hat genau einen Nutzer und läuft self-hosted.

Der Unterschied zu Homestyler und Co.: volle Kontrolle beim Nutzer, schnelles Onboarding, KI nur gezielt und auf Abruf.

1. **Nutzer im Driver Seat.** Die KI ändert nie still etwas. Jede KI-Aktion erzeugt einen sichtbaren Vorschlag (Diff), den man annimmt, anpasst oder verwirft.
2. **KI auf Abruf.** Keine ungefragten Vorschläge im UI. KI ist ein Werkzeug, das man bewusst aufruft: Grundriss erkennen, Möbel importieren, später Befehle in natürlicher Sprache.
3. **Daten statt Meshes.** Die Wohnung ist ein parametrisches Modell (Wände, Öffnungen, Räume). Das 3D-Mesh wird immer daraus abgeleitet.
4. **Schnell zum ersten Erfolg.** Ziel: unter 5 Minuten vom Foto zur begehbaren Wohnung, inklusive Korrektur.
5. **Alles ist rückgängig machbar.** Unbegrenztes Undo/Redo, auch für angenommene KI-Vorschläge.

## MVP-Scope und Phasen

Der MVP deckt den ganzen Weg ab: Grundriss rein, korrigieren, einrichten, begehen. Möbel kommen im MVP aus IKEA-Links und Basisformen; Foto-zu-3D folgt in Phase 2.

| Phase | Inhalt | Nicht drin |
| --- | --- | --- |
| 1 – MVP | Grundriss-Import (Foto/PDF) mit KI und Korrektur-Ansicht · Modelliermodus (Wände, Öffnungen, Räume, Masse) · Möbel per IKEA-Link, Basisform oder GLB-Upload · Ansichtsmodus mit Begehung, Tageslicht nach Ort und Zeit, einfachen Lampen · Materialien manuell · Autosave · ein Nutzer | Foto-zu-3D, Materialien aus Fotos, Pathtracing, Login |
| 2 – KI-Ausbau | Möbel aus Foto (TRELLIS.2 lokal) · Materialien aus Wohnungsfotos · KI-Befehle in natürlicher Sprache · Lichtszenen · pathtraced Standbilder | Mehrere Nutzer |
| 3 – Öffnen | Login und mehrere Nutzer · Teilen per Link (nur Ansicht) · Export (GLB, Grundriss-PDF) · Mobile-Begehung | – |

## Kern-User-Flows

Der wichtigste Flow ist der Grundriss-Import. Die Korrektur-Ansicht entscheidet, ob das Onboarding gelingt.

**A – Neues Projekt aus Grundriss**

1. Projekt anlegen: Name, optional Adresse (für den Sonnenstand).
2. Grundriss hochladen: JPG, PNG oder PDF, auch ein schiefes Handyfoto.
3. Zuschneiden und entzerren: vier Ecken ziehen, falls das Foto perspektivisch verzerrt ist.
4. KI-Analyse mit Fortschrittsanzeige, Ziel unter 60 Sekunden.
5. Korrektur-Ansicht: erkanntes Modell halbtransparent über dem Originalbild. Unsichere Elemente sind markiert.
6. Referenzmass bestätigen: KI schlägt eines aus einer Masslinie vor; sonst zwei Punkte klicken und die Länge eintippen.
7. Nordpfeil setzen (KI schlägt vor, falls auf dem Plan vorhanden).
8. Übernehmen, weiter im Modelliermodus.

**B – Einrichten**

1. Möbel-Panel öffnen.
2. IKEA-Link einfügen, Basisform wählen oder GLB hochladen.
3. Im Grundriss oder in 3D platzieren; das Möbel rastet an Wänden ein.
4. Drehen, Masse anpassen, Abstände zu Wänden ablesen.

**C – Begehen**

1. In den Ansichtsmodus wechseln, Startpunkt im Grundriss antippen.
2. Mit Tastatur/Maus oder Touch durch die Wohnung gehen.
3. Datum und Uhrzeit per Slider ändern, Lampen ein- und ausschalten.

**Ohne Grundriss:** leeres Projekt, Räume von Hand zeichnen.

## Datenmodell

Die Wohnung ist ein Graph: Wände verbinden Knoten, darum verschiebt das Ziehen einer Ecke alle angeschlossenen Wände mit. Alle Längen sind ganzzahlige Millimeter, damit keine Rundungsfehler entstehen.

```rust
struct Project {
    id: Uuid,
    name: String,
    location: Option<GeoLocation>,   // für Sonnenstand
    north_angle_deg: f32,            // Plan-Norden vs. geografischer Norden
    floors: Vec<Floor>,              // MVP: genau ein Stockwerk
    furniture: Vec<FurnitureInstance>,
    lights: Vec<Light>,
    version: u64,
}

struct Floor {
    ceiling_height_mm: u32,          // Default 2500
    nodes: Vec<Node>,                // { id, x_mm: i32, y_mm: i32 }
    walls: Vec<Wall>,
    openings: Vec<Opening>,
    rooms: Vec<RoomMeta>,
    reference_image: Option<RefImage>, // Original-Grundriss + Transformation
}

struct Wall {
    id: WallId, a: NodeId, b: NodeId,
    thickness_mm: u32,
    kind: WallKind,                  // Aussen | Innen
    finish_left: MaterialRef, finish_right: MaterialRef,
}

struct Opening {
    id: OpeningId, wall: WallId,
    kind: OpeningKind,               // Door { hinge, swing } | Window { sill_mm } | Passage
    offset_mm: u32,                  // ab Wandanfang a
    width_mm: u32, height_mm: u32,
}

struct RoomMeta {
    id: RoomId,
    anchor: (i32, i32),              // Punkt im Raum, siehe unten
    name: String,
    floor_material: MaterialRef, ceiling_material: MaterialRef,
}

struct FurnitureInstance {
    id: Uuid, asset: AssetId,
    pos_mm: [i32; 3], rotation_deg: f32,
    dims_mm: [u32; 3],               // Zielmasse, Mesh wird darauf skaliert
}

struct Light {
    id: Uuid, kind: LightKind,       // Point | Spot { angle } | Area { w, h }
    pos_mm: [i32; 3], lumen: u32, color_temp_k: u16, on: bool,
    attached_to: Option<Uuid>,       // z. B. Stehlampe aus IKEA-Import
}
```

**Räume werden abgeleitet, nicht gespeichert.** Die Raumpolygone entstehen aus dem Wandgraph (geschlossene Flächen). Name und Materialien hängen an einem Ankerpunkt: der Raum ist die Fläche, die diesen Punkt enthält. So überleben Raumdaten jede Wandänderung.

**Änderungen sind Operationen.** Jede Änderung ist eine Operation wie `MoveNode`, `AddOpening` oder `SetWallThickness`, mit einer Umkehr-Operation. Daraus ergeben sich Undo/Redo, Autosave als Operations-Batches und KI-Vorschläge als Diff in einem Mechanismus.

## Modelliermodus

Bearbeitet wird primär im 2D-Grundriss, daneben eine 3D-Vorschau (Orbit), die live mitläuft. Auf schmalen Bildschirmen schaltet man zwischen beiden um. Masse tippt man direkt ein, wie in einem CAD: Während des Ziehens eine Zahl eingeben, Enter, fertig.

| Werkzeug | Funktion |
| --- | --- |
| Auswahl | Wand, Öffnung, Raum, Möbel oder Lampe wählen; Eigenschaften im Seitenpanel |
| Wand zeichnen | Klick-Klick-Kette, schliesst automatisch an bestehende Knoten an |
| Ecke verschieben | Knoten ziehen, angeschlossene Wände folgen |
| Wand verschieben | Wand parallel versetzen, Nachbarwände werden verlängert oder gekürzt |
| Masskette | Auf ein Mass klicken und neuen Wert eingeben; man wählt, welche Seite fix bleibt |
| Öffnung | Tür, Fenster oder Durchgang auf eine Wand setzen; Breite, Höhe, Brüstung, Anschlag |
| Raum | Name und Boden-/Deckenmaterial setzen; Fläche in m² wird angezeigt |
| Material | Wandseite, Boden oder Decke mit Farbe oder Material aus Bibliothek belegen |
| Referenzbild | Original-Grundriss als Overlay ein/aus, Deckkraft einstellbar |

**Snapping:** Raster 10 mm, Winkel in 15°-Schritten, rechtwinklig zu bestehenden Wänden, Knoten und Wandmitten. Mit gedrückter Taste deaktivierbar.

**Validierung (nicht blockierend):** Öffnung breiter als Wand, überlappende Öffnungen, Wände ohne Anschluss, offene Räume. Probleme erscheinen als Hinweis-Liste, nicht als Fehlerdialog.

## Ansichtsmodus

Tageslicht ist im MVP physikalisch plausibel, Kunstlicht in Echtzeit mit Schatten-Budget. Fotorealistische Standbilder per Pathtracing kommen in Phase 2.

**Navigation**

- Ego-Perspektive: WASD + Maus am Desktop, virtueller Joystick auf Touch. Augenhöhe Default 1,65 m, einstellbar.
- Kollision mit Wänden und Möbeln; Türöffnungen sind begehbar.
- Puppenhaus-Ansicht: Orbit von oben, Decke ausgeblendet.
- Teleport: Punkt im Mini-Grundriss antippen.

**Tageslicht**

- Sonnenstand aus Standort, Datum und Uhrzeit (NOAA-Algorithmus im Rust-Kern), gedreht um die Nordausrichtung des Plans.
- Physikalischer Himmel, der zur Sonne passt; Regler für bewölkt/klar über Himmelsintensität.
- Slider für Tageszeit, Datumsauswahl, Schnellwahl "Sommer-/Wintersonnenwende".
- Fenster lassen Licht und Schatten in den Raum fallen, das ist der eigentliche Nutzen.

**Kunstlicht**

- Lichtquellen als Punkt, Spot oder Fläche; Einheiten Lumen und Kelvin, damit Werte vom Leuchtmittel-Karton direkt passen.
- Lampen können an Möbel gekoppelt sein (Stehlampe aus IKEA-Import bringt ihre Lichtquelle mit).
- Schatten-Budget: maximal 4 schattenwerfende Lichter gleichzeitig, der Rest ohne Schatten. Budget im Qualitätsschalter anpassbar.

**Rendering**

- three.js WebGPURenderer mit PBR-Materialien, Tone Mapping (AgX), Ambient Occlusion; Fallback WebGL2.
- Qualitätsschalter: Leistung / Ausgewogen / Schön.
- Screenshot-Export als PNG.
- Phase 2: progressiver Pathtracer für ein fotorealistisches Standbild auf Knopfdruck.

## Möbel

IKEA-Import per Link ist die Hauptquelle im MVP: echte Masse, passende Low-Poly-Modelle, kein GPU-Aufwand. Basisformen und GLB-Upload decken den Rest ab.

**IKEA-Import (MVP)**

1. Produkt-URL ins Möbel-Panel einfügen.
2. Backend lädt die Produktseite und extrahiert Artikelnummer, Name, Masse, Preis, Produktbild und das GLB-Modell, das IKEA für "In 3D ansehen" ausliefert.
3. Asset landet in der lokalen Bibliothek (einmal laden, danach gecacht).
4. Plausibilitätscheck: Bounding Box des Modells gegen die Produktmasse; bei mehr als 3 % Abweichung Warnung.

Einschränkungen: Die Schnittstelle ist inoffiziell und kann jederzeit brechen. Der Parser ist darum ein isoliertes Modul mit Fallback auf manuellen GLB-Upload. Nicht jedes Produkt hat ein 3D-Modell. Die Modelle bleiben geistiges Eigentum von IKEA: für private Nutzung unproblematisch, für eine öffentliche App vorher rechtlich klären (siehe Risiken).

**Basisformen (MVP)**

Parametrische Platzhalter mit Breite × Tiefe × Höhe: Box, Zylinder, Platte sowie einfache Typen Sofa, Bett, Tisch, Stuhl, Regal, Schrank. Generiert im Rust-Kern, keine Assets nötig.

**Eigenes GLB (MVP)**

Upload mit Angabe der Zielmasse; das Modell wird darauf skaliert.

**Foto zu 3D (Phase 2)**

1. Foto hochladen, Hintergrund wird entfernt, Nutzer prüft den Ausschnitt.
2. Masse eingeben (Pflicht, das Modell kennt die Grösse nicht).
3. TRELLIS.2 generiert das Mesh lokal auf dem Mac.
4. Nachbearbeitung: Mesh auf rund 50'000 Dreiecke reduzieren, Texturen backen, auf Masse skalieren.
5. Review in 3D, dann in die Bibliothek.

**Platzierung**

- Rastet mit der Rückseite an Wänden ein, steht auf dem Boden.
- Rotation im 90°-Raster oder frei.
- Abstände zu den nächsten Wänden werden beim Ziehen eingeblendet ("passt der Schrank?").
- Überschneidungen werden markiert, aber nicht verhindert.

## KI-Integration

Alle LLM-Aufrufe laufen über den eigenen LiteLLM-Endpunkt (OpenAI-kompatibel), Default-Modell Opus 5.5, per Config austauschbar. Die KI liefert immer strukturierte Daten per Tool Use oder JSON Schema, nie direkte Änderungen. Das Backend validiert die Antwort und übersetzt sie in eine Operations-Liste; das Frontend zeigt sie als Vorschlag.

**Grundriss-Pipeline (MVP)**

LLMs sind stark bei Semantik, aber nicht pixelgenau. Darum ein Hybrid: Opus versteht den Plan, klassische Bildverarbeitung liefert die exakte Geometrie.

1. Vorverarbeitung (Rust): Entzerren, Kontrast, Binarisierung, auf maximal 2048 px skalieren.
2. Linienerkennung (Rust, klassische CV): Wandkandidaten als Liniensegmente mit Dicke, pixelgenau.
3. Semantik (Opus Vision): Räume mit Namen, Türen mit Anschlagrichtung, Fenster, Masstexte samt Position, Nordpfeil, Massstab. Dazu eine grobe Wandgeometrie.
4. Fusion: Opus-Wände werden auf die nächsten CV-Linien gesnappt; Wände ohne CV-Treffer erhalten tiefe Konfidenz.
5. Skalierung: aus mehreren erkannten Massen den Faktor schätzen (Median), Ausreisser melden.
6. Bereinigung: fast rechte Winkel auf 90° ziehen, nahe Knoten verschmelzen, Wandstärken auf übliche Werte runden.
7. Ergebnis: Operations-Liste plus Konfidenz pro Element für die Korrektur-Ansicht.

**Qualität messen:** Ein Testset aus 20 bis 30 echten Grundrissen (Mietinserate, eigene Pläne) mit von Hand korrigierter Wahrheit. Jede Pipeline-Änderung läuft dagegen; Metrik ist der Anteil Wände innerhalb ±5 cm und die Zahl nötiger Korrekturen.

**Weitere KI-Funktionen**

| Funktion | Phase | Eingabe → Ausgabe |
| --- | --- | --- |
| IKEA-Fallback | 1 | Produktseite → Masse und Name, falls der Parser scheitert |
| Befehle in natürlicher Sprache | 2 | "Wohnzimmer 50 cm breiter" → Operations-Liste als Diff |
| Materialien aus Fotos | 2 | Wohnungsfotos → Material-Vorschläge pro Raum und Fläche |
| Rückfrage auf Abruf | 2 | "Ist der Durchgang breit genug?" → Antwort mit Bezug auf das Modell |

Für die Befehle bekommt Opus die Operationen als Tools (`move_wall`, `add_opening`, `place_furniture` usw.) und den aktuellen Modellzustand als kompaktes JSON.

## Architektur und Tech-Stack

Herzstück ist ein Rust-Crate `plan-core`, das im Backend nativ und im Browser als WebAssembly läuft. Geometrie, Operationen und Validierung existieren so genau einmal; das Frontend ist nur Darstellung und Interaktion.

&#91;embedded content: Architektur · Browser, Mac mini, externe Dienste\]

Das Frontend rechnet Meshes lokal (keine Latenz beim Ziehen von Wänden) und schickt nur Operationen ans Backend. Der gestrichelte Worker kommt erst in Phase 2.

| Bereich | Wahl | Warum |
| --- | --- | --- |
| Frontend | React, TypeScript, Vite | Grösstes Ökosystem für Editor-UIs |
| 3D | three.js mit React Three Fiber, WebGPURenderer | Reif, WebGPU mit WebGL2-Fallback |
| 2D-Editor | SVG in React | Präzise Hit-Tests, für Wohnungsgrössen schnell genug |
| Gemeinsamer Kern | Rust-Crate `plan-core`, wasm-bindgen; TS-Typen per `ts-rs` generiert | Eine Quelle der Wahrheit, keine doppelte Geometrie-Logik |
| Backend | axum, tokio, sqlx | Standard-Stack, async |
| Datenbank | Postgres: Snapshot als JSONB plus Operations-Log | Schnelles Laden, nachvollziehbare Historie |
| Jobs | Postgres-Tabelle mit `FOR UPDATE SKIP LOCKED` | Keine zusätzliche Infrastruktur |
| Bildverarbeitung | `image` und `imageproc` (Rust) | Kein Python im MVP |
| LLM | OpenAI-kompatibler Client gegen LiteLLM | Modell per Config austauschbar |
| Worker (Phase 2) | Python, PyTorch MPS, TRELLIS.2 | Die Modelle gibt es nur in Python |

## API-Skizze

REST mit JSON, lange Aufgaben als Jobs mit Server-Sent Events für den Fortschritt. Speichern läuft über Operations-Batches mit Versionsnummer; bei veralteter Version antwortet der Server mit 409.

| Methode | Pfad | Zweck |
| --- | --- | --- |
| GET / POST | `/api/projects` | Projekte auflisten / anlegen |
| GET | `/api/projects/{id}` | Projekt-Snapshot laden |
| POST | `/api/projects/{id}/ops` | Operations-Batch mit `base_version` anwenden |
| POST | `/api/projects/{id}/floorplan` | Grundriss hochladen, startet Analyse-Job |
| GET | `/api/jobs/{id}` | Job-Status und Ergebnis (Vorschlag als Operations-Liste) |
| GET | `/api/jobs/{id}/events` | Fortschritt als SSE-Stream |
| POST | `/api/assets/ikea` | IKEA-Import aus `{ url }` |
| POST | `/api/assets/upload` | Eigenes GLB mit Zielmassen |
| POST | `/api/assets/primitive` | Basisform anlegen |
| GET | `/api/assets`, `/api/assets/{id}/model.glb` | Bibliothek, Modell-Datei |
| POST | `/api/assets/from-photo` | Foto zu 3D (Phase 2) |
| POST | `/api/projects/{id}/ai/command` | Befehl in natürlicher Sprache (Phase 2) |

Auth im MVP: ein statisches Token, App nur über Tailscale erreichbar (siehe Deployment).

## Deployment

Für den MVP reicht der Mac mini locker: Opus läuft über LiteLLM in der Cloud, lokal braucht es nur Backend, Datenbank und Dateien. Erst Foto-zu-3D in Phase 2 braucht echte Rechenleistung.

**MVP auf dem Mac mini**

- Rust-Backend als einzelnes Binary, gestartet über launchd.
- Postgres via Homebrew; Assets und Uploads im Dateisystem hinter einer S3-kompatiblen Abstraktion (`object_store`-Crate), damit später MinIO oder S3 möglich ist.
- Frontend als statische Dateien, vom Backend ausgeliefert.
- Zugriff nur über Tailscale, mit HTTPS-Zertifikat von Tailscale. Wichtig: WebGPU läuft nur in einem sicheren Kontext (HTTPS oder localhost).
- Backup: nächtlich `pg_dump` plus Asset-Ordner, z. B. via Time Machine.

**Phase 2: Foto zu 3D**

- TRELLIS.2 gibt es als Community-Port für Apple Silicon ([trellis-mac](https://github.com/shivampkumar/trellis-mac)): rund 3,5 Minuten pro Modell auf einem M4 Pro, mindestens 24 GB Unified Memory und rund 15 GB für die Gewichte ([Quelle](https://korben.info/en/trellis-mac-microsoft-image-to-3d-apple-silicon.html)).
- Ein Mac mini mit 16 GB reicht nicht; ab 24 GB geht es, ein Mac Studio ist deutlich angenehmer.
- Der Python-Worker muss nativ laufen, nicht in Docker: Docker auf macOS hat keinen Zugriff auf die GPU (Metal).
- Worker hinter einer schmalen Schnittstelle (Job rein, GLB raus), damit eine gehostete API ihn ersetzen kann, falls der Port nicht mehr gepflegt wird.

## Risiken und offene Fragen

Das grösste Risiko ist die Qualität der Grundriss-Erkennung; die Korrektur-Ansicht und das Testset sind deshalb Teil des MVP, nicht Kür.

| Risiko | Auswirkung | Gegenmassnahme |
| --- | --- | --- |
| Grundriss-Erkennung ungenau | Onboarding dauert lange, Frust | Korrektur-Ansicht mit Konfidenz, Testset, manuelles Zeichnen als Fallback |
| IKEA ändert die Website oder sperrt Zugriffe | Import fällt aus | Parser isoliert, Assets lokal gecacht, GLB-Upload als Fallback |
| IKEA-Modelle sind IKEA-Eigentum | Kein Problem privat, heikel bei öffentlicher App | Vor Phase 3 rechtlich klären; dann kein geteilter Katalog |
| TRELLIS-Mac-Port wird nicht gepflegt | Foto zu 3D fällt aus | Austauschbarer Worker, gehostete API als Alternative |
| Performance im Browser bei vielen Möbeln und Lampen | Ruckelnde Begehung | Schatten-Budget, Instancing, Detailstufen, Qualitätsschalter |

**Offene Fragen**

- [ ] Maisonette oder mehrere Stockwerke schon im MVP?
- [ ] Dachschrägen unterstützen? Braucht geneigte Decken im Datenmodell.
- [ ] Welche Genauigkeit ist das Ziel, z. B. ±2 cm nach Korrektur?
- [ ] Frontend mit React einverstanden, oder lieber Svelte/Solid?
- [ ] Braucht es Export (GLB, PDF) früher als Phase 3?
- [ ] Name der App.
