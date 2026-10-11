// First-person navigation: WASD/arrows + mouse look (pointer lock), drag-to-look, virtual joystick.
// Collision via the core (engine.walk_move, plan mm).
import { useEffect, useRef } from 'react'
import { useFrame, useThree } from '@react-three/fiber'
import { Vector3, type PerspectiveCamera } from 'three'
import { useRooms } from '../core/derived'
import { useEditor } from '../state/editor'
import { useProject } from '../state/project'
import type { Bounds } from './bounds'
import { live, useView3D } from './store'

const SPEED = 1.4 // m/s
const RADIUS = 250 // mm
const LOOK = 0.0024

function isFormTarget(t: EventTarget | null) {
  const el = t as HTMLElement | null
  return !!el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.tagName === 'SELECT' || el.isContentEditable)
}

export function WalkControls({ bounds }: { bounds: Bounds }) {
  const camera = useThree((s) => s.camera) as PerspectiveCamera
  const gl = useThree((s) => s.gl)
  const eye = useEditor((s) => s.view.eyeHeightMm)
  const rooms = useRooms()
  const keys = useRef(new Set<string>())
  const acc = useRef(0)

  // initial pose: saved walker, else the largest room's label point
  useEffect(() => {
    const saved = useEditor.getState().view.walker
    if (saved) {
      live.walker = { ...saved, pitch: 0 }
    } else {
      const big = [...rooms].sort((a, b) => b.area_m2 - a.area_m2)[0]
      const [x, y] = big ? big.label : [bounds.center.x * 1000, bounds.center.z * 1000]
      live.walker = { x, y, yaw: 0, pitch: 0 }
    }
    camera.fov = 70
    camera.near = 0.03
    camera.updateProjectionMatrix()
    return () => {
      const w = live.walker
      useEditor.getState().setView({ walker: { x: w.x, y: w.y, yaw: w.yaw } })
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  useEffect(() => {
    const el = gl.domElement
    const down = (e: KeyboardEvent) => {
      if (isFormTarget(e.target) || e.metaKey || e.ctrlKey) return
      keys.current.add(e.code)
      if (e.code.startsWith('Arrow')) e.preventDefault()
    }
    const up = (e: KeyboardEvent) => keys.current.delete(e.code)
    const blur = () => keys.current.clear()
    const look = (dx: number, dy: number) => {
      const w = live.walker
      w.yaw += dx * LOOK
      w.pitch = Math.max(-1.35, Math.min(1.35, w.pitch - dy * LOOK))
    }
    const onMouse = (e: MouseEvent) => {
      if (document.pointerLockElement === el) look(e.movementX, e.movementY)
    }
    // drag to look (touch, or mouse without pointer lock)
    let drag: { id: number; x: number; y: number; moved: boolean } | null = null
    const pDown = (e: PointerEvent) => {
      if (document.pointerLockElement === el) return
      drag = { id: e.pointerId, x: e.clientX, y: e.clientY, moved: false }
    }
    const pMove = (e: PointerEvent) => {
      if (!drag || drag.id !== e.pointerId) return
      const dx = e.clientX - drag.x
      const dy = e.clientY - drag.y
      if (Math.abs(dx) + Math.abs(dy) > 2) drag.moved = true
      drag.x = e.clientX
      drag.y = e.clientY
      look(-dx * 1.6, -dy * 1.6) // drag the world, like a touch panorama
    }
    const pUp = (e: PointerEvent) => {
      if (!drag || drag.id !== e.pointerId) return
      const wasClick = !drag.moved
      drag = null
      if (wasClick && e.pointerType === 'mouse' && document.pointerLockElement !== el) {
        try {
          const p = el.requestPointerLock() as unknown as Promise<void> | undefined
          p?.catch?.(() => {})
        } catch {
          /* pointer lock unavailable – drag-to-look still works */
        }
      }
    }
    window.addEventListener('keydown', down)
    window.addEventListener('keyup', up)
    window.addEventListener('blur', blur)
    document.addEventListener('mousemove', onMouse)
    el.addEventListener('pointerdown', pDown)
    window.addEventListener('pointermove', pMove)
    window.addEventListener('pointerup', pUp)
    return () => {
      window.removeEventListener('keydown', down)
      window.removeEventListener('keyup', up)
      window.removeEventListener('blur', blur)
      document.removeEventListener('mousemove', onMouse)
      el.removeEventListener('pointerdown', pDown)
      window.removeEventListener('pointermove', pMove)
      window.removeEventListener('pointerup', pUp)
      if (document.pointerLockElement === el) document.exitPointerLock()
    }
  }, [gl])

  const tmp = useRef(new Vector3())
  useFrame((_, dtRaw) => {
    const dt = Math.min(dtRaw, 0.1)
    const w = live.walker
    if (live.teleport) {
      w.x = live.teleport.x
      w.y = live.teleport.y
      w.yaw = live.teleport.yaw
      live.teleport = null
    }
    const k = keys.current
    let fwd = 0
    let side = 0
    let turn = 0
    if (k.has('KeyW') || k.has('ArrowUp')) fwd += 1
    if (k.has('KeyS') || k.has('ArrowDown')) fwd -= 1
    if (k.has('KeyD')) side += 1
    if (k.has('KeyA')) side -= 1
    if (k.has('ArrowRight')) turn += 1
    if (k.has('ArrowLeft')) turn -= 1
    fwd += live.joy.y
    side += live.joy.x
    w.yaw += turn * dt * 1.8
    const len = Math.hypot(fwd, side)
    if (len > 0.01) {
      const n = Math.max(1, len)
      const speed = SPEED * (k.has('ShiftLeft') || k.has('ShiftRight') ? 2.5 : 1) * 1000 * dt
      const cx = Math.cos(w.yaw)
      const sy = Math.sin(w.yaw)
      // plan: forward (cos, sin), right (−sin, cos) (y points down)
      const dx = ((fwd * cx - side * sy) / n) * speed
      const dy = ((fwd * sy + side * cx) / n) * speed
      const engine = useProject.getState().engine
      if (engine) {
        const p = engine.walk_move(w.x, w.y, dx, dy, RADIUS)
        w.x = p[0]
        w.y = p[1]
      } else {
        w.x += dx
        w.y += dy
      }
    }
    camera.position.set(w.x / 1000, eye / 1000, w.y / 1000)
    const cp = Math.cos(w.pitch)
    tmp.current.set(camera.position.x + cp * Math.cos(w.yaw), camera.position.y + Math.sin(w.pitch), camera.position.z + cp * Math.sin(w.yaw))
    camera.lookAt(tmp.current)
    acc.current += dt
    if (acc.current > 0.1) {
      acc.current = 0
      const cur = useView3D.getState().walker
      if (!cur || Math.abs(cur.x - w.x) > 5 || Math.abs(cur.y - w.y) > 5 || Math.abs(cur.yaw - w.yaw) > 0.02) useView3D.getState().setWalker({ x: w.x, y: w.y, yaw: w.yaw })
    }
  })
  return null
}
