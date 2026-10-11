// Pure lamp geometry (plan mm, z up unless noted). Mirrors plan-core `lights.rs` where both exist.
import type { FurnitureInstance } from '../core/generated/FurnitureInstance'
import type { GlowFace } from '../core/generated/GlowFace'
import type { Light } from '../core/generated/Light'
import type { LightKind } from '../core/generated/LightKind'

export type V3 = [number, number, number]
export const LED_LM_PER_M = 900
export const LED_KELVIN = 3000
const STRIP_W = 12
const STRIP_T = 10
const rad = (d: number) => (d * Math.PI) / 180

/** Unit direction a spot points to (plan, z up); null for other kinds. */
export function lightDir(l: Light, host?: FurnitureInstance): V3 | null {
  if (l.kind.type !== 'Spot') return null
  const t = rad(l.kind.tilt_deg)
  const h = rad(l.kind.heading_deg + (l.attached_to && host ? host.rotation_deg : 0))
  return [Math.sin(t) * Math.cos(h), Math.sin(t) * Math.sin(h), -Math.cos(t)]
}

/** Spot aim (whole degrees) from `from` towards `to`; heading relative to a host furniture turned by `hostRot`. */
export function aimAt(from: V3, to: V3, hostRot = 0): { tilt_deg: number; heading_deg: number } {
  const d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]]
  const len = Math.hypot(d[0], d[1], d[2]) || 1
  const tilt = (Math.acos(Math.max(-1, Math.min(1, -d[2] / len))) * 180) / Math.PI
  const flat = Math.hypot(d[0], d[1]) > 1e-6
  const heading = flat ? ((((Math.atan2(d[1], d[0]) * 180) / Math.PI - hostRot) % 360) + 360) % 360 : 0
  return { tilt_deg: Math.round(tilt), heading_deg: Math.round(heading) % 360 }
}

/** Emitting rectangle of a glowing object's side, in the three furniture frame (m, Y up, +Z front, origin bottom
 *  centre). The rect emits along its local −Z after rotating by rotX/rotY. */
export function surfaceRect(face: Exclude<GlowFace, 'All'>, dims: V3) {
  const [w, d, h] = dims.map((v) => v / 1000)
  switch (face) {
    case 'Down':
      return { center: [0, 0, 0] as V3, size: [w, d] as [number, number], rotX: -Math.PI / 2, rotY: 0 }
    case 'Up':
      return { center: [0, h, 0] as V3, size: [w, d] as [number, number], rotX: Math.PI / 2, rotY: 0 }
    case 'Front':
      return { center: [0, h / 2, d / 2] as V3, size: [w, h] as [number, number], rotX: 0, rotY: Math.PI }
  }
}

/** three.js intensity of a glowing object: luminance over the side (nits) or candela for the whole object. */
export function surfaceIntensity(face: GlowFace, dims: V3, lumen: number): number {
  if (face === 'All') return lumen / (4 * Math.PI)
  const [a, b] = surfaceRect(face, dims).size
  return lumen / (Math.PI * Math.max(1e-4, a * b))
}

/** Luminance (nits) of a glowing surface: lumen spread over the side, or over the whole object's surface. */
export function glowLuminance(face: GlowFace, dims: V3, lumen: number): number {
  if (face !== 'All') return surfaceIntensity(face, dims, lumen)
  const [w, d, h] = dims.map((v) => v / 1000)
  return lumen / (Math.PI * Math.max(1e-4, 2 * (w * d + w * h + d * h)))
}

/** A surface point (plan mm, z up) with outward normal and the furniture it belongs to. */
export type Mount = { at: V3; normal: V3; furniture: string | null }

/** Where a lamp of type `tool` goes for a clicked surface: area panels only exist flat under the ceiling
 *  (they always shine down), so they go to the ceiling above the hit – moved clear of a clicked wall by half
 *  their size (`sizeMm`) plus a gap; everything else stays put. */
export function mountFor(tool: string, h: Mount, ceilingMm: number, sizeMm = 0): Mount {
  if (tool !== 'Area') return h
  const flat = Math.hypot(h.normal[0], h.normal[1])
  const off = flat > 0.7 ? sizeMm / 2 + 30 : 0
  const nx = flat > 0 ? h.normal[0] / flat : 0
  const ny = flat > 0 ? h.normal[1] / flat : 0
  return { at: [h.at[0] + nx * off, h.at[1] + ny * off, ceilingMm], normal: [0, 0, -1], furniture: null }
}

/** An uplight always points up, whatever surface it was put on (the heading from the surface is kept). */
export function keepUplight(tool: string, kind: LightKind): LightKind {
  return tool === 'Uplight' && kind.type === 'Spot' ? { ...kind, tilt_deg: 180 } : kind
}

/** `n` turned towards the viewer (ray direction `dir`). */
export function facingNormal(n: V3, dir: V3): V3 {
  return n[0] * dir[0] + n[1] * dir[1] + n[2] * dir[2] > 0 ? [-n[0], -n[1], -n[2]] : n
}

/** LED strip dragged from a to b on a surface with outward normal n. null if shorter than 5 cm. */
export function stripFromDrag(a: V3, b: V3, n: V3): { pos_mm: V3; rotation_deg: number; dims_mm: V3; face: GlowFace } | null {
  const len = Math.hypot(b[0] - a[0], b[1] - a[1])
  if (len < 50) return null
  if (Math.abs(n[2]) > 0.7) {
    // ceiling / floor / top or bottom of furniture: lies flat, glows away from the surface
    const down = n[2] < 0
    return {
      pos_mm: [Math.round((a[0] + b[0]) / 2), Math.round((a[1] + b[1]) / 2), Math.round(down ? a[2] - STRIP_T : a[2])],
      rotation_deg: Math.round(((Math.atan2(b[1] - a[1], b[0] - a[0]) * 180) / Math.PI + 360) % 360),
      dims_mm: [Math.round(len), STRIP_W, STRIP_T],
      face: down ? 'Down' : 'Up',
    }
  }
  // wall: horizontal run along the wall at the start height, front into the room
  const rot = ((((Math.atan2(-n[0], n[1]) * 180) / Math.PI) % 360) + 360) % 360
  return {
    pos_mm: [Math.round((a[0] + b[0]) / 2 + (n[0] * STRIP_T) / 2), Math.round((a[1] + b[1]) / 2 + (n[1] * STRIP_T) / 2), Math.round(a[2] - STRIP_W / 2)],
    rotation_deg: Math.round(rot),
    dims_mm: [Math.round(len), STRIP_T, STRIP_W],
    face: 'Front',
  }
}
