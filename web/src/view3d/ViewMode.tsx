// View mode: dollhouse orbit or first-person walk-through with daylight and artificial light.
import { useEffect, useMemo, useRef, useState } from 'react'
import { Canvas, useThree } from '@react-three/fiber'
import { OrbitControls } from '@react-three/drei'
import { EffectComposer, N8AO, ToneMapping } from '@react-three/postprocessing'
import { ToneMappingMode } from 'postprocessing'
import { PCFShadowMap, type PerspectiveCamera, type WebGLRenderer } from 'three'
import type { OrbitControls as OrbitControlsImpl } from 'three-stdlib'
import { useEditor, type Quality } from '../state/editor'
import { fmtNumber } from '../core/format'
import { Building } from './Building'
import { useBounds, type Bounds } from './bounds'
import { QUALITY, QUALITY_LABEL, RendererSetup, fitCamera } from './common'
import { Daylight } from './Daylight'
import { DaylightControls, LampList, LightScenes, saveCanvasPng, useLampLux } from './Mood'
import { FurnitureLayer } from './Furniture'
import { LightRig } from './Lights'
import { setWallCut } from './materials3d'
import { Joystick, MiniMap } from './MiniMap'
import { PathTraceHost, PathTraceOverlay, usePathTrace } from './PathTrace'
import { Ruler3D, RulerButton } from './Ruler3D'
import { WalkControls } from './WalkControls'
import { DoorsButton, useDoorCollision, useDoorIds, useDoorKey } from './doors'


export function ViewMode() {
  const quality = useEditor((s) => s.view.quality)
  const nav = useEditor((s) => s.view.nav)
  const q = QUALITY[quality]
  const glRef = useRef<WebGLRenderer | null>(null)
  const dpr = Math.min(q.dpr, typeof window !== 'undefined' ? window.devicePixelRatio || 1 : 1)
  const coarse = useMemo(() => typeof window !== 'undefined' && window.matchMedia?.('(pointer: coarse)').matches, [])
  const tracing = usePathTrace((s) => s.active)
  const doors = useDoorIds()
  useDoorCollision()
  useDoorKey(nav === 'walk')

  useEffect(() => setWallCut(null), [])
  // leaving view mode ends a running render
  useEffect(() => () => usePathTrace.getState().stop(), [])

  return (
    <div className="v3d-view">
      <Canvas
        shadows={{ enabled: true, type: PCFShadowMap }}
        dpr={dpr}
        gl={{ antialias: true, preserveDrawingBuffer: true }}
        camera={{ fov: 50, near: 0.03, far: 2500, position: [10, 12, 14] }}
        onCreated={({ gl }) => {
          glRef.current = gl
        }}
      >
        <RendererSetup postTone={q.ao} />
        <ViewScene />
        <PathTraceHost />
        {q.ao && !tracing && (
          <EffectComposer multisampling={4}>
            <N8AO aoRadius={0.6} distanceFalloff={0.6} intensity={2.2} halfRes quality="medium" />
            <ToneMapping mode={ToneMappingMode.AGX} />
          </EffectComposer>
        )}
      </Canvas>
      <ViewPanel glRef={glRef} />
      <PathTraceOverlay onSave={() => saveCanvasPng(glRef.current, 'Foto')} />
      <MiniMap />
      <div className="v3d-overlay v3d-preview-tools">
        {nav === 'dollhouse' && <RulerButton hotkey />}
        <DoorsButton ids={doors} />
      </div>
      {nav === 'walk' && coarse && <Joystick />}
      {nav === 'walk' && !coarse && <div className="v3d-overlay v3d-hint v3d-walk-hint">Klicken zum Umsehen · WASD/Pfeiltasten gehen · Shift schneller · E Tür auf/zu · Esc gibt die Maus frei</div>}
    </div>
  )
}

