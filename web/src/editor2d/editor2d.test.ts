import { describe, expect, it } from 'vitest'
import { parseLength } from '../core/format'
import { angleFromUp } from '../import/Compass'
import { closestOnSegment, leftNormal, obb, pointInPolygon, rotate, type Vec } from './geom'
import { pointAtLength, snapDirection, snapPoint, snapToGrid } from './snap'
import { fitBounds, gridLevels, toScreen, toWorld, zoomAt } from './view'

const close = (a: Vec, b: Vec, eps = 1e-6) => {
  expect(a[0]).toBeCloseTo(b[0], -Math.log10(eps))
  expect(a[1]).toBeCloseTo(b[1], -Math.log10(eps))
}

describe('view transform', () => {
  it('round-trips screen ↔ world', () => {
    const v = { scale: 0.13, tx: 57, ty: -12 }
    close(toWorld(v, toScreen(v, [1234, -567])), [1234, -567])
  })
  it('zooms around the cursor', () => {
    const v = { scale: 0.1, tx: 10, ty: 20 }
    const s: Vec = [300, 200]
    const w = toWorld(v, s)
    const z = zoomAt(v, s, 2.5)
    expect(z.scale).toBeCloseTo(0.25)
    close(toScreen(z, w), s)
  })
  it('fits bounds centred', () => {
    const v = fitBounds({ minX: 0, minY: 0, maxX: 10000, maxY: 5000 }, 1000, 600, 50)
    close(toScreen(v, [5000, 2500]), [500, 300])
    expect(10000 * v.scale).toBeLessThanOrEqual(900 + 1e-9)
  })
  it('fades the 10 cm grid out when zoomed out', () => {
    expect(gridLevels(0.2).map((g) => g.step)).toContain(100)
    expect(gridLevels(0.02).map((g) => g.step)).not.toContain(100)
    expect(gridLevels(0.02).map((g) => g.step)).toContain(1000)
  })
})

describe('geometry', () => {
  it('left normal follows the convention (d.y, −d.x)', () => {
    expect(leftNormal([1, 0])).toEqual([0, -1])
  })
  it('rotates clockwise on screen (+x → +y)', () => {
    close(rotate([1, 0], 90), [0, 1])
  })
  it('projects onto segments', () => {
    const [t, q, d] = closestOnSegment([5, 3], [0, 0], [10, 0])
    expect(t).toBeCloseTo(0.5)
    close(q, [5, 0])
    expect(d).toBeCloseTo(3)
  })
  it('point in polygon and oriented boxes', () => {
    const box = obb([0, 0], 2000, 1000, 90)
    expect(pointInPolygon([0, 900], box)).toBe(true)
    expect(pointInPolygon([900, 0], box)).toBe(false)
  })
})

describe('snapping', () => {
  const nodes = [{ id: 'n1', p: [0, 0] as Vec }, { id: 'n2', p: [4000, 0] as Vec }]
  const walls = [{ id: 'w1', a: [0, 0] as Vec, b: [4000, 0] as Vec }]
  it('prefers nodes within tolerance', () => {
    const s = snapPoint([4040, 30], nodes, walls, { enabled: true, tol: 100 })
    expect(s).toMatchObject({ kind: 'node', node: 'n2', p: [4000, 0] })
  })
  it('snaps to wall midpoints', () => {
    expect(snapPoint([1990, 40], nodes, walls, { enabled: true, tol: 100 })).toMatchObject({ kind: 'mid', wall: 'w1', p: [2000, 0] })
  })
  it('snaps onto walls (for splitting)', () => {
    expect(snapPoint([1234, 50], nodes, walls, { enabled: true, tol: 100 })).toMatchObject({ kind: 'wall', wall: 'w1', p: [1234, 0] })
  })
  it('falls back to the 10 mm grid', () => {
    expect(snapPoint([1234, 2346], nodes, walls, { enabled: true, tol: 100 })).toMatchObject({ kind: 'grid', p: [1230, 2350] })
  })
  it('snaps angles to 15° steps and lengths to 10 mm from the previous point', () => {
    const s = snapPoint([3003, 2010], [], [], { enabled: true, tol: 50, prev: [0, 0] })
    // 33.8° → 30°, length 3614 → 3610
    expect(s.kind).toBe('angle')
    expect(Math.hypot(s.p[0], s.p[1])).toBeCloseTo(3610, -1)
    expect((Math.atan2(s.p[1], s.p[0]) * 180) / Math.PI).toBeCloseTo(30, 0)
  })
  it('prefers perpendicular to a wall attached to the previous point', () => {
    const rad = (deg: number) => (deg * Math.PI) / 180
    const ref: Vec = [Math.cos(rad(50)), Math.sin(rad(50))] // wall at 50° → perpendicular 140°
    const sd = snapDirection([0, 0], [1000 * Math.cos(rad(141)), 1000 * Math.sin(rad(141))], 15, 10, [ref])
    expect((Math.atan2(sd.dir[1], sd.dir[0]) * 180) / Math.PI).toBeCloseTo(140, 5)
    expect(sd.kind).toBe('perp')
  })
  it('does nothing when disabled (Alt)', () => {
    expect(snapPoint([4040.4, 30.6], nodes, walls, { enabled: false, tol: 100 })).toMatchObject({ kind: 'free', p: [4040, 31] })
  })
  it('places typed lengths along the direction', () => {
    expect(pointAtLength([1000, 1000], [1000, 5000], 2500)).toEqual([1000, 3500])
    expect(snapToGrid([14, -16], 10)).toEqual([10, -20])
  })
})

describe('length input (CAD typing)', () => {
  it('parses the formats the wall tool accepts', () => {
    expect(parseLength('3450')).toBe(3450)
    expect(parseLength('3.45')).toBe(3450)
    expect(parseLength('345cm')).toBe(3450)
    expect(parseLength("3'450")).toBe(3450)
    expect(parseLength('2,5 m')).toBe(2500)
    expect(parseLength('abc')).toBeNull()
  })
})

describe('compass', () => {
  it('measures clockwise from screen up', () => {
    expect(angleFromUp(0, -1)).toBeCloseTo(0)
    expect(angleFromUp(1, 0)).toBeCloseTo(90)
    expect(angleFromUp(0, 1)).toBeCloseTo(180)
    expect(angleFromUp(-1, 0)).toBeCloseTo(270)
  })
})
