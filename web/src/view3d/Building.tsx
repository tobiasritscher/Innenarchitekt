// Walls, floors, ceilings and openings from the core's scene meshes (shared by preview and view mode).
import { useEffect, useMemo, type ReactNode } from 'react'
import { BoxGeometry, DoubleSide, MeshBasicMaterial, PlaneGeometry, type BufferGeometry, type Material } from 'three'
import type { ThreeEvent } from '@react-three/fiber'
import { useSceneMeshes } from '../core/derived'
import type { OpeningPlacement } from '../core/generated/OpeningPlacement'
import type { Floor } from '../core/generated/Floor'
import type { Wall } from '../core/generated/Wall'
import type { RoomMeta } from '../core/generated/RoomMeta'
import { useEditor, type Selection } from '../state/editor'
import { useFloor } from '../state/project'
import { GeometryCache } from './geometry'
import { mats, surfaceMaterial, wallClip } from './materials3d'

/** Ceiling rendering: not at all, only into shadow maps (layer 1), or visible. */
export type CeilingMode = 'none' | 'shadow' | 'visible'

const WALL_SLOTS = ['left', 'right', 'edge', 'top']
const FLOOR_SLOTS = ['floor']
const CEIL_SLOTS = ['ceiling']

export const unitBox = new BoxGeometry(1, 1, 1)
const unitPlane = new PlaneGeometry(1, 1)

const overlay = {
  select: new MeshBasicMaterial({ color: '#1f6feb', transparent: true, opacity: 0.28, depthWrite: false, polygonOffset: true, polygonOffsetFactor: -2, polygonOffsetUnits: -2, clippingPlanes: [wallClip], side: DoubleSide }),
  hover: new MeshBasicMaterial({ color: '#1f6feb', transparent: true, opacity: 0.12, depthWrite: false, polygonOffset: true, polygonOffsetFactor: -2, polygonOffsetUnits: -2, clippingPlanes: [wallClip], side: DoubleSide }),
}

function isSel(sel: Selection, kind: string, id: string) {
  return !!sel && sel.kind === kind && sel.id === id
}

export type PickHandlers = {
  onClick?: (e: ThreeEvent<MouseEvent>, s: NonNullable<Selection>) => void
  onOver?: (e: ThreeEvent<PointerEvent>, s: NonNullable<Selection>) => void
  onOut?: (e: ThreeEvent<PointerEvent>, s: NonNullable<Selection>) => void
}

function pickProps(h: PickHandlers | undefined, s: NonNullable<Selection>) {
  if (!h) return {}
  return {
    onClick: h.onClick ? (e: ThreeEvent<MouseEvent>) => h.onClick!(e, s) : undefined,
    onPointerOver: h.onOver ? (e: ThreeEvent<PointerEvent>) => h.onOver!(e, s) : undefined,
    onPointerOut: h.onOut ? (e: ThreeEvent<PointerEvent>) => h.onOut!(e, s) : undefined,
  }
}

