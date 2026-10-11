// Step 2: drag four corners to rectify a skewed photo (with a loupe while dragging).
import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import type { PreparedImage } from './image'

export type Corners = [number, number][]

const LOUPE = 128
const LOUPE_ZOOM = 3
const LABELS = ['oben links', 'oben rechts', 'unten rechts', 'unten links']

export function defaultCorners(img: { width: number; height: number }): Corners {
  return [
    [0, 0],
    [img.width, 0],
    [img.width, img.height],
    [0, img.height],
  ]
}

export function isDefaultCorners(c: Corners, img: { width: number; height: number }): boolean {
  return defaultCorners(img).every((d, i) => Math.abs(d[0] - c[i][0]) < 1 && Math.abs(d[1] - c[i][1]) < 1)
}

export function CropStep({ image, initial, onBack, onDone }: { image: PreparedImage; initial: Corners | null; onBack: () => void; onDone: (c: Corners | null) => void }) {
  const wrap = useRef<HTMLDivElement>(null)
  const loupeRef = useRef<HTMLCanvasElement>(null)
  const imgEl = useRef<HTMLImageElement | null>(null)
  const [box, setBox] = useState({ w: 0, h: 0 })
  const [corners, setCorners] = useState<Corners>(initial ?? defaultCorners(image))
  const [drag, setDrag] = useState<number | null>(null)

  useLayoutEffect(() => {
    const el = wrap.current
    if (!el) return
    const ro = new ResizeObserver(() => setBox({ w: el.clientWidth, h: el.clientHeight }))
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  useEffect(() => {
    const i = new Image()
    i.src = image.url
    imgEl.current = i
  }, [image.url])

  const pad = 24
  const s = box.w ? Math.min((box.w - 2 * pad) / image.width, (box.h - 2 * pad) / image.height) : 1
  const ox = (box.w - image.width * s) / 2
  const oy = (box.h - image.height * s) / 2
  const toScreen = (p: [number, number]) => [ox + p[0] * s, oy + p[1] * s] as const

  const drawLoupe = (c: [number, number]) => {
    const cv = loupeRef.current
    const im = imgEl.current
    if (!cv || !im || !im.complete) return
    const ctx = cv.getContext('2d')!
    const r = LOUPE / (2 * LOUPE_ZOOM * s)
    ctx.fillStyle = '#888'
    ctx.fillRect(0, 0, LOUPE, LOUPE)
    ctx.imageSmoothingEnabled = false
    ctx.drawImage(im, c[0] - r, c[1] - r, 2 * r, 2 * r, 0, 0, LOUPE, LOUPE)
    ctx.strokeStyle = '#e0457b'
    ctx.lineWidth = 1
    ctx.beginPath()
    ctx.moveTo(LOUPE / 2, 0)
    ctx.lineTo(LOUPE / 2, LOUPE)
    ctx.moveTo(0, LOUPE / 2)
    ctx.lineTo(LOUPE, LOUPE / 2)
    ctx.stroke()
  }

  const onMove = (e: React.PointerEvent) => {
    if (drag === null || !wrap.current) return
    const r = wrap.current.getBoundingClientRect()
    const x = Math.max(0, Math.min(image.width, (e.clientX - r.left - ox) / s))
    const y = Math.max(0, Math.min(image.height, (e.clientY - r.top - oy) / s))
    const next = corners.map((c, i) => (i === drag ? [x, y] : c)) as Corners
    setCorners(next)
    drawLoupe([x, y])
  }

  useEffect(() => {
    if (drag !== null) drawLoupe(corners[drag])
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [drag])

  const pts = corners.map((c) => toScreen(c))
  const dragPt = drag !== null ? pts[drag] : null

  return (
    <div className="import-step crop-step">
      <div className="crop-head">
        <div>
          <h2>Zuschneiden und entzerren</h2>
          <p className="muted">Ziehe die vier Ecken auf die Ecken des Plans, falls das Foto schief ist.</p>
        </div>
        <div className="row actions">
          <button onClick={onBack}>Zurück</button>
          <button onClick={() => setCorners(defaultCorners(image))}>Zurücksetzen</button>
          <button onClick={() => onDone(null)} title="Das Bild ist bereits gerade">
            Überspringen
          </button>
          <button className="primary" onClick={() => onDone(isDefaultCorners(corners, image) ? null : corners.map(([x, y]) => [Math.round(x), Math.round(y)]))}>
            Weiter
          </button>
        </div>
      </div>
      <div ref={wrap} className="crop-area" onPointerMove={onMove} onPointerUp={() => setDrag(null)} onPointerCancel={() => setDrag(null)}>
        {box.w > 0 && (
          <>
            <img src={image.url} alt="Grundriss" style={{ left: ox, top: oy, width: image.width * s, height: image.height * s }} draggable={false} />
            <svg width={box.w} height={box.h}>
              <path
                className="crop-shade"
                fillRule="evenodd"
                d={`M0 0H${box.w}V${box.h}H0Z M${pts.map((p) => `${p[0]} ${p[1]}`).join(' L')} Z`}
              />
              <polygon className="crop-quad" points={pts.map((p) => p.join(',')).join(' ')} />
              {pts.map((p, i) => (
                <g key={i}>
                  <circle
                    className={`crop-handle${drag === i ? ' active' : ''}`}
                    cx={p[0]}
                    cy={p[1]}
                    r={11}
                    onPointerDown={(e) => {
                      ;(e.currentTarget.ownerSVGElement?.parentElement as HTMLElement | null)?.setPointerCapture(e.pointerId)
                      setDrag(i)
                    }}
                  >
                    <title>{LABELS[i]}</title>
                  </circle>
                </g>
              ))}
            </svg>
            {dragPt && (
              <canvas
                ref={loupeRef}
                className="loupe"
                width={LOUPE}
                height={LOUPE}
                style={{
                  left: Math.min(box.w - LOUPE - 8, Math.max(8, dragPt[0] + (dragPt[0] > box.w / 2 ? -LOUPE - 30 : 30))),
                  top: Math.min(box.h - LOUPE - 8, Math.max(8, dragPt[1] - LOUPE - 30)),
                }}
              />
            )}
          </>
        )}
      </div>
    </div>
  )
}