function ViewScene() {
  const nav = useEditor((s) => s.view.nav)
  const quality = useEditor((s) => s.view.quality)
  const q = QUALITY[quality]
  const bounds = useBounds()
  const lampLux = useLampLux()
  const walk = nav === 'walk'
  return (
    <>
      <Daylight walk={walk} bounds={bounds} mapSize={q.sunMap} lampLux={lampLux} />
      <Building ceilings={walk ? 'visible' : 'shadow'} />
      <FurnitureLayer />
      <LightRig budget={q.budget} mapSize={q.lampMap} />
      {walk ? <WalkControls bounds={bounds} /> : <Dollhouse bounds={bounds} />}
      {!walk && <Ruler3D />}
    </>
  )
}

function Dollhouse({ bounds }: { bounds: Bounds }) {
  const camera = useThree((s) => s.camera) as PerspectiveCamera
  const controls = useThree((s) => s.controls) as OrbitControlsImpl | null
  useEffect(() => {
    if (!controls) return
    const c = bounds.center
    const r = bounds.radius
    camera.fov = 45
    camera.near = 0.05
    camera.updateProjectionMatrix()
    fitCamera(camera, controls, c, r)
    controls.update()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [controls])
  return <OrbitControls makeDefault enableDamping dampingFactor={0.12} maxPolarAngle={Math.PI * 0.47} minDistance={2} maxDistance={120} />
}

// ---------------------------------------------------------------- controls panel

function ViewPanel({ glRef }: { glRef: React.RefObject<WebGLRenderer | null> }) {
  const view = useEditor((s) => s.view)
  const setView = useEditor((s) => s.setView)
  const setMode = useEditor((s) => s.setMode)
  const [open, setOpen] = useState(true)

  const screenshot = () => saveCanvasPng(glRef.current)
  const tracing = usePathTrace((s) => s.active)

  return (
    <div className={`v3d-panel ${open ? '' : 'collapsed'}`}>
      <div className="v3d-panel-head">
        <button className="ghost" onClick={() => setMode('model')} title="Zurück zum Modellieren">
          ← Modellieren
        </button>
        <div className="v3d-seg" role="tablist">
          <button role="tab" aria-selected={view.nav === 'dollhouse'} onClick={() => setView({ nav: 'dollhouse' })}>
            Puppenhaus
          </button>
          <button role="tab" aria-selected={view.nav === 'walk'} onClick={() => setView({ nav: 'walk' })}>
            Begehen
          </button>
        </div>
        <button className="ghost v3d-collapse" onClick={() => setOpen(!open)} title={open ? 'Einklappen' : 'Ausklappen'}>
          {open ? '▾' : '▸'}
        </button>
      </div>
      {open && (
        <div className="v3d-panel-body">
          {view.nav === 'dollhouse' && !view.walker && <p className="v3d-note">Tipp: In der Karte einen Startpunkt antippen, um die Wohnung zu begehen.</p>}
          <DaylightControls />

          <h3>Lampen</h3>
          <LampList hint={<>Keine Lampen im Projekt. <button className="link" onClick={() => setMode('light')}>Im Lichtmodus setzen</button></>} />
          <LightScenes />

          <h3>Darstellung</h3>
          <div className="v3d-seg v3d-seg-full" role="tablist">
            {(Object.keys(QUALITY) as Quality[]).map((k) => (
              <button key={k} role="tab" aria-selected={view.quality === k} onClick={() => setView({ quality: k })}>
                {QUALITY_LABEL[k]}
              </button>
            ))}
          </div>
          <label className="field">
            <span className="v3d-row">
              Augenhöhe <strong>{fmtNumber(view.eyeHeightMm / 1000, 2)} m</strong>
            </span>
            <input type="range" min={900} max={2000} step={10} value={view.eyeHeightMm} onChange={(e) => setView({ eyeHeightMm: Number(e.target.value) })} />
          </label>
          <div className="v3d-row v3d-shots">
            <button className="v3d-shot" onClick={screenshot} disabled={tracing}>
              Screenshot (PNG)
            </button>
            <button className="primary v3d-shot" onClick={() => usePathTrace.getState().start()} disabled={tracing} title="Fotorealistisches Standbild mit Pathtracing (dauert je nach Gerät 1–3 Minuten)">
              Foto rendern
            </button>
          </div>
        </div>
      )}
    </div>
  )
}

