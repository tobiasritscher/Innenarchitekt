// Reusable 2D plan editor (SVG). Renders a PlanSource (main project or a temporary engine)
// and implements the tools of the modelling mode. All model changes go through the source's
// commit / setTransient / endTransient.
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { PlanEngine } from '../core/engine'
import type { ChainPoint } from '../core/generated/ChainPoint'
import type { Clearances } from '../core/generated/Clearances'
import type { FurnitureInstance } from '../core/generated/FurnitureInstance'
import type { Op } from '../core/generated/Op'
import type { Opening } from '../core/generated/Opening'
import type { SnapResult } from '../core/generated/SnapResult'
import { fmtLength, parseLength } from '../core/format'
import { instantiateAsset, movedElevation, standHeight } from '../core/furniture'
import { newId } from '../core/engine'
import { useAssets } from '../state/assets'
import { useEditor, type Selection, type SelectionKind } from '../state/editor'
import { add, closestOnSegment, dist, mul, norm, polyPoints, rotate, sub, type Vec } from './geom'
import {
  DimensionsLayer,
  FurnitureLayer,
  FurnitureShape,
  GridLayer,
  LABEL_PX,
  LightsLayer,
  NodesLayer,
  OpeningShape,
  OpeningsLayer,
  RefImageLayer,
  RoomLabelsLayer,
  RoomsLayer,
  WallsLayer,
  wallDimLabels,
} from './layers'
import {
  allBounds,
  deleteOps,
  elementsBounds,
  flipDoor,
  furnitureCorners,
  lightWorldPos,
  newCeilingLight,
  newRoomMeta,
  nodeMap,
  openingGeom,
  roomAt,
  wallPoints,
  wallSideAt,
} from './model'
import { pointAtLength, snapPoint, type SnapNode, type SnapWall, type Snapped } from './snap'
import { useDerivedOf, type PlanSource } from './source'
import { DEFAULT_VIEW, fitBounds, panBy, toScreen, toWorld, zoomAt, type View } from './view'

export type PlanCanvasProps = {
  source: PlanSource
  /** Confidence per element id (import); elements below 0.6 are highlighted. */
  confidence?: Record<string, number>
  /** Override of the reference image opacity. */
  refOpacity?: number
  /** Pick mode: clicks deliver (snapped) plan points instead of running the current tool. */
  pick?: { onPick: (p: Vec) => void; hint?: string } | null
  /** Extra content in world coordinates (rendered on top). */
  world?: (view: View) => ReactNode
}

type Hit = { k: string; id: string } | null

type Gesture =
  | { t: 'press'; s0: Vec; w0: Vec; hit: Hit; shift: boolean; alt: boolean; button: number }
  | { t: 'pan'; s0: Vec; view0: View }
  | { t: 'pinch'; d0: number; m0: Vec; view0: View }
  | { t: 'node'; id: string; w0: Vec; orig: Vec; base: PlanEngine; label: string }
  | { t: 'wall'; id: string; w0: Vec; n: Vec; base: PlanEngine }
  | { t: 'opening'; id: string; w0: Vec; orig: Opening; d: Vec; wallLen: number }
  | { t: 'furniture'; id: string; w0: Vec; orig: FurnitureInstance; rot: number; last: Vec; alt: boolean }
  | { t: 'light'; id: string; w0: Vec; orig: [number, number, number] }

type WallDraft = { points: ChainPoint[]; pos: Vec[] }

const LOW_CONF = 0.6
const CLICK_PX = 4
const SNAP_PX = 10
const FURNITURE_SNAP_MM = 250

const KIND_LABEL: Record<SelectionKind, string> = {
  node: 'Ecke',
  wall: 'Wand',
  opening: 'Öffnung',
  room: 'Raum',
  furniture: 'Möbel',
  light: 'Lampe',
}

const TOOL_HINT: Record<string, string> = {
  select: 'Klicken = auswählen · Ziehen = verschieben · Entf = löschen · R = drehen (Möbel) · F = Türanschlag',
  wall: 'Klicken = Punkt setzen · Zahl tippen + Enter = Länge · Doppelklick/Enter = fertig · Startpunkt = schliessen · Alt = ohne Fang',
  measure: 'Auf ein Mass klicken, neue Länge eingeben, fixe Seite wählen',
  ruler: 'Klicken = Punkt A · Klicken = Punkt B · rastet an Ecken, Wandflächen, Möbeln · Shift = waagrecht/senkrecht · Alt = ohne Fang · Esc = löschen',
  opening: 'Auf eine Wand klicken, um die Öffnung zu setzen · F = Anschlag wechseln',
  room: 'In einen geschlossenen Raum klicken, um ihn zu benennen',
  material: 'Wandseite oder Boden anklicken · Shift = Decke',
  light: 'Klicken = Deckenleuchte setzen',
  furniture: 'Klicken = platzieren · R = 90° drehen · Shift+R = 15° · Esc = abbrechen',
}

function hitOf(target: EventTarget | null): Hit {
  const el = target instanceof Element ? target.closest('[data-k]') : null
  if (!el) return null
  return { k: el.getAttribute('data-k')!, id: el.getAttribute('data-id')! }
}

function isTyping(e: KeyboardEvent): boolean {
  const t = e.target as HTMLElement | null
  return !!t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable)
}

