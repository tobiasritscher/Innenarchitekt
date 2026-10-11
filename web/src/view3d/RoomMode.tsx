// Room mode «Einrichten»: full-screen 3D of one room. The camera orbits the room, walls between camera
// and room drop to a stub, only furniture and lamps are edited (drag, R to rotate, Del to delete).
import { useEffect, useMemo, useRef, useState, type PointerEvent as RPointerEvent } from 'react'
import { Canvas } from '@react-three/fiber'
import { PCFShadowMap } from 'three'
import { useFootprints, useRooms } from '../core/derived'
import { fmtArea } from '../core/format'
import type { DerivedRoom } from '../core/generated/DerivedRoom'
import { pointInPolygon } from '../editor2d/geom'
import { deleteOps } from '../editor2d/model'
import { PropertiesPanel } from '../editor2d/PropertiesPanel'
import { usePlanSource } from '../editor2d/source'
import { useEditor } from '../state/editor'
import { useProject } from '../state/project'
import { RendererSetup, cssVar } from './common'
import { FurniturePanel } from './FurniturePanel'
import { EditScene, LAMP_LABEL, PlacingHint, recentlyPlaced } from './Preview3D'
import { RulerButton } from './Ruler3D'
import { useView3D, type LampType } from './store'

/** Focus id of a room: its RoomMeta id, else the derived key. */
const roomId = (r: DerivedRoom) => r.meta ?? r.key

export function RoomMode() {
  const rooms = useRooms()
  const project = useProject((s) => s.project)
  const focus = useEditor((s) => s.room)
  const placing = useEditor((s) => s.placingAsset)
  const selection = useEditor((s) => s.selection)
  const source = usePlanSource()
  const bg = useMemo(() => cssVar('--plan-bg', '#fbfaf8'), [])
  const [tab, setTab] = useState<'furniture' | 'props'>('furniture')

  const names = useMemo(() => new Map((project?.floors[0].rooms ?? []).map((m) => [m.id, m.name])), [project])
  const nameOf = (r: DerivedRoom) => (r.meta && names.get(r.meta)) || 'Raum ohne Namen'
  const list = useMemo(() => [...rooms].sort((a, b) => Number(!a.meta) - Number(!b.meta) || b.area_m2 - a.area_m2), [rooms])
  const room = rooms.find((r) => roomId(r) === focus) ?? null

  // No (or a vanished) focus: the selected room, else the largest one.
  useEffect(() => {
    if (room || !rooms.length) return
    const sel = useEditor.getState().selection
    const picked = sel?.kind === 'room' ? rooms.find((r) => roomId(r) === sel.id || `derived:${r.key}` === sel.id) : undefined
    const largest = rooms.reduce((a, b) => (b.area_m2 > a.area_m2 ? b : a))
    useEditor.getState().setRoom(roomId(picked ?? largest))
  }, [room, rooms])

  // Only furniture and lamps are edited here.
  useEffect(() => {
    if (selection && selection.kind !== 'furniture' && selection.kind !== 'light') useEditor.getState().select(null)
    else if (selection) setTab('props')
  }, [selection])

  const step = (d: number) => {
    if (!list.length) return
    const i = room ? list.indexOf(room) : -1
    const next = list[(i + d + list.length) % list.length]
    useEditor.getState().select(null)
    useEditor.getState().setRoom(roomId(next))
  }

  const placingLight = useView3D((s) => s.placingLight)
  const toggleLamp = (t: LampType) => {
    const on = placingLight !== t
    if (on) useEditor.getState().setPlacingAsset(null)
    useView3D.getState().setPlacingLight(on ? t : null)
  }
  // an asset placement started from the library ends lamp placement
  useEffect(() => {
    if (!placing) return
    useView3D.getState().setPlacingLight(null)
    useView3D.getState().setRuler(false)
  }, [placing])
  useEffect(() => () => useView3D.getState().setPlacingLight(null), [])

  // keys: R/⇧R rotate the selected furniture, Del deletes, Esc clears the selection
  const keyRef = useRef<(e: KeyboardEvent) => void>(() => {})
  keyRef.current = (e) => {
    const t = e.target as HTMLElement
    if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT')) return
    if (e.metaKey || e.ctrlKey || useEditor.getState().placingAsset) return
    const sel = useEditor.getState().selection
    if (e.key === 'Escape') useEditor.getState().select(null)
    else if ((e.key === 'Delete' || e.key === 'Backspace') && sel && source) {
      const r = deleteOps(source, sel)
      if (r && r[0].length && source.commit(r[0], r[1])) useEditor.getState().select(null)
      e.preventDefault()
    } else if ((e.key === 'r' || e.key === 'R') && sel?.kind === 'furniture') {
      const f = project?.furniture.find((x) => x.id === sel.id)
      if (!f) return
      const rot = Math.round((((f.rotation_deg + (e.shiftKey ? 15 : 90)) % 360) + 360) % 360)
      useProject.getState().commit([{ op: 'PutFurniture', item: { ...f, rotation_deg: rot } }], 'Möbel drehen')
    }
  }
  useEffect(() => {
    const down = (e: KeyboardEvent) => keyRef.current(e)
    window.addEventListener('keydown', down)
    return () => window.removeEventListener('keydown', down)
  }, [])

  if (!rooms.length) {
    return (
      <div className="v3d-view v3d-room">
        <div className="splash">Noch keine geschlossenen Räume. Zuerst im Modelliermodus Wände zeichnen.</div>
      </div>
    )
  }

  const editable = selection && (selection.kind === 'furniture' || selection.kind === 'light')
  return (
    <div className={`v3d-view v3d-room ${placing ? 'v3d-placing' : ''}`}>
      <Canvas
        frameloop="demand"
        shadows={{ enabled: true, type: PCFShadowMap }}
        dpr={[1, 1.5]}
        camera={{ fov: 45, near: 0.05, far: 500, position: [8, 10, 12] }}
        gl={{ antialias: true }}
        onPointerMissed={(e) => {
          if (e.button === 0 && !useEditor.getState().placingAsset && !recentlyPlaced()) useEditor.getState().select(null)
        }}
      >
        <color attach="background" args={[bg]} />
        <RendererSetup />
        {room && <EditScene room={room} />}
      </Canvas>

      <div className="v3d-overlay v3d-preview-tools">
        <div className="v3d-seg v3d-lamp-tools" role="group" aria-label="Lampe platzieren">
          {(Object.keys(LAMP_LABEL) as LampType[]).map((t) => (
            <button key={t} aria-pressed={placingLight === t} onClick={() => toggleLamp(t)} title={`${LAMP_LABEL[t]} an die Decke setzen: anklicken, dann Stelle an der Decke klicken (Shift = mehrere)`}>
              + {LAMP_LABEL[t]}
            </button>
          ))}
        </div>
        <button onClick={() => useView3D.getState().requestRoomFit()} title="Kamera wieder auf den ganzen Raum">
          Ansicht zurücksetzen
        </button>
        <RulerButton hotkey />
      </div>

      <div className="v3d-panel v3d-room-panel">
        <div className="v3d-panel-head">
          <button className="ghost" onClick={() => step(-1)} title="Vorheriger Raum" disabled={list.length < 2}>
            ◀
          </button>
          <select className="v3d-room-select" value={room ? roomId(room) : ''} onChange={(e) => (useEditor.getState().select(null), useEditor.getState().setRoom(e.target.value))}>
            {list.map((r) => (
              <option key={r.key} value={roomId(r)}>
                {nameOf(r)} · {fmtArea(r.area_m2)}
              </option>
            ))}
          </select>
          <button className="ghost" onClick={() => step(1)} title="Nächster Raum" disabled={list.length < 2}>
            ▶
          </button>
        </div>
        <div className="v3d-seg v3d-room-tabs" role="tablist">
          <button role="tab" aria-selected={tab === 'furniture'} onClick={() => setTab('furniture')}>
            Möbel
          </button>
          <button role="tab" aria-selected={tab === 'props'} onClick={() => setTab('props')}>
            Auswahl
          </button>
        </div>
        <div className="v3d-panel-body">
          {tab === 'furniture' && <FurniturePanel />}
          {tab === 'props' &&
            (editable ? (
              <PropertiesPanel />
            ) : (
              <p className="muted v3d-small">Nichts ausgewählt. Möbel oder Lampe im Raum anklicken.</p>
            ))}
        </div>
      </div>

      <RoomPicker rooms={rooms} focus={room} />
      {placing ? (
        <PlacingHint assetId={placing} />
      ) : placingLight ? (
        <div className="v3d-overlay v3d-hint">{LAMP_LABEL[placingLight]} platzieren: Stelle an der Decke klicken · Shift = weitere · Esc = abbrechen</div>
      ) : (
        <div className="v3d-overlay v3d-hint">Ziehen dreht um den Raum · Scrollen zoomt · Möbel ziehen verschiebt · R dreht · Entf löscht · D misst</div>
      )}
    </div>
  )
}

