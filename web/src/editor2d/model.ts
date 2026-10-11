// Model lookups and op builders for the 2D editor.
import { newId } from '../core/engine'
import type { DerivedRoom } from '../core/generated/DerivedRoom'
import type { Floor } from '../core/generated/Floor'
import type { FurnitureInstance } from '../core/generated/FurnitureInstance'
import type { Light } from '../core/generated/Light'
import type { Op } from '../core/generated/Op'
import type { Opening } from '../core/generated/Opening'
import type { Project } from '../core/generated/Project'
import type { RoomMeta } from '../core/generated/RoomMeta'
import type { Wall } from '../core/generated/Wall'
import type { Selection, SelectionKind } from '../state/editor'
import { add, boundsOf, dist, leftNormal, mul, norm, obb, pointInPolygon, polygonArea, rotate, sub, type Bounds, type Vec } from './geom'
import type { PlanSource } from './source'

export type NodeMap = Map<string, Vec>

export function nodeMap(f: Floor): NodeMap {
  return new Map(f.nodes.map((n) => [n.id, [n.x_mm, n.y_mm] as Vec]))
}

export function wallPoints(nodes: NodeMap, w: Wall): [Vec, Vec] | null {
  const a = nodes.get(w.a)
  const b = nodes.get(w.b)
  return a && b ? [a, b] : null
}

/** Geometry of an opening along its wall. */
export type OpeningGeom = { p0: Vec; p1: Vec; d: Vec; n: Vec; t: number; wallLen: number }

export function openingGeom(nodes: NodeMap, wall: Wall, o: Opening): OpeningGeom | null {
  const pts = wallPoints(nodes, wall)
  if (!pts) return null
  const [a, b] = pts
  const d = norm(sub(b, a))
  return {
    p0: add(a, mul(d, o.offset_mm)),
    p1: add(a, mul(d, o.offset_mm + o.width_mm)),
    d,
    n: leftNormal(d),
    t: wall.thickness_mm,
    wallLen: dist(a, b),
  }
}

/** World position of a light (attached lights are relative to their furniture). */
export function lightWorldPos(p: Project, l: Light): Vec {
  if (l.attached_to) {
    const f = p.furniture.find((x) => x.id === l.attached_to)
    if (f) return add([f.pos_mm[0], f.pos_mm[1]], rotate([l.pos_mm[0], l.pos_mm[1]], f.rotation_deg))
  }
  return [l.pos_mm[0], l.pos_mm[1]]
}

export function furnitureCorners(f: FurnitureInstance): Vec[] {
  return obb([f.pos_mm[0], f.pos_mm[1]], f.dims_mm[0], f.dims_mm[1], f.rotation_deg)
}

/** Occupied parts of the box (e.g. the arms of an L-shaped wardrobe); empty if it fills the whole box. Mirrors plan-core `footprint_parts`. */
export function furnitureParts(f: FurnitureInstance): Vec[][] {
  const [w, d] = f.dims_mm
  return f.footprint.map(([x0, y0, x1, y1]) => {
    const local: Vec = [((x0 + x1) / 2 - 0.5) * w, ((y0 + y1) / 2 - 0.5) * d]
    return obb(add([f.pos_mm[0], f.pos_mm[1]], rotate(local, f.rotation_deg)), (x1 - x0) * w, (y1 - y0) * d, f.rotation_deg)
  })
}

/** Kind of the element with this id (searches the first floor + project lists). */
export function kindOf(p: Project, id: string): SelectionKind | null {
  const f = p.floors[0]
  if (f.walls.some((w) => w.id === id)) return 'wall'
  if (f.openings.some((o) => o.id === id)) return 'opening'
  if (f.rooms.some((r) => r.id === id)) return 'room'
  if (f.nodes.some((n) => n.id === id)) return 'node'
  if (p.furniture.some((x) => x.id === id)) return 'furniture'
  if (p.lights.some((x) => x.id === id)) return 'light'
  return null
}

export function derivedRoomOfMeta(rooms: DerivedRoom[], metaId: string): DerivedRoom | undefined {
  return rooms.find((r) => r.meta === metaId)
}

/** Smallest derived room containing p. */
export function roomAt(rooms: DerivedRoom[], p: Vec): DerivedRoom | null {
  let best: DerivedRoom | null = null
  let bestA = Infinity
  for (const r of rooms) {
    if (!pointInPolygon(p, r.polygon)) continue
    const a = polygonArea(r.polygon)
    if (a < bestA) {
      bestA = a
      best = r
    }
  }
  return best
}

