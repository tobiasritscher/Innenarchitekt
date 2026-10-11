// three.js materials for MaterialRefs (library ids or "#rrggbb"), with procedural CanvasTextures
// in world-scale UVs (the core emits UVs in metres). Cached per ref; never disposed during a session
// (a handful of materials and 512² textures).
import {
  CanvasTexture,
  Color,
  DoubleSide,
  FrontSide,
  MeshStandardMaterial,
  Plane,
  RepeatWrapping,
  SRGBColorSpace,
  Vector3,
  type Material,
} from 'three'
import { resolveMaterial, type MaterialDef } from '../core/materials'

/** Shared clipping plane for "Wände kappen" (preview). Disabled = far away. */
export const wallClip = new Plane(new Vector3(0, -1, 0), 1e6)
const wallClipPlanes = [wallClip]
export function setWallCut(heightM: number | null) {
  wallClip.constant = heightM ?? 1e6
}

// --- deterministic PRNG so textures look the same on every load ---
function rng(seed: number) {
  let s = seed >>> 0 || 1
  return () => {
    s ^= s << 13
    s ^= s >>> 17
    s ^= s << 5
    return ((s >>> 0) % 100000) / 100000
  }
}
function hashStr(s: string) {
  let h = 2166136261
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619)
  return h >>> 0
}

function shade(hex: string, f: number, alpha = 1): string {
  const c = new Color(hex)
  const r = Math.round(Math.min(255, Math.max(0, c.r * 255 * f)))
  const g = Math.round(Math.min(255, Math.max(0, c.g * 255 * f)))
  const b = Math.round(Math.min(255, Math.max(0, c.b * 255 * f)))
  return `rgba(${r},${g},${b},${alpha})`
}

type Tex = { canvas: HTMLCanvasElement; periodU: number; periodV: number }

