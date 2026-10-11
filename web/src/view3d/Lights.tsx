// Artificial light: lamp markers (preview) and physically based lamps with a shadow budget (view mode).
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { useFrame, type ThreeEvent } from '@react-three/fiber'
import { Color, DoubleSide, MeshBasicMaterial, MeshStandardMaterial, Object3D, SphereGeometry, Vector3, type PointLight, type RectAreaLight, type SpotLight } from 'three'
import type { FurnitureInstance } from '../core/generated/FurnitureInstance'
import type { Light } from '../core/generated/Light'
import { useEditor } from '../state/editor'
import { useProject } from '../state/project'
import { noRaycast, type PickHandlers } from './Building'
import { furniturePosition, furnitureRotationY } from './Furniture'
import { lightDir, surfaceIntensity, surfaceRect, type V3 } from './lightGeom'
import { kelvinToColor, lumenToIntensity } from './photometry'
import { live } from './store'

const NO_LIGHTS: Light[] = []
const NO_FURNITURE: FurnitureInstance[] = []
const bulbGeo = new SphereGeometry(0.04, 16, 12)
const markerSel = new MeshBasicMaterial({ color: '#1f6feb', transparent: true, opacity: 0.35, depthWrite: false })

export type PlacedLight = { light: Light; pos: Vector3 }

/** World positions (metres) of all lights; attached ones follow their furniture pose. */
export function placeLights(lights: Light[], furniture: FurnitureInstance[]): PlacedLight[] {
  const byId = new Map(furniture.map((f) => [f.id, f]))
  const out: PlacedLight[] = []
  for (const l of lights) {
    let [x, y, z] = l.pos_mm
    if (l.attached_to) {
      const f = byId.get(l.attached_to)
      if (!f) continue
      const r = (f.rotation_deg * Math.PI) / 180
      const lx = x
      const ly = y
      x = f.pos_mm[0] + lx * Math.cos(r) - ly * Math.sin(r)
      y = f.pos_mm[1] + lx * Math.sin(r) + ly * Math.cos(r)
      z = f.pos_mm[2] + z
    }
    out.push({ light: l, pos: new Vector3(x / 1000, z / 1000, y / 1000) })
  }
  return out
}

export function usePlacedLights(): PlacedLight[] {
  const lights = useProject((s) => s.project?.lights ?? NO_LIGHTS)
  const furniture = useProject((s) => s.project?.furniture ?? NO_FURNITURE)
  return useMemo(() => placeLights(lights, furniture), [lights, furniture])
}

function useBulbMaterial(l: Light) {
  const mat = useMemo(() => new MeshStandardMaterial({ color: '#ffffff', roughness: 0.3 }), [])
  useEffect(() => () => mat.dispose(), [mat])
  useLayoutEffect(() => {
    const c = kelvinToColor(l.color_temp_k)
    mat.color.copy(l.on ? c : new Color('#9a9a9a'))
    mat.emissive.copy(c)
    mat.emissiveIntensity = l.on ? 6 : 0
  }, [mat, l.on, l.color_temp_k])
  return mat
}

export type LightHandlers = PickHandlers & {
  onDown?: (e: ThreeEvent<PointerEvent>, p: PlacedLight) => void
}

/** Clickable lamp markers for the modelling preview. */
export function LightMarkers({ pick, only }: { pick?: LightHandlers; only?: (p: PlacedLight) => boolean }) {
  const placed = usePlacedLights()
  const selection = useEditor((s) => s.selection)
  return (
    <group>
      {(only ? placed.filter(only) : placed).filter((p) => p.light.kind.type !== 'Surface').map((p) => (
        <Marker key={p.light.id} p={p} pick={pick} selected={selection?.kind === 'light' && selection.id === p.light.id} />
      ))}
    </group>
  )
}

function Marker({ p, pick, selected }: { p: PlacedLight; pick?: LightHandlers; selected: boolean }) {
  const mat = useBulbMaterial(p.light)
  const s = { kind: 'light' as const, id: p.light.id }
  return (
    <group position={p.pos}>
      <mesh
        geometry={bulbGeo}
        material={mat}
        scale={1.6}
        userData={{ marker: true }}
        onClick={pick?.onClick ? (e) => pick.onClick!(e, s) : undefined}
        onPointerOver={pick?.onOver ? (e) => pick.onOver!(e, s) : undefined}
        onPointerOut={pick?.onOut ? (e) => pick.onOut!(e, s) : undefined}
        onPointerDown={pick?.onDown ? (e) => pick.onDown!(e, p) : undefined}
      />
      {selected && <mesh geometry={bulbGeo} material={markerSel} scale={3.2} raycast={noRaycast} />}
    </group>
  )
}

/** Physically based lamps for view mode. At most `budget` lamps that are on cast shadows (nearest to the camera). */
export function LightRig({ budget, mapSize }: { budget: number; mapSize: number }) {
  const placed = usePlacedLights()
  const [shadowIds, setShadowIds] = useState<string[]>([])
  const acc = useRef(10)
  useFrame((state, dt) => {
    acc.current += dt
    if (acc.current < 0.5) return
    acc.current = 0
    const cam = state.camera.position
    const ids = placed
      .filter((p) => p.light.on && (p.light.kind.type === 'Point' || p.light.kind.type === 'Spot'))
      .map((p) => ({ id: p.light.id, d: p.pos.distanceToSquared(cam) }))
      .sort((a, b) => a.d - b.d)
      .slice(0, budget)
      .map((x) => x.id)
      .sort()
    if (ids.join() !== shadowIds.join()) setShadowIds(ids)
  })
  return (
    <group>
      {placed.map((p) =>
        p.light.kind.type === 'Surface' ? <SurfaceLamp key={p.light.id} l={p.light} /> : <Lamp key={p.light.id} p={p} castShadow={shadowIds.includes(p.light.id)} mapSize={mapSize} />,
      )}
    </group>
  )
}