/** Bounds of the elements with the given ids. */
export function elementsBounds(p: Project, rooms: DerivedRoom[], ids: string[]): Bounds | null {
  const f = p.floors[0]
  const nodes = nodeMap(f)
  const pts: Vec[] = []
  for (const id of ids) {
    const n = nodes.get(id)
    if (n) pts.push(n)
    const w = f.walls.find((x) => x.id === id)
    if (w) {
      const wp = wallPoints(nodes, w)
      if (wp) pts.push(...wp)
    }
    const o = f.openings.find((x) => x.id === id)
    if (o) {
      const ww = f.walls.find((x) => x.id === o.wall)
      const g = ww && openingGeom(nodes, ww, o)
      if (g) pts.push(g.p0, g.p1)
    }
    const r = f.rooms.find((x) => x.id === id)
    if (r) {
      const dr = derivedRoomOfMeta(rooms, r.id)
      if (dr) pts.push(...(dr.polygon as Vec[]))
      else pts.push(r.anchor)
    }
    const fu = p.furniture.find((x) => x.id === id)
    if (fu) pts.push(...furnitureCorners(fu))
    const l = p.lights.find((x) => x.id === id)
    if (l) pts.push(lightWorldPos(p, l))
  }
  return boundsOf(pts)
}

export function allBounds(p: Project, rooms: DerivedRoom[]): Bounds | null {
  const f = p.floors[0]
  const pts: Vec[] = f.nodes.map((n) => [n.x_mm, n.y_mm])
  for (const fu of p.furniture) pts.push(...furnitureCorners(fu))
  for (const r of rooms) pts.push(...(r.polygon as Vec[]))
  const img = f.reference_image
  if (img && pts.length === 0) {
    pts.push(img.origin_mm, [img.origin_mm[0] + img.width_px * img.mm_per_px, img.origin_mm[1] + img.height_px * img.mm_per_px])
  }
  return boundsOf(pts)
}

/** Ops deleting the selected element, with a German label. */
export function deleteOps(src: PlanSource, sel: NonNullable<Selection>): [Op[], string] | null {
  const floor = src.project.floors[0].id
  const e = src.engine
  try {
    switch (sel.kind) {
      case 'node':
        return [e.cmd_delete_node(floor, sel.id) as Op[], 'Ecke löschen']
      case 'wall':
        return [e.cmd_delete_wall(floor, sel.id) as Op[], 'Wand löschen']
      case 'opening':
        return [[{ op: 'DeleteOpening', floor, id: sel.id }], 'Öffnung löschen']
      case 'room':
        if (!src.project.floors[0].rooms.some((r) => r.id === sel.id)) return null
        return [[{ op: 'DeleteRoom', floor, id: sel.id }], 'Raum löschen']
      case 'furniture':
        return [e.cmd_delete_furniture(sel.id) as Op[], 'Möbel löschen']
      case 'light':
        return [[{ op: 'DeleteLight', id: sel.id }], 'Lampe löschen']
    }
  } catch (err) {
    console.error(err)
    return null
  }
}

/** Flip a door: cycles swing, then hinge (4 states). */
export function flipDoor(o: Opening): Opening | null {
  if (o.kind.type !== 'Door') return null
  const { hinge, swing, glass } = o.kind
  const next =
    swing === 'Left'
      ? { hinge, swing: 'Right' as const }
      : { hinge: hinge === 'Start' ? ('End' as const) : ('Start' as const), swing: 'Left' as const }
  return { ...o, kind: { type: 'Door', ...next, glass } }
}

export function newRoomMeta(anchor: Vec, n: number, floorMaterial = 'wood:oak'): RoomMeta {
  return {
    id: newId(),
    anchor: [Math.round(anchor[0]), Math.round(anchor[1])],
    name: `Raum ${n}`,
    floor_material: floorMaterial,
    ceiling_material: 'paint:white',
    open_ceiling: false,
  }
}

export function newCeilingLight(p: Project, at: Vec): Light {
  const h = p.floors[0].ceiling_height_mm
  return {
    id: newId(),
    name: 'Deckenleuchte',
    kind: { type: 'Point' },
    pos_mm: [Math.round(at[0]), Math.round(at[1]), Math.max(0, h - 100)],
    lumen: 800,
    color_temp_k: 2700,
    on: true,
    attached_to: null,
  }
}

/** Which side of wall a→b the point lies on. */
export function wallSideAt(a: Vec, b: Vec, p: Vec): 'left' | 'right' {
  const n = leftNormal(norm(sub(b, a)))
  return (p[0] - a[0]) * n[0] + (p[1] - a[1]) * n[1] >= 0 ? 'left' : 'right'
}
