// Memoised SVG layers of the plan (world coordinates, mm). Strokes are screen-constant
// via `vector-effect: non-scaling-stroke` (CSS); text sizes are divided by the view scale.
import { memo, useMemo, type ReactNode } from 'react'
import type { DerivedRoom } from '../core/generated/DerivedRoom'
import type { Floor } from '../core/generated/Floor'
import type { FurnitureInstance } from '../core/generated/FurnitureInstance'
import type { Light } from '../core/generated/Light'
import type { Opening } from '../core/generated/Opening'
import type { Project } from '../core/generated/Project'
import type { RefImage } from '../core/generated/RefImage'
import type { WallFootprint } from '../core/generated/WallFootprint'
import { fmtArea, fmtLength } from '../core/format'
import { resolveMaterial } from '../core/materials'
import { add, dist, leftNormal, mul, norm, pointInPolygon, polyPoints, rotate, sub, type Vec } from './geom'
import { podiumSteps } from '../core/furniture'
import { furnitureCorners, furnitureParts, lightWorldPos, nodeMap, openingGeom, wallPoints, type NodeMap } from './model'
import { gridLevels, toWorld, type View } from './view'

/** Label font size in px (screen). */
export const LABEL_PX = 11

// ------------------------------------------------------------------ grid (screen space)

export const GridLayer = memo(function GridLayer({ view, w, h }: { view: View; w: number; h: number }) {
  const [x0, y0] = toWorld(view, [0, 0])
  const [x1, y1] = toWorld(view, [w, h])
  const paths: ReactNode[] = []
  for (const { step, alpha } of gridLevels(view.scale)) {
    let d = ''
    const sx = Math.floor(x0 / step) * step
    const sy = Math.floor(y0 / step) * step
    for (let x = sx; x <= x1; x += step) {
      const px = Math.round(x * view.scale + view.tx) + 0.5
      d += `M${px} 0V${h}`
    }
    for (let y = sy; y <= y1; y += step) {
      const py = Math.round(y * view.scale + view.ty) + 0.5
      d += `M0 ${py}H${w}`
    }
    paths.push(<path key={step} d={d} className={`pl-grid pl-grid-${step}`} style={{ opacity: alpha }} />)
  }
  return <g pointerEvents="none">{paths}</g>
})

// ------------------------------------------------------------------ reference image

export const RefImageLayer = memo(function RefImageLayer({ img, opacity }: { img: RefImage; opacity: number }) {
  return (
    <image
      className="pl-refimg"
      href={img.url}
      x={img.origin_mm[0]}
      y={img.origin_mm[1]}
      width={img.width_px * img.mm_per_px}
      height={img.height_px * img.mm_per_px}
      opacity={opacity}
      preserveAspectRatio="none"
      pointerEvents="none"
    />
  )
})

// ------------------------------------------------------------------ rooms

export const RoomsLayer = memo(function RoomsLayer({ rooms, floor }: { rooms: DerivedRoom[]; floor: Floor }) {
  const metas = useMemo(() => new Map(floor.rooms.map((r) => [r.id, r])), [floor.rooms])
  return (
    <g>
      {rooms.map((r) => {
        const meta = r.meta ? metas.get(r.meta) : undefined
        const color = meta ? resolveMaterial(meta.floor_material).color : undefined
        return (
          <polygon
            key={r.key}
            data-k="room"
            data-id={r.meta ?? `derived:${r.key}`}
            className={`pl-room${meta ? '' : ' unnamed'}`}
            points={polyPoints(r.polygon)}
            style={color ? { fill: color } : undefined}
          />
        )
      })}
    </g>
  )
})

export const RoomLabelsLayer = memo(function RoomLabelsLayer({ rooms, floor, scale }: { rooms: DerivedRoom[]; floor: Floor; scale: number }) {
  const fs = LABEL_PX / scale
  return (
    <g pointerEvents="none">
      {rooms.map((r) => {
        const meta = r.meta ? floor.rooms.find((m) => m.id === r.meta) : undefined
        if (r.area_m2 * 1e6 * scale * scale < 1500) return null
        return (
          <text key={r.key} className="pl-room-label" x={r.label[0]} y={r.label[1]} fontSize={fs} textAnchor="middle">
            <tspan x={r.label[0]} dy={-0.2 * fs} className="name">
              {meta?.name ?? 'Raum'}
            </tspan>
            <tspan x={r.label[0]} dy={1.25 * fs} className="area">
              {fmtArea(r.area_m2)}
            </tspan>
          </text>
        )
      })}
    </g>
  )
})

