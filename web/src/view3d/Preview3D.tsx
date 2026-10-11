// Modelling-mode 3D preview: orbit camera, live rebuild per revision, click-to-select, hover,
// drag furniture on the floor with wall snapping, place library assets in 3D.
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { Canvas, useFrame, useThree, type ThreeEvent } from '@react-three/fiber'
import { OrbitControls } from '@react-three/drei'
import { SphereGeometry, DirectionalLight, type PerspectiveCamera, MeshStandardMaterial, Object3D, PCFShadowMap, Plane, Raycaster, Vector2, Vector3 } from 'three'
import type { OrbitControls as OrbitControlsImpl } from 'three-stdlib'
import { instantiateAsset, movedElevation, standHeight } from '../core/furniture'
import type { FurnitureInstance } from '../core/generated/FurnitureInstance'
import type { Light } from '../core/generated/Light'
import type { SnapResult } from '../core/generated/SnapResult'
import { useAssets } from '../state/assets'
import { useEditor, type Selection } from '../state/editor'
import { floorId, useProject } from '../state/project'
import { Building, type PickHandlers } from './Building'
import { useBounds, type Bounds } from './bounds'
import { RendererSetup, cssVar, fitCamera } from './common'
import { FurnitureLayer, PrimitiveModel, furnitureRotationY } from './Furniture'
import { LightMarkers, type PlacedLight } from './Lights'
import { setWallCut, wallClip } from './materials3d'
import type { DerivedRoom } from '../core/generated/DerivedRoom'
import { pointInPolygon } from '../editor2d/geom'
import { Ruler3D, RulerButton } from './Ruler3D'
import { toolActive, useView3D, type LampType } from './store'
import { newCeilingLight } from '../editor2d/model'

const SNAP_MM = 150
/** Time of the last 3D placement: the click that placed must not also select the floor below. */
let justPlaced = 0
export const recentlyPlaced = () => performance.now() - justPlaced < 400


export function Preview3D() {
  const cutWalls = useView3D((s) => s.cutWalls)
  const setCutWalls = useView3D((s) => s.setCutWalls)
  const placing = useEditor((s) => s.placingAsset)
  const bg = useMemo(() => cssVar('--plan-bg', '#fbfaf8'), [])
  return (
    <div className={`v3d-preview ${placing ? 'v3d-placing' : ''}`}>
      <Canvas
        frameloop="demand"
        shadows={{ enabled: true, type: PCFShadowMap }}
        dpr={[1, 1.5]}
        camera={{ fov: 45, near: 0.05, far: 500, position: [8, 10, 12] }}
        gl={{ antialias: true }}
        onPointerMissed={(e) => {
          if (e.button === 0 && !useEditor.getState().placingAsset && !recentlyPlaced()) useEditor.getState().select(null)
        }}
      >
        <color attach="background" args={[bg]} />
        <RendererSetup />
        <EditScene cutWalls={cutWalls} />
      </Canvas>
      <div className="v3d-overlay v3d-preview-tools">
        <button aria-pressed={cutWalls} onClick={() => setCutWalls(!cutWalls)} title="Wände auf 1.2 m Höhe abschneiden">
          Wände kappen
        </button>
        <RulerButton />
      </div>
      {placing && <PlacingHint assetId={placing} />}
    </div>
  )
}

export function PlacingHint({ assetId }: { assetId: string }) {
  const asset = useAssets((s) => s.assets.find((a) => a.id === assetId))
  return (
    <div className="v3d-overlay v3d-hint">
      {asset?.name ?? 'Möbel'} platzieren: auf den Boden klicken · R drehen · Esc abbrechen
    </div>
  )
}

/** Editable 3D scene: select, drag furniture and lamps, place assets. With `room` the scene shows only
 *  that room (walls facing the camera cut away), the camera orbits the room and edits stay inside it. */
