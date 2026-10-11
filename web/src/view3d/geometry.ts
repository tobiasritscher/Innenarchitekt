// Core Mesh (plain arrays) → three.js BufferGeometry, with a per-key cache that reuses
// geometries whose data did not change (walls that were not touched by a drag).
import { BufferAttribute, BufferGeometry } from 'three'
import type { Mesh } from '../core/generated/Mesh'
import { primitiveMesh } from '../core/engine'
import type { PrimitiveShape } from '../core/generated/PrimitiveShape'

/** Converts a core mesh. Group i gets materialIndex = slots.indexOf(group.slot) (or 0). */
export function toGeometry(mesh: Mesh, slots: string[]): BufferGeometry {
  const g = new BufferGeometry()
  g.setAttribute('position', new BufferAttribute(new Float32Array(mesh.positions), 3))
  g.setAttribute('normal', new BufferAttribute(new Float32Array(mesh.normals), 3))
  g.setAttribute('uv', new BufferAttribute(new Float32Array(mesh.uvs), 2))
  const n = mesh.positions.length / 3
  g.setIndex(new BufferAttribute(n > 65535 ? new Uint32Array(mesh.indices) : new Uint16Array(mesh.indices), 1))
  for (const gr of mesh.groups) g.addGroup(gr.start, gr.count, Math.max(0, slots.indexOf(gr.slot)))
  g.computeBoundingSphere()
  g.computeBoundingBox()
  return g
}

function sameMesh(a: Mesh, b: Mesh): boolean {
  if (a === b) return true
  if (a.positions.length !== b.positions.length || a.indices.length !== b.indices.length || a.groups.length !== b.groups.length) return false
  for (let i = 0; i < a.positions.length; i++) if (a.positions[i] !== b.positions[i]) return false
  for (let i = 0; i < a.groups.length; i++) if (a.groups[i].slot !== b.groups[i].slot || a.groups[i].count !== b.groups[i].count) return false
  return true
}

/** Keeps geometries per key; `sync` returns the geometry for each entry and disposes stale ones. */
export class GeometryCache {
  private entries = new Map<string, { mesh: Mesh; geo: BufferGeometry }>()
  constructor(private slots: string[]) {}

  sync(items: { key: string; mesh: Mesh }[]): Map<string, BufferGeometry> {
    const out = new Map<string, BufferGeometry>()
    const next = new Map<string, { mesh: Mesh; geo: BufferGeometry }>()
    for (const { key, mesh } of items) {
      const prev = this.entries.get(key)
      if (prev && sameMesh(prev.mesh, mesh)) {
        next.set(key, prev)
        out.set(key, prev.geo)
      } else {
        prev?.geo.dispose()
        const e = { mesh, geo: toGeometry(mesh, this.slots) }
        next.set(key, e)
        out.set(key, e.geo)
      }
      this.entries.delete(key)
    }
    for (const e of this.entries.values()) e.geo.dispose()
    this.entries = next
    return out
  }

  dispose() {
    for (const e of this.entries.values()) e.geo.dispose()
    this.entries.clear()
  }
}

// --- primitive meshes, ref-counted by shape+dims ---
const prims = new Map<string, { geo: BufferGeometry; refs: number }>()
export const PRIMITIVE_SLOTS = ['main', 'accent']

export function acquirePrimitive(shape: PrimitiveShape, dims: [number, number, number]): BufferGeometry {
  const key = `${shape}|${dims.join('x')}`
  let e = prims.get(key)
  if (!e) {
    const mesh = primitiveMesh(shape, Math.max(1, dims[0]), Math.max(1, dims[1]), Math.max(1, dims[2])) as Mesh
    e = { geo: toGeometry(mesh, PRIMITIVE_SLOTS), refs: 0 }
    prims.set(key, e)
  }
  e.refs++
  return e.geo
}

export function releasePrimitive(shape: PrimitiveShape, dims: [number, number, number]) {
  const key = `${shape}|${dims.join('x')}`
  const e = prims.get(key)
  if (!e) return
  e.refs--
  if (e.refs <= 0) {
    // keep briefly: a drag that changes dims back and forth would thrash otherwise
    setTimeout(() => {
      const cur = prims.get(key)
      if (cur && cur.refs <= 0) {
        cur.geo.dispose()
        prims.delete(key)
      }
    }, 2000)
  }
}