// ------------------------------------------------------------------ walls

export const WallsLayer = memo(function WallsLayer({ footprints, floor }: { footprints: WallFootprint[]; floor: Floor }) {
  const kinds = useMemo(() => new Map(floor.walls.map((w) => [w.id, w])), [floor.walls])
  return (
    <g>
      {footprints.map((fp) => (
        <polygon
          key={fp.wall}
          data-k="wall"
          data-id={fp.wall}
          className={`pl-wall${kinds.get(fp.wall)?.kind === 'Exterior' ? ' ext' : ''}${kinds.get(fp.wall)?.glazed ? ' glass' : ''}${kinds.get(fp.wall)?.height_mm != null && kinds.get(fp.wall)!.height_mm! < floor.ceiling_height_mm ? ' low' : ''}`}
          points={polyPoints(fp.corners)}
        />
      ))}
    </g>
  )
})

// ------------------------------------------------------------------ openings

/** SVG for one opening (also used for the placement ghost). */
export function OpeningShape({ o, nodes, floor, className = '', interactive = true }: { o: Opening; nodes: NodeMap; floor: Floor; className?: string; interactive?: boolean }) {
  const wall = floor.walls.find((w) => w.id === o.wall)
  if (!wall) return null
  const g = openingGeom(nodes, wall, o)
  if (!g) return null
  const { p0, p1, d, n, t } = g
  const ht = t / 2 + 1
  const cut = [add(p0, mul(n, ht)), add(p1, mul(n, ht)), add(p1, mul(n, -ht)), add(p0, mul(n, -ht))]
  const props = interactive ? { 'data-k': 'opening', 'data-id': o.id } : {}
  const parts: ReactNode[] = [<polygon key="cut" className="pl-open-cut" points={polyPoints(cut)} />]
  if (o.kind.type === 'Door') {
    const hingeAtStart = o.kind.hinge === 'Start'
    const side = o.kind.swing === 'Left' ? 1 : -1
    const sn = mul(n, side)
    const h = add(hingeAtStart ? p0 : p1, mul(sn, t / 2))
    const other = add(hingeAtStart ? p1 : p0, mul(sn, t / 2))
    const leafEnd = add(h, mul(sn, o.width_mm))
    // arc from the open leaf end to the closed position; sweep depends on orientation
    const cr = (leafEnd[0] - h[0]) * (other[1] - h[1]) - (leafEnd[1] - h[1]) * (other[0] - h[0])
    const sweep = cr > 0 ? 1 : 0
    parts.push(<line key="leaf" className="pl-door-leaf" x1={h[0]} y1={h[1]} x2={leafEnd[0]} y2={leafEnd[1]} />)
    parts.push(<path key="arc" className="pl-door-arc" d={`M${leafEnd[0]} ${leafEnd[1]} A${o.width_mm} ${o.width_mm} 0 0 ${sweep} ${other[0]} ${other[1]}`} />)
    // jambs
    parts.push(<line key="j0" className="pl-jamb" x1={cut[0][0]} y1={cut[0][1]} x2={cut[3][0]} y2={cut[3][1]} />)
    parts.push(<line key="j1" className="pl-jamb" x1={cut[1][0]} y1={cut[1][1]} x2={cut[2][0]} y2={cut[2][1]} />)
  } else if (o.kind.type === 'Window') {
    const q = t / 6
    for (const s of [-1, 1]) {
      const a = add(p0, mul(n, s * q))
      const b = add(p1, mul(n, s * q))
      parts.push(<line key={`g${s}`} className="pl-window" x1={a[0]} y1={a[1]} x2={b[0]} y2={b[1]} />)
    }
    parts.push(<polygon key="frame" className="pl-window-frame" points={polyPoints(cut)} />)
  } else {
    parts.push(<line key="j0" className="pl-jamb" x1={cut[0][0]} y1={cut[0][1]} x2={cut[3][0]} y2={cut[3][1]} />)
    parts.push(<line key="j1" className="pl-jamb" x1={cut[1][0]} y1={cut[1][1]} x2={cut[2][0]} y2={cut[2][1]} />)
    const m0 = add(p0, mul(d, 0))
    parts.push(<line key="pass" className="pl-passage" x1={m0[0]} y1={m0[1]} x2={p1[0]} y2={p1[1]} />)
  }
  // generous invisible hit area
  parts.push(<polygon key="hit" className="pl-hit" points={polyPoints(cut)} />)
  return (
    <g className={`pl-opening ${className}`} {...props}>
      {parts}
    </g>
  )
}

