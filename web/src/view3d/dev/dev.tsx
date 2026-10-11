// DEV ONLY – not part of the production build (Vite only builds web/index.html).
// Serves the real <App/> with a mocked /api and a test apartment, for working on the 3D side
// while the backend is not available. Open http://localhost:5173/src/view3d/dev/index.html
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { App } from '../../ui/App'
import '../../ui/styles.css'
import { useEditor } from '../../state/editor'
import { useProject } from '../../state/project'
import { useView3D } from '../store'

Object.assign(window, { __v3d: { useEditor, useProject, useView3D } })

const id = () => crypto.randomUUID()
const now = new Date().toISOString()

function asset(name: string, primitive: string, dims: [number, number, number], light: unknown = null) {
  return { id: id(), kind: 'Primitive', name, dims_mm: dims, model_url: null, thumbnail_url: null, primitive, ikea: null, light, warnings: [], created_at: now }
}

const assets = [
  asset('Sofa', 'Sofa', [2200, 950, 850]),
  asset('Esstisch', 'Table', [1600, 900, 750]),
  asset('Stuhl', 'Chair', [450, 520, 850]),
  asset('Bett', 'Bed', [1600, 2050, 900]),
  asset('Schrank', 'Wardrobe', [1500, 600, 2200]),
  asset('Regal', 'Shelf', [800, 350, 1800]),
  asset('Stehlampe', 'Cylinder', [300, 300, 1600], { kind: { type: 'Point' }, offset_mm: [0, 0, 1650], lumen: 1200, color_temp_k: 2700 }),
  { ...asset('Kommode (Modell fehlt)', 'Box', [800, 450, 900]), kind: 'Upload', model_url: '/api/assets/x/model.glb', warnings: ['Abweichung 4 %'] },
]
const glbAsset = { ...asset('Sessel (GLB-Test)', 'Box', [800, 850, 900]), kind: 'Upload', model_url: '/api/assets/demo/model.glb' }
assets.push(glbAsset)

/** A test GLB in centimetres with an off-centre origin, to exercise the auto-normalisation. */
async function demoGlb(): Promise<ArrayBuffer> {
  const THREE = await import('three')
  const { GLTFExporter } = await import('three/examples/jsm/exporters/GLTFExporter.js')
  const g = new THREE.Group()
  const fabric = new THREE.MeshStandardMaterial({ color: '#4f6d7a', roughness: 0.9 })
  const wood = new THREE.MeshStandardMaterial({ color: '#8b5a2b', roughness: 0.6 })
  const box = (w: number, h: number, d: number, x: number, y: number, z: number, m: import("three").Material) => {
    const mesh = new THREE.Mesh(new THREE.BoxGeometry(w, h, d), m)
    mesh.position.set(x, y, z)
    g.add(mesh)
  }
  box(80, 15, 80, 0, 40, 0, fabric) // seat
  box(80, 50, 15, 0, 70, -35, fabric) // back (−Z = back)
  box(12, 25, 80, -34, 55, 0, fabric)
  box(12, 25, 80, 34, 55, 0, fabric)
  for (const [x, z] of [[-35, -35], [35, -35], [-35, 35], [35, 35]]) box(5, 33, 5, x, 16.5, z, wood)
  g.position.set(500, 200, 300)
  const res = await new GLTFExporter().parseAsync(g, { binary: true })
  return res as ArrayBuffer
}

const A = Object.fromEntries(assets.map((a) => [a.name, a]))

