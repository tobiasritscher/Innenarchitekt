// Small 2D vector helpers for the plan editor (plan mm, y down).

export type Vec = [number, number]

export const add = (a: Vec, b: Vec): Vec => [a[0] + b[0], a[1] + b[1]]
export const sub = (a: Vec, b: Vec): Vec => [a[0] - b[0], a[1] - b[1]]
export const mul = (a: Vec, s: number): Vec => [a[0] * s, a[1] * s]
export const dot = (a: Vec, b: Vec): number => a[0] * b[0] + a[1] * b[1]
export const cross = (a: Vec, b: Vec): number => a[0] * b[1] - a[1] * b[0]
export const len = (a: Vec): number => Math.hypot(a[0], a[1])
export const dist = (a: Vec, b: Vec): number => Math.hypot(a[0] - b[0], a[1] - b[1])
export const norm = (a: Vec): Vec => {
  const l = len(a)
  return l < 1e-9 ? [0, 0] : [a[0] / l, a[1] / l]
}
/** Left normal of a direction (see conventions: (d.y, −d.x)). */
export const leftNormal = (d: Vec): Vec => [d[1], -d[0]]
export const lerp = (a: Vec, b: Vec, t: number): Vec => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]

/** Rotate by `deg` (+x towards +y, i.e. clockwise on screen). */
export function rotate(p: Vec, deg: number): Vec {
  const r = (deg * Math.PI) / 180
  const c = Math.cos(r)
  const s = Math.sin(r)
  return [p[0] * c - p[1] * s, p[0] * s + p[1] * c]
}

/** Closest point on segment ab to p: [t (0..1), point, distance]. */
export function closestOnSegment(p: Vec, a: Vec, b: Vec): [number, Vec, number] {
  const ab = sub(b, a)
  const l2 = dot(ab, ab)
  const t = l2 < 1e-9 ? 0 : Math.max(0, Math.min(1, dot(sub(p, a), ab) / l2))
  const q = add(a, mul(ab, t))
  return [t, q, dist(p, q)]
}

/** Intersection of lines p + s·d and q + t·e; null if parallel. */
export function lineIntersect(p: Vec, d: Vec, q: Vec, e: Vec): Vec | null {
  const den = cross(d, e)
  if (Math.abs(den) < 1e-9) return null
  const s = cross(sub(q, p), e) / den
  return add(p, mul(d, s))
}

export function pointInPolygon(p: Vec, poly: ReadonlyArray<readonly [number, number]>): boolean {
  let inside = false
  for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) {
    const [xi, yi] = poly[i]
    const [xj, yj] = poly[j]
    if (yi > p[1] !== yj > p[1] && p[0] < ((xj - xi) * (p[1] - yi)) / (yj - yi) + xi) inside = !inside
  }
  return inside
}

export function polygonArea(poly: ReadonlyArray<readonly [number, number]>): number {
  let a = 0
  for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) a += poly[j][0] * poly[i][1] - poly[i][0] * poly[j][1]
  return Math.abs(a / 2)
}

/** Corners of an oriented rectangle (centre, width along local x, depth along local y, rotation deg). */
export function obb(c: Vec, w: number, d: number, rotDeg: number): Vec[] {
  const hw = w / 2
  const hd = d / 2
  return (
    [
      [-hw, -hd],
      [hw, -hd],
      [hw, hd],
      [-hw, hd],
    ] as Vec[]
  ).map((p) => add(c, rotate(p, rotDeg)))
}

export type Bounds = { minX: number; minY: number; maxX: number; maxY: number }

export function boundsOf(points: Iterable<readonly [number, number]>): Bounds | null {
  let b: Bounds | null = null
  for (const [x, y] of points) {
    if (!b) b = { minX: x, minY: y, maxX: x, maxY: y }
    else {
      b.minX = Math.min(b.minX, x)
      b.minY = Math.min(b.minY, y)
      b.maxX = Math.max(b.maxX, x)
      b.maxY = Math.max(b.maxY, y)
    }
  }
  return b
}

/** Normalise an angle in degrees to [0, 360). */
export function normDeg(d: number): number {
  return ((d % 360) + 360) % 360
}

export function polyPoints(pts: ReadonlyArray<readonly [number, number]>): string {
  return pts.map((p) => `${p[0]},${p[1]}`).join(' ')
}
