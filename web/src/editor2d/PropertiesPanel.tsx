// Properties of the current selection (sidebar tab «Eigenschaften»). Every change = one commit.
import { useEffect, useRef, useState, type ReactNode } from 'react'
import type { FurnitureInstance } from '../core/generated/FurnitureInstance'
import type { Light } from '../core/generated/Light'
import type { LightKind } from '../core/generated/LightKind'
import type { Op } from '../core/generated/Op'
import type { Opening } from '../core/generated/Opening'
import type { OpeningKind } from '../core/generated/OpeningKind'
import type { RoomMeta } from '../core/generated/RoomMeta'
import type { Wall } from '../core/generated/Wall'
import { fmtArea, fmtLength, fmtNumber } from '../core/format'
import { useOverlaps } from '../core/derived'
import { useAssets } from '../state/assets'
import { useEditor } from '../state/editor'
import { LengthField, MaterialPicker, NumberField } from '../ui/fields'
import { dist } from './geom'
import { deleteOps, flipDoor, newRoomMeta, nodeMap, wallPoints } from './model'
import { useDerivedOf, usePlanSource, type PlanSource } from './source'

export function PropertiesPanel() {
  const source = usePlanSource()
  const selection = useEditor((s) => s.selection)
  const derived = useDerivedOf(source)
  if (!source) return null
  const p = source.project
  const f = p.floors[0]
  if (!selection) return <NothingSelected />

  const del = () => {
    const r = deleteOps(source, selection)
    if (r && r[0].length && source.commit(r[0], r[1])) useEditor.getState().select(null)
  }

  switch (selection.kind) {
    case 'wall': {
      const w = f.walls.find((x) => x.id === selection.id)
      return w ? <WallProps key={w.id} src={source} wall={w} onDelete={del} /> : <Gone />
    }
    case 'opening': {
      const o = f.openings.find((x) => x.id === selection.id)
      return o ? <OpeningProps src={source} o={o} onDelete={del} /> : <Gone />
    }
    case 'room': {
      if (selection.id.startsWith('derived:')) {
        const r = derived.rooms.find((x) => `derived:${x.key}` === selection.id)
        if (!r) return <Gone />
        return (
          <Section title="Raum (ohne Namen)">
            <p>Fläche: {fmtArea(r.area_m2)}</p>
            <button
              className="primary"
              onClick={() => {
                const meta = newRoomMeta(r.label, f.rooms.length + 1)
                if (source.commit([{ op: 'PutRoom', floor: f.id, room: meta }], 'Raum benennen')) {
                  useEditor.getState().select({ kind: 'room', id: meta.id })
                  useEditor.getState().setFocusField('room-name')
                }
              }}
            >
              Raum benennen
            </button>
          </Section>
        )
      }
      const r = f.rooms.find((x) => x.id === selection.id)
      if (!r) return <Gone />
      const area = derived.rooms.find((x) => x.meta === r.id)?.area_m2
      return <RoomProps src={source} r={r} area={area} onDelete={del} />
    }
    case 'node': {
      const n = f.nodes.find((x) => x.id === selection.id)
      if (!n) return <Gone />
      const put = (x: number, y: number) => source.commit([{ op: 'PutNode', floor: f.id, node: { ...n, x_mm: Math.round(x), y_mm: Math.round(y) } }], 'Ecke verschieben')
      return (
        <Section title="Ecke" onDelete={del} deleteLabel="Ecke löschen">
          <div className="row">
            <NumberField label="x" suffix="mm" value={n.x_mm} onCommit={(v) => put(v, n.y_mm)} />
            <NumberField label="y" suffix="mm" value={n.y_mm} onCommit={(v) => put(n.x_mm, v)} />
          </div>
          <p className="muted small">Wände an {f.walls.filter((w) => w.a === n.id || w.b === n.id).length} Seite(n). Löschen einer Ecke mit zwei Wänden verbindet diese.</p>
        </Section>
      )
    }
    case 'furniture': {
      const it = p.furniture.find((x) => x.id === selection.id)
      return it ? <FurnitureProps src={source} it={it} onDelete={del} /> : <Gone />
    }
    case 'light': {
      const l = p.lights.find((x) => x.id === selection.id)
      return l ? <LightProps src={source} l={l} onDelete={del} /> : <Gone />
    }
  }
}

function Gone() {
  return <p className="muted">Element existiert nicht mehr.</p>
}

