// Doors open/shut (view state): toggle buttons, E key while walking, shut doors block walking.
import { useEffect } from 'react'
import { useSceneMeshes } from '../core/derived'
import { useProject } from '../state/project'
import { live, useView3D } from './store'

/** Ids of all doors (optionally only those in `walls`). */
export function useDoorIds(walls?: readonly string[]): string[] {
  const scene = useSceneMeshes()
  return (scene?.openings ?? []).filter((o) => o.kind.type === 'Door' && (!walls || walls.includes(o.wall))).map((o) => o.opening)
}

/** Keeps the engine's walk colliders in sync with shut doors. */
export function useDoorCollision() {
  const engine = useProject((s) => s.engine)
  const doorOpen = useView3D((s) => s.doorOpen)
  useEffect(() => {
    if (!engine) return
    try {
      engine.set_closed_doors(Object.entries(doorOpen).filter(([, o]) => !o).map(([id]) => id))
    } catch (e) {
      console.warn(e)
    }
  }, [engine, doorOpen])
}

/** E while walking: open/shut the nearest door within 2 m. */
export function useDoorKey(enabled: boolean) {
  const scene = useSceneMeshes()
  useEffect(() => {
    if (!enabled) return
    const onKey = (e: KeyboardEvent) => {
      if (e.code !== 'KeyE' || e.metaKey || e.ctrlKey) return
      const t = e.target as HTMLElement
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT')) return
      const w = live.walker
      let best: { id: string; d: number } | null = null
      for (const o of scene?.openings ?? []) {
        if (o.kind.type !== 'Door') continue
        const d = Math.hypot(o.position[0] * 1000 - w.x, o.position[2] * 1000 - w.y)
        if (d < 2000 && (!best || d < best.d)) best = { id: o.opening, d }
      }
      if (best) useView3D.getState().toggleDoor(best.id)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [enabled, scene])
}

/** «Türen zu» / «Türen auf» for the given doors. */
export function DoorsButton({ ids }: { ids: string[] }) {
  const doorOpen = useView3D((s) => s.doorOpen)
  if (!ids.length) return null
  const anyOpen = ids.some((id) => doorOpen[id] ?? true)
  return (
    <button onClick={() => useView3D.getState().setDoors(ids, !anyOpen)} title={anyOpen ? 'Alle Türen schliessen' : 'Alle Türen öffnen'}>
      🚪 {anyOpen ? 'Türen zu' : 'Türen auf'}
    </button>
  )
}
