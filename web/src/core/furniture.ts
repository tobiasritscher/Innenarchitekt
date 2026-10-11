// Turning a library asset into model ops (furniture instance + attached light).
import type { Asset } from './generated/Asset'
import type { Op } from './generated/Op'
import { newId, type PlanEngine } from './engine'
import type { FurnitureInstance } from './generated/FurnitureInstance'

/** Ops to place `asset` at plan (x, y) mm with rotation (deg). Returns [ops, furnitureId]. */
export function instantiateAsset(asset: Asset, x: number, y: number, rotationDeg = 0, elevationMm = 0): [Op[], string] {
  const id = newId()
  const ops: Op[] = [
    {
      op: 'PutFurniture',
      item: {
        id,
        asset: asset.id,
        name: asset.name,
        pos_mm: [Math.round(x), Math.round(y), Math.round(elevationMm)],
        rotation_deg: rotationDeg,
        dims_mm: [...asset.dims_mm] as [number, number, number],
        material: asset.primitive === 'Podest' ? 'wood:oak' : null,
        walkable: asset.primitive === 'Podest',
        overlap_ok: asset.primitive === 'Chair',
      },
    },
  ]
  if (asset.light) {
    ops.push({
      op: 'PutLight',
      light: {
        id: newId(),
        name: `Licht ${asset.name}`,
        kind: asset.light.kind,
        pos_mm: asset.light.offset_mm,
        lumen: asset.light.lumen,
        color_temp_k: asset.light.color_temp_k,
        on: true,
        attached_to: id,
      },
    })
  }
  return [ops, id]
}

/** Floor height (mm) at plan (x, y): top of a podium there (ignoring furniture `exceptId`), else 0. */
export function standHeight(engine: PlanEngine | null | undefined, x: number, y: number, exceptId = ''): number {
  if (!engine) return 0
  try {
    return engine.floor_height_at(x, y, exceptId)
  } catch {
    return 0
  }
}

/** Elevation for `item` moved to (x, y): keeps its height above the surface below (floor or podium). */
export function movedElevation(engine: PlanEngine | null | undefined, item: FurnitureInstance, x: number, y: number): number {
  const before = standHeight(engine, item.pos_mm[0], item.pos_mm[1], item.id)
  return Math.max(0, Math.round(item.pos_mm[2] - before + standHeight(engine, x, y, item.id)))
}

/** Steps of a podium (mirrors plan-core `podium_steps`): rises ≈ 170 mm, the front `rises − 1` treads lie inside the footprint. */
export function podiumSteps(dims: readonly [number, number, number] | number[]): { rises: number; tread: number } {
  const rises = Math.max(1, Math.round(dims[2] / 170))
  const tread = rises > 1 ? Math.min(280, dims[1] / 2 / (rises - 1)) : 0
  return { rises, tread }
}