function NothingSelected() {
  return (
    <div className="props-help">
      <p className="muted">Nichts ausgewählt. Element im Plan anklicken, um es zu bearbeiten.</p>
      <h3>Werkzeuge</h3>
      <ul className="hint-list">
        <li><kbd>V</kbd> Auswahl – ziehen verschiebt Ecken, Wände, Öffnungen, Möbel</li>
        <li><kbd>W</kbd> Wand zeichnen – Zahl tippen + Enter für exakte Länge</li>
        <li><kbd>M</kbd> Masskette – Mass anklicken, neue Länge eingeben</li>
        <li><kbd>O</kbd> Öffnung – Tür, Fenster, Durchgang auf eine Wand</li>
        <li><kbd>R</kbd> Raum – in einen Raum klicken, um ihn zu benennen</li>
        <li><kbd>P</kbd> Material – Wandseite oder Boden anklicken</li>
        <li><kbd>L</kbd> Lampe – Deckenleuchte setzen</li>
      </ul>
      <h3>Navigation</h3>
      <ul className="hint-list">
        <li>Mausrad / Pinch = Zoom, Leertaste + Ziehen / Mitteltaste = verschieben</li>
        <li><kbd>Pos1</kbd> Zoom auf alles · <kbd>Esc</kbd> abbrechen · <kbd>Entf</kbd> löschen</li>
        <li><kbd>⌘Z</kbd> rückgängig · <kbd>⇧⌘Z</kbd> wiederholen</li>
      </ul>
    </div>
  )
}

function Section({ title, children, onDelete, deleteLabel = 'Löschen' }: { title: ReactNode; children: ReactNode; onDelete?: () => void; deleteLabel?: string }) {
  return (
    <div className="form props">
      <h2>{title}</h2>
      {children}
      {onDelete && (
        <button className="danger" onClick={onDelete}>
          {deleteLabel}
        </button>
      )}
    </div>
  )
}

function Seg<T extends string>({ value, options, onChange }: { value: T; options: [T, string][]; onChange: (v: T) => void }) {
  return (
    <div className="seg">
      {options.map(([v, l]) => (
        <button key={v} aria-pressed={value === v} onClick={() => value !== v && onChange(v)}>
          {l}
        </button>
      ))}
    </div>
  )
}

function TextField({ label, value, onCommit, autoFocus }: { label: ReactNode; value: string; onCommit: (v: string) => void; autoFocus?: boolean }) {
  const [text, setText] = useState(value)
  const ref = useRef<HTMLInputElement>(null)
  useEffect(() => setText(value), [value])
  useEffect(() => {
    if (autoFocus) {
      ref.current?.focus()
      ref.current?.select()
    }
  }, [autoFocus])
  const commit = () => {
    const v = text.trim()
    if (v && v !== value) onCommit(v)
    else setText(value)
  }
  return (
    <label className="field">
      {label}
      <input
        ref={ref}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === 'Enter') (e.target as HTMLInputElement).blur()
          if (e.key === 'Escape') setText(value)
        }}
      />
    </label>
  )
}

// ------------------------------------------------------------------ wall