function buildProject() {
  const floorId = id()
  const n = (x: number, y: number) => ({ id: id(), x_mm: x, y_mm: y })
  const N = {
    a: n(0, 0),
    b: n(6000, 0),
    c: n(10000, 0),
    d: n(10000, 3500),
    e: n(10000, 7000),
    f: n(6000, 7000),
    g: n(0, 7000),
    h: n(6000, 3500),
  }
  const W = (a: { id: string }, b: { id: string }, t: number, ext: boolean, l = 'paint:white', r = 'paint:white') => ({
    id: id(),
    a: a.id,
    b: b.id,
    thickness_mm: t,
    kind: ext ? 'Exterior' : 'Interior',
    finish_left: l,
    finish_right: r,
    glazed: false,
    height_mm: null,
  })
  // exterior clockwise on screen (a→b→c…): right side = inside
  const north1 = W(N.a, N.b, 300, true, 'stone:concrete')
  const north2 = W(N.b, N.c, 300, true, 'stone:concrete')
  const east1 = W(N.c, N.d, 300, true, 'stone:concrete')
  const east2 = W(N.d, N.e, 300, true, 'stone:concrete')
  const south1 = W(N.e, N.f, 300, true, 'stone:concrete')
  const south2 = W(N.f, N.g, 300, true, 'stone:concrete')
  const west = W(N.g, N.a, 300, true, 'stone:concrete', 'paint:sage')
  const midV1 = { ...W(N.b, N.h, 120, false), glazed: true } // glass partition with a door

  const midV2 = W(N.h, N.f, 120, false)
  const midH = { ...W(N.h, N.d, 120, false, 'paint:greige', 'tile:white'), height_mm: 1100 } // half-height wall
  const walls = [north1, north2, east1, east2, south1, south2, west, midV1, midV2, midH]
  const win = (wall: { id: string }, offset: number, w: number, h = 1500, sill = 800) => ({ id: id(), wall: wall.id, kind: { type: 'Window', sill_mm: sill }, offset_mm: offset, width_mm: w, height_mm: h })
  const door = (wall: { id: string }, offset: number, hinge: 'Start' | 'End', swing: 'Left' | 'Right', w = 900) => ({ id: id(), wall: wall.id, kind: { type: 'Door', hinge, swing }, offset_mm: offset, width_mm: w, height_mm: 2050 })
  const openings = [
    win(south2, 800, 1800, 2100, 200), // living, south (french window)
    win(south2, 3600, 1500), // living, south
    win(west, 2500, 1400), // living, west
    win(east1, 1000, 1400), // bedroom, east
    win(south1, 1200, 1200, 1200, 1000), // bath, south
    door(north1, 600, 'Start', 'Right', 1000), // entrance
    door(midV1, 2300, 'End', 'Left'), // living → bedroom
    door(midV2, 500, 'Start', 'Left', 800), // living → bath
    { id: id(), wall: midH.id, kind: { type: 'Passage' }, offset_mm: 2600, width_mm: 900, height_mm: 2100 },
  ]
  const rooms = [
    { id: id(), anchor: [3000, 3500], name: 'Wohnen', floor_material: 'wood:herringbone', ceiling_material: 'paint:white', open_ceiling: false },
    { id: id(), anchor: [8000, 1700], name: 'Schlafen', floor_material: 'wood:oak', ceiling_material: 'paint:white', open_ceiling: false },
    { id: id(), anchor: [8000, 5200], name: 'Bad', floor_material: 'tile:grey', ceiling_material: 'paint:white', open_ceiling: false },
  ]
  const F = (a: (typeof assets)[number], x: number, y: number, rot = 0, material: string | null = null) => ({ id: id(), asset: a.id, name: a.name, pos_mm: [x, y, 0], rotation_deg: rot, dims_mm: [...a.dims_mm], material })
  const lamp = F(A['Stehlampe'], 600, 6300, 0, 'metal:black')
  const furniture = [
    F(A['Sofa'], 2600, 1500, 0, 'fabric:linen'),
    F(A['Esstisch'], 3000, 5000, 0, 'wood:walnut'),
    F(A['Stuhl'], 2500, 4300, 0),
    F(A['Stuhl'], 3500, 4300, 0),
    F(A['Stuhl'], 2500, 5700, 180),
    F(A['Stuhl'], 3500, 5700, 180),
    F(A['Regal'], 5700, 5000, 90),
    F(A['Bett'], 8000, 1250, 0, 'fabric:linen'),
    F(A['Schrank'], 6600, 2500, 90),
    F(A['Kommode (Modell fehlt)'], 9500, 2600, 270),
    F(A['Regal'], 3100, 5100, 30), // overlaps the table on purpose
    F(A['Sessel (GLB-Test)'], 4600, 2000, 300),
    lamp,
  ]
  const light = (name: string, x: number, y: number, z: number, lumen: number, k: number, kind: unknown = { type: 'Point' }, attached: string | null = null) => ({ id: id(), name, kind, pos_mm: [x, y, z], lumen, color_temp_k: k, on: true, attached_to: attached })
  const lights = [
    light('Pendel Esstisch', 3000, 5000, 1900, 1500, 2700),
    light('Spot Sofa', 2600, 2400, 2450, 600, 3000, { type: 'Spot', angle_deg: 60 }),
    light('Deckenleuchte Schlafen', 8000, 1750, 2450, 1000, 3000),
    light('Panel Bad', 8000, 5250, 2480, 2000, 4000, { type: 'Area', w_mm: 600, h_mm: 600 }),
    light('Licht Stehlampe', 0, 0, 1650, 1200, 2700, { type: 'Point' }, lamp.id),
  ]
  return {
    id: '00000000-0000-4000-8000-00000000d3d0',
    name: 'Testwohnung 3D',
    location: { lat: 47.37, lon: 8.54, address: 'Zürich' },
    north_angle_deg: Number(new URLSearchParams(location.search).get('north') ?? 0),
    floors: [{ id: floorId, name: 'EG', ceiling_height_mm: 2500, nodes: Object.values(N), walls, openings, rooms, reference_image: null }],
    furniture,
    lights,
    version: 1,
  }
}

const project = buildProject()
let version = 1
const realFetch = window.fetch.bind(window)
window.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
  const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url
  const path = new URL(url, location.href).pathname
  if (!path.startsWith('/api/')) return realFetch(input, init)
  const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } })
  const method = init?.method ?? 'GET'
  if (path === '/api/auth') return json({ ok: true })
  if (path === '/api/config') return json({ llm_enabled: false, llm_model: '' })
  if (path === '/api/projects/00000000-0000-4000-8000-00000000d3d0' && method === 'GET') return json({ ...project, version })
  if (path === '/api/projects/00000000-0000-4000-8000-00000000d3d0/ops') return json({ version: ++version })
  if (path === '/api/assets' && method === 'GET') return json(assets)
  if (path === '/api/assets/demo/model.glb') return new Response(await demoGlb(), { headers: { 'Content-Type': 'model/gltf-binary' } })
  if (path === '/api/assets/primitive') {
    const b = JSON.parse(String(init?.body))
    const a = asset(b.name ?? b.shape, b.shape, b.dims_mm ?? [500, 500, 500])
    assets.unshift(a)
    return json(a)
  }
  if (path === '/api/assets/ikea') return json({ error: 'Produktseite konnte nicht geladen werden (Dev-Mock)' }, 502)
  if (path.startsWith('/api/assets/') && method === 'DELETE') return new Response(null, { status: 204 })
  return json({ error: 'nicht gefunden (Dev-Mock)' }, 404)
}

if (!new URLSearchParams(location.search).get('p')) history.replaceState(null, '', `${location.pathname}?p=00000000-0000-4000-8000-00000000d3d0`)

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
