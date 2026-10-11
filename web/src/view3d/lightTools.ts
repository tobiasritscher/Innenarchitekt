// Light mode helpers: what's under the cursor (surface point + normal + furniture), lamp templates, LED strip asset.
import { Vector2, Vector3, type Camera, type Mesh, type Object3D, type Raycaster, type Scene, type WebGLRenderer } from 'three'
import { api } from '../core/api'
import type { Light } from '../core/generated/Light'
import { useAssets } from '../state/assets'
import { facingNormal, type V3 } from './lightGeom'
import { wallClip } from './materials3d'

export type LampTemplate = Omit<Light, 'id' | 'pos_mm' | 'attached_to'>

export const LAMP_TEMPLATES = {
  Point: { name: 'Deckenleuchte', kind: { type: 'Point' }, lumen: 800, color_temp_k: 2700, on: true },
  Spot: { name: 'Spot', kind: { type: 'Spot', angle_deg: 40, tilt_deg: 0, heading_deg: 0 }, lumen: 450, color_temp_k: 3000, on: true },
  Uplight: { name: 'Uplight', kind: { type: 'Spot', angle_deg: 60, tilt_deg: 180, heading_deg: 0 }, lumen: 600, color_temp_k: 2700, on: true },
  Area: { name: 'Leuchtfläche', kind: { type: 'Area', w_mm: 600, h_mm: 600 }, lumen: 3000, color_temp_k: 4000, on: true },
} satisfies Record<string, LampTemplate>
export type TemplateKey = keyof typeof LAMP_TEMPLATES

export type HitCtx = { gl: WebGLRenderer; camera: Camera; scene: Scene; raycaster: Raycaster; ceilingMm: number }
export type SurfaceHit = { at: V3; normal: V3; furniture: string | null }

const _n = new Vector3()

/** Aim the raycaster at the cursor. */
export function aim(e: { clientX: number; clientY: number }, ctx: HitCtx) {
  const r = ctx.gl.domElement.getBoundingClientRect()
  ctx.raycaster.setFromCamera(new Vector2(((e.clientX - r.left) / r.width) * 2 - 1, -((e.clientY - r.top) / r.height) * 2 + 1), ctx.camera)
}

/** Surface under the cursor (plan mm, z up): walls, floor, furniture – not lamps or helpers. Normal faces the viewer.
 *  `preferCeiling`: a floor hit becomes the ceiling point above it (from above, the ceiling itself can't be clicked). */
export function surfaceHit(e: { clientX: number; clientY: number }, ctx: HitCtx, preferCeiling: boolean): SurfaceHit | null {
  aim(e, ctx)
  const hit = ctx.raycaster
    .intersectObjects(ctx.scene.children, true)
    .find((h) => h.face && (h.object as Mesh).isMesh && !h.object.userData.bulb && !h.object.userData.marker && wallClip.distanceToPoint(h.point) >= 0)
  if (!hit || !hit.face) return null
  let furniture: string | null = null
  for (let o: Object3D | null = hit.object; o; o = o.parent) {
    if (typeof o.userData.furniture === 'string') {
      furniture = o.userData.furniture
      break
    }
  }
  _n.copy(hit.face.normal).transformDirection(hit.object.matrixWorld)
  const d = ctx.raycaster.ray.direction
  // three (x, y up, z) → plan (x, z, y)
  const normal = facingNormal([_n.x, _n.z, _n.y], [d.x, d.z, d.y])
  const at: V3 = [hit.point.x * 1000, hit.point.z * 1000, hit.point.y * 1000]
  const isFloor = !furniture && normal[2] > 0.9 && at[2] < 50
  if (preferCeiling && isFloor) return { at: [at[0], at[1], ctx.ceilingMm], normal: [0, 0, -1], furniture: null }
  return { at, normal, furniture }
}

/** Point where the cursor ray meets the plane through `p` with normal `n` (plan mm). */
export function planeHit(e: { clientX: number; clientY: number }, ctx: HitCtx, p: V3, n: V3): V3 | null {
  aim(e, ctx)
  const normal = new Vector3(n[0], n[2], n[1])
  const point = new Vector3(p[0] / 1000, p[2] / 1000, p[1] / 1000)
  const denom = normal.dot(ctx.raycaster.ray.direction)
  if (Math.abs(denom) < 1e-6) return null
  const t = point.sub(ctx.raycaster.ray.origin).dot(normal) / denom
  if (t < 0) return null
  const q = ctx.raycaster.ray.at(t, new Vector3())
  return [q.x * 1000, q.z * 1000, q.y * 1000]
}

/** Library asset id of the plate primitive used for LED strips (created on first use). */
export async function plateAssetId(): Promise<string> {
  const st = useAssets.getState()
  const plate = st.assets.find((a) => a.primitive === 'Plate')
  if (plate) return plate.id
  const a = await api.createPrimitive('Plate', 'LED-Streifen', [1000, 12, 10])
  st.add(a)
  return a.id
}