function WallProps({ src, wall, onDelete }: { src: PlanSource; wall: Wall; onDelete: () => void }) {
  const f = src.project.floors[0]
  const [fixedA, setFixedA] = useState(true)
  const pts = wallPoints(nodeMap(f), wall)
  const len = pts ? dist(pts[0], pts[1]) : 0
  const put = (w: Wall, label: string) => src.commit([{ op: 'PutWall', floor: f.id, wall: w }], label)
  const openings = f.openings.filter((o) => o.wall === wall.id)
  return (
    <Section title={wall.kind === 'Exterior' ? 'Aussenwand' : 'Innenwand'} onDelete={onDelete} deleteLabel="Wand löschen">
      <LengthField
        label="Länge"
        valueMm={Math.round(len)}
        onCommit={(mm) => {
          try {
            src.commit(src.engine.cmd_set_wall_length(f.id, wall.id, mm, fixedA) as Op[], 'Wandlänge ändern')
          } catch (e) {
            useEditor.getState().showToast(String(e))
          }
        }}
      />
      <div className="field">
        Fixes Ende
        <Seg value={fixedA ? 'a' : 'b'} options={[['a', 'Anfang A'], ['b', 'Ende B']]} onChange={(v) => setFixedA(v === 'a')} />
      </div>
      <div className="row">
        <LengthField label="Stärke" valueMm={wall.thickness_mm} onCommit={(mm) => put({ ...wall, thickness_mm: Math.max(20, mm) }, 'Wandstärke ändern')} />
        <LengthField
          label="Höhe"
          valueMm={Math.min(wall.height_mm ?? f.ceiling_height_mm, f.ceiling_height_mm)}
          onCommit={(mm) => put({ ...wall, height_mm: mm >= f.ceiling_height_mm ? null : Math.max(50, mm) }, 'Wandhöhe ändern')}
        />
      </div>
      <div className="field">
        Art
        <Seg value={wall.kind} options={[['Interior', 'Innen'], ['Exterior', 'Aussen']]} onChange={(k) => put({ ...wall, kind: k }, 'Wandart ändern')} />
      </div>
      <div className="field">
        Ausführung
        <Seg value={wall.glazed ? 'glass' : 'solid'} options={[['solid', 'Massiv'], ['glass', 'Glas']]} onChange={(v) => put({ ...wall, glazed: v === 'glass' }, v === 'glass' ? 'Glaswand' : 'Massivwand')} />
      </div>
      <MaterialPicker label="Oberfläche links" value={wall.finish_left} onChange={(m) => put({ ...wall, finish_left: m }, 'Wandmaterial links')} />
      <MaterialPicker label="Oberfläche rechts" value={wall.finish_right} onChange={(m) => put({ ...wall, finish_right: m }, 'Wandmaterial rechts')} />
      <p className="muted small">Höhe bis zur Decke = raumhoch; tiefer z. B. für Gartenmauer oder Brüstung. Links/rechts = in Zeichenrichtung von A nach B gesehen. {openings.length > 0 && `${openings.length} Öffnung(en).`}</p>
      <div className="row">
        <button
          onClick={() => {
            if (!pts) return
            const m = [Math.round((pts[0][0] + pts[1][0]) / 2), Math.round((pts[0][1] + pts[1][1]) / 2)]
            src.commit(src.engine.cmd_split_wall(f.id, wall.id, m[0], m[1]) as Op[], 'Wand teilen')
          }}
        >
          In der Mitte teilen
        </button>
      </div>
    </Section>
  )
}

// ------------------------------------------------------------------ opening

const OPENING_NAMES: Record<OpeningKind['type'], string> = { Door: 'Tür', Window: 'Fenster', Passage: 'Durchgang' }

function OpeningProps({ src, o, onDelete }: { src: PlanSource; o: Opening; onDelete: () => void }) {
  const f = src.project.floors[0]
  const put = (n: Opening, label: string) => src.commit([{ op: 'PutOpening', floor: f.id, opening: n }], label)
  const setKind = (t: OpeningKind['type']) => {
    const kind: OpeningKind = t === 'Door' ? { type: 'Door', hinge: 'Start', swing: 'Left', glass: false } : t === 'Window' ? { type: 'Window', sill_mm: 850 } : { type: 'Passage' }
    const height = t === 'Window' ? Math.min(o.height_mm, 1400) : Math.max(o.height_mm, 2000)
    put({ ...o, kind, height_mm: height }, 'Öffnungsart ändern')
  }
  return (
    <Section title={OPENING_NAMES[o.kind.type]} onDelete={onDelete} deleteLabel="Öffnung löschen">
      <Seg value={o.kind.type} options={[['Door', 'Tür'], ['Window', 'Fenster'], ['Passage', 'Durchgang']]} onChange={setKind} />
      <div className="row">
        <LengthField label="Breite" valueMm={o.width_mm} onCommit={(v) => put({ ...o, width_mm: v }, 'Öffnungsbreite')} />
        <LengthField label="Höhe" valueMm={o.height_mm} onCommit={(v) => put({ ...o, height_mm: v }, 'Öffnungshöhe')} />
      </div>
      <div className="row">
        <NumberField label="Abstand ab A" suffix="mm" min={0} value={o.offset_mm} onCommit={(v) => put({ ...o, offset_mm: Math.round(v) }, 'Öffnung verschieben')} />
        {o.kind.type === 'Window' && (
          <NumberField label="Brüstung" suffix="mm" min={0} value={o.kind.sill_mm} onCommit={(v) => put({ ...o, kind: { type: 'Window', sill_mm: Math.round(v) } }, 'Brüstungshöhe')} />
        )}
      </div>
      {o.kind.type === 'Door' && (
        <>
          <div className="field">
            Band (Anschlag)
            <Seg value={o.kind.hinge} options={[['Start', 'bei A'], ['End', 'bei B']]} onChange={(h) => o.kind.type === 'Door' && put({ ...o, kind: { ...o.kind, hinge: h } }, 'Türanschlag ändern')} />
          </div>
          <div className="field">
            Öffnet nach
            <Seg value={o.kind.swing} options={[['Left', 'links'], ['Right', 'rechts']]} onChange={(s) => o.kind.type === 'Door' && put({ ...o, kind: { ...o.kind, swing: s } }, 'Türanschlag ändern')} />
          </div>
          <button onClick={() => put(flipDoor(o)!, 'Türanschlag ändern')}>Anschlag wechseln (F)</button>
          <div className="field">
            Ausführung
            <Seg value={o.kind.glass ? 'glass' : 'solid'} options={[['solid', 'Vollblatt'], ['glass', 'Glas']]} onChange={(v) => o.kind.type === 'Door' && put({ ...o, kind: { ...o.kind, glass: v === 'glass' } }, v === 'glass' ? 'Glastür' : 'Vollblatt-Tür')} />
          </div>
        </>
      )}
    </Section>
  )
}

