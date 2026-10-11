# Grundriss-Testset

Qualitätsmessung der Grundriss-Pipeline (Spec «Qualität messen»): Jede Pipeline-Änderung läuft gegen dieses Set.
Metrik: Anteil Wände innerhalb ±5 cm (beide Endpunkte) und Zahl nötiger Korrekturen
(fehlende + überzählige + falsch platzierte Wände und Öffnungen).

## Ausführen

```sh
# synthetische Fälle (synth_*) neu erzeugen und alles auswerten
cargo run --release -p innenarchitekt --bin floorplan_eval -- --generate

# nur CV (LLM_* ignorieren), nur bestimmte Fälle, Overlays + Vorschläge als JSON schreiben
cargo run --release -p innenarchitekt --bin floorplan_eval -- --no-llm --case korridor --overlay /tmp/fp-eval

# beliebiges Bild ohne Wahrheit durchlaufen lassen (Zusammenfassung + Overlay)
cargo run --release -p innenarchitekt --bin floorplan_eval -- --image plan.jpg --overlay /tmp/fp-eval
```

Mit gesetztem `LLM_BASE_URL` + `LLM_API_KEY` (optional `LLM_MODEL`, `LLM_IMAGE_MAX_PX`) läuft die KI-Stufe mit.
Overlay-Farben: Wände rot, Knoten grün, Türen/Durchgänge orange, Fenster blau.

## Aufbau eines Falls

```
server/testset/<fall>/
  plan.png | plan.jpg     Grundriss (wie ihn der Nutzer hochladen würde)
  truth.json              von Hand korrigierte Wahrheit
  corners.json            optional: [[x,y],[x,y],[x,y],[x,y]] Perspektiv-Ecken (TL, TR, BR, BL) in Bildpixeln
```

`truth.json` ist ein `plan_core::Floor` (gleiches JSON wie im Projekt-Snapshot, alle Längen in mm), alternativ ein
ganzes `Project` (dann zählt `floors[0]`). Massgebend sind `nodes`, `walls` (Mittellinien, Stärke) und `openings`;
Räume werden aus den Wänden abgeleitet.

- `reference_image` (empfohlen): `mm_per_px` = wahrer Massstab **des Originalbilds** und `origin_mm` = Planposition
  des Pixels (0,0). Damit kennt die Auswertung Massstab und Lage exakt (die Pipeline schneidet Ränder ab und skaliert;
  das wird herausgerechnet) und meldet zusätzlich den Massstabsfehler der Pipeline («Mst.%»).
  Einfachster Weg: Plan in der App als Referenzbild importieren, Wände sauber nachzeichnen/korrigieren,
  Stockwerk exportieren → das ist bereits das richtige Format.
- Ohne `reference_image` wird über Bounding-Box + Knotenzuordnung ausgerichtet (Massstabsfehler dann «–»).

## Inhalt

- `synth_*`: synthetisch erzeugt (`--generate`), damit das Tool sofort läuft. Wände gefüllt, Türen mit Blatt und
  Bogen, Fenster als Linien, Pseudo-Text, Möbel, Masslinien.
- Echte Pläne gehören hierher (Ziel 20–30 Fälle): Grundrisse aus Mietinseraten, eigene Pläne, Scans/Fotos.
  Nur Bilder einchecken, deren Lizenz das erlaubt – sonst lokal halten (z. B. `.gitignore`).
