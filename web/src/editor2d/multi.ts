// Multi selection: frame pick, moving, deleting and duplicating several elements at once.
import type { Op } from '../core/generated/Op'
import type { Project } from '../core/generated/Project'
import type { Sel } from '../state/editor'
import type { Vec } from './geom'
import { deleteOps, kindOf } from './model'
import type { PlanSource } from './source'

/** Furniture and free lamps whose centre lies in the frame a–b (plan mm). */
export function boxSelect(p: Project, a: Vec, b: Vec): Sel[] {
  const [x0, x1] = [Math.min(a[0], b[0]), Math.max(a[0], b[0])]
  const [y0, y1] = [Math.min(a[1], b[1]), Math.max(a[1], b[1])]
  const inside = (x: number, y: number) => x >= x0 && x <= x1 && y >= y0 && y <= y1
  return [
    ...p.furniture.filter((f) => inside(f.pos_mm[0], f.pos_mm[1])).map((f) => ({ kind: 'furniture' as const, id: f.id })),
    ...p.lights.filter((l) => !l.attached_to && inside(l.pos_mm[0], l.pos_mm[1])).map((l) => ({ kind: 'light' as const, id: l.id })),
  ]
}

/** Ops moving the selected furniture and free lamps by `d` (attached lamps follow their furniture). */
export function moveOps(p: Project, sels: Sel[], d: Vec): Op[] {
  const ops: Op[] = []
  for (const s of sels) {
    if (s.kind === 'furniture') {
      const f = p.furniture.find((x) => x.id === s.id)
      if (f) ops.push({ op: 'PutFurniture', item: { ...f, pos_mm: [Math.round(f.pos_mm[0] + d[0]), Math.round(f.pos_mm[1] + d[1]), f.pos_mm[2]] } })
    } else if (s.kind === 'light') {
      const l = p.lights.find((x) => x.id === s.id)
      if (l && !l.attached_to) ops.push({ op: 'PutLight', light: { ...l, pos_mm: [Math.round(l.pos_mm[0] + d[0]), Math.round(l.pos_mm[1] + d[1]), l.pos_mm[2]] } })
    }
  }
  return ops
}

/** Concatenate op lists, dropping exact repeats (e.g. a lamp deleted with its furniture and on its own). */
export function mergeOps(lists: Op[][]): Op[] {
  const seen = new Set<string>()
  return lists.flat().filter((o) => {
    const k = JSON.stringify(o)
    if (seen.has(k)) return false
    seen.add(k)
    return true
  })
}

/** Ops deleting all selected elements, with a German label. */
export function deleteManyOps(src: PlanSource, sels: Sel[]): [Op[], string] | null {
  if (sels.length === 1) return deleteOps(src, sels[0])
  const lists = sels.map((s) => deleteOps(src, s)?.[0] ?? [])
  const ops = mergeOps(lists)
  return ops.length ? [ops, `${sels.length} Elemente löschen`] : null
}

/** Plan offset of duplicates. */
export const DUPLICATE_OFFSET: Vec = [300, 300]

/** Copies of the selected furniture (with their lamps) and lamps; null if nothing can be duplicated. */
export function duplicateOps(src: PlanSource, sels: Sel[]): [Op[], Sel[]] | null {
  // glowing surfaces come along with their object only (a second glow on the same object makes no sense)
  const surface = new Set(src.project.lights.filter((l) => l.kind.type === 'Surface').map((l) => l.id))
  const ids = sels.filter((s) => s.kind === 'furniture' || (s.kind === 'light' && !surface.has(s.id))).map((s) => s.id)
  if (!ids.length) return null
  try {
    const [ops, newIds] = src.engine.cmd_duplicate(ids, DUPLICATE_OFFSET[0], DUPLICATE_OFFSET[1]) as [Op[], string[]]
    const p = { ...src.project, furniture: [...src.project.furniture], lights: [...src.project.lights] }
    for (const o of ops) {
      if (o.op === 'PutFurniture') p.furniture.push(o.item)
      if (o.op === 'PutLight') p.lights.push(o.light)
    }
    return [ops, newIds.flatMap((id) => (kindOf(p, id) ? [{ kind: kindOf(p, id)!, id }] : []))]
  } catch (err) {
    console.error(err)
    return null
  }
}