function Lamp({ p, castShadow, mapSize }: { p: PlacedLight; castShadow: boolean; mapSize: number }) {
  const l = p.light
  const ref = useRef<PointLight | SpotLight | RectAreaLight>(null)
  const target = useMemo(() => new Object3D(), [])
  const base = l.on ? lumenToIntensity(l.kind, l.lumen) : 0
  const color = useMemo(() => kelvinToColor(l.color_temp_k), [l.color_temp_k])
  const mat = useBulbMaterial(l)
  const host = useProject((s) => (l.attached_to ? s.project?.furniture.find((f) => f.id === l.attached_to) : undefined))
  const dir = lightDir(l, host) ?? [0, 0, -1]
  useFrame(() => {
    if (ref.current) ref.current.intensity = base * live.lightScale
  })
  useLayoutEffect(() => {
    const li = ref.current as PointLight | SpotLight | null
    if (!li || !('shadow' in li) || !li.shadow) return
    if (li.shadow.mapSize.x !== mapSize) {
      li.shadow.mapSize.set(mapSize, mapSize)
      li.shadow.map?.dispose()
      li.shadow.map = null
    }
    li.shadow.bias = -0.0015
    li.shadow.normalBias = 0.02
    li.shadow.camera.near = 0.05
    li.shadow.camera.far = 30
    li.shadow.camera.updateProjectionMatrix()
  }, [mapSize, castShadow, l.kind.type])

  let lightEl: React.JSX.Element
  if (l.kind.type === 'Spot') {
    lightEl = (
      <>
        {/* plan (x, y, z up) → three (x, z, y) */}
        <primitive object={target} position={[dir[0], dir[2], dir[1]]} />
        <spotLight ref={ref as React.Ref<SpotLight>} color={color} angle={(Math.min(170, l.kind.angle_deg) / 2) * (Math.PI / 180)} penumbra={0.35} decay={2} distance={0} target={target} castShadow={castShadow} />
      </>
    )
  } else if (l.kind.type === 'Area') {
    lightEl = <rectAreaLight ref={ref as React.Ref<RectAreaLight>} color={color} width={l.kind.w_mm / 1000} height={l.kind.h_mm / 1000} rotation={[-Math.PI / 2, 0, 0]} />
  } else {
    lightEl = <pointLight ref={ref as React.Ref<PointLight>} color={color} decay={2} distance={0} castShadow={castShadow} />
  }
  return (
    <group position={p.pos}>
      {lightEl}
      {/* the light sits inside the bulb: the path tracer must let shadow rays pass */}
      <mesh geometry={bulbGeo} material={mat} raycast={noRaycast} userData={{ bulb: true }} />
    </group>
  )
}

/** Glowing object: emitting side (or whole object, see Furniture) + shadowless stand-in light, in the furniture's frame. */
function SurfaceLamp({ l }: { l: Light }) {
  const f = useProject((s) => s.project?.furniture.find((x) => x.id === l.attached_to))
  const ref = useRef<PointLight | RectAreaLight>(null)
  const color = useMemo(() => kelvinToColor(l.color_temp_k), [l.color_temp_k])
  const glow = useMemo(() => new MeshStandardMaterial({ color: '#000000', side: DoubleSide }), [])
  useEffect(() => () => glow.dispose(), [glow])
  useLayoutEffect(() => {
    glow.emissive.copy(color)
    glow.emissiveIntensity = l.on ? 4 : 0
  }, [glow, color, l.on])
  const face = l.kind.type === 'Surface' ? l.kind.face : 'All'
  const dims = (f?.dims_mm ?? [1, 1, 1]) as V3
  const base = l.on && f ? surfaceIntensity(face, dims, l.lumen) : 0
  useFrame(() => {
    if (ref.current) ref.current.intensity = base * live.lightScale
  })
  if (!f) return null
  const frame = { position: furniturePosition(f), rotation: [0, furnitureRotationY(f.rotation_deg), 0] as [number, number, number] }
  if (face === 'All') {
    return (
      <group {...frame}>
        <pointLight ref={ref as React.Ref<PointLight>} position={[0, dims[2] / 2000, 0]} color={color} decay={2} distance={0} castShadow={false} />
      </group>
    )
  }
  const r = surfaceRect(face, dims)
  const off = 0.0015 // just outside the surface (no z-fighting)
  return (
    <group {...frame}>
      <group position={r.center} rotation={[r.rotX, r.rotY, 0]}>
        <rectAreaLight ref={ref as React.Ref<RectAreaLight>} color={color} width={r.size[0]} height={r.size[1]} position={[0, 0, -off]} />
        <mesh material={glow} position={[0, 0, -off]} raycast={noRaycast} userData={{ bulb: true }}>
          <planeGeometry args={r.size} />
        </mesh>
      </group>
    </group>
  )
}