export function EditScene({ cutWalls = false, room = null }: { cutWalls?: boolean; room?: DerivedRoom | null }) {
  const bounds = useBounds()
  const camera = useThree((s) => s.camera)
  const gl = useThree((s) => s.gl)
  const controls = useThree((s) => s.controls) as OrbitControlsImpl | null
  const invalidate = useThree((s) => s.invalidate)
  const placing = useEditor((s) => s.placingAsset)
  // demand frameloop: redraw after any model/selection change, also ones made outside the canvas
  const revision = useProject((s) => s.revision)
  const selection = useEditor((s) => s.selection)
  const hover = useEditor((s) => s.hover)
  const assetsVersion = useAssets((s) => s.assets)
  useLayoutEffect(() => invalidate(), [revision, selection, hover, assetsVersion, invalidate])

  useEffect(() => {
    setWallCut(cutWalls && !room ? 1.2 : null)
    invalidate()
    return () => setWallCut(null)
  }, [cutWalls, room, invalidate])

  /** Plan point (mm) allowed for edits: anywhere, or inside the focused room. */
  const inside = useCallback((x: number, y: number) => !room || pointInPolygon([x, y], room.polygon as [number, number][]), [room])

  const raycaster = useMemo(() => new Raycaster(), [])
  const groundHit = useCallback(
    (clientX: number, clientY: number, y = 0): Vector3 | null => {
      const r = gl.domElement.getBoundingClientRect()
      const ndc = new Vector2(((clientX - r.left) / r.width) * 2 - 1, -((clientY - r.top) / r.height) * 2 + 1)
      raycaster.setFromCamera(ndc, camera)
      const out = new Vector3()
      return raycaster.ray.intersectPlane(new Plane(new Vector3(0, 1, 0), -y), out)
    },
    [gl, camera, raycaster],
  )

  const scene = useThree((s) => s.scene)
  /** Plan point (mm) for furniture under the cursor. Pointing at a wall (in front of the floor behind it)
   *  yields a point just in front of the wall on its room side, so the item never lands outside. */
  const floorPoint = useCallback(
    (clientX: number, clientY: number, depthMm: number): [number, number] | null => {
      const ground = groundHit(clientX, clientY, 0)
      const wallHit = raycaster
        .intersectObjects(scene.children, true)
        .find((h) => typeof h.object.userData.wall === 'string' && wallClip.distanceToPoint(h.point) >= 0)
      const st = useProject.getState()
      const f = st.project?.floors[0]
      if (!wallHit || !f || !st.engine || (ground && ground.distanceTo(raycaster.ray.origin) < wallHit.distance)) {
        return ground ? [ground.x * 1000, ground.z * 1000] : null
      }
      const wall = f.walls.find((w) => w.id === wallHit.object.userData.wall)
      const a = wall && f.nodes.find((n) => n.id === wall.a)
      const b = wall && f.nodes.find((n) => n.id === wall.b)
      if (!wall || !a || !b) return ground ? [ground.x * 1000, ground.z * 1000] : null
      const len = Math.hypot(b.x_mm - a.x_mm, b.y_mm - a.y_mm) || 1
      const n = [-(b.y_mm - a.y_mm) / len, (b.x_mm - a.x_mm) / len]
      const p = [wallHit.point.x * 1000, wallHit.point.z * 1000]
      const off = wall.thickness_mm / 2 + depthMm / 2 + 20
      const cands = [1, -1].map((sg) => [p[0] + sg * n[0] * off, p[1] + sg * n[1] * off] as [number, number])
      const rooms = st.engine.rooms(f.id) as DerivedRoom[]
      const inRoom = (q: [number, number]) => rooms.some((r) => pointInPolygon(q, r.polygon as [number, number][]))
      const cam = [camera.position.x * 1000 - p[0], camera.position.z * 1000 - p[1]]
      const camSide = cam[0] * n[0] + cam[1] * n[1] >= 0 ? 0 : 1
      const order = [cands[camSide], cands[1 - camSide]]
      return order.find(inRoom) ?? order[0]
    },
    [groundHit, raycaster, scene, camera],
  )

  const pick: PickHandlers = useMemo(
    () => ({
      onClick: (e, s) => {
        e.stopPropagation()
        if (e.delta > 5 || useEditor.getState().placingAsset || recentlyPlaced() || toolActive()) return
        useEditor.getState().select(s)
      },
      onOver: (e, s) => {
        e.stopPropagation()
        if (!useEditor.getState().placingAsset && !toolActive()) useEditor.getState().setHover(s)
      },
      onOut: (_e, s) => {
        const h = useEditor.getState().hover
        if (h && h.kind === s.kind && h.id === s.id) useEditor.getState().setHover(null)
      },
    }),
    [],
  )

  const onDown = useCallback(
    (e: ThreeEvent<PointerEvent>, item: FurnitureInstance) => {
      if (e.button !== 0 || useEditor.getState().placingAsset || toolActive()) return
      e.stopPropagation()
      const ne = e.nativeEvent
      const hit = groundHit(ne.clientX, ne.clientY, 0)
      if (!hit) return
      useEditor.getState().select({ kind: 'furniture', id: item.id })
      const grab = [item.pos_mm[0] - hit.x * 1000, item.pos_mm[1] - hit.z * 1000]
      const sx = ne.clientX
      const sy = ne.clientY
      let moved = false
      if (controls) controls.enabled = false
      const move = (ev: PointerEvent) => {
        if (!moved && Math.hypot(ev.clientX - sx, ev.clientY - sy) < 4) return
        moved = true
        const h = floorPoint(ev.clientX, ev.clientY, item.dims_mm[1])
        if (!h) return
        // keep the grab offset only while over the floor; over a wall the point is already placed
        const g = groundHit(ev.clientX, ev.clientY, 0)
        const onFloor = !!g && Math.hypot(g.x * 1000 - h[0], g.z * 1000 - h[1]) < 1
        let x = h[0] + (onFloor ? grab[0] : 0)
        let y = h[1] + (onFloor ? grab[1] : 0)
        let rot = item.rotation_deg
        const st = useProject.getState()
        if (!ev.altKey && useEditor.getState().snapping && st.engine) {
          try {
            const s = st.engine.snap_furniture(floorId(), x, y, Uint32Array.from(item.dims_mm), rot, SNAP_MM) as SnapResult | null
            if (s) {
              x = s.x_mm
              y = s.y_mm
              rot = s.rotation_deg
            }
          } catch (err) {
            console.warn(err)
          }
        }
        if (!inside(x, y)) return
        st.setTransient([{ op: 'PutFurniture', item: { ...item, pos_mm: [Math.round(x), Math.round(y), movedElevation(st.engine, item, x, y)], rotation_deg: rot } }])
      }
      const up = () => {
        window.removeEventListener('pointermove', move)
        window.removeEventListener('pointerup', up)
        window.removeEventListener('pointercancel', up)
        if (controls) controls.enabled = true
        if (moved) useProject.getState().endTransient(true, 'Möbel verschieben')
      }
      window.addEventListener('pointermove', move)
      window.addEventListener('pointerup', up)
      window.addEventListener('pointercancel', up)
    },
    [groundHit, floorPoint, controls, inside],
  )

  /** Drag a free lamp in the horizontal plane at its own height (attached lamps move with their furniture). */
  const onLightDown = useCallback(
    (e: ThreeEvent<PointerEvent>, p: PlacedLight) => {
      if (e.button !== 0 || useEditor.getState().placingAsset || toolActive()) return
      e.stopPropagation()
      useEditor.getState().select({ kind: 'light', id: p.light.id })
      if (p.light.attached_to) return
      const ne = e.nativeEvent
      const h0 = p.pos.y
      const hit = groundHit(ne.clientX, ne.clientY, h0)
      if (!hit) return
      const grab = [p.light.pos_mm[0] - hit.x * 1000, p.light.pos_mm[1] - hit.z * 1000]
      const sx = ne.clientX
      const sy = ne.clientY
      let moved = false
      if (controls) controls.enabled = false
      const move = (ev: PointerEvent) => {
        if (!moved && Math.hypot(ev.clientX - sx, ev.clientY - sy) < 4) return
        moved = true
        const h = groundHit(ev.clientX, ev.clientY, h0)
        if (!h) return
        const x = Math.round(h.x * 1000 + grab[0])
        const y = Math.round(h.z * 1000 + grab[1])
        if (!inside(x, y)) return
        useProject.getState().setTransient([{ op: 'PutLight', light: { ...p.light, pos_mm: [x, y, p.light.pos_mm[2]] } }])
      }
      const up = () => {
        window.removeEventListener('pointermove', move)
        window.removeEventListener('pointerup', up)
        window.removeEventListener('pointercancel', up)
        if (controls) controls.enabled = true
        if (moved) useProject.getState().endTransient(true, 'Lampe verschieben')
      }
      window.addEventListener('pointermove', move)
      window.addEventListener('pointerup', up)
      window.addEventListener('pointercancel', up)
    },
    [groundHit, controls, inside],
  )

  const furniturePick = useMemo(() => ({ ...pick, onDown }), [pick, onDown])
  const lightPick = useMemo(() => ({ ...pick, onDown: onLightDown }), [pick, onLightDown])
  /** Room mode edits only furniture and lamps: clicking a door opens/shuts it, the rest clears the selection. */
  const buildingPick: PickHandlers = useMemo(
    () => ({
      onClick: (e, s) => {
        e.stopPropagation()
        if (e.delta > 5 || useEditor.getState().placingAsset || recentlyPlaced() || toolActive()) return
        const door = s.kind === 'opening' && useProject.getState().project?.floors[0].openings.some((o) => o.id === s.id && o.kind.type === 'Door')
        if (door) useView3D.getState().toggleDoor(s.id)
        else useEditor.getState().select(null)
      },
    }),
    [],
  )
  const furnitureIn = useCallback((f: FurnitureInstance) => inside(f.pos_mm[0], f.pos_mm[1]), [inside])
  const lightIn = useCallback((p: PlacedLight) => inside(p.pos.x * 1000, p.pos.z * 1000), [inside])
  const ground = room ? roomFrame(room) : { cx: bounds.center.x, cz: bounds.center.z, r: bounds.radius }

  return (
    <>
      <PreviewLights bounds={bounds} />
      <Building ceilings="none" pick={room ? buildingPick : pick} highlight={!room} caps={cutWalls && !room} focus={room} />
      <FurnitureLayer pick={furniturePick} highlight only={room ? furnitureIn : undefined} />
      <LightMarkers pick={lightPick} only={room ? lightIn : undefined} />
      {placing && <Placement assetId={placing} floorPoint={floorPoint} inside={inside} />}
      <Ruler3D />
      <LightPlacement inside={inside} />
      {room ? (
        <RoomCamera room={room} />
      ) : (
        <>
          <OrbitControls makeDefault enableDamping={false} maxPolarAngle={Math.PI * 0.49} minDistance={1} maxDistance={80} />
          <FitCamera bounds={bounds} />
        </>
      )}
      <mesh rotation={[-Math.PI / 2, 0, 0]} position={[ground.cx, -0.01, ground.cz]} receiveShadow raycast={() => {}}>
        <planeGeometry args={[ground.r * 6, ground.r * 6]} />
        <shadowMaterial opacity={0.12} />
      </mesh>
    </>
  )
}