// ------------------------------------------------------------------ room

function RoomProps({ src, r, area, onDelete }: { src: PlanSource; r: RoomMeta; area: number | undefined; onDelete: () => void }) {
  const f = src.project.floors[0]
  const focusField = useEditor((s) => s.focusField)
  const setFocusField = useEditor((s) => s.setFocusField)
  const focus = focusField === 'room-name'
  useEffect(() => {
    if (focus) setFocusField(null)
  }, [focus, setFocusField])
  const put = (n: RoomMeta, label: string) => src.commit([{ op: 'PutRoom', floor: f.id, room: n }], label)
  return (
    <Section title="Raum" onDelete={onDelete} deleteLabel="Raumdaten löschen">
      <TextField key={r.id} label="Name" value={r.name} autoFocus={focus} onCommit={(name) => put({ ...r, name }, 'Raum umbenennen')} />
      <p>{area !== undefined ? `Fläche: ${fmtArea(area)}` : <span className="warning-text">Raum ist nicht geschlossen</span>}</p>
      {area !== undefined && (
        <button
          onClick={() => {
            useEditor.getState().setRoom(r.id)
            useEditor.getState().select(null)
            useEditor.getState().setMode('room')
          }}
          title="Raum in 3D möblieren und beleuchten"
        >
          In 3D einrichten
        </button>
      )}
      <MaterialPicker label="Boden" value={r.floor_material} onChange={(m) => put({ ...r, floor_material: m }, 'Bodenmaterial')} />
      <div className="field">
        Decke
        <Seg value={r.open_ceiling ? 'open' : 'closed'} options={[['closed', 'Vorhanden'], ['open', 'Keine (offen)']]} onChange={(v) => put({ ...r, open_ceiling: v === 'open' }, v === 'open' ? 'Decke entfernen' : 'Decke hinzufügen')} />
      </div>
      {!r.open_ceiling && <MaterialPicker label="Deckenmaterial" value={r.ceiling_material} onChange={(m) => put({ ...r, ceiling_material: m }, 'Deckenmaterial')} />}
    </Section>
  )
}

// ------------------------------------------------------------------ furniture

