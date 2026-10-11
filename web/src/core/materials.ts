// Material library. A MaterialRef is either a library id (e.g. "wood:oak") or a colour "#rrggbb".

export type MaterialDef = {
  id: string
  name: string
  group: 'Farbe' | 'Holz' | 'Fliesen' | 'Stein' | 'Textil' | 'Metall'
  color: string
  roughness: number
  metalness: number
  /** Procedural pattern for the texture generator in 3D. */
  pattern?: 'planks' | 'tiles' | 'herringbone' | 'noise' | 'weave'
  /** Pattern scale in metres (plank width, tile size …). */
  scale?: number
}

export const MATERIALS: MaterialDef[] = [
  { id: 'paint:white', name: 'Weiss', group: 'Farbe', color: '#f2f0eb', roughness: 0.9, metalness: 0 },
  { id: 'paint:offwhite', name: 'Gebrochenes Weiss', group: 'Farbe', color: '#e8e1d3', roughness: 0.9, metalness: 0 },
  { id: 'paint:greige', name: 'Greige', group: 'Farbe', color: '#c9bfae', roughness: 0.9, metalness: 0 },
  { id: 'paint:sage', name: 'Salbei', group: 'Farbe', color: '#a7b49b', roughness: 0.9, metalness: 0 },
  { id: 'paint:terracotta', name: 'Terrakotta', group: 'Farbe', color: '#c0714f', roughness: 0.9, metalness: 0 },
  { id: 'paint:navy', name: 'Nachtblau', group: 'Farbe', color: '#2d3a52', roughness: 0.85, metalness: 0 },
  { id: 'paint:anthracite', name: 'Anthrazit', group: 'Farbe', color: '#3c3f42', roughness: 0.85, metalness: 0 },
  { id: 'wood:oak', name: 'Eiche Parkett', group: 'Holz', color: '#b98d5f', roughness: 0.6, metalness: 0, pattern: 'planks', scale: 0.18 },
  { id: 'wood:oak-light', name: 'Eiche hell', group: 'Holz', color: '#d5b68c', roughness: 0.6, metalness: 0, pattern: 'planks', scale: 0.2 },
  { id: 'wood:walnut', name: 'Nussbaum', group: 'Holz', color: '#6b4a32', roughness: 0.55, metalness: 0, pattern: 'planks', scale: 0.16 },
  { id: 'wood:herringbone', name: 'Fischgrat Eiche', group: 'Holz', color: '#b08455', roughness: 0.6, metalness: 0, pattern: 'herringbone', scale: 0.09 },
  { id: 'tile:white', name: 'Fliese weiss 30×60', group: 'Fliesen', color: '#e9e9e6', roughness: 0.3, metalness: 0, pattern: 'tiles', scale: 0.3 },
  { id: 'tile:grey', name: 'Feinsteinzeug grau 60×60', group: 'Fliesen', color: '#9d9b97', roughness: 0.45, metalness: 0, pattern: 'tiles', scale: 0.6 },
  { id: 'tile:terrazzo', name: 'Terrazzo', group: 'Fliesen', color: '#d8d2c8', roughness: 0.35, metalness: 0, pattern: 'noise', scale: 0.4 },
  { id: 'stone:concrete', name: 'Sichtbeton', group: 'Stein', color: '#a3a19c', roughness: 0.85, metalness: 0, pattern: 'noise', scale: 1.2 },
  { id: 'stone:marble', name: 'Marmor', group: 'Stein', color: '#ece9e4', roughness: 0.2, metalness: 0, pattern: 'noise', scale: 0.8 },
  { id: 'fabric:carpet', name: 'Teppich grau', group: 'Textil', color: '#8a8580', roughness: 1, metalness: 0, pattern: 'weave', scale: 0.05 },
  { id: 'fabric:linen', name: 'Leinen', group: 'Textil', color: '#cfc4b0', roughness: 1, metalness: 0, pattern: 'weave', scale: 0.03 },
  { id: 'metal:steel', name: 'Edelstahl', group: 'Metall', color: '#b8bcbf', roughness: 0.35, metalness: 1 },
  { id: 'metal:black', name: 'Metall schwarz', group: 'Metall', color: '#222324', roughness: 0.5, metalness: 0.8 },
]

const byId = new Map(MATERIALS.map((m) => [m.id, m]))

export function resolveMaterial(ref: string | null | undefined): MaterialDef {
  if (!ref) return byId.get('paint:white')!
  if (ref.startsWith('#')) return { id: ref, name: ref, group: 'Farbe', color: ref, roughness: 0.9, metalness: 0 }
  return byId.get(ref) ?? byId.get('paint:white')!
}

export function materialName(ref: string | null | undefined): string {
  return resolveMaterial(ref).name
}