export function Building({ ceilings, pick, highlight = false, caps = false }: { ceilings: CeilingMode; pick?: PickHandlers; highlight?: boolean; caps?: boolean }) {
  const scene = useSceneMeshes()
  const floor = useFloor()
  const selection = useEditor((s) => (highlight ? s.selection : null))
  const hover = useEditor((s) => (highlight ? s.hover : null))

  const caches = useMemo(() => ({ walls: new GeometryCache(WALL_SLOTS), floors: new GeometryCache(FLOOR_SLOTS), ceilings: new GeometryCache(CEIL_SLOTS) }), [])
  useEffect(
    () => () => {
      caches.walls.dispose()
      caches.floors.dispose()
      caches.ceilings.dispose()
    },
    [caches],
  )

  const wallGeos = useMemo(() => caches.walls.sync(scene ? scene.walls.map((w) => ({ key: w.wall, mesh: w.mesh })) : []), [scene, caches])
  const floorGeos = useMemo(() => caches.floors.sync(scene ? scene.rooms.map((r) => ({ key: r.room.key, mesh: r.floor })) : []), [scene, caches])
  const ceilGeos = useMemo(
    () => caches.ceilings.sync(scene && ceilings !== 'none' ? scene.rooms.map((r) => ({ key: r.room.key, mesh: r.ceiling })) : []),
    [scene, caches, ceilings],
  )

  const wallById = useMemo(() => new Map<string, Wall>((floor?.walls ?? []).map((w) => [w.id, w])), [floor])
  const roomById = useMemo(() => new Map<string, RoomMeta>((floor?.rooms ?? []).map((r) => [r.id, r])), [floor])

  if (!scene) return null

  return (
    <group>
      {scene.walls.map((w) => {
        const geo = wallGeos.get(w.wall)
        const wall = wallById.get(w.wall)
        if (!geo || !wall) return null
        const glazed = !!wall.glazed
        const materials: Material[] = glazed
          ? [mats.glass(), mats.glass(), mats.frame(), mats.frame()]
          : [surfaceMaterial(wall.finish_left, 'wall'), surfaceMaterial(wall.finish_right, 'wall'), mats.wallEdge(), mats.wallTop()]
        const s = { kind: 'wall' as const, id: w.wall }
        const ov = isSel(selection, 'wall', w.wall) ? overlay.select : isSel(hover, 'wall', w.wall) ? overlay.hover : null
        return (
          <group key={w.wall}>
            {/* Glass lets the sun through: no shadow casting, drawn after opaque geometry. */}
            <mesh geometry={geo} material={materials} castShadow={!glazed} receiveShadow={!glazed} renderOrder={glazed ? 1 : 0} userData={{ wall: w.wall }} {...pickProps(pick, s)} />
            {caps && !glazed && <mesh geometry={geo} material={mats.wallCap()} raycast={noRaycast} />}
            {glazed && floor && <GlassFrame wall={wall} floor={floor} height={(wall.height_mm != null ? Math.min(wall.height_mm / 1000, scene.ceiling_height) : scene.ceiling_height)} pp={pickProps(pick, s)} />}
            {ov && <mesh geometry={geo} material={ov} renderOrder={2} raycast={noRaycast} />}
          </group>
        )
      })}
      {scene.rooms.map((r) => {
        const geo = floorGeos.get(r.room.key)
        if (!geo) return null
        const meta = r.room.meta ? roomById.get(r.room.meta) : undefined
        const id = r.room.meta ?? r.room.key
        const s = { kind: 'room' as const, id }
        const ov = isSel(selection, 'room', id) ? overlay.select : isSel(hover, 'room', id) ? overlay.hover : null
        return (
          <group key={r.room.key}>
            <mesh geometry={geo} material={surfaceMaterial(meta?.floor_material ?? 'wood:oak', 'floor')} receiveShadow {...pickProps(pick, s)} />
            {ov && <mesh geometry={geo} material={ov} renderOrder={2} raycast={noRaycast} />}
          </group>
        )
      })}
      {ceilings !== 'none' &&
        scene.rooms.map((r) => {
          const geo = ceilGeos.get(r.room.key)
          if (!geo) return null
          const meta = r.room.meta ? roomById.get(r.room.meta) : undefined
          if (meta?.open_ceiling) return null
          return <Ceiling key={r.room.key} geo={geo} material={surfaceMaterial(meta?.ceiling_material ?? 'paint:white', 'ceiling')} visible={ceilings === 'visible'} />
        })}
      {scene.openings.map((o) => (
        <OpeningMesh key={o.opening} o={o} pick={pick} selected={isSel(selection, 'opening', o.opening)} hovered={isSel(hover, 'opening', o.opening)} />
      ))}
    </group>
  )
}

export function noRaycast() {}

const GLASS_POST_SPACING = 1.25
const GLASS_PROFILE = 0.05

/** Aluminium profiles of a glass wall: floor and ceiling rail plus vertical posts (metres, three.js frame). */
function GlassFrame({ wall, floor, height, pp }: { wall: Wall; floor: Floor; height: number; pp: Record<string, unknown> }) {
  const a = floor.nodes.find((n) => n.id === wall.a)
  const b = floor.nodes.find((n) => n.id === wall.b)
  if (!a || !b) return null
  const ax = a.x_mm / 1000, az = a.y_mm / 1000
  const dx = b.x_mm / 1000 - ax, dz = b.y_mm / 1000 - az
  const len = Math.hypot(dx, dz)
  if (len < 0.05) return null
  const t = Math.min(wall.thickness_mm / 1000, 0.08) + 0.01
  const f = GLASS_PROFILE
  const posts = Math.max(0, Math.ceil(len / GLASS_POST_SPACING) - 1)
  const frame = mats.frame()
  const cuts = floor.openings.filter((o) => o.wall === wall.id).map((o) => [o.offset_mm / 1000 - f, (o.offset_mm + o.width_mm) / 1000 + f])
  const us = Array.from({ length: posts }, (_, i) => ((i + 1) * len) / (posts + 1)).filter((u) => !cuts.some(([s, e]) => u > s && u < e))
  return (
    <group position={[ax + dx / 2, 0, az + dz / 2]} rotation={[0, Math.atan2(-dz, dx), 0]}>
      <Box p={[0, f / 2, 0]} s={[len, f, t]} material={frame} {...pp} />
      <Box p={[0, height - f / 2, 0]} s={[len, f, t]} material={frame} {...pp} />
      {us.map((u) => (
        <Box key={u} p={[-len / 2 + u, height / 2, 0]} s={[f * 0.6, height - 2 * f, t]} material={frame} {...pp} />
      ))}
    </group>
  )
}