function PreviewLights({ bounds }: { bounds: Bounds }) {
  const ref = useRef<DirectionalLight>(null)
  const target = useMemo(() => new Object3D(), [])
  const r = bounds.radius + 1
  useLayoutEffect(() => {
    const l = ref.current
    if (!l) return
    target.position.copy(bounds.center)
    target.updateMatrixWorld()
    l.position.copy(bounds.center).add(new Vector3(-0.45, 0.8, 0.4).normalize().multiplyScalar(r + 20))
    const c = l.shadow.camera
    c.left = -r
    c.right = r
    c.top = r
    c.bottom = -r
    c.near = 1
    c.far = 2 * (r + 20)
    c.updateProjectionMatrix()
    l.shadow.mapSize.set(2048, 2048)
    l.shadow.bias = -0.0004
    l.shadow.normalBias = 0.02
  }, [bounds, r, target])
  return (
    <>
      <hemisphereLight args={['#ffffff', '#b9ae9e', 1.7]} />
      <primitive object={target} />
      <directionalLight ref={ref} target={target} intensity={2.8} castShadow />
    </>
  )
}

function FitCamera({ bounds }: { bounds: Bounds }) {
  const camera = useThree((s) => s.camera)
  const controls = useThree((s) => s.controls) as OrbitControlsImpl | null
  const invalidate = useThree((s) => s.invalidate)
  const projectId = useProject((s) => s.project?.id)
  const fitted = useRef<string | null>(null)
  useEffect(() => {
    if (!controls || bounds.empty || !projectId || fitted.current === projectId) return
    fitted.current = projectId
    const c = bounds.center
    const r = bounds.radius
    fitCamera(camera as PerspectiveCamera, controls, c, r)
    controls.update()
    invalidate()
  }, [bounds, controls, camera, projectId, invalidate])
  return null
}

