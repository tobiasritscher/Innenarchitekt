// DEV ONLY: a hand-made FloorplanProposal to exercise the correction view without the backend
// pipeline. Enabled with `?fixture=1` in dev builds (see UploadStep).
import { newId, PlanEngine } from '../core/engine'
import type { FloorplanProposal } from '../core/generated/FloorplanProposal'
import type { Op } from '../core/generated/Op'
import type { Opening } from '../core/generated/Opening'
import type { Project } from '../core/generated/Project'

const S = 0.95 // the "detected" scale is 5 % off → reference step corrects it
const M = 500 // margin (mm)
const MM_PER_PX = 10
const tf = (x: number, y: number): [number, number] => [Math.round(M + x * S), Math.round(M + y * S)]

function svgImage(): { url: string; w: number; h: number } {
  const w = Math.round((2 * M + 8000 * S) / MM_PER_PX)
  const h = Math.round((2 * M + 6000 * S) / MM_PER_PX)
  const p = (x: number, y: number) => tf(x, y).map((v) => v / MM_PER_PX).join(',')
  const line = (x1: number, y1: number, x2: number, y2: number, t: number) =>
    `<line x1="${tf(x1, y1)[0] / MM_PER_PX}" y1="${tf(x1, y1)[1] / MM_PER_PX}" x2="${tf(x2, y2)[0] / MM_PER_PX}" y2="${tf(x2, y2)[1] / MM_PER_PX}" stroke="#222" stroke-width="${(t * S) / MM_PER_PX}" stroke-linecap="square"/>`
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}">
<rect width="100%" height="100%" fill="#fffdf7"/>
${line(0, 0, 8000, 0, 300)}${line(8000, 0, 8000, 6000, 300)}${line(8000, 6000, 0, 6000, 300)}${line(0, 6000, 0, 0, 300)}
${line(4500, 0, 4500, 6000, 120)}
<polyline points="${p(0, -350)} ${p(8000, -350)}" stroke="#555" fill="none"/>
<text x="${tf(4000, -420)[0] / MM_PER_PX}" y="${tf(4000, -420)[1] / MM_PER_PX}" font-size="14" text-anchor="middle" font-family="sans-serif">8.00</text>
<text x="${tf(2200, 3000)[0] / MM_PER_PX}" y="${tf(2200, 3000)[1] / MM_PER_PX}" font-size="16" text-anchor="middle" font-family="sans-serif">Wohnen</text>
<text x="${tf(6250, 3000)[0] / MM_PER_PX}" y="${tf(6250, 3000)[1] / MM_PER_PX}" font-size="16" text-anchor="middle" font-family="sans-serif">Zimmer</text>
<g transform="translate(${w - 40} 40) rotate(20)"><path d="M0 -22 L7 10 L0 4 L-7 10 Z" fill="#222"/><text y="-26" font-size="12" text-anchor="middle" font-family="sans-serif">N</text></g>
</svg>`
  return { url: `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`, w, h }
}

export function makeFixtureProposal(project: Project): FloorplanProposal {
  const base = structuredClone(project)
  const fl = base.floors[0]
  fl.nodes = []
  fl.walls = []
  fl.openings = []
  fl.rooms = []
  fl.reference_image = null
  base.furniture = []
  base.lights = []
  const floor = fl.id
  const eng = new PlanEngine(base)
  try {
    const cp = (x: number, y: number) => {
      const [a, b] = tf(x, y)
      return { node: null, wall: null, x: a, y: b }
    }
    eng.apply(eng.cmd_add_wall_chain(floor, [cp(0, 0), cp(8000, 0), cp(8000, 6000), cp(0, 6000)], true, 290, true))
    eng.apply(eng.cmd_add_wall_chain(floor, [cp(4500, 0), cp(4500, 6000)], false, 120, false))
    const p = eng.project() as Project
    const f = p.floors[0]
    const pos = new Map(f.nodes.map((n) => [n.id, [n.x_mm, n.y_mm] as const]))
    const wallWhere = (pred: (a: readonly [number, number], b: readonly [number, number]) => boolean) =>
      f.walls.find((w) => pred(pos.get(w.a)!, pos.get(w.b)!))!
    const [ix] = tf(4500, 0)
    const [, top] = tf(0, 0)
    const [right] = tf(8000, 0)
    const [, bottom] = tf(0, 6000)
    const interior = wallWhere((a, b) => a[0] === ix && b[0] === ix)
    const topLeft = wallWhere((a, b) => a[1] === top && b[1] === top && Math.min(a[0], b[0]) < ix)
    const topRight = wallWhere((a, b) => a[1] === top && b[1] === top && Math.min(a[0], b[0]) >= ix)
    const rightW = wallWhere((a, b) => a[0] === right && b[0] === right)
    const bottomLeft = wallWhere((a, b) => a[1] === bottom && b[1] === bottom && Math.min(a[0], b[0]) < ix)
    const op = (wall: string, kind: Opening['kind'], offset: number, width: number, height: number): Opening => ({ id: newId(), wall, kind, offset_mm: offset, width_mm: width, height_mm: height })
    const door = op(interior.id, { type: 'Door', hinge: 'Start', swing: 'Left' }, 2300, 850, 2050)
    const entry = op(bottomLeft.id, { type: 'Door', hinge: 'End', swing: 'Right' }, 600, 950, 2100)
    const win1 = op(topLeft.id, { type: 'Window', sill_mm: 850 }, 900, 1500, 1400)
    const win2 = op(topRight.id, { type: 'Window', sill_mm: 850 }, 1000, 1400, 1400)
    const win3 = op(rightW.id, { type: 'Window', sill_mm: 900 }, 2000, 1200, 1300)
    const roomA = { id: newId(), anchor: tf(2000, 3000), name: 'Wohnen', floor_material: 'wood:oak', ceiling_material: 'paint:white', open_ceiling: false }
    const roomB = { id: newId(), anchor: tf(6200, 3000), name: 'Zimmer', floor_material: 'wood:oak', ceiling_material: 'paint:white', open_ceiling: false }
    const ops: Op[] = [
      ...[door, entry, win1, win2, win3].map((o): Op => ({ op: 'PutOpening', floor, opening: o })),
      { op: 'PutRoom', floor, room: roomA },
      { op: 'PutRoom', floor, room: roomB },
    ]
    eng.apply(ops)
    const empty = new PlanEngine(base)
    const all = empty.diff_to(eng.project()) as Op[]
    empty.free()
    const confidence: Record<string, number> = {}
    for (const w of (eng.project() as Project).floors[0].walls) confidence[w.id] = 0.92
    confidence[interior.id] = 0.45
    for (const o of [door, entry, win1, win2]) confidence[o.id] = 0.85
    confidence[win3.id] = 0.5
    confidence[roomA.id] = 0.9
    confidence[roomB.id] = 0.55
    const img = svgImage()
    return {
      ops: all,
      confidence,
      reference: { a: tf(0, -350), b: tf(8000, -350), length_mm: 8000, label: '8.00' },
      north_angle_deg: 20,
      image: { url: img.url, width_px: img.w, height_px: img.h, origin_mm: [0, 0], mm_per_px: MM_PER_PX, opacity: 0.5, visible: true },
      mm_per_px: MM_PER_PX,
      notes: ['Testdaten (Fixture): Massstab absichtlich 5 % zu klein.', 'Innenwand ohne CV-Treffer – bitte prüfen.'],
      used_llm: false,
    }
  } finally {
    eng.free()
  }
}
