// Furniture instances: primitive meshes from the core or normalised GLB models.
import { Component, Suspense, useEffect, useMemo, type ReactNode } from 'react'
import { Box3, EdgesGeometry, LineBasicMaterial, Vector3, type Object3D, type Material, type Mesh } from 'three'
import type { ThreeEvent } from '@react-three/fiber'
import { useGLTF } from '@react-three/drei'
import { useOverlaps } from '../core/derived'
import type { FurnitureInstance } from '../core/generated/FurnitureInstance'
import type { Light } from '../core/generated/Light'
import type { PrimitiveShape } from '../core/generated/PrimitiveShape'
import { useAssets } from '../state/assets'
import { useEditor } from '../state/editor'
import { useProject } from '../state/project'
import { acquirePrimitive, releasePrimitive } from './geometry'
import { furnitureMaterial, mats } from './materials3d'
import { noRaycast, unitBox, type PickHandlers } from './Building'

export const DRACO_PATH = '/draco/'

const edges = new EdgesGeometry(unitBox)
const outline = {
  overlap: new LineBasicMaterial({ color: '#d93025', depthTest: false, transparent: true }),
  select: new LineBasicMaterial({ color: '#1f6feb', depthTest: false, transparent: true }),
  hover: new LineBasicMaterial({ color: '#1f6feb', depthTest: false, transparent: true, opacity: 0.45 }),
}

const NO_FURNITURE: FurnitureInstance[] = []
const NO_LIGHTS: Light[] = []

export type FurnitureHandlers = PickHandlers & {
  onDown?: (e: ThreeEvent<PointerEvent>, item: FurnitureInstance) => void
}

export function FurnitureLayer({ pick, highlight = false }: { pick?: FurnitureHandlers; highlight?: boolean }) {
  const furniture = useProject((s) => s.project?.furniture ?? NO_FURNITURE)
  const lights = useProject((s) => s.project?.lights ?? NO_LIGHTS)
  const overlaps = useOverlaps()
  const selection = useEditor((s) => (highlight ? s.selection : null))
  const hover = useEditor((s) => (highlight ? s.hover : null))
  const overlapSet = useMemo(() => new Set(highlight ? overlaps : []), [overlaps, highlight])
  const withLight = useMemo(() => new Set(lights.filter((l) => l.attached_to).map((l) => l.attached_to!)), [lights])
  return (
    <group>
      {furniture.map((f) => {
        const state = selection?.kind === 'furniture' && selection.id === f.id ? 'select' : overlapSet.has(f.id) ? 'overlap' : hover?.kind === 'furniture' && hover.id === f.id ? 'hover' : null
        return <FurnitureItem key={f.id} item={f} pick={pick} outlineState={state} castShadow={!withLight.has(f.id)} />
      })}
    </group>
  )
}

/** World transform helpers (plan mm → three metres). */
export function furniturePosition(f: FurnitureInstance): [number, number, number] {
  return [f.pos_mm[0] / 1000, f.pos_mm[2] / 1000, f.pos_mm[1] / 1000]
}
export function furnitureRotationY(rotationDeg: number) {
  return (-rotationDeg * Math.PI) / 180
}