/** Draws a periodic pattern tile. Returns the canvas and its period in metres. */
function drawPattern(def: MaterialDef): Tex | null {
  if (!def.pattern || typeof document === 'undefined') return null
  const N = 512
  const canvas = document.createElement('canvas')
  canvas.width = N
  canvas.height = N
  const g = canvas.getContext('2d')!
  const rand = rng(hashStr(def.id))
  const scale = def.scale ?? 0.2
  const base = def.color
  g.fillStyle = base
  g.fillRect(0, 0, N, N)
  // draw with horizontal + vertical wrap
  const wrap = (fn: (ox: number, oy: number) => void) => {
    for (const ox of [-N, 0, N]) for (const oy of [-N, 0, N]) fn(ox, oy)
  }

  switch (def.pattern) {
    case 'planks': {
      const rows = Math.max(2, Math.round(1.2 / scale))
      const period = rows * scale // square tile
      const rowH = N / rows
      for (let r = 0; r < rows; r++) {
        const y = r * rowH
        // 2 planks per row with staggered joint
        const joint = rand() * N
        const segs = [joint, joint + N / 2 + (rand() - 0.5) * N * 0.3]
        for (let k = 0; k < 2; k++) {
          const x0 = segs[k]
          const x1 = k === 0 ? segs[1] : segs[0] + N
          const f = 0.86 + rand() * 0.26
          g.fillStyle = shade(base, f)
          for (const ox of [-N, 0]) g.fillRect(x0 + ox, y, x1 - x0, rowH)
          // grain
          g.strokeStyle = shade(base, f * 0.88, 0.35)
          g.lineWidth = 1
          for (let i = 0; i < 7; i++) {
            const yy = y + 3 + rand() * (rowH - 6)
            const amp = 1 + rand() * 2
            for (const ox of [-N, 0]) {
              g.beginPath()
              for (let x = x0; x <= x1; x += 16) {
                const yv = yy + Math.sin((x / N) * Math.PI * 6 + i) * amp
                if (x === x0) g.moveTo(x + ox, yv)
                else g.lineTo(x + ox, yv)
              }
              g.stroke()
            }
          }
          g.fillStyle = shade(base, 0.55, 0.7)
          for (const ox of [-N, 0, N]) g.fillRect(x0 + ox - 1, y, 2, rowH)
        }
        g.fillStyle = shade(base, 0.5, 0.8)
        g.fillRect(0, y, N, 1.5)
      }
      return { canvas, periodU: period, periodV: period }
    }
    case 'tiles': {
      const rect = def.name.includes('30×60')
      const tw = rect ? 2 : 1
      const cols = 2
      const rows = rect ? 4 : 2
      const period = scale * (rect ? 2 : 2)
      const cw = N / cols
      const ch = N / rows
      for (let r = 0; r < rows; r++)
        for (let c = 0; c < cols; c++) {
          const off = rect && r % 2 ? cw / 2 : 0
          g.fillStyle = shade(base, 0.96 + rand() * 0.08)
          wrap((ox, oy) => g.fillRect(c * cw + off + ox, r * ch + oy, cw, ch))
        }
      void tw
      g.fillStyle = shade(base, 0.72)
      const grout = Math.max(1.5, (0.003 / period) * N)
      for (let r = 0; r <= rows; r++) g.fillRect(0, r * ch - grout / 2, N, grout)
      for (let r = 0; r < rows; r++) {
        const off = rect && r % 2 ? cw / 2 : 0
        for (let c = 0; c <= cols; c++) g.fillRect(((c * cw + off) % N) - grout / 2, r * ch, grout, ch)
      }
      return { canvas, periodU: period, periodV: period }
    }
    case 'herringbone': {
      // chevron-like herringbone: columns of slanted planks alternating direction
      const plankW = scale
      const cols = 2
      const period = 0.6
      const colW = N / cols
      const step = (plankW / period) * N * Math.SQRT2
      for (let c = 0; c < cols; c++) {
        const dir = c % 2 ? 1 : -1
        g.save()
        g.beginPath()
        g.rect(c * colW, 0, colW, N)
        g.clip()
        for (let k = -4; k < N / step + 4; k++) {
          const y0 = k * step
          g.fillStyle = shade(base, 0.86 + rand() * 0.26)
          g.beginPath()
          const x0 = c * colW
          const x1 = x0 + colW
          const dy = dir * colW
          g.moveTo(x0, y0)
          g.lineTo(x1, y0 + dy)
          g.lineTo(x1, y0 + dy + step)
          g.lineTo(x0, y0 + step)
          g.closePath()
          g.fill()
          g.strokeStyle = shade(base, 0.55, 0.8)
          g.lineWidth = 1.5
          g.stroke()
        }
        g.restore()
      }
      return { canvas, periodU: period, periodV: period }
    }
    case 'noise': {
      const period = scale * 2
      if (def.id.includes('terrazzo')) {
        const chips = ['#a59d92', '#7f7a73', '#efe9df', '#c27d5c', '#5f6b6e', '#b8b2a7']
        for (let i = 0; i < 1400; i++) {
          const x = rand() * N
          const y = rand() * N
          const r = 1 + rand() * rand() * 7
          g.fillStyle = chips[Math.floor(rand() * chips.length)]
          wrap((ox, oy) => {
            g.beginPath()
            g.ellipse(x + ox, y + oy, r, r * (0.5 + rand() * 0.5), rand() * 3, 0, Math.PI * 2)
            g.fill()
          })
        }
      } else if (def.id.includes('marble')) {
        for (let i = 0; i < 2000; i++) {
          g.fillStyle = shade(base, 0.95 + rand() * 0.08, 0.25)
          const x = rand() * N
          const y = rand() * N
          wrap((ox, oy) => g.fillRect(x + ox, y + oy, 8, 8))
        }
        g.lineWidth = 1.2
        for (let v = 0; v < 6; v++) {
          g.strokeStyle = shade(base, 0.62 + rand() * 0.2, 0.55)
          let x = rand() * N
          let y = 0
          g.beginPath()
          g.moveTo(x, y)
          while (y < N) {
            x += (rand() - 0.5) * 30
            y += 10 + rand() * 20
            g.lineTo(x, y)
          }
          g.stroke()
        }
      } else {
        // concrete: soft blotches + pores
        for (let i = 0; i < 260; i++) {
          const x = rand() * N
          const y = rand() * N
          const r = 10 + rand() * 50
          const grd = g.createRadialGradient(x, y, 0, x, y, r)
          grd.addColorStop(0, shade(base, 0.85 + rand() * 0.3, 0.25))
          grd.addColorStop(1, shade(base, 1, 0))
          g.fillStyle = grd
          wrap((ox, oy) => g.fillRect(x - r + ox, y - r + oy, r * 2, r * 2))
        }
        for (let i = 0; i < 500; i++) {
          g.fillStyle = shade(base, 0.6, 0.5)
          g.fillRect(rand() * N, rand() * N, 1.5, 1.5)
        }
      }
      return { canvas, periodU: period, periodV: period }
    }
    case 'weave': {
      const n = 32
      const cell = N / n
      const period = scale * 4
      for (let r = 0; r < n; r++)
        for (let c = 0; c < n; c++) {
          const over = (r + c) % 2 === 0
          g.fillStyle = shade(base, over ? 1.06 : 0.9, 1)
          g.fillRect(c * cell, r * cell, cell, cell)
          g.fillStyle = shade(base, 0.8 + rand() * 0.15, 0.4)
          if (over) g.fillRect(c * cell, r * cell + cell * 0.45, cell, 1)
          else g.fillRect(c * cell + cell * 0.45, r * cell, 1, cell)
        }
      return { canvas, periodU: period, periodV: period }
    }
  }
  return null
}