export const OpeningsLayer = memo(function OpeningsLayer({ floor }: { floor: Floor }) {
  const nodes = useMemo(() => nodeMap(floor), [floor])
  return (
    <g>
      {floor.openings.map((o) => (
        <OpeningShape key={o.id} o={o} nodes={nodes} floor={floor} />
      ))}
    </g>
  )
})

// ------------------------------------------------------------------ furniture

export function FurnitureShape({ f, overlap, scale, className = '', interactive = true }: { f: FurnitureInstance; overlap?: boolean; scale: number; className?: string; interactive?: boolean }) {
  const c = furnitureCorners(f)
  const parts = furnitureParts(f)
  const fs = LABEL_PX / scale
  const [w, d] = f.dims_mm
  const showName = w * scale > f.name.length * 6 + 6 && d * scale > 16
  const props = interactive ? { 'data-k': 'furniture', 'data-id': f.id } : {}
  return (
    <g className={`pl-furn${overlap ? ' overlap' : ''} ${className}`} {...props}>
      <polygon className={parts.length ? 'box' : undefined} points={polyPoints(c)} style={f.material && !parts.length ? { fill: resolveMaterial(f.material).color } : undefined} />
      {parts.map((p, i) => (
        <polygon key={i} points={polyPoints(p)} style={f.material ? { fill: resolveMaterial(f.material).color } : undefined} />
      ))}
      {/* front edge (local +y) */}
      <line className="front" x1={c[2][0]} y1={c[2][1]} x2={c[3][0]} y2={c[3][1]} />
      {f.walkable && <StepLines f={f} />}
      {showName && (
        <text x={f.pos_mm[0]} y={f.pos_mm[1]} fontSize={fs} textAnchor="middle" dominantBaseline="central" pointerEvents="none">
          {f.name}
        </text>
      )}
    </g>
  )
}

/** Tread edges of a podium: one line per step, parallel to the front. */
function StepLines({ f }: { f: FurnitureInstance }) {
  const { rises, tread } = podiumSteps(f.dims_mm)
  const [w, d] = f.dims_mm
  const c: Vec = [f.pos_mm[0], f.pos_mm[1]]
  const lines = Array.from({ length: rises - 1 }, (_, i) => {
    const y = d / 2 - (i + 1) * tread
    return [add(c, rotate([-w / 2, y], f.rotation_deg)), add(c, rotate([w / 2, y], f.rotation_deg))]
  })
  return (
    <>
      {lines.map(([a, b], i) => (
        <line key={i} className="step" x1={a[0]} y1={a[1]} x2={b[0]} y2={b[1]} />
      ))}
    </>
  )
}

export const FurnitureLayer = memo(function FurnitureLayer({ items, overlaps, scale }: { items: FurnitureInstance[]; overlaps: string[]; scale: number }) {
  const ov = useMemo(() => new Set(overlaps), [overlaps])
  // podiums underneath everything standing on them
  const sorted = useMemo(() => [...items].sort((a, b) => Number(b.walkable) - Number(a.walkable)), [items])
  return (
    <g>
      {sorted.map((f) => (
        <FurnitureShape key={f.id} f={f} overlap={ov.has(f.id)} scale={scale} />
      ))}
    </g>
  )
})

