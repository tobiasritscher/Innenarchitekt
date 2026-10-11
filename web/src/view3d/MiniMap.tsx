// Mini floor plan (SVG) with walker position; click/tap = teleport into walk mode.
import { useRef, type PointerEvent as RPointerEvent } from 'react'
import { useFootprints, useRooms } from '../core/derived'
import { useEditor } from '../state/editor'
import { useBounds } from './bounds'
import { live, useView3D } from './store'

export function MiniMap() {
  const fps = useFootprints()
  const rooms = useRooms()
  const b = useBounds()
  const walker = useView3D((s) => s.walker)
  const nav = useEditor((s) => s.view.nav)
  const svg = useRef<SVGSVGElement>(null)
  const pad = 600
  const vb = [b.minX - pad, b.minY - pad, b.maxX - b.minX + 2 * pad, b.maxY - b.minY + 2 * pad]
  const s = Math.max(vb[2], vb[3]) / 100 // 1 % of the map, in mm

  const teleport = (e: RPointerEvent<SVGSVGElement>) => {
    const el = svg.current
    const m = el?.getScreenCTM()
    if (!el || !m) return
    const p = new DOMPoint(e.clientX, e.clientY).matrixTransform(m.inverse())
    const yaw = live.walker.yaw
    const w = { x: Math.round(p.x), y: Math.round(p.y), yaw }
    live.teleport = w
    live.walker = { ...w, pitch: 0 }
    useView3D.getState().setWalker(w)
    useEditor.getState().setView({ walker: w, nav: 'walk' })
  }

  return (
    <div className="v3d-minimap" title="Antippen, um dorthin zu gehen">
      <svg ref={svg} viewBox={vb.join(' ')} preserveAspectRatio="xMidYMid meet" onPointerUp={teleport}>
        {rooms.map((r) => (
          <polygon key={r.key} points={r.polygon.map((p) => p.join(',')).join(' ')} className="v3d-mm-room" />
        ))}
        {fps.map((f) => (
          <polygon key={f.wall} points={f.corners.map((p) => p.join(',')).join(' ')} className="v3d-mm-wall" />
        ))}
        {walker && (
          <g className={nav === 'walk' ? '' : 'v3d-mm-idle'} transform={`translate(${walker.x} ${walker.y}) rotate(${(walker.yaw * 180) / Math.PI})`}>
            <path d={`M 0 0 L ${s * 9} ${-s * 5} A ${s * 10} ${s * 10} 0 0 1 ${s * 9} ${s * 5} Z`} className="v3d-mm-fov" />
            <circle r={s * 2.4} className="v3d-mm-walker" />
          </g>
        )}
      </svg>
    </div>
  )
}

/** Touch joystick: writes live.joy while dragged. */
export function Joystick() {
  const knob = useRef<HTMLDivElement>(null)
  const base = useRef<HTMLDivElement>(null)
  const active = useRef<number | null>(null)
  const set = (cx: number, cy: number) => {
    const el = base.current
    if (!el) return
    const r = el.getBoundingClientRect()
    const R = r.width / 2
    let dx = cx - (r.left + R)
    let dy = cy - (r.top + R)
    const l = Math.hypot(dx, dy)
    if (l > R) {
      dx = (dx / l) * R
      dy = (dy / l) * R
    }
    live.joy = { x: dx / R, y: -dy / R }
    if (knob.current) knob.current.style.transform = `translate(${dx}px, ${dy}px)`
  }
  const reset = () => {
    active.current = null
    live.joy = { x: 0, y: 0 }
    if (knob.current) knob.current.style.transform = ''
  }
  return (
    <div
      ref={base}
      className="v3d-joystick"
      onPointerDown={(e) => {
        active.current = e.pointerId
        ;(e.target as HTMLElement).setPointerCapture(e.pointerId)
        set(e.clientX, e.clientY)
      }}
      onPointerMove={(e) => active.current === e.pointerId && set(e.clientX, e.clientY)}
      onPointerUp={reset}
      onPointerCancel={reset}
    >
      <div ref={knob} className="v3d-joystick-knob" />
    </div>
  )
}