export function PlanCanvas({ source, confidence, refOpacity, pick, world }: PlanCanvasProps) {
  const project = source.project
  const floor = project.floors[0]
  const floorId = floor.id
  const derived = useDerivedOf(source)

  const tool = useEditor((s) => s.tool)
  const selection = useEditor((s) => s.selection)
  const hover = useEditor((s) => s.hover)
  const snapping = useEditor((s) => s.snapping)
  const showRefImage = useEditor((s) => s.showRefImage)
  const placingAsset = useEditor((s) => s.placingAsset)
  const draw = useEditor((s) => s.draw)
  const focusRequest = useEditor((s) => s.focusRequest)
  const fitRequest = useEditor((s) => s.fitRequest)
  const asset = useAssets((s) => (placingAsset ? s.assets.find((a) => a.id === placingAsset) : undefined))

  const wrapRef = useRef<HTMLDivElement>(null)
  const svgRef = useRef<SVGSVGElement>(null)
  const [size, setSize] = useState({ w: 0, h: 0 })
  const [view, setView] = useState<View>(DEFAULT_VIEW)
  const viewRef = useRef(view)
  viewRef.current = view

  // transient UI state
  const [cursor, setCursor] = useState<Vec | null>(null)
  /** Ruler tool: A fixed, B fixed once the second click is done (else follows the cursor). */
  const [ruler, setRuler] = useState<{ a: Vec; b: Vec | null } | null>(null)
  const [snap, setSnapState] = useState<Snapped | null>(null)
  const setSnap = (v: Snapped | null) => {
    snapRef.current = v
    setSnapState(v)
  }
  // Wall-tool state lives in refs (fresh in key/pointer handlers even before a re-render)
  // mirrored into React state for rendering.
  const [draft, setDraftState] = useState<WallDraft>({ points: [], pos: [] })
  const [typed, setTypedState] = useState('')
  const draftRef = useRef(draft)
  const typedRef = useRef(typed)
  const snapRef = useRef<Snapped | null>(null)
  const setDraft = (d: WallDraft) => {
    draftRef.current = d
    setDraftState(d)
  }
  const setTyped = (t: string) => {
    typedRef.current = t
    setTypedState(t)
  }
  const [measure, setMeasure] = useState<{ wall: string; text: string; fixedA: boolean } | null>(null)
  const [openingFlip, setOpeningFlip] = useState<{ hinge: 'Start' | 'End'; swing: 'Left' | 'Right' }>({ hinge: 'Start', swing: 'Left' })
  const [placeRot, setPlaceRot] = useState(0)
  const [dragInfo, setDragInfo] = useState<{ kind: string; id: string } | null>(null)
  const [altDown, setAltDown] = useState(false)
  const gesture = useRef<Gesture | null>(null)
  const pointers = useRef(new Map<number, Vec>())
  const spaceDown = useRef(false)
  const didFit = useRef(false)

  // ---------------------------------------------------------------- geometry caches
  const nodes = useMemo(() => nodeMap(floor), [floor])
  const snapNodes = useMemo<SnapNode[]>(() => floor.nodes.map((n) => ({ id: n.id, p: [n.x_mm, n.y_mm] })), [floor.nodes])
  const snapWalls = useMemo<SnapWall[]>(
    () =>
      floor.walls.flatMap((w) => {
        const p = wallPoints(nodes, w)
        return p ? [{ id: w.id, a: p[0], b: p[1] }] : []
      }),
    [floor.walls, nodes],
  )
  /** Ruler targets: wall footprint corners/faces and furniture footprint corners/edges (besides axis nodes). */
  const [rulerNodes, rulerFaces] = useMemo(() => {
    const polys: Vec[][] = derived.footprints.map((f) => f.corners as Vec[])
    for (const f of project.furniture) {
      const r = (f.rotation_deg * Math.PI) / 180
      const [w, d] = [f.dims_mm[0] / 2, f.dims_mm[1] / 2]
      polys.push(([[-w, -d], [w, -d], [w, d], [-w, d]] as Vec[]).map(([x, y]) => [f.pos_mm[0] + x * Math.cos(r) - y * Math.sin(r), f.pos_mm[1] + x * Math.sin(r) + y * Math.cos(r)]))
    }
    const ns: SnapNode[] = [...snapNodes]
    const fs: SnapWall[] = []
    polys.forEach((poly, i) =>
      poly.forEach((p, j) => {
        ns.push({ id: `r${i}:${j}`, p })
        fs.push({ id: `r${i}:${j}`, a: p, b: poly[(j + 1) % poly.length] })
      }),
    )
    return [ns, fs]
  }, [derived.footprints, project.furniture, snapNodes])
  const dimLabels = useMemo(() => wallDimLabels(floor, derived.rooms, view.scale), [floor, derived.rooms, view.scale])

  // ---------------------------------------------------------------- size / fit
  useLayoutEffect(() => {
    const el = wrapRef.current
    if (!el) return
    const ro = new ResizeObserver(() => setSize({ w: el.clientWidth, h: el.clientHeight }))
    ro.observe(el)
    setSize({ w: el.clientWidth, h: el.clientHeight })
    return () => ro.disconnect()
  }, [])

  const fitAll = useCallback(() => {
    const el = wrapRef.current
    if (!el || !el.clientWidth) return
    const b = allBounds(source.project, derived.rooms)
    if (b) setView(fitBounds(b, el.clientWidth, el.clientHeight, 48))
    else setView({ scale: 0.08, tx: el.clientWidth / 2, ty: el.clientHeight / 2 })
  }, [source.project, derived.rooms])

  useEffect(() => {
    if (!didFit.current && size.w > 0) {
      didFit.current = true
      fitAll()
    }
  }, [size.w, fitAll])

  const fitCount = useRef(fitRequest)
  useEffect(() => {
    if (fitRequest !== fitCount.current) {
      fitCount.current = fitRequest
      fitAll()
    }
  }, [fitRequest, fitAll])

  const focusN = useRef(focusRequest?.n ?? 0)
  useEffect(() => {
    if (!focusRequest || focusRequest.n === focusN.current) return
    focusN.current = focusRequest.n
    const b = elementsBounds(project, derived.rooms, focusRequest.ids)
    const el = wrapRef.current
    if (b && el) {
      const pad = 1200
      setView(fitBounds({ minX: b.minX - pad, minY: b.minY - pad, maxX: b.maxX + pad, maxY: b.maxY + pad }, el.clientWidth, el.clientHeight, 40, 0.25))
    }
  }, [focusRequest, project, derived.rooms])

  // ---------------------------------------------------------------- helpers
  const screenOf = (e: { clientX: number; clientY: number }): Vec => {
    const r = svgRef.current!.getBoundingClientRect()
    return [e.clientX - r.left, e.clientY - r.top]
  }
  const tolMm = SNAP_PX / view.scale
  const snapOn = (alt: boolean) => snapping && !alt

  const select = useEditor.getState().select
  const setHover = useEditor.getState().setHover

  const selFromHit = (h: Hit): Selection => {
    if (!h) return null
    if (h.k === 'dim') return { kind: 'wall', id: h.id }
    return { kind: h.k as SelectionKind, id: h.id }
  }

  const wallSnap = (raw: Vec, alt: boolean): Snapped => {
    const pos = draftRef.current.pos
    const prev = pos.length ? pos[pos.length - 1] : undefined
    // points of the chain being drawn are snap targets too (closing on the start point)
    const extra: SnapNode[] = pos.slice(0, -1).map((p, i) => ({ id: `draft:${i}`, p }))
    return snapPoint(raw, extra.length ? [...snapNodes, ...extra] : snapNodes, snapWalls, { enabled: snapOn(alt), tol: tolMm, prev })
  }

  /** Ruler snapping: wall corners (faces), nodes, furniture corners, wall faces; Shift = horizontal/vertical from A. */
  const rulerSnap = (raw: Vec, alt: boolean, shift: boolean): Snapped => {
    const a = ruler && !ruler.b ? ruler.a : undefined
    if (a && shift) {
      const d = sub(raw, a)
      raw = Math.abs(d[0]) >= Math.abs(d[1]) ? [raw[0], a[1]] : [a[0], raw[1]]
    }
    const s = snapPoint(raw, rulerNodes, rulerFaces, { enabled: snapOn(alt), tol: tolMm, grid: 1, prev: shift ? undefined : a })
    if (a && shift && s.kind !== 'node') return { p: Math.abs(raw[0] - a[0]) > Math.abs(raw[1] - a[1]) ? [s.p[0], a[1]] : [a[0], s.p[1]], kind: 'free' }
    return s
  }

  const openingAt = (raw: Vec): Opening | null => {
    let best: { o: Opening; d: number } | null = null
    const kind = draw.openingKind
    const width = kind === 'Window' ? draw.windowWidthMm : draw.doorWidthMm
    for (const w of floor.walls) {
      const p = wallPoints(nodes, w)
      if (!p) continue
      const [t, , d] = closestOnSegment(raw, p[0], p[1])
      if (d > w.thickness_mm / 2 + tolMm) continue
      if (best && d >= best.d) continue
      const L = dist(p[0], p[1])
      let off = t * L - width / 2
      if (snapping) off = Math.round(off / 10) * 10
      off = Math.max(0, Math.min(Math.max(0, L - width), off))
      const o: Opening = {
        id: '__ghost',
        wall: w.id,
        offset_mm: Math.round(off),
        width_mm: width,
        height_mm: kind === 'Window' ? draw.windowHeightMm : draw.doorHeightMm,
        kind:
          kind === 'Door'
            ? { type: 'Door', hinge: openingFlip.hinge, swing: openingFlip.swing, glass: draw.doorGlass }
            : kind === 'Window'
              ? { type: 'Window', sill_mm: draw.windowSillMm }
              : { type: 'Passage' },
      }
      best = { o, d }
    }
    return best?.o ?? null
  }

  const furnitureSnap = (x: number, y: number, dims: [number, number, number], rot: number, alt: boolean): SnapResult | null => {
    if (!snapOn(alt)) return null
    try {
      return (source.engine.snap_furniture(floorId, x, y, new Uint32Array(dims), rot, FURNITURE_SNAP_MM) as SnapResult | null) ?? null
    } catch {
      return null
    }
  }

  const placementGhost = useMemo(() => {
    if (tool !== 'furniture' || !asset || !cursor) return null
    const dims = [...asset.dims_mm] as [number, number, number]
    const s = furnitureSnap(cursor[0], cursor[1], dims, placeRot, altDown)
    const item: FurnitureInstance = {
      id: '__ghost',
      asset: asset.id,
      name: asset.name,
      pos_mm: [s ? s.x_mm : Math.round(cursor[0]), s ? s.y_mm : Math.round(cursor[1]), 0],
      rotation_deg: s ? s.rotation_deg : placeRot,
      dims_mm: dims,
      material: null,
      walkable: asset.primitive === 'Podest',
      overlap_ok: asset.primitive === 'Chair',
      footprint: asset.footprint ?? [],
      passable: false,
    }
    item.pos_mm[2] = Math.round(standHeight(source.engine, item.pos_mm[0], item.pos_mm[1]))
    return item
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tool, asset, cursor, placeRot, altDown, source.revision, snapping])

  // clearances: furniture being dragged / placed / selected
  const clearanceItem: FurnitureInstance | null = useMemo(() => {
    if (placementGhost) return placementGhost
    const id = dragInfo?.kind === 'furniture' ? dragInfo.id : selection?.kind === 'furniture' ? selection.id : null
    return id ? (project.furniture.find((f) => f.id === id) ?? null) : null
  }, [placementGhost, dragInfo, selection, project.furniture])
  const clearances = useMemo<Clearances | null>(() => {
    if (!clearanceItem) return null
    try {
      return source.engine.clearances(floorId, clearanceItem) as Clearances
    } catch {
      return null
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [clearanceItem, source.revision])

  // ---------------------------------------------------------------- commits
  const finishChain = (closed: boolean, extra?: ChainPoint) => {
    const pts = extra ? [...draftRef.current.points, extra] : draftRef.current.points
    setDraft({ points: [], pos: [] })
    setTyped('')
    if (pts.length < 2) return
    try {
      const ops = source.engine.cmd_add_wall_chain(floorId, pts, closed, draw.wallThicknessMm, draw.wallExterior) as Op[]
      source.commit(ops, closed ? 'Raum zeichnen' : 'Wände zeichnen')
    } catch (e) {
      useEditor.getState().showToast(`Wände konnten nicht erstellt werden: ${String(e)}`)
    }
  }

  const addChainPoint = (s: Snapped) => {
    const realNode = s.node && !s.node.startsWith('draft:') ? s.node : null
    const cp: ChainPoint = { node: realNode, wall: s.node ? null : (s.wall ?? null), x: s.p[0], y: s.p[1] }
    const pos = draftRef.current.pos
    if (pos.length && dist(pos[pos.length - 1], s.p) < 1) return
    // closing on the start point
    if (pos.length >= 2) {
      const first = draftRef.current.points[0]
      if (s.node === 'draft:0' || (s.node && first.node === s.node) || dist(pos[0], s.p) < 1) {
        finishChain(true)
        return
      }
    }
    // landing on an existing node/wall ends an open chain (after the first segment)
    setDraft({ points: [...draftRef.current.points, cp], pos: [...pos, s.p] })
    setTyped('')
  }

  const placeTypedLength = () => {
    const mm = parseLength(typedRef.current)
    if (mm === null || mm <= 0 || draftRef.current.pos.length === 0) {
      setTyped('')
      return
    }
    const last = draftRef.current.pos[draftRef.current.pos.length - 1]
    const sp = snapRef.current?.p
    const towards = sp && dist(sp, last) > 1 ? sp : add(last, [1, 0])
    const p = pointAtLength(last, towards, mm)
    addChainPoint({ p, kind: 'free' })
  }

  const commitMeasure = () => {
    if (!measure) return
    const mm = parseLength(measure.text)
    setMeasure(null)
    if (mm === null || mm <= 0) return
    try {
      const ops = source.engine.cmd_set_wall_length(floorId, measure.wall, mm, measure.fixedA) as Op[]
      source.commit(ops, 'Wandlänge ändern')
    } catch (e) {
      useEditor.getState().showToast(String(e))
    }
  }

  const openMeasure = (wallId: string) => {
    const w = floor.walls.find((x) => x.id === wallId)
    const p = w && wallPoints(nodes, w)
    if (!p) return
    select({ kind: 'wall', id: wallId })
    setMeasure({ wall: wallId, text: fmtLength(Math.round(dist(p[0], p[1]))), fixedA: true })
  }

  const rotateFurniture = (stepDeg: number) => {
    const g = gesture.current
    if (g?.t === 'furniture') {
      g.rot = (((g.rot + stepDeg) % 360) + 360) % 360
      updateDrag(g.last, g.alt)
      return true
    }
    if (tool === 'furniture' && asset) {
      setPlaceRot((r) => (((r + stepDeg) % 360) + 360) % 360)
      return true
    }
    if (selection?.kind === 'furniture') {
      const f = project.furniture.find((x) => x.id === selection.id)
      if (f) {
        const rot = Math.round((((f.rotation_deg + stepDeg) % 360) + 360) % 360)
        source.commit([{ op: 'PutFurniture', item: { ...f, rotation_deg: rot } }], 'Möbel drehen')
        return true
      }
    }
    return false
  }

  const flipSelectedDoor = () => {
    if (tool === 'opening') {
      setOpeningFlip((f) => (f.swing === 'Left' ? { hinge: f.hinge, swing: 'Right' } : { hinge: f.hinge === 'Start' ? 'End' : 'Start', swing: 'Left' }))
      return
    }
    if (selection?.kind !== 'opening') return
    const o = floor.openings.find((x) => x.id === selection.id)
    const f = o && flipDoor(o)
    if (f) source.commit([{ op: 'PutOpening', floor: floorId, opening: f }], 'Türanschlag ändern')
  }

  // ---------------------------------------------------------------- clicks per tool
  const onClick = (w: Vec, hit: Hit, shift: boolean, alt: boolean) => {
    if (pick) {
      const s = snapPoint(w, snapNodes, snapWalls, { enabled: snapOn(alt), tol: tolMm, grid: 1 })
      pick.onPick(s.p)
      return
    }
    switch (tool) {
      case 'select': {
        select(selFromHit(hit))
        break
      }
      case 'wall':
        addChainPoint(wallSnap(w, alt))
        break
      case 'ruler': {
        const p = rulerSnap(w, alt, shift).p
        setRuler(!ruler || ruler.b ? { a: p, b: null } : { a: ruler.a, b: p })
        break
      }
      case 'measure':
        if (hit && (hit.k === 'dim' || hit.k === 'wall')) openMeasure(hit.id)
        else setMeasure(null)
        break
      case 'opening': {
        const o = openingAt(w)
        if (!o) {
          useEditor.getState().showToast('Öffnungen werden auf eine Wand gesetzt.')
          break
        }
        const placed = { ...o, id: newId() }
        if (source.commit([{ op: 'PutOpening', floor: floorId, opening: placed }], `${o.kind.type === 'Door' ? 'Tür' : o.kind.type === 'Window' ? 'Fenster' : 'Durchgang'} setzen`))
          select({ kind: 'opening', id: placed.id })
        break
      }
      case 'room': {
        const r = roomAt(derived.rooms, w)
        if (!r) {
          useEditor.getState().showToast('Kein geschlossener Raum an dieser Stelle.')
          break
        }
        if (r.meta) select({ kind: 'room', id: r.meta })
        else {
          const meta = newRoomMeta(w, floor.rooms.length + 1)
          if (source.commit([{ op: 'PutRoom', floor: floorId, room: meta }], 'Raum benennen')) {
            select({ kind: 'room', id: meta.id })
          }
        }
        useEditor.getState().setFocusField('room-name')
        break
      }
      case 'material': {
        const mat = draw.material
        if (hit?.k === 'wall') {
          const wall = floor.walls.find((x) => x.id === hit.id)
          const p = wall && wallPoints(nodes, wall)
          if (wall && p) {
            const side = wallSideAt(p[0], p[1], w)
            const nw = side === 'left' ? { ...wall, finish_left: mat } : { ...wall, finish_right: mat }
            source.commit([{ op: 'PutWall', floor: floorId, wall: nw }], 'Wandmaterial')
          }
        } else if (hit?.k === 'furniture') {
          const f = project.furniture.find((x) => x.id === hit.id)
          if (f) source.commit([{ op: 'PutFurniture', item: { ...f, material: mat } }], 'Möbelmaterial')
        } else {
          const r = roomAt(derived.rooms, w)
          if (!r) break
          const meta = r.meta ? floor.rooms.find((m) => m.id === r.meta) : undefined
          const base = meta ?? newRoomMeta(w, floor.rooms.length + 1)
          const nm = shift ? { ...base, ceiling_material: mat } : { ...base, floor_material: mat }
          source.commit([{ op: 'PutRoom', floor: floorId, room: nm }], shift ? 'Deckenmaterial' : 'Bodenmaterial')
        }
        break
      }
      case 'light': {
        const l = newCeilingLight(project, w)
        if (source.commit([{ op: 'PutLight', light: l }], 'Lampe setzen')) select({ kind: 'light', id: l.id })
        break
      }
      case 'furniture': {
        if (!asset || !placementGhost) break
        const [ops, id] = instantiateAsset(asset, placementGhost.pos_mm[0], placementGhost.pos_mm[1], placementGhost.rotation_deg, placementGhost.pos_mm[2])
        if (source.commit(ops, 'Möbel platzieren')) {
          select({ kind: 'furniture', id })
          if (!shift) useEditor.getState().setPlacingAsset(null)
        }
        break
      }
    }
  }

  // ---------------------------------------------------------------- drags
  const startDrag = (g: Extract<Gesture, { t: 'press' }>): boolean => {
    const hit = g.hit
    if (tool !== 'select' || pick || !hit) return false
    const w0 = g.w0
    switch (hit.k) {
      case 'node': {
        const p = nodes.get(hit.id)
        if (!p) return false
        gesture.current = { t: 'node', id: hit.id, w0, orig: p, base: new PlanEngine(project), label: 'Ecke verschieben' }
        break
      }
      case 'wall': {
        const w = floor.walls.find((x) => x.id === hit.id)
        const p = w && wallPoints(nodes, w)
        if (!p) return false
        const d = norm(sub(p[1], p[0]))
        gesture.current = { t: 'wall', id: hit.id, w0, n: [d[1], -d[0]], base: new PlanEngine(project) }
        break
      }
      case 'opening': {
        const o = floor.openings.find((x) => x.id === hit.id)
        const w = o && floor.walls.find((x) => x.id === o.wall)
        const geo = o && w && openingGeom(nodes, w, o)
        if (!o || !geo) return false
        gesture.current = { t: 'opening', id: hit.id, w0, orig: o, d: geo.d, wallLen: geo.wallLen }
        break
      }
      case 'furniture': {
        const f = project.furniture.find((x) => x.id === hit.id)
        if (!f) return false
        gesture.current = { t: 'furniture', id: hit.id, w0, orig: f, rot: f.rotation_deg, last: w0, alt: g.alt }
        break
      }
      case 'light': {
        const l = project.lights.find((x) => x.id === hit.id)
        if (!l || l.attached_to) return false
        gesture.current = { t: 'light', id: hit.id, w0, orig: l.pos_mm }
        break
      }
      default:
        return false
    }
    select({ kind: hit.k as SelectionKind, id: hit.id })
    setDragInfo({ kind: hit.k, id: hit.id })
    return true
  }

  const updateDrag = (w: Vec, alt: boolean) => {
    const g = gesture.current
    if (!g) return
    const delta = sub(w, (g as { w0: Vec }).w0)
    let ops: Op[] = []
    try {
      switch (g.t) {
        case 'node': {
          const raw = add(g.orig, delta)
          const attached = new Set(floor.walls.filter((x) => x.a === g.id || x.b === g.id).map((x) => x.id))
          const s = snapPoint(raw, snapNodes, snapWalls, { enabled: snapOn(alt), tol: tolMm, excludeNodes: new Set([g.id]), excludeWalls: attached })
          if (s.node) {
            ops = g.base.cmd_merge_nodes(floorId, s.node, g.id) as Op[]
            g.label = 'Ecken verbinden'
          } else {
            ops = [{ op: 'PutNode', floor: floorId, node: { id: g.id, x_mm: s.p[0], y_mm: s.p[1] } }]
            g.label = 'Ecke verschieben'
          }
          setSnap(s)
          break
        }
        case 'wall': {
          let off = delta[0] * g.n[0] + delta[1] * g.n[1]
          if (snapOn(alt)) off = Math.round(off / 10) * 10
          ops = g.base.cmd_move_wall(floorId, g.id, off) as Op[]
          break
        }
        case 'opening': {
          let off = g.orig.offset_mm + delta[0] * g.d[0] + delta[1] * g.d[1]
          if (snapOn(alt)) off = Math.round(off / 10) * 10
          off = Math.max(0, Math.min(Math.max(0, g.wallLen - g.orig.width_mm), off))
          ops = [{ op: 'PutOpening', floor: floorId, opening: { ...g.orig, offset_mm: Math.round(off) } }]
          break
        }
        case 'furniture': {
          g.last = w
          g.alt = alt
          const x = g.orig.pos_mm[0] + delta[0]
          const y = g.orig.pos_mm[1] + delta[1]
          const s = furnitureSnap(x, y, g.orig.dims_mm, g.rot, alt)
          const nx = s ? s.x_mm : Math.round(snapOn(alt) ? Math.round(x / 10) * 10 : x)
          const ny = s ? s.y_mm : Math.round(snapOn(alt) ? Math.round(y / 10) * 10 : y)
          const item: FurnitureInstance = {
            ...g.orig,
            pos_mm: [nx, ny, movedElevation(source.engine, g.orig, nx, ny)],
            rotation_deg: s ? s.rotation_deg : g.rot,
          }
          ops = [{ op: 'PutFurniture', item }]
          break
        }
        case 'light': {
          const l = project.lights.find((x) => x.id === g.id)
          if (!l) return
          const x = g.orig[0] + delta[0]
          const y = g.orig[1] + delta[1]
          const r = (v: number) => Math.round(snapOn(alt) ? Math.round(v / 10) * 10 : v)
          ops = [{ op: 'PutLight', light: { ...l, pos_mm: [r(x), r(y), g.orig[2]] } }]
          break
        }
        default:
          return
      }
    } catch (e) {
      console.error(e)
      return
    }
    if (ops.length) source.setTransient(ops)
  }

  const endDrag = (commit: boolean) => {
    const g = gesture.current
    gesture.current = null
    setDragInfo(null)
    setSnap(null)
    if (!g) return
    const labels: Record<string, string> = {
      wall: 'Wand verschieben',
      opening: 'Öffnung verschieben',
      furniture: 'Möbel verschieben',
      light: 'Lampe verschieben',
    }
    if (g.t === 'node' || g.t === 'wall' || g.t === 'opening' || g.t === 'furniture' || g.t === 'light') {
      source.endTransient(commit, g.t === 'node' ? g.label : labels[g.t])
    }
    if (g.t === 'node' || g.t === 'wall') g.base.free()
    if (g.t === 'node' && commit && g.label === 'Ecken verbinden') select(null)
  }

  // ---------------------------------------------------------------- pointer events
  const onPointerDown = (e: React.PointerEvent<SVGSVGElement>) => {
    const s = screenOf(e)
    svgRef.current?.setPointerCapture(e.pointerId)
    pointers.current.set(e.pointerId, s)
    if (pointers.current.size === 2) {
      // pinch: cancel any element interaction
      if (gesture.current && gesture.current.t !== 'press' && gesture.current.t !== 'pan') endDrag(false)
      const [a, b] = [...pointers.current.values()]
      gesture.current = { t: 'pinch', d0: dist(a, b), m0: [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2], view0: viewRef.current }
      return
    }
    if (pointers.current.size > 2) return
    if (e.button === 1 || (e.button === 0 && spaceDown.current)) {
      e.preventDefault()
      gesture.current = { t: 'pan', s0: s, view0: viewRef.current }
      return
    }
    if (e.button === 2) {
      if (tool === 'wall' && draftRef.current.points.length) finishChain(false)
      return
    }
    if (e.button !== 0) return
    gesture.current = { t: 'press', s0: s, w0: toWorld(viewRef.current, s), hit: hitOf(e.target), shift: e.shiftKey, alt: e.altKey, button: e.button }
  }

  const onPointerMove = (e: React.PointerEvent<SVGSVGElement>) => {
    const s = screenOf(e)
    if (pointers.current.has(e.pointerId)) pointers.current.set(e.pointerId, s)
    const g = gesture.current
    const w = toWorld(viewRef.current, s)
    if (e.altKey !== altDown) setAltDown(e.altKey)
    if (g?.t === 'pinch') {
      if (pointers.current.size < 2) return
      const [a, b] = [...pointers.current.values()]
      const m: Vec = [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2]
      const f = dist(a, b) / Math.max(1, g.d0)
      let v = zoomAt(g.view0, g.m0, f)
      v = panBy(v, m[0] - g.m0[0], m[1] - g.m0[1])
      setView(v)
      return
    }
    if (g?.t === 'pan') {
      setView(panBy(g.view0, s[0] - g.s0[0], s[1] - g.s0[1]))
      return
    }
    if (g?.t === 'press') {
      if (dist(s, g.s0) > CLICK_PX) {
        if (!startDrag(g)) gesture.current = { t: 'pan', s0: g.s0, view0: viewRef.current }
        else updateDrag(w, e.altKey)
      }
      return
    }
    if (g) {
      updateDrag(w, e.altKey)
      return
    }
    // hover
    if (e.pointerType === 'mouse') {
      const h = hitOf(e.target)
      const hs = tool === 'select' || tool === 'material' || tool === 'measure' ? selFromHit(h) : null
      const cur = useEditor.getState().hover
      if (hs?.id !== cur?.id || hs?.kind !== cur?.kind) setHover(hs)
    }
    if (tool === 'ruler' && !pick) {
      setCursor(w)
      setSnap(rulerSnap(w, e.altKey, e.shiftKey))
    }
    if (tool === 'wall' || tool === 'opening' || tool === 'furniture' || pick) {
      setCursor(w)
      if (tool === 'wall' && !pick) setSnap(wallSnap(w, e.altKey))
      if (pick) setSnap(snapPoint(w, snapNodes, snapWalls, { enabled: snapOn(e.altKey), tol: tolMm, grid: 1 }))
    }
  }

  const onPointerUp = (e: React.PointerEvent<SVGSVGElement>) => {
    pointers.current.delete(e.pointerId)
    const g = gesture.current
    if (!g) return
    if (g.t === 'pinch') {
      if (pointers.current.size < 2) gesture.current = null
      return
    }
    if (g.t === 'press') {
      gesture.current = null
      onClick(g.w0, g.hit, g.shift, g.alt)
      return
    }
    if (g.t === 'pan') {
      gesture.current = null
      return
    }
    endDrag(true)
  }

  const onPointerCancel = (e: React.PointerEvent<SVGSVGElement>) => {
    pointers.current.delete(e.pointerId)
    const g = gesture.current
    if (g && g.t !== 'press' && g.t !== 'pan' && g.t !== 'pinch') endDrag(false)
    gesture.current = null
  }

  const onPointerLeave = () => {
    if (!gesture.current) {
      setCursor(null)
      if (useEditor.getState().hover) setHover(null)
    }
  }

  // wheel (non-passive)
  useEffect(() => {
    const el = svgRef.current
    if (!el) return
    const onWheel = (e: WheelEvent) => {
      e.preventDefault()
      const r = el.getBoundingClientRect()
      const s: Vec = [e.clientX - r.left, e.clientY - r.top]
      const v = viewRef.current
      if (!e.ctrlKey && e.deltaMode === 0 && (Math.abs(e.deltaX) > 0.5 || (Math.abs(e.deltaY) < 40 && !Number.isInteger(e.deltaY)))) {
        // trackpad two-finger scroll → pan
        setView(panBy(v, -e.deltaX, -e.deltaY))
        return
      }
      const k = e.deltaMode === 1 ? 0.05 : e.ctrlKey ? 0.01 : 0.0015
      setView(zoomAt(v, s, Math.exp(-e.deltaY * k)))
    }
    el.addEventListener('wheel', onWheel, { passive: false })
    return () => el.removeEventListener('wheel', onWheel)
  }, [])

  // ---------------------------------------------------------------- keyboard
  const keyRef = useRef<(e: KeyboardEvent) => void>(() => {})
  keyRef.current = (e: KeyboardEvent) => {
    if (isTyping(e)) return
    const mod = e.metaKey || e.ctrlKey
    if (source.local && mod && (e.key.toLowerCase() === 'z' || e.key.toLowerCase() === 'y')) {
      // temporary engine: own undo stack; keep the main project's handler out of it
      e.preventDefault()
      e.stopImmediatePropagation()
      if (e.key.toLowerCase() === 'y' || e.shiftKey) source.redo()
      else source.undo()
      return
    }
    if (mod) return
    const ed = useEditor.getState()
    const key = e.key
    if (key === ' ') {
      spaceDown.current = true
      e.preventDefault()
      return
    }
    if (key === 'Alt') setAltDown(true)
    if (key === 'Escape') {
      if (gesture.current && gesture.current.t !== 'press' && gesture.current.t !== 'pan' && gesture.current.t !== 'pinch') {
        endDrag(false)
        return
      }
      if (measure) return setMeasure(null)
      if (ruler) return setRuler(null)
      if (typedRef.current) return setTyped('')
      if (draftRef.current.points.length) return setDraft({ points: [], pos: [] })
      if (ed.placingAsset) return ed.setPlacingAsset(null)
      if (ed.tool !== 'select') return ed.setTool('select')
      ed.select(null)
      return
    }
    if (tool === 'wall' && !pick) {
      if (/^[0-9.,']$/.test(key) || (typedRef.current && /^[mc ]$/i.test(key))) {
        if (draftRef.current.pos.length) {
          setTyped(typedRef.current + key)
          e.preventDefault()
          return
        }
      }
      if (key === 'Backspace') {
        if (typedRef.current) setTyped(typedRef.current.slice(0, -1))
        else if (draftRef.current.points.length) setDraft({ points: draftRef.current.points.slice(0, -1), pos: draftRef.current.pos.slice(0, -1) })
        e.preventDefault()
        return
      }
      if (key === 'Enter') {
        if (typedRef.current) placeTypedLength()
        else finishChain(false)
        e.preventDefault()
        return
      }
    }
    if ((key === 'Delete' || key === 'Backspace') && ed.selection) {
      const r = deleteOps(source, ed.selection)
      if (r && r[0].length && source.commit(r[0], r[1])) ed.select(null)
      e.preventDefault()
      return
    }
    if (key === 'Home' || (key === '!' && e.shiftKey)) {
      fitAll()
      return
    }
    const k = key.toLowerCase()
    if (k === 'r') {
      if (rotateFurniture(e.shiftKey ? 15 : 90)) {
        e.preventDefault()
        return
      }
    }
    if (k === 'f') {
      flipSelectedDoor()
      return
    }
    if (e.shiftKey || e.altKey) return
    const map: Record<string, typeof tool> = { v: 'select', w: 'wall', m: 'measure', d: 'ruler', o: 'opening', r: 'room', p: 'material', l: 'light' }
    if (map[k] && !pick) {
      if (draftRef.current.points.length) finishChain(false)
      ed.setTool(map[k])
    }
  }
  useEffect(() => {
    const down = (e: KeyboardEvent) => keyRef.current(e)
    const up = (e: KeyboardEvent) => {
      if (e.key === ' ') spaceDown.current = false
      if (e.key === 'Alt') setAltDown(false)
    }
    window.addEventListener('keydown', down, true)
    window.addEventListener('keyup', up)
    return () => {
      window.removeEventListener('keydown', down, true)
      window.removeEventListener('keyup', up)
    }
  }, [])

  // tool change: commit a pending wall chain, reset transient UI
  const prevTool = useRef(tool)
  useEffect(() => {
    if (prevTool.current === tool) return
    if (prevTool.current === 'wall' && draftRef.current.points.length >= 2) finishChain(false)
    else setDraft({ points: [], pos: [] })
    prevTool.current = tool
    setMeasure(null)
    setRuler(null)
    setTyped('')
    setSnap(null)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tool])

  // cancel a running drag when unmounting
  useEffect(
    () => () => {
      const g = gesture.current
      if (g && g.t !== 'press' && g.t !== 'pan' && g.t !== 'pinch') {
        source.endTransient(false)
        if (g.t === 'node' || g.t === 'wall') g.base.free()
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  )

  // ---------------------------------------------------------------- render helpers
  const scale = view.scale
  const fs = LABEL_PX / scale

  const highlight = (sel: Selection, cls: string): ReactNode => {
    if (!sel) return null
    switch (sel.kind) {
      case 'wall': {
        const fp = derived.footprints.find((f) => f.wall === sel.id)
        return fp ? <polygon className={cls} points={polyPoints(fp.corners)} /> : null
      }
      case 'node': {
        const p = nodes.get(sel.id)
        const r = 7 / scale
        return p ? <rect className={cls} x={p[0] - r} y={p[1] - r} width={2 * r} height={2 * r} /> : null
      }
      case 'opening': {
        const o = floor.openings.find((x) => x.id === sel.id)
        const w = o && floor.walls.find((x) => x.id === o.wall)
        const g = o && w && openingGeom(nodes, w, o)
        if (!g) return null
        const ht = g.t / 2 + 4 / scale
        const pts = [add(g.p0, mul(g.n, ht)), add(g.p1, mul(g.n, ht)), add(g.p1, mul(g.n, -ht)), add(g.p0, mul(g.n, -ht))]
        return <polygon className={cls} points={polyPoints(pts)} />
      }
      case 'room': {
        const r = sel.id.startsWith('derived:') ? derived.rooms.find((x) => `derived:${x.key}` === sel.id) : derived.rooms.find((x) => x.meta === sel.id)
        return r ? <polygon className={`${cls} room`} points={polyPoints(r.polygon)} /> : null
      }
      case 'furniture': {
        const f = project.furniture.find((x) => x.id === sel.id)
        return f ? <polygon className={cls} points={polyPoints(furnitureCorners(f))} /> : null
      }
      case 'light': {
        const l = project.lights.find((x) => x.id === sel.id)
        return l ? <circle className={cls} cx={lightWorldPos(project, l)[0]} cy={lightWorldPos(project, l)[1]} r={11 / scale} /> : null
      }
    }
  }

  const lowConfidence = useMemo(() => {
    if (!confidence) return null
    const low = new Set(Object.entries(confidence).filter(([, c]) => c < LOW_CONF).map(([id]) => id))
    if (!low.size) return null
    const out: ReactNode[] = []
    for (const fp of derived.footprints) if (low.has(fp.wall)) out.push(<polygon key={fp.wall} className="pl-lowconf" points={polyPoints(fp.corners)} />)
    for (const o of floor.openings) {
      if (!low.has(o.id)) continue
      const w = floor.walls.find((x) => x.id === o.wall)
      const g = w && openingGeom(nodes, w, o)
      if (!g) continue
      const ht = g.t / 2 + 60
      out.push(<polygon key={o.id} className="pl-lowconf" points={polyPoints([add(g.p0, mul(g.n, ht)), add(g.p1, mul(g.n, ht)), add(g.p1, mul(g.n, -ht)), add(g.p0, mul(g.n, -ht))])} />)
    }
    for (const r of floor.rooms) {
      if (!low.has(r.id)) continue
      const dr = derived.rooms.find((x) => x.meta === r.id)
      if (dr) out.push(<polygon key={r.id} className="pl-lowconf room" points={polyPoints(dr.polygon)} />)
    }
    return out
  }, [confidence, derived, floor, nodes])

  const clearanceLines = (() => {
    if (!clearanceItem || !clearances) return null
    const f = clearanceItem
    const c: Vec = [f.pos_mm[0], f.pos_mm[1]]
    const [w, d] = f.dims_mm
    const sides: [keyof Clearances, Vec, number][] = [
      ['right', [1, 0], w / 2],
      ['left', [-1, 0], w / 2],
      ['front', [0, 1], d / 2],
      ['back', [0, -1], d / 2],
    ]
    return sides.map(([k, dir, half]) => {
      const v = clearances[k]
      if (v === null || v === undefined || v > 6000 || v < 1) return null
      const dw = rotate(dir, f.rotation_deg)
      const a = add(c, mul(dw, half))
      const b = add(a, mul(dw, v))
      const m = add(a, mul(dw, v / 2))
      const tick = mul([dw[1], -dw[0]], 5 / scale)
      return (
        <g key={k} className="pl-clearance">
          <line x1={a[0]} y1={a[1]} x2={b[0]} y2={b[1]} />
          <line x1={b[0] - tick[0]} y1={b[1] - tick[1]} x2={b[0] + tick[0]} y2={b[1] + tick[1]} />
          <text x={m[0]} y={m[1]} fontSize={fs} textAnchor="middle" dominantBaseline="central" className="halo">
            {fmtLength(v)}
          </text>
          <text x={m[0]} y={m[1]} fontSize={fs} textAnchor="middle" dominantBaseline="central">
            {fmtLength(v)}
          </text>
        </g>
      )
    })
  })()

  const wallPreview = (() => {
    if (tool !== 'wall' || pick) return null
    const pts = [...draft.pos]
    let target = snap?.p ?? cursor
    if (typed && draft.pos.length) {
      const mm = parseLength(typed)
      const last = draft.pos[draft.pos.length - 1]
      if (mm && target) target = pointAtLength(last, target, mm)
    }
    if (target) pts.push(target)
    const t = draw.wallThicknessMm
    const last = draft.pos[draft.pos.length - 1]
    const segLen = last && target ? dist(last, target) : 0
    const mid = last && target ? mul(add(last, target), 0.5) : null
    return (
      <g className="pl-draft" pointerEvents="none">
        {pts.length >= 2 && <polyline className="body" points={polyPoints(pts)} strokeWidth={t} />}
        {pts.length >= 2 && <polyline className="axis" points={polyPoints(pts)} />}
        {snap?.guide && <line className="guide" x1={snap.guide[0][0]} y1={snap.guide[0][1]} x2={snap.guide[1][0]} y2={snap.guide[1][1]} />}
        {draft.pos.map((p, i) => (
          <circle key={i} className={`pt${i === 0 ? ' first' : ''}`} cx={p[0]} cy={p[1]} r={4 / scale} />
        ))}
        {target && <SnapMarker p={target} kind={snap?.kind ?? 'free'} scale={scale} />}
        {mid && segLen > 1 && (
          <text x={mid[0]} y={mid[1] - 14 / scale} fontSize={fs} textAnchor="middle" className="seglen">
            {fmtLength(Math.round(segLen))}
          </text>
        )}
      </g>
    )
  })()

  const rulerMarks = (() => {
    if (tool !== 'ruler' || pick) return null
    const live = snap && cursor ? snap.p : cursor
    const a = ruler?.a
    const b = ruler ? (ruler.b ?? live) : null
    const len = a && b ? dist(a, b) : 0
    const dx = a && b ? Math.abs(b[0] - a[0]) : 0
    const dy = a && b ? Math.abs(b[1] - a[1]) : 0
    const mid = a && b ? mul(add(a, b), 0.5) : null
    const tick = 6 / scale
    const n = a && b && len > 1 ? mul(norm(sub(b, a)), tick) : null
    return (
      <g className="pl-ruler" pointerEvents="none">
        {a && b && <line className="line" x1={a[0]} y1={a[1]} x2={b[0]} y2={b[1]} />}
        {n &&
          [a!, b!].map((p, i) => <line key={i} className="tick" x1={p[0] - n[1]} y1={p[1] + n[0]} x2={p[0] + n[1]} y2={p[1] - n[0]} />)}
        {a && <circle className="pt" cx={a[0]} cy={a[1]} r={3.5 / scale} />}
        {ruler?.b && <circle className="pt" cx={ruler.b[0]} cy={ruler.b[1]} r={3.5 / scale} />}
        {!ruler?.b && live && <SnapMarker p={live} kind={snap?.kind ?? 'free'} scale={scale} />}
        {mid && len > 1 && (
          <text x={mid[0]} y={mid[1] - 10 / scale} fontSize={fs * 1.15} textAnchor="middle" className="len">
            {fmtLength(Math.round(len))}
            {dx > 5 && dy > 5 && <tspan className="parts" fontSize={fs * 0.85}>{`  ↔ ${fmtLength(Math.round(dx))} ↕ ${fmtLength(Math.round(dy))}`}</tspan>}
          </text>
        )}
      </g>
    )
  })()

  const measureMarks = (() => {
    if (!measure) return null
    const w = floor.walls.find((x) => x.id === measure.wall)
    const p = w && wallPoints(nodes, w)
    if (!p) return null
    const fixed = measure.fixedA ? p[0] : p[1]
    const moving = measure.fixedA ? p[1] : p[0]
    const r = 8 / scale
    return (
      <g pointerEvents="none" className="pl-measure">
        <rect className="fixed" x={fixed[0] - r} y={fixed[1] - r} width={2 * r} height={2 * r} />
        <circle className="moving" cx={moving[0]} cy={moving[1]} r={r} />
        <text x={p[0][0]} y={p[0][1] - 12 / scale} fontSize={fs} textAnchor="middle">
          A
        </text>
        <text x={p[1][0]} y={p[1][1] - 12 / scale} fontSize={fs} textAnchor="middle">
          B
        </text>
      </g>
    )
  })()

  const measureLabel = measure ? dimLabels.find((l) => l.wall === measure.wall) : undefined
  const measureScreen = measure
    ? (() => {
        if (measureLabel) return toScreen(view, measureLabel.pos)
        const w = floor.walls.find((x) => x.id === measure.wall)
        const p = w && wallPoints(nodes, w)
        return p ? toScreen(view, mul(add(p[0], p[1]), 0.5)) : null
      })()
    : null

  const openingGhost = tool === 'opening' && cursor && !pick ? openingAt(cursor) : null
  const refImg = floor.reference_image
  const showNodes = (tool === 'select' || tool === 'wall') && !pick && scale > 0.02
  const cursorClass = pick || tool === 'ruler' ? 'crosshair' : tool === 'select' ? (dragInfo ? 'grabbing' : 'default') : tool === 'measure' ? 'pointer' : 'crosshair'
  const typedMm = typed ? parseLength(typed) : null
  const typedScreen = typed && cursor ? toScreen(view, cursor) : null

  return (
    <div ref={wrapRef} className={`plan-canvas tool-${tool}`} style={{ cursor: cursorClass }}>
      <svg
        ref={svgRef}
        className="plan-svg"
        width={size.w}
        height={size.h}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerCancel}
        onPointerLeave={onPointerLeave}
        onDoubleClick={() => {
          if (tool === 'wall' && !pick && draft.points.length >= 2) finishChain(false)
          else if (tool === 'select' && hover?.kind === 'wall') openMeasure(hover.id)
        }}
        onContextMenu={(e) => e.preventDefault()}
      >
        <rect className="pl-bg" width={size.w} height={size.h} />
        <GridLayer view={view} w={size.w} h={size.h} />
        <g className="plan-world" transform={`translate(${view.tx} ${view.ty}) scale(${view.scale})`}>
          {refImg && showRefImage && refImg.visible && <RefImageLayer img={refImg} opacity={refOpacity ?? refImg.opacity} />}
          <RoomsLayer rooms={derived.rooms} floor={floor} />
          <WallsLayer footprints={derived.footprints} floor={floor} />
          <OpeningsLayer floor={floor} />
          <RoomLabelsLayer rooms={derived.rooms} floor={floor} scale={scale} />
          <FurnitureLayer items={project.furniture} overlaps={derived.overlaps} scale={scale} />
          <LightsLayer project={project} scale={scale} />
          {lowConfidence && <g pointerEvents="none">{lowConfidence}</g>}
          <DimensionsLayer labels={dimLabels} scale={scale} clickable={tool === 'measure' && !pick} hidden={measure?.wall} />
          {showNodes && <NodesLayer floor={floor} scale={scale} />}
          <g pointerEvents="none">
            {hover && !dragInfo && (hover.kind !== selection?.kind || hover.id !== selection?.id) && highlight(hover, 'pl-hover')}
            {highlight(selection, 'pl-selected')}
          </g>
          {clearanceLines && <g pointerEvents="none">{clearanceLines}</g>}
          {openingGhost && (
            <g pointerEvents="none" className="pl-ghost">
              <OpeningShape o={openingGhost} nodes={nodes} floor={floor} interactive={false} />
            </g>
          )}
          {placementGhost && (
            <g pointerEvents="none" className="pl-ghost">
              <FurnitureShape f={placementGhost} scale={scale} interactive={false} />
            </g>
          )}
          {dragInfo?.kind === 'node' && snap?.node && <SnapMarker p={snap.p} kind="node" scale={scale} />}
          {wallPreview}
          {measureMarks}
          {rulerMarks}
          {pick && cursor && <SnapMarker p={snap?.p ?? cursor} kind={snap?.kind ?? 'free'} scale={scale} />}
          {world?.(view)}
        </g>
      </svg>

      {typedScreen && (
        <div className="pl-typed" style={{ left: typedScreen[0] + 14, top: typedScreen[1] + 14 }}>
          {typed}
          <span className="muted"> → {typedMm ? fmtLength(typedMm) : '?'} · Enter</span>
        </div>
      )}

      {measure && measureScreen && (
        <form
          className="pl-measure-edit card"
          style={{ left: Math.min(Math.max(8, measureScreen[0] - 110), size.w - 230), top: Math.min(Math.max(8, measureScreen[1] + 14), size.h - 110) }}
          onSubmit={(e) => {
            e.preventDefault()
            commitMeasure()
          }}
          onPointerDown={(e) => e.stopPropagation()}
        >
          <label>
            Neue Länge
            <input autoFocus value={measure.text} onFocus={(e) => e.target.select()} onChange={(e) => setMeasure({ ...measure, text: e.target.value })} onKeyDown={(e) => e.key === 'Escape' && setMeasure(null)} />
          </label>
          <div className="row">
            <span className="muted small">Fix:</span>
            <button type="button" aria-pressed={measure.fixedA} onClick={() => setMeasure({ ...measure, fixedA: true })} title="Ende A bleibt stehen">
              A
            </button>
            <button type="button" aria-pressed={!measure.fixedA} onClick={() => setMeasure({ ...measure, fixedA: false })} title="Ende B bleibt stehen">
              B
            </button>
            <button type="submit" className="primary">
              OK
            </button>
          </div>
        </form>
      )}

      <div className="pl-zoom">
        <button className="ghost" title="Vergrössern" onClick={() => setView(zoomAt(view, [size.w / 2, size.h / 2], 1.3))}>
          +
        </button>
        <button className="ghost" title="Verkleinern" onClick={() => setView(zoomAt(view, [size.w / 2, size.h / 2], 1 / 1.3))}>
          −
        </button>
        <button className="ghost" title="Zoom auf alles (Pos1 / ⇧1)" onClick={fitAll}>
          ⤢
        </button>
      </div>
      <div className="pl-status">
        {pick?.hint ?? (tool === 'furniture' && !asset ? 'Möbel im Möbel-Panel wählen' : TOOL_HINT[tool])}
        {selection && tool === 'select' && <span className="sel"> · {KIND_LABEL[selection.kind]} gewählt</span>}
      </div>
    </div>
  )
}

function SnapMarker({ p, kind, scale }: { p: Vec; kind: string; scale: number }) {
  const r = 6 / scale
  if (kind === 'node') return <rect className="pl-snap node" x={p[0] - r} y={p[1] - r} width={2 * r} height={2 * r} pointerEvents="none" />
  if (kind === 'mid') return <path className="pl-snap mid" d={`M${p[0]} ${p[1] - r}L${p[0] + r} ${p[1] + r}L${p[0] - r} ${p[1] + r}Z`} pointerEvents="none" />
  if (kind === 'wall') return <path className="pl-snap wall" d={`M${p[0] - r} ${p[1] - r}L${p[0] + r} ${p[1] + r}M${p[0] - r} ${p[1] + r}L${p[0] + r} ${p[1] - r}`} pointerEvents="none" />
  return <circle className={`pl-snap ${kind}`} cx={p[0]} cy={p[1]} r={r * 0.6} pointerEvents="none" />
}

