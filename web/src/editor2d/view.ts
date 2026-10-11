// View transform of the 2D plan: screen px = world mm · scale + (tx, ty).
import type { Bounds, Vec } from './geom'

export type View = { scale: number; tx: number; ty: number }

export const MIN_SCALE = 0.005 // 1 px = 200 mm
export const MAX_SCALE = 2 // 1 px = 0.5 mm

export const DEFAULT_VIEW: View = { scale: 0.08, tx: 40, ty: 40 }

export function toScreen(v: View, p: Vec): Vec {
  return [p[0] * v.scale + v.tx, p[1] * v.scale + v.ty]
}

export function toWorld(v: View, s: Vec): Vec {
  return [(s[0] - v.tx) / v.scale, (s[1] - v.ty) / v.scale]
}

/** Zoom by `factor` keeping the world point under screen point `s` fixed. */
export function zoomAt(v: View, s: Vec, factor: number): View {
  const scale = Math.min(MAX_SCALE, Math.max(MIN_SCALE, v.scale * factor))
  const w = toWorld(v, s)
  return { scale, tx: s[0] - w[0] * scale, ty: s[1] - w[1] * scale }
}

export function panBy(v: View, dx: number, dy: number): View {
  return { ...v, tx: v.tx + dx, ty: v.ty + dy }
}

/** View that fits `b` into a viewport of size w×h with `pad` px margin. */
export function fitBounds(b: Bounds, w: number, h: number, pad = 40, maxScale = 0.4): View {
  const bw = Math.max(b.maxX - b.minX, 500)
  const bh = Math.max(b.maxY - b.minY, 500)
  const scale = Math.min(maxScale, Math.max(MIN_SCALE, Math.min((w - 2 * pad) / bw, (h - 2 * pad) / bh)))
  const cx = (b.minX + b.maxX) / 2
  const cy = (b.minY + b.maxY) / 2
  return { scale, tx: w / 2 - cx * scale, ty: h / 2 - cy * scale }
}

/** Grid spacing levels (mm) and their opacity at the given scale; fades out when lines get too dense. */
export function gridLevels(scale: number): { step: number; alpha: number }[] {
  const out: { step: number; alpha: number }[] = []
  for (const step of [100, 1000]) {
    const px = step * scale
    const alpha = Math.max(0, Math.min(1, (px - 6) / 14))
    if (alpha > 0) out.push({ step, alpha })
  }
  // very far out: 10 m grid
  if (1000 * scale < 8) out.push({ step: 10000, alpha: Math.max(0, Math.min(1, (10000 * scale - 6) / 14)) })
  return out
}
