// Apartment bounds from wall footprints (plan mm and three metres).
import { useMemo } from 'react'
import { Vector3 } from 'three'
import { useFootprints } from '../core/derived'

export type Bounds = {
  minX: number
  minY: number
  maxX: number
  maxY: number
  /** Centre in three coordinates (m), at floor level. */
  center: Vector3
  /** Horizontal half-diagonal (m). */
  radius: number
  empty: boolean
}

export function useBounds(): Bounds {
  const fps = useFootprints()
  return useMemo(() => {
    let minX = Infinity
    let minY = Infinity
    let maxX = -Infinity
    let maxY = -Infinity
    for (const f of fps)
      for (const [x, y] of f.corners) {
        minX = Math.min(minX, x)
        minY = Math.min(minY, y)
        maxX = Math.max(maxX, x)
        maxY = Math.max(maxY, y)
      }
    const empty = !isFinite(minX)
    if (empty) {
      minX = -3000
      minY = -3000
      maxX = 3000
      maxY = 3000
    }
    const center = new Vector3((minX + maxX) / 2000, 0, (minY + maxY) / 2000)
    const radius = Math.max(2, Math.hypot(maxX - minX, maxY - minY) / 2000)
    return { minX, minY, maxX, maxY, center, radius, empty }
  }, [fps])
}