const LAMPS: Record<LampType, { name: string; kind: Light['kind']; lumen: number }> = {
  Point: { name: 'Deckenleuchte', kind: { type: 'Point' }, lumen: 800 },
  Spot: { name: 'Spot', kind: { type: 'Spot', angle_deg: 40, tilt_deg: 0, heading_deg: 0 }, lumen: 450 },
  Area: { name: 'Leuchtfläche', kind: { type: 'Area', w_mm: 600, h_mm: 600 }, lumen: 3000 },
}
export const LAMP_LABEL: Record<LampType, string> = { Point: 'Deckenleuchte', Spot: 'Spot', Area: 'Leuchtfläche' }
const lampGhost = new MeshStandardMaterial({ color: '#ffd27a', emissive: '#ffd27a', emissiveIntensity: 1.5, transparent: true, opacity: 0.8, depthTest: false })
const lampGeo = new SphereGeometry(0.07, 16, 12)

/** Click to place a ceiling lamp (room mode): the cursor is projected onto the ceiling plane. */
function LightPlacement({ inside }: { inside: (x: number, y: number) => boolean }) {
  const type = useView3D((s) => s.placingLight)
  const gl = useThree((s) => s.gl)
  const camera = useThree((s) => s.camera)
  const invalidate = useThree((s) => s.invalidate)
  const ceiling = useProject((s) => s.project?.floors[0].ceiling_height_mm ?? 2500)
  const [ghost, setGhost] = useState<[number, number] | null>(null)
  const raycaster = useMemo(() => new Raycaster(), [])

  useEffect(() => {
    if (!type) {
      setGhost(null)
      return
    }
    const el = gl.domElement
    const h = Math.max(0, ceiling - 100)
    const at = (cx: number, cy: number): [number, number] | null => {
      const r = el.getBoundingClientRect()
      raycaster.setFromCamera(new Vector2(((cx - r.left) / r.width) * 2 - 1, -((cy - r.top) / r.height) * 2 + 1), camera)
      const p = raycaster.ray.intersectPlane(new Plane(new Vector3(0, 1, 0), -h / 1000), new Vector3())
      if (!p) return null
      const q: [number, number] = [Math.round(p.x * 1000), Math.round(p.z * 1000)]
      return inside(q[0], q[1]) ? q : null
    }
    let down: { x: number; y: number } | null = null
    const onMove = (e: PointerEvent) => {
      setGhost(at(e.clientX, e.clientY))
      invalidate()
    }
    const onDown = (e: PointerEvent) => {
      if (e.button === 0) down = { x: e.clientX, y: e.clientY }
    }
    const onUp = (e: PointerEvent) => {
      if (!down || e.button !== 0) return
      const moved = Math.hypot(e.clientX - down.x, e.clientY - down.y) > 5
      down = null
      const p = moved ? null : at(e.clientX, e.clientY)
      const project = useProject.getState().project
      if (!p || !project) return
      const def = LAMPS[type]
      const l = { ...newCeilingLight(project, p), name: def.name, kind: def.kind, lumen: def.lumen }
      justPlaced = performance.now()
      if (useProject.getState().commit([{ op: 'PutLight', light: l }], `${def.name} setzen`)) {
        // Shift keeps placing (several spots in a row)
        if (!e.shiftKey) useView3D.getState().setPlacingLight(null)
        useEditor.getState().select({ kind: 'light', id: l.id })
      }
    }
    const onLeave = () => setGhost(null)
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') useView3D.getState().setPlacingLight(null)
    }
    const cursor = el.style.cursor
    el.style.cursor = 'copy'
    el.addEventListener('pointermove', onMove)
    el.addEventListener('pointerdown', onDown)
    el.addEventListener('pointerup', onUp)
    el.addEventListener('pointerleave', onLeave)
    window.addEventListener('keydown', onKey)
    return () => {
      el.style.cursor = cursor
      el.removeEventListener('pointermove', onMove)
      el.removeEventListener('pointerdown', onDown)
      el.removeEventListener('pointerup', onUp)
      el.removeEventListener('pointerleave', onLeave)
      window.removeEventListener('keydown', onKey)
    }
  }, [type, gl, camera, raycaster, ceiling, inside, invalidate])

  useLayoutEffect(() => invalidate(), [ghost, invalidate])
  if (!type || !ghost) return null
  return <mesh geometry={lampGeo} material={lampGhost} position={[ghost[0] / 1000, Math.max(0, ceiling - 100) / 1000, ghost[1] / 1000]} renderOrder={10} raycast={() => {}} />
}