/** In 'shadow' mode the ceiling is invisible to the camera (no colour/depth writes) but still renders into
 *  shadow maps, so the sun only enters through windows even in the dollhouse view. */
function Ceiling({ geo, material, visible }: { geo: BufferGeometry; material: Material; visible: boolean }) {
  return <mesh geometry={geo} material={visible ? material : mats.shadowOnly()} castShadow receiveShadow={visible} raycast={visible ? undefined : noRaycast} />
}

/** A box from the shared unit geometry. */
function Box({ p, s, material, cast = true, ...rest }: { p: [number, number, number]; s: [number, number, number]; material: Material; cast?: boolean } & Record<string, unknown>) {
  return <mesh geometry={unitBox} material={material} position={p} scale={s} castShadow={cast} receiveShadow {...rest} />
}

const DOOR_OPEN_DEG = 70

function OpeningMesh({ o, pick, selected, hovered }: { o: OpeningPlacement; pick?: PickHandlers; selected: boolean; hovered: boolean }) {
  const s = { kind: 'opening' as const, id: o.opening }
  const pp = pickProps(pick, s)
  const { width: w, height: h, thickness: t } = o
  const frame = mats.frame()
  const ov = selected ? overlay.select : hovered ? overlay.hover : null
  let body: ReactNode = null
  if (o.kind.type === 'Window') {
    const f = 0.06
    const d = Math.min(t, 0.09)
    body = (
      <>
        <Box p={[0, f / 2, 0]} s={[w, f, d]} material={frame} {...pp} />
        <Box p={[0, h - f / 2, 0]} s={[w, f, d]} material={frame} {...pp} />
        <Box p={[-w / 2 + f / 2, h / 2, 0]} s={[f, h - 2 * f, d]} material={frame} {...pp} />
        <Box p={[w / 2 - f / 2, h / 2, 0]} s={[f, h - 2 * f, d]} material={frame} {...pp} />
        {w > 1.3 && <Box p={[0, h / 2, 0]} s={[f * 0.8, h - 2 * f, d * 0.8]} material={frame} {...pp} />}
        {/* inner sill */}
        <Box p={[0, -0.015, t / 2 - 0.02]} s={[w + 0.06, 0.03, 0.12]} material={frame} {...pp} />
        <mesh geometry={unitPlane} material={mats.glass()} position={[0, h / 2, 0]} scale={[w - 2 * f, h - 2 * f, 1]} renderOrder={1} {...pp} />
      </>
    )
  } else if (o.kind.type === 'Door') {
    const f = 0.05
    const trim = t + 0.02
    const leafT = 0.04
    const sideSign = o.kind.swing === 'Right' ? 1 : -1
    const start = o.kind.hinge === 'Start'
    const hingeX = start ? -w / 2 + f : w / 2 - f
    const leafW = w - 2 * f - 0.006
    const angle = ((start ? -sideSign : sideSign) * DOOR_OPEN_DEG * Math.PI) / 180
    const leafH = h - f - 0.01
    body = (
      <>
        <Box p={[-w / 2 + f / 2, h / 2, 0]} s={[f, h, trim]} material={frame} {...pp} />
        <Box p={[w / 2 - f / 2, h / 2, 0]} s={[f, h, trim]} material={frame} {...pp} />
        <Box p={[0, h - f / 2, 0]} s={[w, f, trim]} material={frame} {...pp} />
        <group position={[hingeX, 0.005, sideSign * (t / 2 - leafT / 2)]} rotation={[0, angle, 0]}>
          <Box p={[(start ? 1 : -1) * (leafW / 2), leafH / 2, 0]} s={[leafW, leafH, leafT]} material={mats.doorLeaf()} {...pp} />
          {/* handle */}
          <Box p={[(start ? 1 : -1) * (leafW - 0.08), 1.05, 0]} s={[0.12, 0.02, leafT + 0.06]} material={frame} cast={false} {...pp} />
        </group>
      </>
    )
  }
  if (!body && !ov) return null
  return (
    <group position={o.position} rotation={[0, o.rotation_y, 0]}>
      {body}
      {ov && <mesh geometry={unitBox} material={ov} position={[0, h / 2, 0]} scale={[w + 0.02, h + 0.02, t + 0.04]} raycast={noRaycast} />}
    </group>
  )
}
