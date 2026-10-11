// Tool bar of the 2D editor (left) with options for the active tool.
import { useState } from 'react'
import type { Op } from '../core/generated/Op'
import { LengthField, MaterialPicker } from '../ui/fields'
import { useEditor, type Tool } from '../state/editor'
import { icons } from './icons'
import { usePlanSource } from './source'

const TOOLS: { tool: Tool; label: string; key: string }[] = [
  { tool: 'select', label: 'Auswahl', key: 'V' },
  { tool: 'wall', label: 'Wand zeichnen', key: 'W' },
  { tool: 'measure', label: 'Masskette', key: 'M' },
  { tool: 'opening', label: 'Öffnung', key: 'O' },
  { tool: 'room', label: 'Raum', key: 'R' },
  { tool: 'material', label: 'Material', key: 'P' },
  { tool: 'light', label: 'Lampe', key: 'L' },
]

export function Toolbar() {
  const tool = useEditor((s) => s.tool)
  const setTool = useEditor((s) => s.setTool)
  const snapping = useEditor((s) => s.snapping)
  const setSnapping = useEditor((s) => s.setSnapping)
  const showRef = useEditor((s) => s.showRefImage)
  const setShowRef = useEditor((s) => s.setShowRefImage)
  const placingAsset = useEditor((s) => s.placingAsset)
  const requestFit = useEditor((s) => s.requestFit)
  const [refOpen, setRefOpen] = useState(false)

  return (
    <div className="toolbar2d" role="toolbar" aria-label="Werkzeuge">
      {TOOLS.map((t) => (
        <button key={t.tool} className="tool-btn ghost" aria-pressed={tool === t.tool} title={`${t.label} (${t.key})`} onClick={() => setTool(t.tool)}>
          {icons[t.tool as keyof typeof icons]}
          <span className="tool-label">{t.label}</span>
        </button>
      ))}
      {placingAsset && (
        <button className="tool-btn ghost" aria-pressed title="Möbel platzieren (Esc = abbrechen)" onClick={() => useEditor.getState().setPlacingAsset(null)}>
          {icons.furniture}
          <span className="tool-label">Möbel</span>
        </button>
      )}
      <div className="tool-sep" />
      <button className="tool-btn ghost" aria-pressed={refOpen} title="Referenzbild" onClick={() => setRefOpen(!refOpen)}>
        {icons.refimage}
        <span className="tool-label">Referenz</span>
      </button>
      <button className="tool-btn ghost" aria-pressed={snapping} title="Fang (Raster, Winkel, Knoten) – Alt gedrückt halten = aus" onClick={() => setSnapping(!snapping)}>
        {icons.snap}
        <span className="tool-label">Fang</span>
      </button>
      <button className="tool-btn ghost" title="Zoom auf alles (Pos1)" onClick={requestFit}>
        {icons.fit}
        <span className="tool-label">Alles</span>
      </button>

      <ToolOptions tool={tool} />
      {refOpen && <RefImageOptions show={showRef} setShow={setShowRef} onClose={() => setRefOpen(false)} />}
    </div>
  )
}

function ToolOptions({ tool }: { tool: Tool }) {
  const draw = useEditor((s) => s.draw)
  const setDraw = useEditor((s) => s.setDraw)
  if (tool === 'wall')
    return (
      <div className="tool-options card">
        <strong>Wand</strong>
        <LengthField label="Wandstärke" valueMm={draw.wallThicknessMm} onCommit={(mm) => setDraw({ wallThicknessMm: Math.max(20, Math.min(1000, mm)) })} />
        <div className="seg">
          <button aria-pressed={!draw.wallExterior} onClick={() => setDraw({ wallExterior: false })}>
            Innen
          </button>
          <button aria-pressed={draw.wallExterior} onClick={() => setDraw({ wallExterior: true })}>
            Aussen
          </button>
        </div>
      </div>
    )
  if (tool === 'opening') {
    const k = draw.openingKind
    return (
      <div className="tool-options card">
        <strong>Öffnung</strong>
        <div className="seg">
          {(['Door', 'Window', 'Passage'] as const).map((x) => (
            <button key={x} aria-pressed={k === x} onClick={() => setDraw({ openingKind: x })}>
              {x === 'Door' ? 'Tür' : x === 'Window' ? 'Fenster' : 'Durchgang'}
            </button>
          ))}
        </div>
        {k === 'Window' ? (
          <>
            <LengthField label="Breite" valueMm={draw.windowWidthMm} onCommit={(v) => setDraw({ windowWidthMm: v })} />
            <LengthField label="Höhe" valueMm={draw.windowHeightMm} onCommit={(v) => setDraw({ windowHeightMm: v })} />
            <LengthField label="Brüstung" valueMm={draw.windowSillMm} onCommit={(v) => setDraw({ windowSillMm: v })} />
          </>
        ) : (
          <>
            <LengthField label="Breite" valueMm={draw.doorWidthMm} onCommit={(v) => setDraw({ doorWidthMm: v })} />
            <LengthField label="Höhe" valueMm={draw.doorHeightMm} onCommit={(v) => setDraw({ doorHeightMm: v })} />
          </>
        )}
        {k === 'Door' && <p className="muted small">F = Anschlag wechseln</p>}
      </div>
    )
  }
  if (tool === 'material')
    return (
      <div className="tool-options card">
        <strong>Material</strong>
        <MaterialPicker label="Auftragen" value={draw.material} onChange={(m) => setDraw({ material: m })} />
        <p className="muted small">Wandseite oder Boden anklicken, Shift = Decke</p>
      </div>
    )
  return null
}

function RefImageOptions({ show, setShow, onClose }: { show: boolean; setShow: (v: boolean) => void; onClose: () => void }) {
  const source = usePlanSource()
  const floor = source?.project.floors[0]
  const img = floor?.reference_image
  const setFloorOp = (opacity: number, visible: boolean): Op[] =>
    floor && img
      ? [{ op: 'SetFloor', floor: floor.id, name: floor.name, ceiling_height_mm: floor.ceiling_height_mm, reference_image: { ...img, opacity, visible } }]
      : []
  return (
    <div className="tool-options card ref-options">
      <div className="row">
        <strong>Referenzbild</strong>
        <button className="ghost small" onClick={onClose} title="Schliessen" style={{ flex: 'none' }}>
          ×
        </button>
      </div>
      {!img ? (
        <p className="muted small">Kein Referenzbild. Über «Grundriss-Import» hinzufügen.</p>
      ) : (
        <>
          <label className="check">
            <input type="checkbox" checked={show && img.visible} onChange={(e) => {
              setShow(e.target.checked)
              if (e.target.checked && !img.visible) source!.commit(setFloorOp(img.opacity, true), 'Referenzbild einblenden')
            }} />
            Anzeigen
          </label>
          <label>
            Deckkraft {Math.round(img.opacity * 100)} %
            <input
              type="range"
              min={0.05}
              max={1}
              step={0.05}
              value={img.opacity}
              onChange={(e) => source!.setTransient(setFloorOp(parseFloat(e.target.value), img.visible))}
              onPointerUp={() => source!.endTransient(true, 'Referenzbild-Deckkraft')}
              onKeyUp={() => source!.endTransient(true, 'Referenzbild-Deckkraft')}
            />
          </label>
        </>
      )}
    </div>
  )
}
