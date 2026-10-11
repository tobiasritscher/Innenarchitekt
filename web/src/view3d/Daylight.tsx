// Sun + physical sky + diffuse sky light for view mode. Also sets the global light scale (exposure).
import { useEffect, useLayoutEffect, useMemo, useRef } from 'react'
import { useFrame } from '@react-three/fiber'
import { BackSide, CircleGeometry, Color, Object3D, Vector3, type DirectionalLight, type HemisphereLight, type ShaderMaterial } from 'three'
import { Sky } from 'three/examples/jsm/objects/Sky.js'
import { sunPosition } from '../core/engine'
import type { SunPosition } from '../core/generated/SunPosition'
import { useEditor } from '../state/editor'
import { useProject } from '../state/project'
import type { Bounds } from './bounds'
import { mats } from './materials3d'
import { daylight, kelvinToColor } from './photometry'
import { live } from './store'

export const DEFAULT_LOCATION = { lat: 47.37, lon: 8.54 }

/** Direction towards the sun in three coordinates, per docs/conventions.md. */
export function sunDirection(azimuthDeg: number, elevationDeg: number, northDeg: number, target = new Vector3()): Vector3 {
  const a = ((northDeg + azimuthDeg) * Math.PI) / 180
  const el = (elevationDeg * Math.PI) / 180
  return target.set(Math.cos(el) * Math.sin(a), Math.sin(el), -Math.cos(el) * Math.cos(a))
}

export function useSun() {
  const location = useProject((s) => s.project?.location ?? null)
  const north = useProject((s) => s.project?.north_angle_deg ?? 0)
  const date = useEditor((s) => s.view.date)
  const minutes = useEditor((s) => s.view.minutes)
  return useMemo(() => {
    const loc = location ?? DEFAULT_LOCATION
    const [y, m, d] = date.split('-').map(Number)
    const local = new Date(y, (m || 1) - 1, d || 1, Math.floor(minutes / 60), minutes % 60)
    const utcOffset = -local.getTimezoneOffset()
    let pos: SunPosition = { azimuth_deg: 180, elevation_deg: 45 }
    try {
      pos = sunPosition(loc.lat, loc.lon, y, m, d, minutes, utcOffset) as SunPosition
    } catch (e) {
      console.warn(e)
    }
    return { ...pos, dir: sunDirection(pos.azimuth_deg, pos.elevation_deg, north), located: !!location }
  }, [location, north, date, minutes])
}

