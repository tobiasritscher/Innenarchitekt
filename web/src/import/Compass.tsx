// Draggable compass dial for the plan's north angle (deg clockwise of plan "up" to geographic north).
import { useRef } from 'react'

/** Angle (deg, clockwise from up, 0..360) of the vector (dx, dy) in screen coordinates. */
export function angleFromUp(dx: number, dy: number): number {
  const a = (Math.atan2(dx, -dy) * 180) / Math.PI
  return ((a % 360) + 360) % 360
}

export function Compass({ value, onChange, size = 180 }: { value: number; onChange: (deg: number) => void; size?: number }) {
  const ref = useRef<SVGSVGElement>(null)
  const dragging = useRef(false)
  const c = size / 2
  const r = c - 10
  const set = (e: React.PointerEvent) => {
    const b = ref.current!.getBoundingClientRect()
    let a = angleFromUp(e.clientX - b.left - c, e.clientY - b.top - c)
    if (!e.shiftKey) a = Math.round(a / 5) * 5
    onChange(a % 360)
  }
  const ticks = []
  for (let i = 0; i < 72; i++) {
    const a = (i * 5 * Math.PI) / 180
    const l = i % 18 === 0 ? 12 : i % 2 === 0 ? 7 : 4
    ticks.push(<line key={i} x1={c + Math.sin(a) * r} y1={c - Math.cos(a) * r} x2={c + Math.sin(a) * (r - l)} y2={c - Math.cos(a) * (r - l)} />)
  }
  return (
    <svg
      ref={ref}
      className="compass"
      width={size}
      height={size}
      onPointerDown={(e) => {
        dragging.current = true
        e.currentTarget.setPointerCapture(e.pointerId)
        set(e)
      }}
      onPointerMove={(e) => dragging.current && set(e)}
      onPointerUp={() => (dragging.current = false)}
      role="slider"
      aria-label="Nordrichtung"
      aria-valuenow={Math.round(value)}
      aria-valuemin={0}
      aria-valuemax={359}
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === 'ArrowRight' || e.key === 'ArrowUp') onChange((value + (e.shiftKey ? 1 : 5)) % 360)
        if (e.key === 'ArrowLeft' || e.key === 'ArrowDown') onChange((value + 360 - (e.shiftKey ? 1 : 5)) % 360)
      }}
    >
      <circle className="dial" cx={c} cy={c} r={r} />
      <g className="ticks">{ticks}</g>
      <g transform={`rotate(${value} ${c} ${c})`}>
        <path className="needle-n" d={`M${c} ${c - r + 16} L${c + 10} ${c} L${c - 10} ${c} Z`} />
        <path className="needle-s" d={`M${c} ${c + r - 16} L${c + 10} ${c} L${c - 10} ${c} Z`} />
        <text className="n-label" x={c} y={c - r + 32} textAnchor="middle">
          N
        </text>
      </g>
      <circle className="hub" cx={c} cy={c} r={4} />
    </svg>
  )
}
