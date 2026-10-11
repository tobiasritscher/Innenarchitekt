// Project session: one PlanEngine (WASM) holds the model. Every change is an op batch
// with an inverse → undo/redo, autosave (batched to the server) and AI proposals share one path.
import { create } from 'zustand'
import { api, ApiError } from '../core/api'
import { PlanEngine } from '../core/engine'
import type { Op } from '../core/generated/Op'
import type { Project } from '../core/generated/Project'
import type { Floor } from '../core/generated/Floor'

export type Batch = { ops: Op[]; inverse: Op[]; label: string }
export type SyncState = 'saved' | 'pending' | 'saving' | 'error' | 'conflict'

type ProjectState = {
  engine: PlanEngine | null
  project: Project | null
  /** Increments on every local change; use as memo key for derived data. */
  revision: number
  undoStack: Batch[]
  redoStack: Batch[]
  sync: SyncState
  syncError: string | null
  lastError: string | null

  open: (id: string) => Promise<void>
  close: () => void
  /** Apply ops as one undoable batch. Returns false (and sets lastError) if the core rejects them. */
  commit: (ops: Op[], label: string) => boolean
  undo: () => void
  redo: () => void
  /** Live preview during drags: replaces the previous transient ops. */
  setTransient: (ops: Op[]) => boolean
  /** Finish a drag: commit (keeps current state as one batch) or cancel (revert). */
  endTransient: (commit: boolean, label?: string) => void
  clearError: () => void
}

let transient: { ops: Op[]; inverse: Op[] } | null = null
let outbox: Op[] = []
let serverVersion = 0
let flushTimer: ReturnType<typeof setTimeout> | null = null
let inFlight = false

function errMsg(e: unknown): string {
  if (e instanceof Error) return e.message
  return String(e)
}

export const useProject = create<ProjectState>((set, get) => {
  const refresh = () => {
    const engine = get().engine
    if (!engine) return
    set({ project: engine.project() as Project, revision: get().revision + 1 })
  }

  const queue = (ops: Op[]) => {
    outbox.push(...ops)
    set({ sync: 'pending' })
    scheduleFlush()
  }

  const scheduleFlush = (delay = 400) => {
    if (flushTimer) clearTimeout(flushTimer)
    flushTimer = setTimeout(flush, delay)
  }

  const flush = async () => {
    flushTimer = null
    const project = get().project
    if (!project || inFlight || outbox.length === 0 || transient) {
      if (transient) scheduleFlush()
      return
    }
    inFlight = true
    const ops = outbox
    outbox = []
    set({ sync: 'saving' })
    try {
      const res = await api.pushOps(project.id, serverVersion, ops)
      serverVersion = res.version
      get().engine?.set_version(res.version)
      set({ sync: outbox.length ? 'pending' : 'saved', syncError: null })
    } catch (e) {
      if (e instanceof ApiError && e.status === 409) {
        // Someone else (another tab) changed the project: reload server state.
        const server = e.body?.project as Project | undefined
        if (server) {
          get().engine?.set_project(server)
          serverVersion = server.version
          outbox = []
          set({ undoStack: [], redoStack: [], sync: 'conflict', syncError: 'Projekt wurde anderswo geändert – neu geladen.' })
          refresh()
        }
      } else {
        // keep ops for retry
        outbox = ops.concat(outbox)
        set({ sync: 'error', syncError: errMsg(e) })
        setTimeout(() => scheduleFlush(0), 3000)
      }
    } finally {
      inFlight = false
      if (outbox.length) scheduleFlush()
    }
  }

  window.addEventListener('beforeunload', (ev) => {
    if (outbox.length || inFlight) {
      ev.preventDefault()
    }
  })

  return {
    engine: null,
    project: null,
    revision: 0,
    undoStack: [],
    redoStack: [],
    sync: 'saved',
    syncError: null,
    lastError: null,

    open: async (id) => {
      const p = await api.getProject(id)
      get().engine?.free()
      const engine = new PlanEngine(p)
      serverVersion = p.version
      outbox = []
      transient = null
      set({ engine, project: p, revision: get().revision + 1, undoStack: [], redoStack: [], sync: 'saved', syncError: null })
    },

    close: () => {
      get().engine?.free()
      transient = null
      set({ engine: null, project: null, undoStack: [], redoStack: [] })
    },

    commit: (ops, label) => {
      const engine = get().engine
      if (!engine || ops.length === 0) return false
      if (transient) get().endTransient(true)
      try {
        const inverse = engine.apply(ops) as Op[]
        set({ undoStack: [...get().undoStack.slice(-499), { ops, inverse, label }], redoStack: [], lastError: null })
        refresh()
        queue(ops)
        return true
      } catch (e) {
        set({ lastError: errMsg(e) })
        return false
      }
    },

    undo: () => {
      const engine = get().engine
      const stack = get().undoStack
      if (!engine || stack.length === 0) return
      if (transient) get().endTransient(false)
      const b = stack[stack.length - 1]
      try {
        engine.apply(b.inverse)
        set({ undoStack: stack.slice(0, -1), redoStack: [...get().redoStack, b] })
        refresh()
        queue(b.inverse)
      } catch (e) {
        set({ lastError: errMsg(e) })
      }
    },

    redo: () => {
      const engine = get().engine
      const stack = get().redoStack
      if (!engine || stack.length === 0) return
      const b = stack[stack.length - 1]
      try {
        engine.apply(b.ops)
        set({ redoStack: stack.slice(0, -1), undoStack: [...get().undoStack, b] })
        refresh()
        queue(b.ops)
      } catch (e) {
        set({ lastError: errMsg(e) })
      }
    },

    setTransient: (ops) => {
      const engine = get().engine
      if (!engine) return false
      try {
        if (transient) engine.apply(transient.inverse)
        transient = null
        const inverse = engine.apply(ops) as Op[]
        transient = { ops, inverse }
        refresh()
        return true
      } catch (e) {
        set({ lastError: errMsg(e) })
        refresh()
        return false
      }
    },

    endTransient: (commit, label = 'Verschieben') => {
      const engine = get().engine
      const t = transient
      transient = null
      if (!engine || !t) return
      if (commit && t.ops.length) {
        set({ undoStack: [...get().undoStack, { ops: t.ops, inverse: t.inverse, label }], redoStack: [] })
        queue(t.ops)
      } else {
        engine.apply(t.inverse)
        refresh()
      }
    },

    clearError: () => set({ lastError: null }),
  }
})

/** Current (first) floor. */
export function useFloor(): Floor | null {
  return useProject((s) => s.project?.floors[0] ?? null)
}

export function floorId(): string {
  return useProject.getState().project!.floors[0].id
}