// ------------------------------------------------------------------ lights

export function LightSymbol({ l, at, scale, className = '', interactive = true }: { l: Light; at: Vec; scale: number; className?: string; interactive?: boolean }) {
  const r = 7 / scale
  const props = interactive ? { 'data-k': 'light', 'data-id': l.id } : {}
  const k = r * 0.7
  return (
    <g className={`pl-light${l.on ? ' on' : ''} ${className}`} transform={`translate(${at[0]} ${at[1]})`} {...props}>
      {l.kind.type === 'Area' ? <rect x={-r} y={-r * 0.7} width={2 * r} height={1.4 * r} /> : <circle r={r} />}
      {l.kind.type === 'Spot' ? <circle r={r * 0.4} className="mark" /> : <path className="mark" d={`M${-k} ${-k}L${k} ${k}M${-k} ${k}L${k} ${-k}`} />}
    </g>
  )
}

export const LightsLayer = memo(function LightsLayer({ project, scale }: { project: Project; scale: number }) {
  return (
    <g>
      {project.lights.map((l) => (
        <LightSymbol key={l.id} l={l} at={lightWorldPos(project, l)} scale={scale} />
      ))}
    </g>
  )
})

// ------------------------------------------------------------------ nodes

export const NodesLayer = memo(function NodesLayer({ floor, scale }: { floor: Floor; scale: number }) {
  const s = 4.5 / scale
  return (
    <g>
      {floor.nodes.map((n) => (
        <rect key={n.id} data-k="node" data-id={n.id} className="pl-node" x={n.x_mm - s} y={n.y_mm - s} width={2 * s} height={2 * s} />
      ))}
    </g>
  )
})

// ------------------------------------------------------------------ wall dimensions

export type DimLabel = { wall: string; pos: Vec; angle: number; text: string; len: number }

/** Dimension label per wall, placed on the side that is not inside a room (outside), if any. */
export function wallDimLabels(floor: Floor, rooms: DerivedRoom[], scale: number): DimLabel[] {
  const nodes = nodeMap(floor)
  const out: DimLabel[] = []
  const off = 12 / scale
  for (const w of floor.walls) {
    const pts = wallPoints(nodes, w)
    if (!pts) continue
    const [a, b] = pts
    const l = dist(a, b)
    if (l * scale < 46) continue
    const d = norm(sub(b, a))
    const n = leftNormal(d)
    const mid: Vec = [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2]
    const probe = w.thickness_mm / 2 + 150
    const inL = rooms.some((r) => pointInPolygon(add(mid, mul(n, probe)), r.polygon))
    const inR = rooms.some((r) => pointInPolygon(add(mid, mul(n, -probe)), r.polygon))
    const side = inL && !inR ? -1 : 1
    const pos = add(mid, mul(n, side * (w.thickness_mm / 2 + off)))
    let angle = (Math.atan2(d[1], d[0]) * 180) / Math.PI
    if (angle > 90) angle -= 180
    if (angle <= -90) angle += 180
    out.push({ wall: w.id, pos, angle, text: fmtLength(Math.round(l)), len: l })
  }
  return out
}

export const DimensionsLayer = memo(function DimensionsLayer({ labels, scale, clickable, hidden }: { labels: DimLabel[]; scale: number; clickable: boolean; hidden?: string }) {
  const fs = LABEL_PX / scale
  return (
    <g className={`pl-dims${clickable ? ' clickable' : ''}`} pointerEvents={clickable ? undefined : 'none'}>
      {labels.map((l) =>
        l.wall === hidden ? null : (
          <g key={l.wall} data-k="dim" data-id={l.wall} transform={`translate(${l.pos[0]} ${l.pos[1]}) rotate(${l.angle})`}>
            <rect x={(-l.text.length * fs * 0.62) / 2 - 3 / scale} y={-fs * 0.75} width={l.text.length * fs * 0.62 + 6 / scale} height={fs * 1.5} rx={3 / scale} />
            <text fontSize={fs} textAnchor="middle" dominantBaseline="central">
              {l.text}
            </text>
          </g>
        ),
      )}
    </g>
  )
})