function FurnitureItem({ item, pick, outlineState, castShadow }: { item: FurnitureInstance; pick?: FurnitureHandlers; outlineState: 'select' | 'overlap' | 'hover' | null; castShadow: boolean }) {
  const asset = useAssets((s) => s.assets.find((a) => a.id === item.asset))
  const s = { kind: 'furniture' as const, id: item.id }
  const dims: [number, number, number] = [item.dims_mm[0], item.dims_mm[1], item.dims_mm[2]]
  const events = pick
    ? {
        onClick: pick.onClick ? (e: ThreeEvent<MouseEvent>) => pick.onClick!(e, s) : undefined,
        onPointerOver: pick.onOver ? (e: ThreeEvent<PointerEvent>) => pick.onOver!(e, s) : undefined,
        onPointerOut: pick.onOut ? (e: ThreeEvent<PointerEvent>) => pick.onOut!(e, s) : undefined,
        onPointerDown: pick.onDown ? (e: ThreeEvent<PointerEvent>) => pick.onDown!(e, item) : undefined,
      }
    : {}
  const shape: PrimitiveShape = asset?.primitive ?? 'Box'
  const fallback = <PrimitiveModel shape={shape} dims={dims} material={item.material} castShadow={castShadow} />
  const w = dims[0] / 1000
  const d = dims[1] / 1000
  const h = dims[2] / 1000
  return (
    <group position={furniturePosition(item)} rotation={[0, furnitureRotationY(item.rotation_deg), 0]} {...events}>
      {asset?.model_url ? (
        <ModelBoundary fallback={fallback} resetKey={asset.model_url}>
          <Suspense fallback={fallback}>
            <GlbModel url={asset.model_url} dims={dims} castShadow={castShadow} />
          </Suspense>
        </ModelBoundary>
      ) : (
        fallback
      )}
      {outlineState && <lineSegments geometry={edges} material={outline[outlineState]} position={[0, h / 2, 0]} scale={[w + 0.02, h + 0.02, d + 0.02]} renderOrder={5} raycast={noRaycast} />}
    </group>
  )
}

export function PrimitiveModel({ shape, dims, material, castShadow = true, override }: { shape: PrimitiveShape; dims: [number, number, number]; material: string | null; castShadow?: boolean; override?: Material }) {
  const key = `${shape}|${dims.join('x')}`
  const geo = useMemo(() => acquirePrimitive(shape, dims), [key]) // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => () => releasePrimitive(shape, dims), [key]) // eslint-disable-line react-hooks/exhaustive-deps
  const materials = override ?? (material ? [furnitureMaterial(material, 'main'), furnitureMaterial(material, 'accent')] : [mats.furnitureMain(), mats.furnitureAccent()])
  return <mesh geometry={geo} material={materials} castShadow={castShadow} receiveShadow />
}

/** GLB scaled per axis to the instance dims; origin bottom centre. */
function GlbModel({ url, dims, castShadow }: { url: string; dims: [number, number, number]; castShadow: boolean }) {
  const gltf = useGLTF(url, DRACO_PATH, true)
  const norm = useMemo(() => {
    const src = gltf.scene
    src.updateMatrixWorld(true)
    const box = new Box3().setFromObject(src)
    const size = box.getSize(new Vector3())
    const center = box.getCenter(new Vector3())
    const obj = src.clone(true)
    return { obj, size, center, min: box.min.clone() }
  }, [gltf])
  useEffect(() => {
    norm.obj.traverse((o: Object3D) => {
      const m = o as Mesh
      if (m.isMesh) {
        m.castShadow = castShadow
        m.receiveShadow = true
      }
    })
  }, [norm, castShadow])
  const sx = dims[0] / 1000 / Math.max(1e-4, norm.size.x)
  const sy = dims[2] / 1000 / Math.max(1e-4, norm.size.y)
  const sz = dims[1] / 1000 / Math.max(1e-4, norm.size.z)
  return (
    <group scale={[sx, sy, sz]}>
      <group position={[-norm.center.x, -norm.min.y, -norm.center.z]}>
        <primitive object={norm.obj} />
      </group>
    </group>
  )
}

/** Falls back to the primitive if a model fails to load. */
class ModelBoundary extends Component<{ fallback: ReactNode; resetKey: string; children: ReactNode }, { failed: string | null }> {
  state = { failed: null as string | null }
  static getDerivedStateFromError() {
    return { failed: 'error' }
  }
  componentDidCatch(e: unknown) {
    console.warn('Modell konnte nicht geladen werden, Basisform als Ersatz', e)
  }
  componentDidUpdate(prev: { resetKey: string }) {
    if (prev.resetKey !== this.props.resetKey && this.state.failed) this.setState({ failed: null })
  }
  render() {
    return this.state.failed ? this.props.fallback : this.props.children
  }
}
