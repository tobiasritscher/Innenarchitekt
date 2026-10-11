// 3D ruler («Messen»): click a point A on any surface, then B; shows the distance. Snaps to the corners of
// the triangle under the cursor (furniture edges, wall corners), Shift locks B to the dominant axis from A.
import { useEffect, useMemo, useRef, useState } from 'react'
import { useThree } from '@react-three/fiber'
import { Html, Line } from '@react-three/drei'
import { Mesh, MeshBasicMaterial, Plane, Raycaster, SphereGeometry, Vector2, Vector3, type Object3D } from 'three'
import { fmtLength } from '../core/format'
import { useEditor } from '../state/editor'
import { noRaycast } from './Building'
import { cssVar } from './common'
import { wallClip } from './materials3d'
import { useView3D } from './store'

const SNAP_PX = 12
const dotGeo = new SphereGeometry(0.025, 12, 8)
const dotMat = new MeshBasicMaterial({ depthTest: false, transparent: true })

function visibleChain(o: Object3D | null): boolean {
  for (; o; o = o.parent) if (!o.visible) return false
  return true
}

export function Ruler3D() {
  const active = useView3D((s) => s.ruler)
  const gl = useThree((s) => s.gl)
  const camera = useThree((s) => s.camera)
  const scene = useThree((s) => s.scene)
  const invalidate = useThree((s) => s.invalidate)
  const [a, setA] = useState<Vector3 | null>(null)
  const [b, setB] = useState<Vector3 | null>(null)
  const [hover, setHover] = useState<Vector3 | null>(null)
  const state = useRef({ a, b })
  state.current = { a, b }
  const raycaster = useMemo(() => new Raycaster(), [])
  const color = useMemo(() => {
    const c = cssVar('--accent', '#c8642e')
    dotMat.color.set(c)
    return c
  }, [])

  useEffect(() => {
    if (!active) {
      setA(null)
      setB(null)
      setHover(null)
      return
    }
    const el = gl.domElement
    const rect = () => el.getBoundingClientRect()
    const toScreen = (p: Vector3) => {
      const r = rect()
      const v = p.clone().project(camera)
      return new Vector2(((v.x + 1) / 2) * r.width, ((1 - v.y) / 2) * r.height)
    }
    const hit = (cx: number, cy: number, shift: boolean): Vector3 | null => {
      const r = rect()
      const ndc = new Vector2(((cx - r.left) / r.width) * 2 - 1, -((cy - r.top) / r.height) * 2 + 1)
      raycaster.setFromCamera(ndc, camera)
      const h = raycaster
        .intersectObjects(scene.children, true)
        .find((x) => (x.object as Mesh).isMesh && !x.object.userData.ruler && visibleChain(x.object) && wallClip.distanceToPoint(x.point) >= 0)
      let p: Vector3 | null = h ? h.point.clone() : raycaster.ray.intersectPlane(new Plane(new Vector3(0, 1, 0), 0), new Vector3())
      if (h?.face && p) {
        // snap to a corner of the hit triangle when it is close on screen
        const pos = (h.object as Mesh).geometry.getAttribute('position')
        const mouse = new Vector2(cx - r.left, cy - r.top)
        let best: { v: Vector3; d: number } | null = null
        for (const i of [h.face.a, h.face.b, h.face.c]) {
          const v = new Vector3().fromBufferAttribute(pos, i).applyMatrix4(h.object.matrixWorld)
          const d = toScreen(v).distanceTo(mouse)
          if (d < SNAP_PX && (!best || d < best.d)) best = { v, d }
        }
        if (best) p = best.v
      }
      const A = state.current.a
      if (p && shift && A && !state.current.b) {
        const d = p.clone().sub(A)
        const ax = Math.abs(d.x) >= Math.abs(d.y) && Math.abs(d.x) >= Math.abs(d.z) ? 'x' : Math.abs(d.y) >= Math.abs(d.z) ? 'y' : 'z'
        p = A.clone()
        p[ax] += d[ax]
      }
      return p
    }
    let down: { x: number; y: number } | null = null
    const onMove = (e: PointerEvent) => {
      setHover(hit(e.clientX, e.clientY, e.shiftKey))
      invalidate()
    }
    const onDown = (e: PointerEvent) => {
      if (e.button === 0) down = { x: e.clientX, y: e.clientY }
    }
    const onUp = (e: PointerEvent) => {
      if (!down || e.button !== 0) return
      const moved = Math.hypot(e.clientX - down.x, e.clientY - down.y) > 5
      down = null
      if (moved) return
      const p = hit(e.clientX, e.clientY, e.shiftKey)
      if (!p) return
      const { a: A, b: B } = state.current
      if (!A || B) {
        setA(p)
        setB(null)
      } else setB(p)
      invalidate()
    }
    const onLeave = () => setHover(null)
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      if (state.current.a) {
        setA(null)
        setB(null)
      } else useView3D.getState().setRuler(false)
      invalidate()
    }
    const cursor = el.style.cursor
    el.style.cursor = 'crosshair'
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
  }, [active, gl, camera, scene, raycaster, invalidate])

  useEffect(() => invalidate(), [a, b, hover, invalidate])

  if (!active) return null
  const end = b ?? hover
  const len = a && end ? a.distanceTo(end) * 1000 : 0
  const dh = a && end ? Math.abs(end.y - a.y) * 1000 : 0
  const dxz = a && end ? Math.hypot(end.x - a.x, end.z - a.z) * 1000 : 0
  return (
    <group userData={{ ruler: true }}>
      {hover && !b && <mesh geometry={dotGeo} material={dotMat} position={hover} renderOrder={10} raycast={noRaycast} userData={{ ruler: true }} />}
      {a && <mesh geometry={dotGeo} material={dotMat} position={a} renderOrder={10} raycast={noRaycast} userData={{ ruler: true }} />}
      {b && <mesh geometry={dotGeo} material={dotMat} position={b} renderOrder={10} raycast={noRaycast} userData={{ ruler: true }} />}
      {a && end && len > 1 && (
        <>
          <Line points={[a, end]} color={color} lineWidth={2.5} depthTest={false} renderOrder={10} raycast={noRaycast} userData={{ ruler: true }} />
          <Html position={a.clone().add(end).multiplyScalar(0.5)} center zIndexRange={[5, 0]} style={{ pointerEvents: 'none' }}>
            <div className="v3d-ruler-label">
              <strong>{fmtLength(Math.round(len))}</strong>
              {dh > 20 && dxz > 20 && (
                <span>
                  ↔ {fmtLength(Math.round(dxz))} · ↕ {fmtLength(Math.round(dh))}
                </span>
              )}
            </div>
          </Html>
        </>
      )}
    </group>
  )
}

/** Toggle for the overlay tool bars; `hotkey` = D toggles too (not in the modelling preview, where D is the 2D tool). */
export function RulerButton({ hotkey = false }: { hotkey?: boolean }) {
  const active = useView3D((s) => s.ruler)
  useEffect(() => {
    if (!hotkey) return
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT')) return
      if (e.metaKey || e.ctrlKey || e.altKey || e.key.toLowerCase() !== 'd') return
      toggleRuler()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [hotkey])
  // leaving the view ends measuring
  useEffect(() => () => useView3D.getState().setRuler(false), [])
  return (
    <button aria-pressed={active} onClick={toggleRuler} title={`Distanz messen: Punkt A und B anklicken · Shift = achsparallel · Esc = löschen${hotkey ? ' (D)' : ''}`}>
      📏 Messen
    </button>
  )
}

function toggleRuler() {
  const on = !useView3D.getState().ruler
  if (on) useEditor.getState().setPlacingAsset(null)
  useView3D.getState().setRuler(on)
}