function FurnitureProps({ src, it, onDelete }: { src: PlanSource; it: FurnitureInstance; onDelete: () => void }) {
  const put = (n: FurnitureInstance, label: string) => src.commit([{ op: 'PutFurniture', item: n }], label)
  const dims = (i: number, v: number) => {
    const d = [...it.dims_mm] as [number, number, number]
    d[i] = Math.max(1, Math.round(v))
    // Attached lights (local offsets) scale along with the furniture, e.g. the bulb of a floor lamp.
    const k = d[i] / Math.max(1, it.dims_mm[i])
    const lightOps: Op[] = src.project.lights
      .filter((l) => l.attached_to === it.id)
      .map((l) => {
        const pos = [...l.pos_mm] as [number, number, number]
        pos[i] = Math.round(pos[i] * k)
        return { op: 'PutLight', light: { ...l, pos_mm: pos } }
      })
    src.commit([{ op: 'PutFurniture', item: { ...it, dims_mm: d } }, ...lightOps], 'Möbelmasse ändern')
  }
  const lights = src.project.lights.filter((l) => l.attached_to === it.id)
  return (
    <Section title="Möbel" onDelete={onDelete} deleteLabel="Möbel löschen">
      <TextField key={it.id} label="Name" value={it.name} onCommit={(name) => put({ ...it, name }, 'Möbel umbenennen')} />
      <div className="row">
        <LengthField label="Breite" valueMm={it.dims_mm[0]} onCommit={(v) => dims(0, v)} />
        <LengthField label="Tiefe" valueMm={it.dims_mm[1]} onCommit={(v) => dims(1, v)} />
        <LengthField label="Höhe" valueMm={it.dims_mm[2]} onCommit={(v) => dims(2, v)} />
      </div>
      <div className="row">
        <NumberField label="Drehung" suffix="°" value={Math.round(it.rotation_deg * 10) / 10} onCommit={(v) => put({ ...it, rotation_deg: ((v % 360) + 360) % 360 }, 'Möbel drehen')} />
        <NumberField label="Höhe ab Boden" suffix="mm" min={0} value={it.pos_mm[2]} onCommit={(v) => put({ ...it, pos_mm: [it.pos_mm[0], it.pos_mm[1], Math.round(v)] }, 'Möbel anheben')} />
      </div>
      <div className="row">
        <button onClick={() => put({ ...it, rotation_deg: (it.rotation_deg + 270) % 360 }, 'Möbel drehen')}>↺ 90°</button>
        <button onClick={() => put({ ...it, rotation_deg: (it.rotation_deg + 90) % 360 }, 'Möbel drehen')}>↻ 90°</button>
      </div>
      <MaterialPicker label="Farbe/Material" value={it.material ?? 'paint:white'} onChange={(m) => put({ ...it, material: m }, 'Möbelmaterial')} />
      {it.material && (
        <button className="link small" onClick={() => put({ ...it, material: null }, 'Möbelmaterial zurücksetzen')}>
          Material zurücksetzen
        </button>
      )}
      <OverlapOk src={src} it={it} />
      <Footprint src={src} it={it} />
      {lights.length > 0 && (
        <>
          <h3>Lichtquellen</h3>
          <ul className="plain-list">
            {lights.map((l) => (
              <li key={l.id}>
                <button className="link" onClick={() => useEditor.getState().select({ kind: 'light', id: l.id })}>
                  {l.name} · {fmtNumber(l.lumen)} lm · {l.on ? 'an' : 'aus'}
                </button>
              </li>
            ))}
          </ul>
        </>
      )}
      <p className="muted small">Ziehen verschiebt (rastet an Wänden ein), R/⇧R dreht 90°/15°.</p>
    </Section>
  )
}

/** «Darf überlappen»: no red overlap mark against other furniture (chairs under a table); optionally for all copies. */
function OverlapOk({ src, it }: { src: PlanSource; it: FurnitureInstance }) {
  const overlaps = useOverlaps()
  const same = src.project.furniture.filter((f) => f.asset === it.asset)
  const differ = same.filter((f) => f.overlap_ok !== it.overlap_ok)
  const flagged = overlaps.includes(it.id)
  return (
    <div className="field">
      <label className="check">
        <input type="checkbox" checked={it.overlap_ok} onChange={(e) => src.commit([{ op: 'PutFurniture', item: { ...it, overlap_ok: e.target.checked } }], e.target.checked ? 'Überlappen erlauben' : 'Überlappen melden')} />
        Darf andere Möbel überlappen
      </label>
      <span className="muted small">z. B. Stühle unter dem Tisch – Wände zählen weiterhin.</span>
      {flagged && !it.overlap_ok && <span className="warning-text small">Überschneidet sich gerade (rot markiert).</span>}
      {differ.length > 0 && (
        <button className="link small" onClick={() => src.commit(differ.map((f) => ({ op: 'PutFurniture' as const, item: { ...f, overlap_ok: it.overlap_ok } })), 'Überlappen für gleiche Möbel')}>
          Für alle {same.length} «{it.name}» übernehmen
        </button>
      )}
    </div>
  )
}

/** Shape from the model (L-wardrobe: only the arms block) and «begehbar» (never blocks walking). */
function Footprint({ src, it }: { src: PlanSource; it: FurnitureInstance }) {
  const model = useAssets((s) => s.assets.find((a) => a.id === it.asset)?.footprint)
  const stale = model != null && JSON.stringify(model) !== JSON.stringify(it.footprint)
  const same = src.project.furniture.filter((f) => f.asset === it.asset)
  return (
    <div className="field">
      <label className="check">
        <input type="checkbox" checked={it.passable} onChange={(e) => src.commit([{ op: 'PutFurniture', item: { ...it, passable: e.target.checked } }], e.target.checked ? 'Möbel begehbar' : 'Möbel blockiert')} />
        Beim Begehen durchlaufbar
      </label>
      <span className="muted small">
        {it.footprint.length ? `Kollision nach Modellform (${it.footprint.length} Teile).` : 'Kollision mit der ganzen Box.'}
      </span>
      {stale && model && (
        <button
          className="link small"
          onClick={() =>
            src.commit(
              same.map((f) => ({ op: 'PutFurniture' as const, item: { ...f, footprint: model } })),
              'Grundfläche aus Modell',
            )
          }
        >
          {model.length ? `Grundfläche aus Modell übernehmen (${model.length} Teile)` : 'Ganze Box als Grundfläche'}
          {same.length > 1 ? ` – alle ${same.length}` : ''}
        </button>
      )}
    </div>
  )
}