/** Centre (m, three frame) and horizontal half-diagonal (m) of a room's net polygon. */
export function roomFrame(room: DerivedRoom): { cx: number; cz: number; r: number } {
  const pts = room.inner.length ? room.inner : room.polygon
  let minX = Infinity
  let minY = Infinity
  let maxX = -Infinity
  let maxY = -Infinity
  for (const [x, y] of pts) {
    minX = Math.min(minX, x)
    minY = Math.min(minY, y)
    maxX = Math.max(maxX, x)
    maxY = Math.max(maxY, y)
  }
  return { cx: (minX + maxX) / 2000, cz: (minY + maxY) / 2000, r: Math.max(1, Math.hypot(maxX - minX, maxY - minY) / 2000) }
}

/** Orbit camera around the focused room: no panning, eases to the room on focus change or «Ansicht zurücksetzen». */
function RoomCamera({ room }: { room: DerivedRoom }) {
  const camera = useThree((s) => s.camera) as PerspectiveCamera
  const controls = useThree((s) => s.controls) as OrbitControlsImpl | null
  const invalidate = useThree((s) => s.invalidate)
  const fitN = useView3D((s) => s.roomFit)
  // refit on a new room only, not on every edit
  const frame = useMemo(() => roomFrame(room), [room.key]) // eslint-disable-line react-hooks/exhaustive-deps
  const anim = useRef<{ t: number; p0: Vector3; p1: Vector3; c0: Vector3; c1: Vector3 } | null>(null)

  useEffect(() => {
    if (!controls) return
    const c1 = new Vector3(frame.cx, 0, frame.cz)
    // keep the horizontal viewing direction, look down at ~50°
    const az = Math.atan2(camera.position.x - controls.target.x, camera.position.z - controls.target.z)
    const el = 0.88
    const fov = (camera.fov * Math.PI) / 180
    const d = (frame.r * 1.15) / Math.tan(fov / 2) / Math.min(1, camera.aspect)
    const p1 = c1.clone().add(new Vector3(Math.cos(el) * Math.sin(az), Math.sin(el), Math.cos(el) * Math.cos(az)).multiplyScalar(d))
    anim.current = { t: 0, p0: camera.position.clone(), p1, c0: controls.target.clone(), c1 }
    invalidate()
  }, [frame, controls, camera, fitN, invalidate])

  useFrame((state, dt) => {
    const a = anim.current
    if (!a || !controls) return
    a.t = Math.min(1, a.t + Math.min(dt, 0.1) / 0.45)
    const k = 1 - Math.pow(1 - a.t, 3)
    camera.position.lerpVectors(a.p0, a.p1, k)
    controls.target.lerpVectors(a.c0, a.c1, k)
    controls.update()
    if (a.t >= 1) anim.current = null
    else state.invalidate()
  })

  return <OrbitControls makeDefault enableDamping dampingFactor={0.12} enablePan={false} minDistance={1.2} maxDistance={Math.max(8, frame.r * 6)} maxPolarAngle={Math.PI * 0.47} />
}

