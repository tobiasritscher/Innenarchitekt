// Furniture asset library (server-side, cached here).
import { create } from 'zustand'
import { api } from '../core/api'
import type { Asset } from '../core/generated/Asset'

type AssetState = {
  assets: Asset[]
  loaded: boolean
  load: () => Promise<void>
  add: (a: Asset) => void
  remove: (id: string) => Promise<void>
  byId: (id: string) => Asset | undefined
}

export const useAssets = create<AssetState>((set, get) => ({
  assets: [],
  loaded: false,
  load: async () => {
    const assets = await api.listAssets()
    set({ assets, loaded: true })
  },
  add: (a) => set({ assets: [a, ...get().assets.filter((x) => x.id !== a.id)] }),
  remove: async (id) => {
    await api.deleteAsset(id)
    set({ assets: get().assets.filter((a) => a.id !== id) })
  },
  byId: (id) => get().assets.find((a) => a.id === id),
}))
