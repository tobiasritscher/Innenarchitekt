// Turning a library asset into model ops (furniture instance + attached light).
import type { Asset } from './generated/Asset'
import type { Op } from './generated/Op'
import { newId } from './engine'

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
        material: null,
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
