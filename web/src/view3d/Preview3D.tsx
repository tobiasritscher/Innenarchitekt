// Modelling-mode 3D preview: orbit camera, live rebuild per revision, click-to-select, hover,
// drag furniture on the floor with wall snapping, place library assets in 3D.
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef } from 'react'
import { Canvas, useThree, type ThreeEvent } from '@react-three/fiber'
import { OrbitControls } from '@react-three/drei'
import { DirectionalLight, type PerspectiveCamera, MeshStandardMaterial, Object3D, PCFShadowMap, Plane, Raycaster, Vector2, Vector3 } from 'three'
import type { OrbitControls as OrbitControlsImpl } from 'three-stdlib'
import { instantiateAsset } from '../core/furniture'
import type { FurnitureInstance } from '../core/generated/FurnitureInstance'
import type { SnapResult } from '../core/generated/SnapResult'
import { useAssets } from '../state/assets'
import { useEditor, type Selection } from '../state/editor'
import { floorId, useProject } from '../state/project'
import { Building, type PickHandlers } from './Building'
import { useBounds, type Bounds } from './bounds'
import { RendererSetup, fitCamera } from './common'
import { FurnitureLayer, PrimitiveModel, furnitureRotationY } from './Furniture'
import { LightMarkers } from './Lights'
import { setWallCut } from './materials3d'
import { useView3D } from './store'

const SNAP_MM = 150
/** Time of the last 3D placement: the click that placed must not also select the floor below. */
let justPlaced = 0
const recentlyPlaced = () => performance.now() - justPlaced < 400

function cssVar(name: string, fallback: string) {
  if (typeof document === 'undefined') return fallback
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim()
  return v || fallback
}

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
        <PreviewContent cutWalls={cutWalls} />
      </Canvas>
      <div className="v3d-overlay v3d-preview-tools">
        <button aria-pressed={cutWalls} onClick={() => setCutWalls(!cutWalls)} title="Wände auf 1.2 m Höhe abschneiden">
          Wände kappen
        </button>
      </div>
      {placing && <PlacingHint assetId={placing} />}
    </div>
  )
}

function PlacingHint({ assetId }: { assetId: string }) {
  const asset = useAssets((s) => s.assets.find((a) => a.id === assetId))
  return (
    <div className="v3d-overlay v3d-hint">
      {asset?.name ?? 'Möbel'} platzieren: auf den Boden klicken · R drehen · Esc abbrechen
    </div>
  )
}

function PreviewContent({ cutWalls }: { cutWalls: boolean }) {
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
    setWallCut(cutWalls ? 1.2 : null)
    invalidate()
    return () => setWallCut(null)
  }, [cutWalls, invalidate])

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

  const pick: PickHandlers = useMemo(
    () => ({
      onClick: (e, s) => {
        e.stopPropagation()
        if (e.delta > 5 || useEditor.getState().placingAsset || recentlyPlaced()) return
        useEditor.getState().select(s)
      },
      onOver: (e, s) => {
        e.stopPropagation()
        if (!useEditor.getState().placingAsset) useEditor.getState().setHover(s)
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
      if (e.button !== 0 || useEditor.getState().placingAsset) return
      e.stopPropagation()
      const ne = e.nativeEvent
      const hit = groundHit(ne.clientX, ne.clientY, 0)
      if (!hit) return
      useEditor.getState().select({ kind: 'furniture', id: item.id })
      const off = [item.pos_mm[0] - hit.x * 1000, item.pos_mm[1] - hit.z * 1000]
      const sx = ne.clientX
      const sy = ne.clientY
      let moved = false
      if (controls) controls.enabled = false
      const move = (ev: PointerEvent) => {
        if (!moved && Math.hypot(ev.clientX - sx, ev.clientY - sy) < 4) return
        moved = true
        const h = groundHit(ev.clientX, ev.clientY, 0)
        if (!h) return
        let x = h.x * 1000 + off[0]
        let y = h.z * 1000 + off[1]
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
        st.setTransient([{ op: 'PutFurniture', item: { ...item, pos_mm: [Math.round(x), Math.round(y), item.pos_mm[2]], rotation_deg: rot } }])
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
    [groundHit, controls],
  )

  const furniturePick = useMemo(() => ({ ...pick, onDown }), [pick, onDown])

  return (
    <>
      <PreviewLights bounds={bounds} />
      <Building ceilings="none" pick={pick} highlight />
      <FurnitureLayer pick={furniturePick} highlight />
      <LightMarkers pick={pick} />
      {placing && <Placement assetId={placing} groundHit={groundHit} />}
      <OrbitControls makeDefault enableDamping={false} maxPolarAngle={Math.PI * 0.49} minDistance={1} maxDistance={80} />
      <FitCamera bounds={bounds} />
      <mesh rotation={[-Math.PI / 2, 0, 0]} position={[bounds.center.x, -0.01, bounds.center.z]} receiveShadow raycast={() => {}}>
        <planeGeometry args={[bounds.radius * 6, bounds.radius * 6]} />
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

const ghostMat = new MeshStandardMaterial({ color: '#1f6feb', transparent: true, opacity: 0.45, depthWrite: false })

function Placement({ assetId, groundHit }: { assetId: string; groundHit: (x: number, y: number) => Vector3 | null }) {
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
      const h = groundHit(cx, cy)
      if (!h) return
      lastRef.current = { x: cx, y: cy }
      let x = h.x * 1000
      let y = h.z * 1000
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
      const [ops, id] = instantiateAsset(asset, g.x, g.y, g.rot)
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
  }, [asset, gl, groundHit, invalidate])

  useLayoutEffect(() => invalidate(), [ghost, invalidate])
  if (!asset || !ghost) return null
  return (
    <group position={[ghost.x / 1000, 0, ghost.y / 1000]} rotation={[0, furnitureRotationY(ghost.rot), 0]}>
      <PrimitiveModel shape={asset.primitive ?? 'Box'} dims={asset.dims_mm} material={null} castShadow={false} override={ghostMat} />
    </group>
  )
}