const cache = new Map<string, MeshStandardMaterial>()

function build(def: MaterialDef, opts: { clip?: boolean } = {}): MeshStandardMaterial {
  const m = new MeshStandardMaterial({
    color: new Color(def.color),
    roughness: def.roughness,
    metalness: def.metalness,
    dithering: true,
  })
  const tex = drawPattern(def)
  if (tex) {
    const t = new CanvasTexture(tex.canvas)
    t.colorSpace = SRGBColorSpace
    t.wrapS = t.wrapT = RepeatWrapping
    t.repeat.set(1 / tex.periodU, 1 / tex.periodV)
    t.anisotropy = 8
    m.map = t
    m.color.set('#ffffff')
  }
  if (opts.clip) m.clippingPlanes = wallClipPlanes
  m.shadowSide = DoubleSide
  return m
}

/** Material for a surface reference. `wall` variants are clippable (preview "Wände kappen"). */
export function surfaceMaterial(ref: string | null | undefined, kind: 'wall' | 'floor' | 'ceiling' = 'floor'): MeshStandardMaterial {
  const def = resolveMaterial(ref)
  const key = `${kind}|${def.id}`
  let m = cache.get(key)
  if (!m) {
    m = build(def, { clip: kind === 'wall' })
    if (kind === 'ceiling') m.side = FrontSide
    cache.set(key, m)
  }
  return m
}

function special(key: string, make: () => MeshStandardMaterial): MeshStandardMaterial {
  let m = cache.get(key)
  if (!m) {
    m = make()
    cache.set(key, m)
  }
  return m
}

export const mats = {
  wallEdge: () =>
    special('wall-edge', () => {
      const m = new MeshStandardMaterial({ color: '#f1efea', roughness: 0.9 })
      m.clippingPlanes = wallClipPlanes
      m.shadowSide = DoubleSide
      return m
    }),
  wallTop: () =>
    special('wall-top', () => {
      const m = new MeshStandardMaterial({ color: '#3b3d40', roughness: 0.95 })
      m.clippingPlanes = wallClipPlanes
      m.shadowSide = DoubleSide
      return m
    }),
  frame: () =>
    special('frame', () => {
      const m = new MeshStandardMaterial({ color: '#f4f3f0', roughness: 0.5 })
      m.clippingPlanes = wallClipPlanes
      return m
    }),
  doorLeaf: () =>
    special('door-leaf', () => {
      const m = new MeshStandardMaterial({ color: '#efece6', roughness: 0.55 })
      m.clippingPlanes = wallClipPlanes
      return m
    }),
  glass: () =>
    special('glass', () => {
      const m = new MeshStandardMaterial({
        color: '#cfe3ea',
        roughness: 0.05,
        metalness: 0,
        transparent: true,
        opacity: 0.18,
        depthWrite: false,
        side: DoubleSide,
      })
      m.clippingPlanes = wallClipPlanes
      return m
    }),
  shadowOnly: () =>
    special('shadow-only', () => {
      const m = new MeshStandardMaterial({ colorWrite: false, depthWrite: false })
      m.shadowSide = DoubleSide
      return m
    }),
  ground: () => special('ground', () => new MeshStandardMaterial({ color: '#8b8f7c', roughness: 1 })),
  furnitureMain: () => special('furn-main', () => new MeshStandardMaterial({ color: '#c9c4bb', roughness: 0.8 })),
  furnitureAccent: () => special('furn-accent', () => new MeshStandardMaterial({ color: '#8d7356', roughness: 0.6 })),
}

/** Material for a furniture override ref (shared cache). */
export function furnitureMaterial(ref: string, slot: 'main' | 'accent'): Material {
  const def = resolveMaterial(ref)
  const key = `furn|${slot}|${def.id}`
  return special(key, () => {
    const m = build(def)
    if (slot === 'accent') m.color.multiplyScalar(def.pattern ? 0.85 : 0.7)
    return m
  })
}
