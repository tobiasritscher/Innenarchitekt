// Snapping for the wall tool and node drags. Pure functions (plan mm).
import { closestOnSegment, dist, lineIntersect, norm, sub, type Vec } from './geom'

export type SnapNode = { id: string; p: Vec }
export type SnapWall = { id: string; a: Vec; b: Vec }

export type SnapKind = 'node' | 'mid' | 'wall' | 'perp' | 'angle' | 'grid' | 'free'

export type Snapped = {
  p: Vec
  kind: SnapKind
  node?: string
  wall?: string
  /** Guide line to draw (e.g. angle ray from the previous point). */
  guide?: [Vec, Vec]
}

export type SnapOptions = {
  enabled: boolean
  /** Tolerance in mm (usually ~10 px converted). */
  tol: number
  /** Previous chain point (enables angle/perpendicular snapping). */
  prev?: Vec
  grid?: number
  angleStep?: number
  excludeNodes?: ReadonlySet<string>
  excludeWalls?: ReadonlySet<string>
}

const round = (p: Vec): Vec => [Math.round(p[0]), Math.round(p[1])]

export function snapToGrid(p: Vec, grid: number): Vec {
  return [Math.round(p[0] / grid) * grid, Math.round(p[1] / grid) * grid]
}

/** Angle (deg, screen clockwise from +x) of v. */
export function angleOf(v: Vec): number {
  return (Math.atan2(v[1], v[0]) * 180) / Math.PI
}

function angleDiff(a: number, b: number): number {
  let d = (((a - b) % 360) + 540) % 360 - 180
  if (d < -180) d += 360
  return Math.abs(d)
}

/**
 * Direction from `prev` towards `raw`, snapped to the nearest preferred direction:
 * parallel/perpendicular to `refDirs` (within 4°) or else `angleStep` steps.
 * The length is rounded to `grid`.
 */
export function snapDirection(prev: Vec, raw: Vec, angleStep: number, grid: number, refDirs: Vec[] = []): { p: Vec; kind: 'perp' | 'angle'; dir: Vec } {
  const v = sub(raw, prev)
  const l = Math.hypot(v[0], v[1])
  const a = angleOf(v)
  let best = Math.round(a / angleStep) * angleStep
  let kind: 'perp' | 'angle' = 'angle'
  let bestDiff = 4
  for (const r of refDirs) {
    const ra = angleOf(r)
    for (const k of [0, 90, 180, 270]) {
      const cand = ra + k
      const d = angleDiff(a, cand)
      if (d < bestDiff && angleDiff(cand, Math.round(cand / angleStep) * angleStep) > 0.01) {
        bestDiff = d
        best = cand
        kind = 'perp'
      }
    }
  }
  const r = (best * Math.PI) / 180
  const dir: Vec = [Math.cos(r), Math.sin(r)]
  const lr = Math.max(grid, Math.round(l / grid) * grid)
  return { p: [prev[0] + dir[0] * lr, prev[1] + dir[1] * lr], kind, dir }
}

/** Snap a raw cursor point. Priority: node > wall midpoint > point on wall > angle/perpendicular > grid. */
export function snapPoint(raw: Vec, nodes: readonly SnapNode[], walls: readonly SnapWall[], o: SnapOptions): Snapped {
  if (!o.enabled) return { p: round(raw), kind: 'free' }
  const grid = o.grid ?? 10
  const step = o.angleStep ?? 15

  let bestNode: SnapNode | null = null
  let bd = o.tol
  for (const n of nodes) {
    if (o.excludeNodes?.has(n.id)) continue
    const d = dist(n.p, raw)
    if (d < bd) {
      bd = d
      bestNode = n
    }
  }
  if (bestNode) return { p: round(bestNode.p), kind: 'node', node: bestNode.id }

  let mid: { w: SnapWall; p: Vec } | null = null
  bd = o.tol
  for (const w of walls) {
    if (o.excludeWalls?.has(w.id)) continue
    const m: Vec = [(w.a[0] + w.b[0]) / 2, (w.a[1] + w.b[1]) / 2]
    const d = dist(m, raw)
    if (d < bd) {
      bd = d
      mid = { w, p: m }
    }
  }
  if (mid) return { p: round(mid.p), kind: 'mid', wall: mid.w.id }

  // walls touching the previous point (for perpendicular snapping) and nearest wall to cursor
  const refDirs: Vec[] = []
  let near: { w: SnapWall; q: Vec; d: number } | null = null
  for (const w of walls) {
    if (o.excludeWalls?.has(w.id)) continue
    const [t, q, d] = closestOnSegment(raw, w.a, w.b)
    if (d < o.tol && t > 0 && t < 1 && (!near || d < near.d)) near = { w, q, d }
    if (o.prev && (dist(w.a, o.prev) < 1 || dist(w.b, o.prev) < 1)) refDirs.push(norm(sub(w.b, w.a)))
  }
  if (near) refDirs.push(norm(sub(near.w.b, near.w.a)))

  if (near) {
    // try to keep the snapped angle while landing on the wall
    if (o.prev) {
      const sd = snapDirection(o.prev, raw, step, 1, refDirs)
      const x = lineIntersect(o.prev, sd.dir, near.w.a, sub(near.w.b, near.w.a))
      if (x && dist(x, raw) < o.tol * 1.5) {
        const [t] = closestOnSegment(x, near.w.a, near.w.b)
        if (t > 0 && t < 1) return { p: round(x), kind: 'wall', wall: near.w.id, guide: [o.prev, x] }
      }
    }
    return { p: round(near.q), kind: 'wall', wall: near.w.id }
  }

  if (o.prev && dist(o.prev, raw) > 1) {
    const sd = snapDirection(o.prev, raw, step, grid, refDirs)
    return { p: round(sd.p), kind: sd.kind, guide: [o.prev, sd.p] }
  }
  return { p: snapToGrid(raw, grid), kind: 'grid' }
}

/** Point at `length` from `from` in the direction of `towards` (CAD-style typed length). */
export function pointAtLength(from: Vec, towards: Vec, length: number): Vec {
  const d = norm(sub(towards, from))
  if (d[0] === 0 && d[1] === 0) return [Math.round(from[0] + length), Math.round(from[1])]
  return [Math.round(from[0] + d[0] * length), Math.round(from[1] + d[1] * length)]
}
