// Memoised derived data from the WASM core, keyed on the project revision.
import { useMemo } from 'react'
import { useProject } from '../state/project'
import type { DerivedRoom } from './generated/DerivedRoom'
import type { Issue } from './generated/Issue'
import type { SceneMeshes } from './generated/SceneMeshes'
import type { WallFootprint } from './generated/WallFootprint'

function useDerived<T>(fn: (engine: NonNullable<ReturnType<typeof useProject.getState>['engine']>, floor: string) => T, empty: T): T {
  const revision = useProject((s) => s.revision)
  const engine = useProject((s) => s.engine)
  const floor = useProject((s) => s.project?.floors[0]?.id)
  return useMemo(() => {
    if (!engine || !floor) return empty
    try {
      return fn(engine, floor)
    } catch (e) {
      console.error(e)
      return empty
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [revision, engine, floor])
}

export const useRooms = () => useDerived<DerivedRoom[]>((e, f) => e.rooms(f), [])
export const useFootprints = () => useDerived<WallFootprint[]>((e, f) => e.footprints(f), [])
export const useIssues = () => useDerived<Issue[]>((e, f) => e.validate(f), [])
export const useSceneMeshes = () => useDerived<SceneMeshes | null>((e, f) => e.scene_meshes(f), null)
export const useOverlaps = () => useDerived<string[]>((e) => e.overlaps(), [])