const ghostMat = new MeshStandardMaterial({ color: '#1f6feb', transparent: true, opacity: 0.45, depthWrite: false })

function Placement({ assetId, floorPoint, inside }: { assetId: string; floorPoint: (x: number, y: number, depthMm: number) => [number, number] | null; inside: (x: number, y: number) => boolean }) {
  const asset = useAssets((s) => s.assets.find((a) => a.id === assetId))
  const gl = useThree((s) => s.gl)
  const invalidate = useThree((s) => s.invalidate)
  const ghost = useView3D((s) => s.ghost)
  const rotRef = useRef(0)
  const lastRef = useRef<{ x: number; y: number } | null>(null)

  useEffect(() => {
    if (!asset) return
    const el = gl.domElement
    const update = (cx: number, cy: number, altKey: boolean) => {
      const h = floorPoint(cx, cy, asset.dims_mm[1])
      if (!h) return
      lastRef.current = { x: cx, y: cy }
      let x = h[0]
      let y = h[1]
      let rot = rotRef.current
      const st = useProject.getState()
      if (!altKey && useEditor.getState().snapping && st.engine) {
        try {
          const s = st.engine.snap_furniture(floorId(), x, y, Uint32Array.from(asset.dims_mm), rot, SNAP_MM) as SnapResult | null
          if (s) {
            x = s.x_mm
            y = s.y_mm
            rot = s.rotation_deg
          }
        } catch (err) {
          console.warn(err)
        }
      }
      if (!inside(x, y)) return
      useView3D.getState().setGhost({ x: Math.round(x), y: Math.round(y), rot })
      invalidate()
    }
    let down: { x: number; y: number } | null = null
    const onMove = (e: PointerEvent) => update(e.clientX, e.clientY, e.altKey)
    const onDown = (e: PointerEvent) => {
      if (e.button === 0) down = { x: e.clientX, y: e.clientY }
    }
    const onUp = (e: PointerEvent) => {
      if (!down || e.button !== 0) return
      const d = Math.hypot(e.clientX - down.x, e.clientY - down.y)
      down = null
      if (d > 5) return
      update(e.clientX, e.clientY, e.altKey)
      const g = useView3D.getState().ghost
      if (!g) return
      const [ops, id] = instantiateAsset(asset, g.x, g.y, g.rot, standHeight(useProject.getState().engine, g.x, g.y))
      justPlaced = performance.now()
      if (useProject.getState().commit(ops, `${asset.name} platzieren`)) {
        useEditor.getState().setPlacingAsset(null)
        useEditor.getState().select({ kind: 'furniture', id } as Selection)
      }
    }
    const onLeave = () => {
      useView3D.getState().setGhost(null)
      invalidate()
    }
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT')) return
      if (e.key === 'Escape') useEditor.getState().setPlacingAsset(null)
      if (e.key === 'r' || e.key === 'R') {
        rotRef.current = (rotRef.current + 90) % 360
        if (lastRef.current) update(lastRef.current.x, lastRef.current.y, true)
      }
    }
    el.addEventListener('pointermove', onMove)
    el.addEventListener('pointerdown', onDown)
    el.addEventListener('pointerup', onUp)
    el.addEventListener('pointerleave', onLeave)
    window.addEventListener('keydown', onKey)
    return () => {
      el.removeEventListener('pointermove', onMove)
      el.removeEventListener('pointerdown', onDown)
      el.removeEventListener('pointerup', onUp)
      el.removeEventListener('pointerleave', onLeave)
      window.removeEventListener('keydown', onKey)
      useView3D.getState().setGhost(null)
    }
  }, [asset, gl, floorPoint, inside, invalidate])

  useLayoutEffect(() => invalidate(), [ghost, invalidate])
  if (!asset || !ghost) return null
  return (
    <group position={[ghost.x / 1000, 0, ghost.y / 1000]} rotation={[0, furnitureRotationY(ghost.rot), 0]}>
      <PrimitiveModel shape={asset.primitive ?? 'Box'} dims={asset.dims_mm} material={null} castShadow={false} override={ghostMat} />
    </group>
  )
}
