// «Foto rendern»: progressive path tracing (three-gpu-pathtracer) of the current view-mode scene.
// While active, this takes over the R3F render loop (useFrame priority 1, the post chain is unmounted),
// and the scene is temporarily adapted: the sky shader becomes an environment map, shadow-only ceilings
// are hidden, single-sided surfaces become double-sided (no light leaks), glass and bulbs let light through.
import { useEffect, useRef } from 'react'
import { useFrame, useThree } from '@react-three/fiber'
import { FullScreenQuad } from 'three/examples/jsm/postprocessing/Pass.js'
import { AgXToneMapping, Color, DoubleSide, Matrix4, Mesh, Spherical, Vector3, type Material, type Scene, type Texture, type ToneMapping, type WebGLRenderer } from 'three'
import type { ProceduralEquirectTexture, WebGLPathTracer } from 'three-gpu-pathtracer'

type PTModule = typeof import('three-gpu-pathtracer')
import { create } from 'zustand'
import { useEditor } from '../state/editor'
import { useProject } from '../state/project'
import { mats } from './materials3d'
import { live } from './store'

export const PT_TARGET_SAMPLES = 512

type Status = 'prepare' | 'trace' | 'done' | 'error'

type PathTraceState = {
  active: boolean
  status: Status
  samples: number
  error: string | null
  /** Edge-preserving smoothing of the remaining noise (display + PNG; the samples stay untouched). */
  denoise: boolean
  setDenoise: (v: boolean) => void
  start: () => void
  stop: () => void
  set: (s: Partial<Pick<PathTraceState, 'status' | 'samples' | 'error'>>) => void
}

export const usePathTrace = create<PathTraceState>((set) => ({
  active: false,
  status: 'prepare',
  samples: 0,
  error: null,
  denoise: true,
  setDenoise: (denoise) => set({ denoise }),
  start: () => set({ active: true, status: 'prepare', samples: 0, error: null }),
  stop: () => set({ active: false }),
  set: (s) => set(s),
}))

/** Mounted inside the view-mode canvas. Restarts the render when the project or the daylight settings change. */
export function PathTraceHost() {
  const active = usePathTrace((s) => s.active)
  const revision = useProject((s) => s.revision)
  const date = useEditor((s) => s.view.date)
  const minutes = useEditor((s) => s.view.minutes)
  const clearness = useEditor((s) => s.view.clearness)
  const nav = useEditor((s) => s.view.nav)
  if (!active) return null
  return <PathTraceRun key={`${revision}|${date}|${minutes}|${clearness}|${nav}`} />
}

/** Path tracer material flag (three-gpu-pathtracer): false = shadow rays pass through. */
type PTMaterial = Material & { castShadow?: boolean }

const _sph = new Vector3()

/** Sky (upper hemisphere, uniform-ish radiance so that its horizontal irradiance ≈ skyLux) and ground below. */
function skyTexture(lib: PTModule): ProceduralEquirectTexture {
  const { skyLux, sunLux, sinEl, day, clearness } = live.sky
  const scale = live.lightScale
  const tint = new Color('#a9c6ec').lerp(new Color('#d3d6da'), 1 - clearness).lerp(new Color('#26314a'), 1 - day)
  const lum = 0.2126 * tint.r + 0.7152 * tint.g + 0.0722 * tint.b || 1
  const sky = tint.multiplyScalar((skyLux * scale) / Math.PI / lum)
  const ground = new Color('#8a8073').multiplyScalar((0.18 * (skyLux + sunLux * sinEl) * scale) / Math.PI / 0.3)
  const tex = new lib.ProceduralEquirectTexture(128, 64)
  tex.generationCallback = (polar: Spherical, _uv: unknown, _coord: unknown, color: Color) => {
    _sph.setFromSpherical(polar)
    const y = _sph.y
    if (y >= 0) {
      // slightly brighter towards the horizon, mean ≈ 1
      color.copy(sky).multiplyScalar(1.15 - 0.3 * y)
    } else {
      color.copy(ground)
    }
  }
  tex.update()
  return tex
}

