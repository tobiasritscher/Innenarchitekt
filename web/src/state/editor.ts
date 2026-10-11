// UI state of the editor (not persisted in the project).
import { create } from 'zustand'

export type Mode = 'model' | 'room' | 'light' | 'view' | 'import'

export type Tool =
  | 'select'
  | 'wall'
  | 'opening'
  | 'room'
  | 'material'
  | 'measure'
  | 'ruler'
  | 'furniture'
  | 'light'

export type SelectionKind = 'node' | 'wall' | 'opening' | 'room' | 'furniture' | 'light'
export type Selection = { kind: SelectionKind; id: string } | null
export type Sel = NonNullable<Selection>
const same = (a: Sel, b: Sel) => a.kind === b.kind && a.id === b.id

export type Quality = 'performance' | 'balanced' | 'beautiful'
export type NavMode = 'walk' | 'dollhouse'

export type ViewSettings = {
  /** ISO date yyyy-mm-dd (local). */
  date: string
  /** Minutes after local midnight. */
  minutes: number
  /** 0 = overcast … 1 = clear sky. */
  clearness: number
  quality: Quality
  eyeHeightMm: number
  nav: NavMode
  /** Walker position (plan mm) and yaw (rad), set via teleport. */
  walker: { x: number; y: number; yaw: number } | null
  exposure: number
}

/** Wall/opening defaults for drawing tools. */
export type DrawDefaults = {
  wallThicknessMm: number
  wallExterior: boolean
  openingKind: 'Door' | 'Window' | 'Passage'
  doorWidthMm: number
  doorHeightMm: number
  doorGlass: boolean
  windowWidthMm: number
  windowHeightMm: number
  windowSillMm: number
  material: string
}

type EditorState = {
  mode: Mode
  tool: Tool
  /** Primary selection (last picked); its properties are shown. */
  selection: Selection
  /** Everything selected (Shift+click / frame), primary included. */
  multi: Sel[]
  hover: Selection
  /** On narrow screens only one of 2D/3D is shown. */
  narrowPane: '2d' | '3d'
  showRefImage: boolean
  snapping: boolean
  draw: DrawDefaults
  view: ViewSettings
  /** Asset chosen in the furniture panel, to be placed by clicking in the plan. */
  placingAsset: string | null
  toast: string | null
  /** 2D editor: request to zoom to elements (ids); `n` changes on every request. */
  focusRequest: { ids: string[]; n: number } | null
  /** 2D editor: request «Zoom auf alles»; changes on every request. */
  fitRequest: number
  /** Properties panel: field to focus next (e.g. 'room-name'). */
  focusField: string | null
  /** Room mode: focused room (RoomMeta id, else DerivedRoom key). */
  room: string | null

  setMode: (m: Mode) => void
  setTool: (t: Tool) => void
  select: (s: Selection) => void
  /** Shift+click: add to / remove from the selection. */
  toggleSelect: (s: Sel) => void
  selectMany: (s: Sel[]) => void
  setHover: (s: Selection) => void
  setNarrowPane: (p: '2d' | '3d') => void
  setShowRefImage: (v: boolean) => void
  setSnapping: (v: boolean) => void
  setDraw: (d: Partial<DrawDefaults>) => void
  setView: (v: Partial<ViewSettings>) => void
  setPlacingAsset: (id: string | null) => void
  showToast: (msg: string | null) => void
  requestFocus: (ids: string[]) => void
  requestFit: () => void
  setFocusField: (f: string | null) => void
  setRoom: (r: string | null) => void
}

const today = new Date()
const iso = `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, '0')}-${String(today.getDate()).padStart(2, '0')}`

export const useEditor = create<EditorState>((set) => ({
  mode: 'model',
  tool: 'select',
  selection: null,
  multi: [],
  hover: null,
  narrowPane: '2d',
  showRefImage: true,
  snapping: true,
  draw: {
    wallThicknessMm: 150,
    wallExterior: false,
    openingKind: 'Door',
    doorWidthMm: 900,
    doorHeightMm: 2050,
    doorGlass: false,
    windowWidthMm: 1200,
    windowHeightMm: 1400,
    windowSillMm: 850,
    material: 'paint:white',
  },
  view: {
    date: iso,
    minutes: 14 * 60,
    clearness: 0.85,
    quality: 'balanced',
    eyeHeightMm: 1650,
    nav: 'dollhouse',
    walker: null,
    exposure: 1,
  },
  placingAsset: null,
  toast: null,
  focusRequest: null,
  fitRequest: 0,
  focusField: null,
  room: null,

  setMode: (mode) => set({ mode, hover: null }),
  setTool: (tool) => set({ tool, placingAsset: null }),
  select: (selection) => set({ selection, multi: selection ? [selection] : [] }),
  toggleSelect: (x) =>
    set((st) => {
      const multi = st.multi.some((m) => same(m, x)) ? st.multi.filter((m) => !same(m, x)) : [...st.multi, x]
      return { multi, selection: multi[multi.length - 1] ?? null }
    }),
  selectMany: (multi) => set({ multi, selection: multi[multi.length - 1] ?? null }),
  setHover: (hover) => set({ hover }),
  setNarrowPane: (narrowPane) => set({ narrowPane }),
  setShowRefImage: (showRefImage) => set({ showRefImage }),
  setSnapping: (snapping) => set({ snapping }),
  setDraw: (d) => set((s) => ({ draw: { ...s.draw, ...d } })),
  setView: (v) => set((s) => ({ view: { ...s.view, ...v } })),
  setPlacingAsset: (placingAsset) => set({ placingAsset, tool: placingAsset ? 'furniture' : 'select' }),
  showToast: (toast) => set({ toast }),
  requestFocus: (ids) => set((s) => ({ focusRequest: { ids, n: (s.focusRequest?.n ?? 0) + 1 } })),
  requestFit: () => set((s) => ({ fitRequest: s.fitRequest + 1 })),
  setFocusField: (focusField) => set({ focusField }),
  setRoom: (room) => set({ room }),
}))
