// 3D-specific UI state (not persisted in the project).
import { create } from 'zustand'

export type LampType = 'Point' | 'Spot' | 'Area'

/** A 3D click tool is active: element clicks/drags must not select or move anything. */
export const toolActive = () => {
  const s = useView3D.getState()
  return s.ruler || !!s.placingLight
}

export type Walker = { x: number; y: number; yaw: number }

type View3DState = {
  /** Preview: cut walls at ~1.2 m for a look inside. */
  cutWalls: boolean
  /** Walker pose for the mini map (throttled copy of the live pose). */
  walker: Walker | null
  /** Ground hit while placing an asset in 3D (plan mm), for the ghost. */
  ghost: { x: number; y: number; rot: number } | null
  /** Room mode: bump to ease the camera back to the room. */
  roomFit: number
  /** 3D ruler active. */
  ruler: boolean
  /** Door state (view only, not saved): missing = open. */
  doorOpen: Record<string, boolean>
  /** Room mode: lamp type being placed by clicking (ceiling mounted). */
  placingLight: LampType | null
  setCutWalls: (v: boolean) => void
  setWalker: (w: Walker | null) => void
  setGhost: (g: { x: number; y: number; rot: number } | null) => void
  requestRoomFit: () => void
  setRuler: (v: boolean) => void
  setPlacingLight: (t: LampType | null) => void
  toggleDoor: (id: string) => void
  setDoors: (ids: string[], open: boolean) => void
}

export const useView3D = create<View3DState>((set) => ({
  cutWalls: false,
  walker: null,
  ghost: null,
  roomFit: 0,
  ruler: false,
  placingLight: null,
  doorOpen: {},
  setCutWalls: (cutWalls) => set({ cutWalls }),
  setWalker: (walker) => set({ walker }),
  setGhost: (ghost) => set({ ghost }),
  requestRoomFit: () => set((s) => ({ roomFit: s.roomFit + 1 })),
  setRuler: (ruler) => set(ruler ? { ruler, placingLight: null } : { ruler }),
  toggleDoor: (id) => set((s) => ({ doorOpen: { ...s.doorOpen, [id]: !(s.doorOpen[id] ?? true) } })),
  setDoors: (ids, open) => set((s) => ({ doorOpen: { ...s.doorOpen, ...Object.fromEntries(ids.map((i) => [i, open])) } })),
  setPlacingLight: (placingLight) => set(placingLight ? { placingLight, ruler: false } : { placingLight }),
}))

/** Live walker pose, mutated every frame by the walk controls (plan mm, yaw rad, pitch rad). */
export const live = {
  walker: { x: 0, y: 0, yaw: 0, pitch: 0 },
  /** Set by the mini map teleport: walk controls pick it up on the next frame. */
  teleport: null as Walker | null,
  /** Multiplier from physical units (lux, cd, nits) to render units incl. exposure (set by Daylight). */
  lightScale: 1,
  /** Virtual joystick (touch): x = strafe right, y = forward, each −1…1. */
  joy: { x: 0, y: 0 },
  /** Current daylight (physical units, set by Daylight) – the path tracer builds its sky from it. */
  sky: { skyLux: 16000, sunLux: 0, sinEl: 0.7, day: 1, clearness: 0.85 },
}