/** Temporarily adapts the scene for the path tracer; returns the undo function. */
function prepareScene(scene: Scene, env: Texture): () => void {
  const undo: (() => void)[] = []
  const glass = mats.glass()
  const sides = new Map<Material, number>()
  const shadows = new Map<Material, boolean | undefined>()
  scene.traverse((o) => {
    if (o.userData.noPathTrace && o.visible) {
      o.visible = false
      undo.push(() => (o.visible = true))
      return
    }
    if (!(o instanceof Mesh)) return
    const list: Material[] = Array.isArray(o.material) ? o.material : [o.material]
    if (o.visible && list.some((m) => m.colorWrite === false || m.type === 'ShaderMaterial')) {
      o.visible = false
      undo.push(() => (o.visible = true))
      return
    }
    for (const m of list) {
      if (m.side !== DoubleSide && !sides.has(m)) sides.set(m, m.side)
      if ((m === glass || o.userData.bulb) && !shadows.has(m)) shadows.set(m, (m as PTMaterial).castShadow)
    }
  })
  for (const [m, side] of sides) {
    m.side = DoubleSide
    undo.push(() => (m.side = side as Material['side']))
  }
  for (const [m, prev] of shadows) {
    const sm = m as PTMaterial
    sm.castShadow = false
    undo.push(() => {
      if (prev === undefined) delete sm.castShadow
      else sm.castShadow = prev
    })
  }
  const { environment, background } = scene
  scene.environment = env
  scene.background = env
  undo.push(() => {
    scene.environment = environment
    scene.background = background
  })
  return () => undo.reverse().forEach((f) => f())
}

/** Exposure key of the accumulated image: log-average over block means (sampled rows, 32-px blocks),
 *  so the per-pixel noise of the first samples averages out instead of dragging the key to black. */
function measureKey(pt: WebGLPathTracer, gl: WebGLRenderer): number | null {
  const t = pt.target
  const w = t.width
  const h = t.height
  if (w < 32 || h < 2) return null
  const rows = 32
  const block = 32
  const buf = new Float32Array(w * 4)
  let sum = 0
  let n = 0
  for (let i = 0; i < rows; i++) {
    const y = Math.floor(((i + 0.5) / rows) * h)
    gl.readRenderTargetPixels(t, 0, y, w, 1, buf)
    for (let x0 = 0; x0 + block <= w; x0 += block) {
      let b = 0
      for (let x = x0; x < x0 + block; x++) {
        const l = 0.2126 * buf[x * 4] + 0.7152 * buf[x * 4 + 1] + 0.0722 * buf[x * 4 + 2]
        if (Number.isFinite(l)) b += Math.max(0, l)
      }
      sum += Math.log(1e-4 + b / block)
      n++
    }
  }
  return n ? Math.exp(sum / n) : null
}

