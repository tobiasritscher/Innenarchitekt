// PlanSource: what the 2D editor edits. Either the main project session (state/project.ts)
// or a local, temporary engine with its own undo stack (import correction view).
import { createContext, useContext, useDeferredValue, useMemo } from 'react'
import { create, type StoreApi, type UseBoundStore } from 'zustand'
import { PlanEngine } from '../core/engine'
import type { DerivedRoom } from '../core/generated/DerivedRoom'
import type { Issue } from '../core/generated/Issue'
import type { Op } from '../core/generated/Op'
import type { Project } from '../core/generated/Project'
import type { WallFootprint } from '../core/generated/WallFootprint'
import { useProject } from '../state/project'

export type PlanSource = {
  engine: PlanEngine
  project: Project
  revision: number
  commit: (ops: Op[], label: string) => boolean
  setTransient: (ops: Op[]) => boolean
  endTransient: (commit: boolean, label?: string) => void
  undo: () => void
  redo: () => void
  undoLabel: string | null
  redoLabel: string | null
  /** true for the import correction view (temporary engine). */
  local: boolean
}

export const PlanSourceContext = createContext<PlanSource | null>(null)

function useMainSource(): PlanSource | null {
  const engine = useProject((s) => s.engine)
  const project = useProject((s) => s.project)
  const revision = useProject((s) => s.revision)
  const undoStack = useProject((s) => s.undoStack)
  const redoStack = useProject((s) => s.redoStack)
  return useMemo(() => {
    if (!engine || !project) return null
    const st = useProject.getState()
    return {
      engine,
      project,
      revision,
      commit: st.commit,
      setTransient: st.setTransient,
      endTransient: st.endTransient,
      undo: st.undo,
      redo: st.redo,
      undoLabel: undoStack.length ? undoStack[undoStack.length - 1].label : null,
      redoLabel: redoStack.length ? redoStack[redoStack.length - 1].label : null,
      local: false,
    }
  }, [engine, project, revision, undoStack, redoStack])
}

/** The plan being edited: the nearest PlanSourceContext provider, else the main project. */
export function usePlanSource(): PlanSource | null {
  const ctx = useContext(PlanSourceContext)
  const main = useMainSource()
  return ctx ?? main
}

export function floorOf(src: PlanSource): string {
  return src.project.floors[0].id
}

// ---------------------------------------------------------------- local session

type Batch = { ops: Op[]; inverse: Op[]; label: string }

export type LocalSession = {
  engine: PlanEngine
  project: Project
  revision: number
  undoStack: Batch[]
  redoStack: Batch[]
  error: string | null
  commit: (ops: Op[], label: string) => boolean
  setTransient: (ops: Op[]) => boolean
  endTransient: (commit: boolean, label?: string) => void
  undo: () => void
  redo: () => void
  /** Apply ops without an undo entry (initial setup). */
  applyRaw: (ops: Op[]) => void
  /** Drop the undo history (e.g. after the initial proposal was applied). */
  resetHistory: () => void
  dispose: () => void
}

export type LocalSessionStore = UseBoundStore<StoreApi<LocalSession>>

