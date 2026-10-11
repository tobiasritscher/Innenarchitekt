// Light mode «Licht»: one room in focus (like «Einrichten») with the real light of «Begehen» (sun, sky, lamps).
// Lamps go on any surface (ceiling, wall, floor, furniture), LED strips are dragged, objects can be made to glow.
// Furniture is not edited here.
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Canvas, useThree, type ThreeEvent } from '@react-three/fiber'
import { EffectComposer, N8AO, ToneMapping } from '@react-three/postprocessing'
import { ToneMappingMode } from 'postprocessing'
import { BufferGeometry, Float32BufferAttribute, LineBasicMaterial, MeshBasicMaterial, PCFShadowMap, Raycaster, SphereGeometry, type WebGLRenderer } from 'three'
import type { OrbitControls as OrbitControlsImpl } from 'three-stdlib'
import { useRooms } from '../core/derived'
import { fmtArea } from '../core/format'
import { newId } from '../core/engine'
import type { DerivedRoom } from '../core/generated/DerivedRoom'
import type { FurnitureInstance } from '../core/generated/FurnitureInstance'
import type { GlowFace } from '../core/generated/GlowFace'
import type { Light } from '../core/generated/Light'
import type { Op } from '../core/generated/Op'
import { pointInPolygon } from '../editor2d/geom'
import { deleteOps } from '../editor2d/model'
import { PropertiesPanel } from '../editor2d/PropertiesPanel'
import { usePlanSource } from '../editor2d/source'
import { useEditor, type Quality } from '../state/editor'
import { useProject } from '../state/project'
import { Building, type PickHandlers } from './Building'
import { useBounds } from './bounds'
import { QUALITY, QUALITY_LABEL, RendererSetup } from './common'
import { Daylight } from './Daylight'
import { DoorsButton, useDoorIds } from './doors'
import { FurnitureLayer, type FurnitureHandlers } from './Furniture'
import { LED_KELVIN, LED_LM_PER_M, aimAt, keepUplight, lightDir, mountFor, stripFromDrag, type V3 } from './lightGeom'
import { LightMarkers, LightRig, usePlacedLights, type LightHandlers, type PlacedLight } from './Lights'
import { LAMP_TEMPLATES, planeHit, plateAssetId, surfaceHit, type HitCtx, type SurfaceHit, type TemplateKey } from './lightTools'
import { DaylightControls, LampList, LightScenes, saveCanvasPng, useLampLux } from './Mood'
import { PathTraceHost, PathTraceOverlay, usePathTrace } from './PathTrace'
import { RoomCamera, lampGhost } from './Preview3D'
import { RoomPicker, roomId } from './RoomMode'
import { useView3D, type LightTool } from './store'

const TOOLS: [LightTool, string, string][] = [
  ['Point', 'Deckenleuchte', 'Boden anklicken = Decke darüber, Wand/Möbel = dort'],
  ['Spot', 'Spot', 'Strahlt von der Fläche weg: Decke nach unten, Wand in den Raum'],
  ['Uplight', 'Uplight', 'Auf Boden oder Möbel: strahlt nach oben (indirektes Licht)'],
  ['Area', 'Leuchtfläche', 'Flaches Panel, immer an der Decke über der angeklickten Stelle'],
  ['Strip', 'LED-Streifen', 'Auf einer Fläche von Anfang bis Ende ziehen'],
  ['Glow', 'Objekt leuchten lassen', 'Möbel anklicken: es wird zur Leuchte'],
]
const HINT: Record<LightTool, string> = {
  Point: 'Deckenleuchte setzen: Boden anklicken = an die Decke darüber, Wand oder Möbel = dort · Shift = weitere · Esc = fertig',
  Spot: 'Spot setzen: Boden = an die Decke, Wand/Möbel = dort, strahlt von der Fläche weg · Shift = weitere · Esc = fertig',
  Uplight: 'Uplight setzen: Boden oder Möbel anklicken, strahlt immer nach oben · Shift = weitere · Esc = fertig',
  Area: 'Leuchtfläche setzen: kommt immer an die Decke über der angeklickten Stelle · Shift = weitere · Esc = fertig',
  Strip: 'LED-Streifen: auf Decke, Wand oder Möbel von Anfang bis Ende ziehen (Alt = Decke) · Esc = fertig',
  Glow: 'Objekt anklicken, das leuchten soll · Esc = fertig',
}
const GLOW_FACES: [GlowFace, string][] = [
  ['All', 'Ganzes Objekt'],
  ['Down', 'Unten'],
  ['Up', 'Oben'],
  ['Front', 'Vorne'],
]
/** Time of the last placement: the click that placed must not also select/deselect what's below. */
let placedAt = 0
const justPlaced = () => performance.now() - placedAt < 400