function PathTraceRun() {
  const gl = useThree((s) => s.gl)
  const scene = useThree((s) => s.scene)
  const camera = useThree((s) => s.camera)
  const ref = useRef<{ pt: WebGLPathTracer; restore: () => void; env: Texture; denoise: FullScreenQuad; cam: Matrix4; proj: Matrix4; key: number; measuredAt: number } | null>(null)
  const frame = useRef(0)
  const toneRef = useRef<{ toneMapping: ToneMapping; exposure: number } | null>(null)
  const lib = useRef<PTModule | null>(null)

  useEffect(() => {
    let alive = true
    // loaded on demand (≈ 200 kB): most sessions never render a photo
    import('three-gpu-pathtracer')
      .then((m) => {
        if (alive) lib.current = m
      })
      .catch((e) => alive && usePathTrace.getState().set({ status: 'error', error: `Pathtracer nicht ladbar: ${e}` }))
    return () => {
      alive = false
    }
  }, [])

  useEffect(() => {
    toneRef.current = { toneMapping: gl.toneMapping, exposure: gl.toneMappingExposure }
    usePathTrace.getState().set({ status: 'prepare', samples: 0, error: null })
    return () => {
      const r = ref.current
      ref.current = null
      if (r) {
        r.pt.dispose()
        r.restore()
        r.env.dispose()
        r.denoise.material.dispose()
        r.denoise.dispose()
      }
      if (toneRef.current) {
        gl.toneMapping = toneRef.current.toneMapping
        gl.toneMappingExposure = toneRef.current.exposure
      }
    }
  }, [gl])

  useFrame(() => {
    frame.current++
    const st = usePathTrace.getState()
    // build on the 2nd frame: lights have their current intensities (set by priority-0 frame callbacks)
    // and the «wird vorbereitet» message got painted
    if (!ref.current) {
      if (frame.current < 3 || !lib.current || st.status === 'error') {
        gl.render(scene, camera)
        return
      }
      try {
        const env = skyTexture(lib.current)
        const restore = prepareScene(scene, env)
        const pt = new lib.current.WebGLPathTracer(gl)
        pt.renderDelay = 0
        pt.minSamples = 3
        pt.fadeDuration = 0
        const denoise = new FullScreenQuad(new lib.current.DenoiseMaterial())
        const dm = denoise.material as InstanceType<PTModule['DenoiseMaterial']>
        pt.renderToCanvasCallback = (target, renderer, quad) => {
          const auto = renderer.autoClear
          renderer.autoClear = false
          const r = ref.current
          if (usePathTrace.getState().denoise && r && pt.samples >= 8) {
            // threshold in linear HDR units: relative to the image key, so it works at any exposure
            dm.uniforms.map.value = target.texture
            dm.uniforms.threshold.value = 0.5 * r.key
            dm.uniforms.sigma.value = pt.samples < 64 ? 5 : 3.5
            denoise.render(renderer)
          } else {
            quad.render(renderer)
          }
          renderer.autoClear = auto
        }
        pt.bounces = 6
        pt.filterGlossyFactor = 0.5
        pt.tiles.set(2, 2)
        gl.toneMapping = AgXToneMapping
        gl.toneMappingExposure = 1
        try {
          pt.setScene(scene, camera)
        } catch (e) {
          pt.dispose()
          restore()
          env.dispose()
          throw e
        }
        ref.current = { pt, restore, env, denoise, cam: camera.matrixWorld.clone(), proj: camera.projectionMatrix.clone(), key: 1, measuredAt: 0 }
        st.set({ status: 'trace', samples: 0 })
      } catch (e) {
        console.error(e)
        st.set({ status: 'error', error: String(e) })
        gl.render(scene, camera)
        return
      }
    }
    const r = ref.current
    // camera moved (walking, orbit, resize) → start over
    if (!r.cam.equals(camera.matrixWorld) || !r.proj.equals(camera.projectionMatrix)) {
      r.cam.copy(camera.matrixWorld)
      r.proj.copy(camera.projectionMatrix)
      r.pt.updateCamera()
      r.measuredAt = 0
      if (st.status !== 'trace') st.set({ status: 'trace' })
    }
    const n = Math.floor(r.pt.samples)
    r.pt.pausePathTracing = n >= PT_TARGET_SAMPLES
    r.pt.renderSample()
    // auto exposure at a few sample counts (cheap, display-only: no re-render needed)
    for (const at of [4, 16, 64]) {
      if (n >= at && r.measuredAt < at) {
        const k = measureKey(r.pt, gl)
        if (k) r.key = k
        r.measuredAt = at
      }
    }
    gl.toneMappingExposure = Math.min(64, Math.max(1 / 64, (0.22 * useEditor.getState().view.exposure) / r.key))
    if (frame.current % 6 === 0 || n >= PT_TARGET_SAMPLES) {
      const status: Status = n >= PT_TARGET_SAMPLES ? 'done' : 'trace'
      if (st.samples !== n || st.status !== status) st.set({ samples: Math.min(n, PT_TARGET_SAMPLES), status })
    }
  }, 1)

  return null
}

/** DOM overlay with progress, cancel and save. */
export function PathTraceOverlay({ onSave }: { onSave: () => void }) {
  const { active, status, samples, error, stop, denoise, setDenoise } = usePathTrace()
  if (!active) return null
  const pct = Math.round((samples / PT_TARGET_SAMPLES) * 100)
  return (
    <div className="v3d-overlay v3d-pt">
      <div className="v3d-pt-head">
        <strong>{status === 'done' ? 'Foto fertig' : status === 'error' ? 'Rendern fehlgeschlagen' : status === 'prepare' ? 'Szene wird vorbereitet …' : 'Foto wird gerendert …'}</strong>
        {status !== 'prepare' && status !== 'error' && (
          <span className="muted">
            {samples} / {PT_TARGET_SAMPLES} Samples
          </span>
        )}
      </div>
      {status === 'error' && <p className="v3d-pt-err">{error}</p>}
      {status !== 'error' && (
        <div className="v3d-pt-bar" aria-hidden>
          <div style={{ width: `${pct}%` }} />
        </div>
      )}
      {status !== 'error' && (
        <div className="v3d-row v3d-pt-opt">
          <label className="check">
            <input type="checkbox" checked={denoise} onChange={(e) => setDenoise(e.target.checked)} />
            Entrauschen
          </label>
          {status === 'trace' && <span className="muted">Kamera bewegen startet neu</span>}
        </div>
      )}
      <div className="v3d-row">
        <button onClick={stop}>{status === 'done' ? 'Schliessen' : 'Abbrechen'}</button>
        <button className="primary" onClick={onSave} disabled={status === 'prepare' || status === 'error' || samples < 1}>
          Als PNG speichern
        </button>
      </div>
    </div>
  )
}
