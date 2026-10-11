import { describe, expect, it } from 'vitest'
import { aimAt, facingNormal, lightDir, stripFromDrag, surfaceIntensity, surfaceRect } from './lightGeom'
import type { Light } from '../core/generated/Light'
import type { FurnitureInstance } from '../core/generated/FurnitureInstance'

const spot = (tilt: number, heading: number, attached: string | null = null): Light => ({
  id: 'l',
  name: 's',
  kind: { type: 'Spot', angle_deg: 40, tilt_deg: tilt, heading_deg: heading },
  pos_mm: [0, 0, 2400],
  lumen: 450,
  color_temp_k: 3000,
  on: true,
  attached_to: attached,
})
const near = (a: number[] | null, b: number[]) => {
  expect(a).not.toBeNull()
  a!.forEach((v, i) => expect(v).toBeCloseTo(b[i], 6))
}

describe('lightDir', () => {
  it('down, up, sideways', () => {
    near(lightDir(spot(0, 0)), [0, 0, -1])
    near(lightDir(spot(180, 0)), [0, 0, 1])
    near(lightDir(spot(90, 90)), [0, 1, 0])
  })
  it('turns with its furniture', () => {
    const f = { rotation_deg: 90 } as FurnitureInstance
    near(lightDir(spot(90, 0, 'f'), f), [0, 1, 0])
  })
  it('null for non-spots', () => expect(lightDir({ ...spot(0, 0), kind: { type: 'Point' } })).toBeNull())
})

describe('surfaceRect', () => {
  it('down face sits at the bottom, sized w × d', () => {
    const r = surfaceRect('Down', [1000, 10, 12])
    expect(r.center).toEqual([0, 0, 0])
    expect(r.size).toEqual([1, 0.01])
  })
  it('front face at +z, sized w × h', () => {
    const r = surfaceRect('Front', [1000, 10, 12])
    expect(r.center[2]).toBeCloseTo(0.005)
    expect(r.size).toEqual([1, 0.012])
  })
})

it('surfaceIntensity: 900 lm on 1 m × 1 cm', () => {
  expect(surfaceIntensity('Down', [1000, 10, 12], 900)).toBeCloseTo(900 / (Math.PI * 0.01))
})

it('facingNormal flips a normal pointing away from the viewer', () => {
  near(facingNormal([1, 0, 0], [1, 0, 0]), [-1, 0, 0])
  near(facingNormal([1, 0, 0], [-1, 0, 0]), [1, 0, 0])
})

describe('stripFromDrag', () => {
  it('ceiling strip: horizontal, glows down', () => {
    const s = stripFromDrag([0, 0, 2500], [2000, 0, 2500], [0, 0, -1])!
    expect(s.dims_mm).toEqual([2000, 12, 10])
    expect(s.pos_mm).toEqual([1000, 0, 2490])
    expect(s.face).toBe('Down')
  })
  it('wall strip: front faces into the room', () => {
    const s = stripFromDrag([0, 0, 1200], [0, 1500, 1200], [1, 0, 0])!
    expect(s.face).toBe('Front')
    expect(s.dims_mm).toEqual([1500, 10, 12])
    expect(s.pos_mm).toEqual([5, 750, 1194])
    // local +y (front) = plan +x  ⇒  rotation −90 ≡ 270
    expect(s.rotation_deg).toBe(270)
  })
  it('too short → null', () => expect(stripFromDrag([0, 0, 0], [30, 0, 0], [0, 0, 1])).toBeNull())
})

describe('aimAt', () => {
  it('straight down, up and towards +y', () => {
    expect(aimAt([0, 0, 2500], [0, 0, 0])).toEqual({ tilt_deg: 0, heading_deg: 0 })
    expect(aimAt([0, 0, 0], [0, 0, 2500]).tilt_deg).toBe(180)
    expect(aimAt([0, 0, 1000], [0, 2000, 1000])).toEqual({ tilt_deg: 90, heading_deg: 90 })
  })
  it('heading relative to the host furniture', () => {
    expect(aimAt([0, 0, 1000], [0, 2000, 1000], 90).heading_deg).toBe(0)
  })
})