/** Physically motivated daylight. `walk` = camera indoors (interior exposure). `lampLux` = rough artificial illuminance. */
export function Daylight({ walk, bounds, mapSize, lampLux }: { walk: boolean; bounds: Bounds; mapSize: number; lampLux: number }) {
  const sun = useSun()
  const clearness = useEditor((s) => s.view.clearness)
  const exposure = useEditor((s) => s.view.exposure)
  const sunRef = useRef<DirectionalLight>(null)
  const hemiRef = useRef<HemisphereLight>(null)
  const target = useMemo(() => new Object3D(), [])

  const sky = useMemo(() => {
    const s = new Sky()
    s.scale.setScalar(1500)
    const mat = s.material as ShaderMaterial
    mat.uniforms.skyScale = { value: 1 }
    mat.onBeforeCompile = (shader) => {
      shader.uniforms.skyScale = mat.uniforms.skyScale
      shader.fragmentShader = 'uniform float skyScale;\n' + shader.fragmentShader.replace('gl_FragColor = vec4( texColor, 1.0 );', 'gl_FragColor = vec4( texColor * skyScale, 1.0 );')
    }
    mat.side = BackSide
    return s
  }, [])
  useEffect(
    () => () => {
      sky.geometry.dispose()
      ;(sky.material as ShaderMaterial).dispose()
    },
    [sky],
  )

  const sinEl = Math.max(0, Math.sin((sun.elevation_deg * Math.PI) / 180))
  const dl = daylight(sun.elevation_deg, clearness)
  // Fake inter-reflection: interiors receive a fraction of the sky (daylight factor) plus bounce of the sun patches.
  const hemiLux = walk ? dl.skyLux * 0.1 + dl.sunLux * sinEl * 0.012 : dl.skyLux * 1.1 + dl.sunLux * sinEl * 0.08
  const keyLux = Math.max(walk ? hemiLux : dl.skyLux * 0.6 + dl.sunLux * sinEl * 0.3, lampLux, 0.4)
  const scale = (exposure * (walk ? 3 : 2.6)) / keyLux
  live.lightScale = scale

  const D = bounds.radius + 30
  const r3 = Math.hypot(bounds.radius, 1.5) + 0.5

  useLayoutEffect(() => {
    const l = sunRef.current
    if (!l) return
    if (l.shadow.mapSize.x !== mapSize) {
      l.shadow.mapSize.set(mapSize, mapSize)
      l.shadow.map?.dispose()
      l.shadow.map = null
    }
    const cam = l.shadow.camera
    cam.left = -r3
    cam.right = r3
    cam.top = r3
    cam.bottom = -r3
    cam.near = 1
    cam.far = 2 * D
    cam.updateProjectionMatrix()
    l.shadow.bias = -0.0004
    l.shadow.normalBias = 0.025
    l.shadow.radius = 2
  }, [mapSize, r3, D])

  const sunTemp = clearness < 0.3 ? 6500 : 2200 + 3600 * Math.min(1, Math.sqrt(sinEl / 0.5))
  const sunColor = useMemo(() => kelvinToColor(sunTemp), [Math.round(sunTemp / 50)]) // eslint-disable-line react-hooks/exhaustive-deps
  const skyColor = useMemo(() => {
    // indoors the "sky" term stands for diffuse daylight through windows → nearly neutral
    const clear = new Color(walk ? '#dfe7f1' : '#a9c6ec')
    const grey = new Color(walk ? '#e4e6e8' : '#d3d6da')
    const night = new Color('#26314a')
    return clear.lerp(grey, 1 - clearness).lerp(night, 1 - dl.day)
  }, [clearness, dl.day, walk])

  const pos = useMemo(() => bounds.center.clone().add(new Vector3(0, 1.25, 0)).addScaledVector(sun.dir, D), [bounds.center, sun.dir, D])

  useLayoutEffect(() => {
    target.position.copy(bounds.center).setY(1.25)
    target.updateMatrixWorld()
  }, [target, bounds.center])

  useFrame(() => {
    live.lightScale = scale
    if (sunRef.current) sunRef.current.intensity = dl.sunLux * scale
    if (hemiRef.current) hemiRef.current.intensity = hemiLux * scale
    const u = (sky.material as ShaderMaterial).uniforms
    u.sunPosition.value.copy(sun.dir)
    u.turbidity.value = 2.5 + (1 - clearness) * 9
    u.rayleigh.value = 1 + (1 - clearness) * 1.5
    u.mieCoefficient.value = 0.005
    u.cloudCoverage.value = 0.15 + (1 - clearness) * 0.8
    u.cloudDensity.value = 0.35 + (1 - clearness) * 0.6
    u.showSunDisc.value = clearness > 0.35 ? 1 : 0
    // the sky shader is calibrated for exposure ≈ 0.5; map it into our scale (diffuse sky ≈ 16'000 lux)
    u.skyScale.value = scale * 4000 * (walk ? 1 : 0.8)
  })

  return (
    <>
      <primitive object={sky} />
      <primitive object={target} />
      <directionalLight ref={sunRef} position={pos} target={target} color={sunColor} castShadow />
      <hemisphereLight ref={hemiRef} color={skyColor} groundColor={walk ? '#cbc2b6' : '#8a8073'} />
      <mesh geometry={groundGeo} material={mats.ground()} rotation={[-Math.PI / 2, 0, 0]} position={[bounds.center.x, -0.02, bounds.center.z]} receiveShadow />
    </>
  )
}

const groundGeo = new CircleGeometry(250, 64)