/** Mini floor plan: the focused room highlighted, tap another room to switch. */
function RoomPicker({ rooms, focus }: { rooms: DerivedRoom[]; focus: DerivedRoom | null }) {
  const fps = useFootprints()
  const svg = useRef<SVGSVGElement>(null)
  const vb = useMemo(() => {
    let minX = Infinity
    let minY = Infinity
    let maxX = -Infinity
    let maxY = -Infinity
    for (const f of fps)
      for (const [x, y] of f.corners) {
        minX = Math.min(minX, x)
        minY = Math.min(minY, y)
        maxX = Math.max(maxX, x)
        maxY = Math.max(maxY, y)
      }
    const pad = 600
    return isFinite(minX) ? [minX - pad, minY - pad, maxX - minX + 2 * pad, maxY - minY + 2 * pad] : [-3000, -3000, 6000, 6000]
  }, [fps])
  const pick = (e: RPointerEvent<SVGSVGElement>) => {
    const m = svg.current?.getScreenCTM()
    if (!m) return
    const p = new DOMPoint(e.clientX, e.clientY).matrixTransform(m.inverse())
    const r = rooms.find((x) => pointInPolygon([p.x, p.y], x.polygon as [number, number][]))
    if (!r || r === focus) return
    useEditor.getState().select(null)
    useEditor.getState().setRoom(roomId(r))
  }
  return (
    <div className="v3d-minimap v3d-roompicker" title="Raum antippen, um ihn einzurichten">
      <svg ref={svg} viewBox={vb.join(' ')} preserveAspectRatio="xMidYMid meet" onPointerUp={pick}>
        {rooms.map((r) => (
          <polygon key={r.key} points={r.polygon.map((p) => p.join(',')).join(' ')} className={`v3d-mm-room ${r === focus ? 'active' : ''}`} />
        ))}
        {fps.map((f) => (
          <polygon key={f.wall} points={f.corners.map((p) => p.join(',')).join(' ')} className="v3d-mm-wall" />
        ))}
      </svg>
    </div>
  )
}
