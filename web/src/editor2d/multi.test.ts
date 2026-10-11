import { describe, expect, it } from 'vitest'
import type { FurnitureInstance } from '../core/generated/FurnitureInstance'
import type { Light } from '../core/generated/Light'
import type { Op } from '../core/generated/Op'
import type { Project } from '../core/generated/Project'
import { boxSelect, mergeOps, moveOps } from './multi'

const furn = (id: string, x: number, y: number): FurnitureInstance => ({
  id, asset: 'a', name: id, pos_mm: [x, y, 0], rotation_deg: 0, dims_mm: [500, 500, 900], material: null, walkable: false, overlap_ok: false, footprint: [], passable: false,
})
const lamp = (id: string, x: number, y: number, on: string | null = null): Light => ({
  id, name: id, kind: { type: 'Point' }, pos_mm: [x, y, 2400], lumen: 800, color_temp_k: 2700, on: true, attached_to: on,
})
const proj = (furniture: FurnitureInstance[], lights: Light[]) => ({ furniture, lights, floors: [{ nodes: [], walls: [], openings: [], rooms: [] }] }) as unknown as Project

describe('multi selection helpers', () => {
  const p = proj([furn('a', 0, 0), furn('b', 2000, 0)], [lamp('l', 500, 500), lamp('glow', 0, 0, 'a')])

  it('frame picks furniture and free lamps by centre, either drag direction', () => {
    expect(boxSelect(p, [1000, 1000], [-100, -100])).toEqual([
      { kind: 'furniture', id: 'a' },
      { kind: 'light', id: 'l' },
    ])
  })

  it('moves furniture and free lamps; attached lamps follow their host by themselves', () => {
    const ops = moveOps(p, [{ kind: 'furniture', id: 'b' }, { kind: 'light', id: 'l' }, { kind: 'light', id: 'glow' }], [100, -50])
    expect(ops).toHaveLength(2)
    expect((ops[0] as { item: FurnitureInstance }).item.pos_mm).toEqual([2100, -50, 0])
    expect((ops[1] as { light: Light }).light.pos_mm).toEqual([600, 450, 2400])
  })

  it('merges op lists without repeating the same op', () => {
    const d: Op = { op: 'DeleteLight', id: 'glow' }
    expect(mergeOps([[d, { op: 'DeleteFurniture', id: 'a' }], [d]])).toHaveLength(2)
  })
})