/** Temporary engine on a deep copy of `project`, with its own undo/redo. */
export function createLocalSession(project: Project): LocalSessionStore {
  const engine = new PlanEngine(structuredClone(project))
  let transient: { ops: Op[]; inverse: Op[] } | null = null
  const msg = (e: unknown) => (e instanceof Error ? e.message : String(e))
  return create<LocalSession>((set, get) => {
    const refresh = () => set({ project: engine.project() as Project, revision: get().revision + 1 })
    return {
      engine,
      project: engine.project() as Project,
      revision: 0,
      undoStack: [],
      redoStack: [],
      error: null,
      commit: (ops, label) => {
        if (!ops.length) return false
        if (transient) get().endTransient(true)
        try {
          const inverse = engine.apply(ops) as Op[]
          set({ undoStack: [...get().undoStack, { ops, inverse, label }], redoStack: [], error: null })
          refresh()
          return true
        } catch (e) {
          set({ error: msg(e) })
          return false
        }
      },
      setTransient: (ops) => {
        try {
          if (transient) engine.apply(transient.inverse)
          transient = null
          transient = { ops, inverse: engine.apply(ops) as Op[] }
          refresh()
          return true
        } catch (e) {
          set({ error: msg(e) })
          refresh()
          return false
        }
      },
      endTransient: (commit, label = 'Verschieben') => {
        const t = transient
        transient = null
        if (!t) return
        if (commit && t.ops.length) set({ undoStack: [...get().undoStack, { ops: t.ops, inverse: t.inverse, label }], redoStack: [] })
        else {
          engine.apply(t.inverse)
          refresh()
        }
      },
      undo: () => {
        const st = get().undoStack
        if (!st.length) return
        if (transient) get().endTransient(false)
        const b = st[st.length - 1]
        try {
          engine.apply(b.inverse)
          set({ undoStack: st.slice(0, -1), redoStack: [...get().redoStack, b] })
          refresh()
        } catch (e) {
          set({ error: msg(e) })
        }
      },
      redo: () => {
        const st = get().redoStack
        if (!st.length) return
        const b = st[st.length - 1]
        try {
          engine.apply(b.ops)
          set({ redoStack: st.slice(0, -1), undoStack: [...get().undoStack, b] })
          refresh()
        } catch (e) {
          set({ error: msg(e) })
        }
      },
      applyRaw: (ops) => {
        if (!ops.length) return
        engine.apply(ops)
        refresh()
      },
      resetHistory: () => set({ undoStack: [], redoStack: [] }),
      dispose: () => {
        try {
          engine.free()
        } catch {
          /* already freed */
        }
      },
    }
  })
}

export function useLocalSource(store: LocalSessionStore): PlanSource {
  const s = store()
  return useMemo(
    () => ({
      engine: s.engine,
      project: s.project,
      revision: s.revision,
      commit: s.commit,
      setTransient: s.setTransient,
      endTransient: s.endTransient,
      undo: s.undo,
      redo: s.redo,
      undoLabel: s.undoStack.length ? s.undoStack[s.undoStack.length - 1].label : null,
      redoLabel: s.redoStack.length ? s.redoStack[s.redoStack.length - 1].label : null,
      local: true,
    }),
    [s],
  )
}

// ---------------------------------------------------------------- derived data

export type Derived = {
  rooms: DerivedRoom[]
  footprints: WallFootprint[]
  overlaps: string[]
}

const EMPTY: Derived = { rooms: [], footprints: [], overlaps: [] }

function safe<T>(fn: () => T, fallback: T): T {
  try {
    return fn()
  } catch (e) {
    console.error(e)
    return fallback
  }
}

const cache = new WeakMap<PlanEngine, { rev: number; floor: string; data: Derived }>()

/** Derived data for an engine at a revision (shared between components). */
export function derivedFor(engine: PlanEngine, rev: number, floor: string): Derived {
  const c = cache.get(engine)
  if (c && c.rev === rev && c.floor === floor) return c.data
  const data: Derived = {
    rooms: safe(() => engine.rooms(floor) as DerivedRoom[], []),
    footprints: safe(() => engine.footprints(floor) as WallFootprint[], []),
    overlaps: safe(() => engine.overlaps() as string[], []),
  }
  cache.set(engine, { rev, floor, data })
  return data
}

/** Derived geometry of the source, memoised on its revision. */
export function useDerivedOf(src: PlanSource | null): Derived {
  const engine = src?.engine
  const rev = src?.revision ?? 0
  const floor = src?.project.floors[0]?.id
  return useMemo(() => (engine && floor ? derivedFor(engine, rev, floor) : EMPTY), [engine, rev, floor])
}

const issueCache = new WeakMap<PlanEngine, { rev: number; floor: string; data: Issue[] }>()

/** Validation hints; deferred so drags don't pay for validation every frame. */
export function useIssuesOf(src: PlanSource | null): Issue[] {
  const engine = src?.engine
  const rev = useDeferredValue(src?.revision ?? 0)
  const floor = src?.project.floors[0]?.id
  return useMemo(() => {
    if (!engine || !floor) return []
    const c = issueCache.get(engine)
    if (c && c.rev === rev && c.floor === floor) return c.data
    const data = safe(() => engine.validate(floor) as Issue[], [])
    issueCache.set(engine, { rev, floor, data })
    return data
  }, [engine, rev, floor])
}