// ------------------------------------------------------------------ light

function LightProps({ src, l, onDelete }: { src: PlanSource; l: Light; onDelete: () => void }) {
  const put = (n: Light, label: string) => src.commit([{ op: 'PutLight', light: n }], label)
  const setKind = (t: LightKind['type']) => {
    const kind: LightKind = t === 'Point' ? { type: 'Point' } : t === 'Spot' ? { type: 'Spot', angle_deg: 40, tilt_deg: 0, heading_deg: 0 } : { type: 'Area', w_mm: 600, h_mm: 600 }
    put({ ...l, kind }, 'Lichtart ändern')
  }
  const owner = l.attached_to ? src.project.furniture.find((f) => f.id === l.attached_to) : undefined
  return (
    <Section title="Lampe" onDelete={onDelete} deleteLabel="Lampe löschen">
      <TextField key={l.id} label="Name" value={l.name} onCommit={(name) => put({ ...l, name }, 'Lampe umbenennen')} />
      <label className="check">
        <input type="checkbox" checked={l.on} onChange={(e) => put({ ...l, on: e.target.checked }, e.target.checked ? 'Lampe ein' : 'Lampe aus')} />
        Eingeschaltet
      </label>
      <Seg value={l.kind.type} options={[['Point', 'Punkt'], ['Spot', 'Spot'], ['Area', 'Fläche']]} onChange={setKind} />
      {l.kind.type === 'Spot' && (
        <NumberField label="Öffnungswinkel" suffix="°" min={5} max={170} value={l.kind.angle_deg} onCommit={(v) => l.kind.type === 'Spot' && put({ ...l, kind: { ...l.kind, angle_deg: v } }, 'Spotwinkel')} />
      )}
      {l.kind.type === 'Area' && (
        <div className="row">
          <LengthField label="Breite" valueMm={l.kind.w_mm} onCommit={(v) => l.kind.type === 'Area' && put({ ...l, kind: { ...l.kind, w_mm: v } }, 'Leuchtfläche')} />
          <LengthField label="Tiefe" valueMm={l.kind.h_mm} onCommit={(v) => l.kind.type === 'Area' && put({ ...l, kind: { ...l.kind, h_mm: v } }, 'Leuchtfläche')} />
        </div>
      )}
      <div className="row">
        <NumberField label="Lichtstrom" suffix="lm" min={0} step={50} value={l.lumen} onCommit={(v) => put({ ...l, lumen: Math.round(v) }, 'Lichtstrom')} />
        <NumberField label="Farbtemperatur" suffix="K" min={1000} max={10000} step={100} value={l.color_temp_k} onCommit={(v) => put({ ...l, color_temp_k: Math.round(v) }, 'Farbtemperatur')} />
      </div>
      <div className="seg">
        {[2700, 3000, 4000].map((k) => (
          <button key={k} aria-pressed={l.color_temp_k === k} onClick={() => put({ ...l, color_temp_k: k }, 'Farbtemperatur')}>
            {fmtNumber(k)} K
          </button>
        ))}
      </div>
      <NumberField label={owner ? 'Höhe über Möbel-Unterkante' : 'Höhe über Boden'} suffix="mm" min={0} value={l.pos_mm[2]} onCommit={(v) => put({ ...l, pos_mm: [l.pos_mm[0], l.pos_mm[1], Math.round(v)] }, 'Lampenhöhe')} />
      {owner && (
        <p className="muted small">
          Gehört zu{' '}
          <button className="link" onClick={() => useEditor.getState().select({ kind: 'furniture', id: owner.id })}>
            {owner.name}
          </button>{' '}
          (bewegt sich mit).
        </p>
      )}
      {!owner && <p className="muted small">Position: {fmtLength(l.pos_mm[0])} / {fmtLength(l.pos_mm[1])}</p>}
    </Section>
  )
}