/** Tools whose floor clicks go to the ceiling above. */
const CEILING_TOOLS: LightTool[] = ['Point', 'Spot', 'Area']

export function LightMode() {
  const rooms = useRooms()
  const project = useProject((s) => s.project)
  const focus = useEditor((s) => s.room)
  const selection = useEditor((s) => s.selection)
  const quality = useEditor((s) => s.view.quality)
  const setView = useEditor((s) => s.setView)
  const source = usePlanSource()
  const tool = useView3D((s) => s.lightTool)
  const glowFace = useView3D((s) => s.glowFace)
  const tracing = usePathTrace((s) => s.active)
  const [tab, setTab] = useState<'place' | 'props' | 'mood'>('place')
  const glRef = useRef<WebGLRenderer | null>(null)
  const q = QUALITY[quality]
  const dpr = Math.min(q.dpr, typeof window !== 'undefined' ? window.devicePixelRatio || 1 : 1)

  const names = useMemo(() => new Map((project?.floors[0].rooms ?? []).map((m) => [m.id, m.name])), [project])
  const nameOf = (r: DerivedRoom) => (r.meta && names.get(r.meta)) || 'Raum ohne Namen'
  const list = useMemo(() => [...rooms].sort((a, b) => Number(!a.meta) - Number(!b.meta) || b.area_m2 - a.area_m2), [rooms])
  const room = rooms.find((r) => roomId(r) === focus) ?? null
  const doorIds = useDoorIds(room?.walls ?? [])

  // No (or a vanished) focus: the largest room.
  useEffect(() => {
    if (room || !rooms.length) return
    const largest = rooms.reduce((a, b) => (b.area_m2 > a.area_m2 ? b : a))
    useEditor.getState().setRoom(roomId(largest))
  }, [room, rooms])

  // Only lamps are edited here.
  useEffect(() => {
    if (selection && selection.kind !== 'light') useEditor.getState().select(null)
    else if (selection) setTab('props')
  }, [selection])

  useEffect(
    () => () => {
      useView3D.getState().setLightTool(null)
      usePathTrace.getState().stop()
    },
    [],
  )

  const step = (d: number) => {
    if (!list.length) return
    const i = room ? list.indexOf(room) : -1
    const next = list[(i + d + list.length) % list.length]
    useEditor.getState().select(null)
    useEditor.getState().setRoom(roomId(next))
  }

  // Esc ends the tool / clears the selection, Del deletes the selected lamp
  const keyRef = useRef<(e: KeyboardEvent) => void>(() => {})
  keyRef.current = (e) => {
    const t = e.target as HTMLElement
    if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT')) return
    if (e.metaKey || e.ctrlKey) return
    const sel = useEditor.getState().selection
    if (e.key === 'Escape') {
      if (useView3D.getState().lightTool) useView3D.getState().setLightTool(null)
      else useEditor.getState().select(null)
    } else if ((e.key === 'Delete' || e.key === 'Backspace') && sel && source) {
      const r = deleteOps(source, sel)
      if (r && r[0].length && source.commit(r[0], r[1])) useEditor.getState().select(null)
      e.preventDefault()
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

  const toggleTool = (t: LightTool) => useView3D.getState().setLightTool(tool === t ? null : t)
  return (
    <div className={`v3d-view v3d-room ${tool ? 'v3d-placing' : ''}`}>
      <Canvas
        shadows={{ enabled: true, type: PCFShadowMap }}
        dpr={dpr}
        gl={{ antialias: true, preserveDrawingBuffer: true }}
        camera={{ fov: 45, near: 0.05, far: 2500, position: [8, 10, 12] }}
        onCreated={({ gl }) => {
          glRef.current = gl
        }}
        onPointerMissed={(e) => {
          if (e.button === 0 && !useView3D.getState().lightTool && !justPlaced()) useEditor.getState().select(null)
        }}
      >
        <RendererSetup postTone={q.ao} />
        {room && <LightScene room={room} />}
        <PathTraceHost />
        {q.ao && !tracing && (
          <EffectComposer multisampling={4}>
            <N8AO aoRadius={0.6} distanceFalloff={0.6} intensity={2.2} halfRes quality="medium" />
            <ToneMapping mode={ToneMappingMode.AGX} />
          </EffectComposer>
        )}
      </Canvas>

      <div className="v3d-overlay v3d-preview-tools">
        <button onClick={() => useView3D.getState().requestRoomFit()} title="Kamera wieder auf den ganzen Raum">
          Ansicht zurücksetzen
        </button>
        <DoorsButton ids={doorIds} />
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
          <button role="tab" aria-selected={tab === 'place'} onClick={() => setTab('place')}>
            Setzen
          </button>
          <button role="tab" aria-selected={tab === 'props'} onClick={() => setTab('props')}>
            Auswahl
          </button>
          <button role="tab" aria-selected={tab === 'mood'} onClick={() => setTab('mood')}>
            Stimmung
          </button>
        </div>
        <div className="v3d-panel-body">
          {tab === 'place' && (
            <>
              <div className="v3d-light-tools">
                {TOOLS.map(([t, label, title]) => (
                  <button key={t} aria-pressed={tool === t} onClick={() => toggleTool(t)} title={title}>
                    + {label}
                  </button>
                ))}
              </div>
              {tool === 'Glow' && (
                <div className="v3d-seg v3d-seg-full" role="group" aria-label="Leuchtende Seite">
                  {GLOW_FACES.map(([f, label]) => (
                    <button key={f} aria-pressed={glowFace === f} onClick={() => useView3D.getState().setGlowFace(f)}>
                      {label}
                    </button>
                  ))}
                </div>
              )}
              <h3>Lampen</h3>
              <LampList />
            </>
          )}
          {tab === 'props' &&
            (selection?.kind === 'light' ? <PropertiesPanel /> : <p className="muted v3d-small">Nichts ausgewählt. Lampe oder leuchtendes Objekt anklicken.</p>)}
          {tab === 'mood' && (
            <>
              <DaylightControls />
              <LightScenes />
              <h3>Darstellung</h3>
              <div className="v3d-seg v3d-seg-full" role="tablist">
                {(Object.keys(QUALITY) as Quality[]).map((k) => (
                  <button key={k} role="tab" aria-selected={quality === k} onClick={() => setView({ quality: k })}>
                    {QUALITY_LABEL[k]}
                  </button>
                ))}
              </div>
              <div className="v3d-row v3d-shots">
                <button className="v3d-shot" onClick={() => saveCanvasPng(glRef.current)} disabled={tracing}>
                  Screenshot (PNG)
                </button>
                <button className="primary v3d-shot" onClick={() => usePathTrace.getState().start()} disabled={tracing} title="Fotorealistisches Standbild mit Pathtracing (dauert je nach Gerät 1–3 Minuten)">
                  Foto rendern
                </button>
              </div>
            </>
          )}
        </div>
      </div>

      <RoomPicker rooms={rooms} focus={room} />
      <PathTraceOverlay onSave={() => saveCanvasPng(glRef.current, 'Foto')} />
      <div className="v3d-overlay v3d-hint">{tool ? HINT[tool] : 'Ziehen dreht um den Raum · Scrollen zoomt · Lampe anklicken = Eigenschaften, ziehen = verschieben · Entf löscht'}</div>
    </div>
  )
}

/** Light, building and furniture of the focused room, plus the tools. */
function LightScene({ room }: { room: DerivedRoom }) {
  const quality = useEditor((s) => s.view.quality)
  const q = QUALITY[quality]
  const bounds = useBounds()
  const lampLux = useLampLux()
  const ctx = useHitCtx()
  const controls = useThree((s) => s.controls) as OrbitControlsImpl | null

  const inside = useCallback((x: number, y: number) => pointInPolygon([x, y], room.polygon as [number, number][]), [room])
  const furnitureIn = useCallback((f: FurnitureInstance) => inside(f.pos_mm[0], f.pos_mm[1]), [inside])
  const lightIn = useCallback((p: PlacedLight) => inside(p.pos.x * 1000, p.pos.z * 1000), [inside])

  /** Doors open/shut on click; anything else clears the selection. */
  const buildingPick: PickHandlers = useMemo(
    () => ({
      onClick: (e, s) => {
        e.stopPropagation()
        if (e.delta > 5 || useView3D.getState().lightTool || justPlaced()) return
        const door = s.kind === 'opening' && useProject.getState().project?.floors[0].openings.some((o) => o.id === s.id && o.kind.type === 'Door')
        if (door) useView3D.getState().toggleDoor(s.id)
        else useEditor.getState().select(null)
      },
    }),
    [],
  )

  /** Furniture: «glow» tool makes it a lamp; a glowing one selects its light. Never moved here. */
  const furniturePick: FurnitureHandlers = useMemo(
    () => ({
      onClick: (e, s) => {
        e.stopPropagation()
        if (e.delta > 5) return
        const st = useProject.getState()
        const p = st.project
        const f = p?.furniture.find((x) => x.id === s.id)
        if (!p || !f) return
        const glow = p.lights.find((l) => l.attached_to === f.id && l.kind.type === 'Surface')
        const tool = useView3D.getState().lightTool
        if (tool === 'Glow') {
          const face = useView3D.getState().glowFace
          const light: Light = glow
            ? { ...glow, kind: { type: 'Surface', face } }
            : { id: newId(), name: `Licht ${f.name}`, kind: { type: 'Surface', face }, pos_mm: [0, 0, Math.round(f.dims_mm[2] / 2)], lumen: 800, color_temp_k: 2700, on: true, attached_to: f.id }
          if (st.commit([{ op: 'PutLight', light }], glow ? 'Leuchtseite ändern' : `${f.name} leuchten lassen`)) {
            useEditor.getState().select({ kind: 'light', id: light.id })
            if (!e.nativeEvent.shiftKey) useView3D.getState().setLightTool(null)
          }
          return
        }
        if (tool || justPlaced()) return
        useEditor.getState().select(glow ? { kind: 'light', id: glow.id } : null)
      },
    }),
    [],
  )

  /** Lamps: click selects, dragging moves a lamp over the surfaces (keeps its aim). */
  const lightPick: LightHandlers = useMemo(
    () => ({
      onClick: (e, s) => {
        e.stopPropagation()
        if (e.delta > 5 || useView3D.getState().lightTool || justPlaced()) return
        useEditor.getState().select(s)
      },
      onDown: (e: ThreeEvent<PointerEvent>, p: PlacedLight) => {
        if (e.button !== 0 || useView3D.getState().lightTool) return
        e.stopPropagation()
        useEditor.getState().select({ kind: 'light', id: p.light.id })
        const l = p.light
        const ceilingFirst = l.kind.type !== 'Spot' || l.kind.tilt_deg < 90
        const sx = e.nativeEvent.clientX
        const sy = e.nativeEvent.clientY
        let moved = false
        if (controls) controls.enabled = false
        const move = (ev: PointerEvent) => {
          if (!moved && Math.hypot(ev.clientX - sx, ev.clientY - sy) < 4) return
          moved = true
          const hit = surfaceHit(ev, ctx(), ceilingFirst !== ev.altKey)
          const h = hit && mountFor(l.kind.type, hit, ctx().ceilingMm, l.kind.type === 'Area' ? Math.max(l.kind.w_mm, l.kind.h_mm) : 0)
          const st = useProject.getState()
          if (!h || !st.engine || !inside(h.at[0], h.at[1])) return
          const ops = st.engine.cmd_place_light(l, h.at[0], h.at[1], h.at[2], h.normal[0], h.normal[1], h.normal[2], h.furniture ?? '') as Op[]
          const placed = (ops[0] as { light: Light }).light
          // a drag moves the lamp, the aim stays (re-aim with the handle)
          st.setTransient([{ op: 'PutLight', light: { ...placed, kind: l.kind } }])
        }
        const up = () => {
          window.removeEventListener('pointermove', move)
          window.removeEventListener('pointerup', up)
          if (controls) controls.enabled = true
          if (moved) useProject.getState().endTransient(true, 'Lampe verschieben')
        }
        window.addEventListener('pointermove', move)
        window.addEventListener('pointerup', up)
      },
    }),
    [controls, ctx, inside],
  )

  return (
    <>
      <Daylight walk={false} bounds={bounds} mapSize={q.sunMap} lampLux={lampLux} />
      <Building ceilings="shadow" focus={room} pick={buildingPick} />
      <FurnitureLayer pick={furniturePick} only={furnitureIn} />
      <LightRig budget={q.budget} mapSize={q.lampMap} only={lightIn} />
      <LightMarkers pick={lightPick} only={lightIn} />
      <AimHandle ctx={ctx} />
      <PlaceTool ctx={ctx} inside={inside} />
      <RoomCamera room={room} />
    </>
  )
}

/** Raycast context of the canvas (fresh camera/scene each call). */
function useHitCtx(): () => HitCtx {
  const gl = useThree((s) => s.gl)
  const camera = useThree((s) => s.camera)
  const scene = useThree((s) => s.scene)
  const raycaster = useMemo(() => new Raycaster(), [])
  return useCallback(() => ({ gl, camera, scene, raycaster, ceilingMm: useProject.getState().project?.floors[0].ceiling_height_mm ?? 2500 }), [gl, camera, scene, raycaster])
}

/** Click (lamps) or drag (LED strip) on the canvas while a light tool is active. */
function PlaceTool({ ctx, inside }: { ctx: () => HitCtx; inside: (x: number, y: number) => boolean }) {
  const tool = useView3D((s) => s.lightTool)
  const gl = useThree((s) => s.gl)
  const controls = useThree((s) => s.controls) as OrbitControlsImpl | null
  const [strip, setStrip] = useState<{ start: SurfaceHit; end: V3 } | null>(null)

  useEffect(() => {
    if (!tool || tool === 'Glow') return
    const el = gl.domElement
    let down: { x: number; y: number; start: SurfaceHit | null; end: V3 | null } | null = null
    /** Strip end on the start surface's plane; off the room (ray past the wall into the background) keeps the last one. */
    const stripEnd = (e: PointerEvent): V3 | null => {
      if (!down?.start) return null
      const end = planeHit(e, ctx(), down.start.at, down.start.normal)
      const ceiling = ctx().ceilingMm
      const n = down.start.normal
      // test a point 5 cm in front of the surface: a wall face lies exactly on the room's edge
      if (end && inside(end[0] + n[0] * 50, end[1] + n[1] * 50) && end[2] > -10 && end[2] < ceiling + 10) down.end = end
      return down.end
    }
    const onDown = (e: PointerEvent) => {
      if (e.button !== 0) return
      const start = tool === 'Strip' ? surfaceHit(e, ctx(), e.altKey) : null
      down = { x: e.clientX, y: e.clientY, start, end: null }
      if (start && controls) controls.enabled = false
    }
    const onMove = (e: PointerEvent) => {
      if (!down?.start) return
      const end = stripEnd(e)
      if (end) setStrip({ start: down.start, end })
    }
    const onUp = async (e: PointerEvent) => {
      if (!down || e.button !== 0) return
      const end = stripEnd(e)
      const d = down
      down = null
      if (controls) controls.enabled = true
      if (tool === 'Strip') {
        setStrip(null)
        if (!d.start || !end) return
        await placeStrip(d.start, end)
        if (!e.shiftKey) useView3D.getState().setLightTool(null)
        return
      }
      if (Math.hypot(e.clientX - d.x, e.clientY - d.y) > 5) return // orbiting
      const key: TemplateKey = tool
      const hit = surfaceHit(e, ctx(), tool !== 'Uplight' && CEILING_TOOLS.includes(tool) !== e.altKey)
      const t = LAMP_TEMPLATES[key]
      const h = hit && mountFor(tool, hit, ctx().ceilingMm, t.kind.type === 'Area' ? Math.max(t.kind.w_mm, t.kind.h_mm) : 0)
      const st = useProject.getState()
      if (!h || !st.engine || !inside(h.at[0], h.at[1])) return
      const placed = (st.engine.cmd_place_light({ ...t, id: newId(), pos_mm: [0, 0, 0], attached_to: null }, h.at[0], h.at[1], h.at[2], h.normal[0], h.normal[1], h.normal[2], h.furniture ?? '') as { light: Light }[])[0].light
      const ops: Op[] = [{ op: 'PutLight', light: { ...placed, kind: keepUplight(tool, placed.kind) } }]
      if (st.commit(ops, `${t.name} setzen`)) {
        placedAt = performance.now()
        useEditor.getState().select({ kind: 'light', id: (ops[0] as { light: Light }).light.id })
        if (!e.shiftKey) useView3D.getState().setLightTool(null)
      }
    }
    const cursor = el.style.cursor
    el.style.cursor = 'copy'
    // move/up on the window: a drag may end over the panel
    el.addEventListener('pointerdown', onDown)
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    return () => {
      el.style.cursor = cursor
      el.removeEventListener('pointerdown', onDown)
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      if (controls) controls.enabled = true
      setStrip(null)
    }
  }, [tool, gl, controls, ctx, inside])

  const s = strip && stripFromDrag(strip.start.at, strip.end, strip.start.normal)
  if (!s) return null
  const [w, d, h] = s.dims_mm.map((v) => v / 1000)
  return (
    <mesh material={lampGhost} position={[s.pos_mm[0] / 1000, s.pos_mm[2] / 1000 + h / 2, s.pos_mm[1] / 1000]} rotation={[0, (-s.rotation_deg * Math.PI) / 180, 0]} renderOrder={10} raycast={() => {}}>
      <boxGeometry args={[w, h, d]} />
    </mesh>
  )
}

/** LED strip from a drag: a thin plate glowing away from the surface, 900 lm per metre. */
async function placeStrip(start: SurfaceHit, end: V3) {
  const s = stripFromDrag(start.at, end, start.normal)
  if (!s) return
  const asset = await plateAssetId()
  const fid = newId()
  const ops: Op[] = [
    {
      op: 'PutFurniture',
      item: { id: fid, asset, name: 'LED-Streifen', pos_mm: s.pos_mm, rotation_deg: s.rotation_deg, dims_mm: s.dims_mm, material: 'paint:white', walkable: false, overlap_ok: true, footprint: [], passable: true },
    },
    {
      op: 'PutLight',
      light: { id: newId(), name: 'LED-Streifen', kind: { type: 'Surface', face: s.face }, pos_mm: [0, 0, 0], lumen: Math.round((LED_LM_PER_M * s.dims_mm[0]) / 1000), color_temp_k: LED_KELVIN, on: true, attached_to: fid },
    },
  ]
  if (useProject.getState().commit(ops, 'LED-Streifen setzen')) {
    placedAt = performance.now()
    useEditor.getState().select({ kind: 'light', id: (ops[1] as { light: Light }).light.id })
  }
}

const AIM_M = 0.6
const aimMat = new MeshBasicMaterial({ color: '#c8642e', depthTest: false, transparent: true })
const aimLineMat = new LineBasicMaterial({ color: '#c8642e', depthTest: false, transparent: true, opacity: 0.8 })
const aimGeo = new SphereGeometry(0.05, 16, 12)

/** Selected spot: a handle 60 cm along the beam; drag it onto a surface to aim the spot there (one undo step). */
function AimHandle({ ctx }: { ctx: () => HitCtx }) {
  const selection = useEditor((s) => s.selection)
  const placed = usePlacedLights()
  const furniture = useProject((s) => s.project?.furniture)
  const controls = useThree((s) => s.controls) as OrbitControlsImpl | null
  const p = selection?.kind === 'light' ? placed.find((x) => x.light.id === selection.id) : undefined
  const host = p?.light.attached_to ? furniture?.find((f) => f.id === p.light.attached_to) : undefined
  const dir = p ? lightDir(p.light, host) : null
  const line = useMemo(() => new BufferGeometry(), [])
  useEffect(() => () => line.dispose(), [line])
  if (!p || !dir) return null
  // plan (x, y, z up) → three (x, z, y)
  const end: V3 = [p.pos.x + dir[0] * AIM_M, p.pos.y + dir[2] * AIM_M, p.pos.z + dir[1] * AIM_M]
  line.setAttribute('position', new Float32BufferAttribute([p.pos.x, p.pos.y, p.pos.z, ...end], 3))
  const onDown = (e: ThreeEvent<PointerEvent>) => {
    if (e.button !== 0) return
    e.stopPropagation()
    const l = p.light
    const from: V3 = [p.pos.x * 1000, p.pos.z * 1000, p.pos.y * 1000]
    let moved = false
    if (controls) controls.enabled = false
    const move = (ev: PointerEvent) => {
      const h = surfaceHit(ev, ctx(), false)
      if (!h || l.kind.type !== 'Spot') return
      moved = true
      useProject.getState().setTransient([{ op: 'PutLight', light: { ...l, kind: { ...l.kind, ...aimAt(from, h.at, host?.rotation_deg ?? 0) } } }])
    }
    const up = () => {
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', up)
      if (controls) controls.enabled = true
      if (moved) useProject.getState().endTransient(true, 'Spot ausrichten')
    }
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
  }
  return (
    <group>
      <lineSegments geometry={line} material={aimLineMat} renderOrder={11} raycast={() => {}} />
      <mesh geometry={aimGeo} material={aimMat} position={end} renderOrder={12} userData={{ marker: true }} onPointerDown={onDown} />
    </group>
  )
}
