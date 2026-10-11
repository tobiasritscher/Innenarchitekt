// Shared lighting panels (view mode «Begehen» and light mode «Licht»): daylight, lamps, light scenes, PNG export.
import { useMemo, useState, type ReactNode } from 'react'
import type { WebGLRenderer } from 'three'
import { useRooms } from '../core/derived'
import { fmtDate, fmtNumber, fmtTime } from '../core/format'
import { newId } from '../core/engine'
import type { Light } from '../core/generated/Light'
import type { LightScene } from '../core/generated/LightScene'
import { useEditor } from '../state/editor'
import { useProject } from '../state/project'
import { useSun } from './Daylight'

const NO_LIGHTS: Light[] = []
const NO_SCENES: LightScene[] = []

/** Rough mean lamp illuminance (lux) for the auto exposure. */
export function useLampLux(): number {
  const lights = useProject((s) => s.project?.lights ?? NO_LIGHTS)
  const rooms = useRooms()
  return useMemo(() => {
    const lm = lights.filter((l) => l.on).reduce((s, l) => s + l.lumen, 0)
    const area = rooms.reduce((s, r) => s + r.area_m2, 0)
    return (lm * 0.5) / Math.max(8, area)
  }, [lights, rooms])
}

/** Download the current canvas as PNG: «<Projekt> <TT.MM.JJJJ> <HHMM>[ suffix].png». */
export function saveCanvasPng(gl: WebGLRenderer | null, suffix?: string) {
  if (!gl) return
  const { view } = useEditor.getState()
  const project = useProject.getState().project
  const url = gl.domElement.toDataURL('image/png')
  const a = document.createElement('a')
  const d = view.date.split('-').reverse().join('.')
  a.download = `${(project?.name ?? 'Ansicht').replace(/[^\p{L}\p{N}_ -]/gu, '')} ${d} ${fmtTime(view.minutes).replace(':', '')}${suffix ? ` ${suffix}` : ''}.png`
  a.href = url
  a.click()
}

function isoOf(y: number, m: number, d: number) {
  return `${y}-${String(m).padStart(2, '0')}-${String(d).padStart(2, '0')}`
}

const QUICK_TIMES: [string, number][] = [
  ['Morgen', 480],
  ['Mittag', 720],
  ['Abend', 1140],
  ['Nacht', 1320],
]

/** Date, time of day (with quick picks), sky and exposure. */
export function DaylightControls() {
  const view = useEditor((s) => s.view)
  const setView = useEditor((s) => s.setView)
  const project = useProject((s) => s.project)
  const sun = useSun()
  const year = Number(view.date.slice(0, 4)) || new Date().getFullYear()
  const ev = Math.log2(view.exposure || 1)
  return (
    <>
      {!project?.location && <p className="v3d-note">Kein Standort gesetzt – Sonnenstand für Zürich. Adresse unter «Projekt» im Modelliermodus setzen.</p>}

      <h3>Tageslicht</h3>
      <label className="field">
        <span className="v3d-row">
          Datum <strong>{fmtDate(view.date)}</strong>
        </span>
        <input type="date" value={view.date} onChange={(e) => e.target.value && setView({ date: e.target.value })} />
      </label>
      <div className="v3d-row v3d-quick">
        <button onClick={() => setView({ date: isoOf(year, 6, 21) })}>Sommersonnenwende</button>
        <button onClick={() => setView({ date: isoOf(year, 12, 21) })}>Wintersonnenwende</button>
      </div>
      <label className="field">
        <span className="v3d-row">
          Uhrzeit <strong>{fmtTime(view.minutes)}</strong>
        </span>
        <input type="range" min={0} max={1435} step={5} value={view.minutes} onChange={(e) => setView({ minutes: Number(e.target.value) })} />
      </label>
      <div className="v3d-row v3d-quick">
        {QUICK_TIMES.map(([label, m]) => (
      <button key={label} aria-pressed={view.minutes === m} onClick={() => setView({ minutes: m })}>
        {label}
      </button>
        ))}
      </div>
      <p className="v3d-sun muted">
        {sun.elevation_deg > 0 ? `Sonne ${fmtNumber(sun.elevation_deg, 0)}° über dem Horizont, Azimut ${fmtNumber(sun.azimuth_deg, 0)}°` : 'Sonne unter dem Horizont'}
      </p>
      <label className="field">
        <span className="v3d-row">
          Himmel <strong>{view.clearness < 0.3 ? 'bedeckt' : view.clearness < 0.7 ? 'bewölkt' : 'klar'}</strong>
        </span>
        <input type="range" min={0} max={1} step={0.01} value={view.clearness} onChange={(e) => setView({ clearness: Number(e.target.value) })} />
      </label>
      <label className="field">
        <span className="v3d-row">
          Belichtung <strong>{ev >= 0 ? '+' : ''}{fmtNumber(ev, 1)} EV</strong>
        </span>
        <input type="range" min={-3} max={3} step={0.1} value={ev} onChange={(e) => setView({ exposure: Math.pow(2, Number(e.target.value)) })} onDoubleClick={() => setView({ exposure: 1 })} />
      </label>
    </>
  )
}

export function LampList({ hint }: { hint?: ReactNode }) {
  const lights = useProject((s) => s.project?.lights ?? NO_LIGHTS)
  const furniture = useProject((s) => s.project?.furniture)
  if (!lights.length) return <p className="muted v3d-small">{hint ?? 'Keine Lampen im Projekt.'}</p>
  const toggle = (l: Light) => useProject.getState().commit([{ op: 'PutLight', light: { ...l, on: !l.on } }], l.on ? `${l.name} ausschalten` : `${l.name} einschalten`)
  const allOn = lights.every((l) => l.on)
  return (
    <div className="v3d-lamps">
      {lights.map((l) => {
        const host = l.attached_to ? furniture?.find((f) => f.id === l.attached_to) : undefined
        return (
          <label key={l.id} className="v3d-lamp">
            <input type="checkbox" checked={l.on} onChange={() => toggle(l)} />
            <span className="v3d-lamp-name">{l.name || host?.name || 'Lampe'}</span>
            <span className="muted">
              {fmtNumber(l.lumen)} lm · {fmtNumber(l.color_temp_k)} K
            </span>
          </label>
        )
      })}
      {lights.length > 1 && (
        <button
          className="link v3d-small"
          onClick={() =>
            useProject.getState().commit(
              lights.filter((l) => l.on === allOn).map((l) => ({ op: 'PutLight' as const, light: { ...l, on: !allOn } })),
              allOn ? 'Alle Lampen aus' : 'Alle Lampen ein',
            )
          }
        >
          {allOn ? 'Alle aus' : 'Alle ein'}
        </button>
      )}
    </div>
  )
}

/** Ops that bring the lamps into the scene's state (lamps deleted since are skipped). */
function sceneOps(lights: Light[], sc: LightScene) {
  return sc.states.flatMap((st) => {
    const l = lights.find((x) => x.id === st.light)
    return l && (l.on !== st.on || l.lumen !== st.lumen) ? [{ op: 'PutLight' as const, light: { ...l, on: st.on, lumen: st.lumen } }] : []
  })
}

const snapshot = (lights: Light[]) => lights.map((l) => ({ light: l.id, on: l.on, lumen: l.lumen }))

export function LightScenes() {
  const lights = useProject((s) => s.project?.lights ?? NO_LIGHTS)
  const scenes = useProject((s) => s.project?.light_scenes ?? NO_SCENES)
  // null = idle, '' = new scene, else id of the scene being renamed
  const [editing, setEditing] = useState<string | null>(null)
  const [name, setName] = useState('')
  if (!lights.length) return null
  const commit = useProject.getState().commit
  const put = (scene: LightScene, label: string) => commit([{ op: 'PutLightScene', scene }], label)
  const startEdit = (id: string, initial: string) => {
    setEditing(id)
    setName(initial)
  }
  const save = () => {
    const n = name.trim()
    if (!n) return
    if (editing === '') put({ id: newId(), name: n, states: snapshot(lights) }, `Lichtszene «${n}» speichern`)
    else {
      const sc = scenes.find((x) => x.id === editing)
      if (sc && sc.name !== n) put({ ...sc, name: n }, 'Lichtszene umbenennen')
    }
    setEditing(null)
  }
  const editor = (
    <div className="v3d-scene-edit">
      <input
        autoFocus
        value={name}
        placeholder="Name, z. B. Abend"
        onChange={(e) => setName(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') save()
          if (e.key === 'Escape') setEditing(null)
        }}
      />
      <button className="primary" onClick={save} disabled={!name.trim()}>
        OK
      </button>
      <button className="ghost" onClick={() => setEditing(null)} title="Abbrechen">
        ×
      </button>
    </div>
  )
  return (
    <div className="v3d-scenes">
      <h4>Lichtszenen</h4>
      {scenes.map((sc) =>
        editing === sc.id ? (
          <div key={sc.id}>{editor}</div>
        ) : (
          <div key={sc.id} className="v3d-scene">
            <button
              className="v3d-scene-apply"
              aria-pressed={sceneOps(lights, sc).length === 0}
              onClick={() => {
                const ops = sceneOps(lights, sc)
                if (ops.length) commit(ops, `Lichtszene: ${sc.name}`)
              }}
            >
              {sc.name}
            </button>
            <button className="ghost" title="Mit aktuellem Zustand überschreiben" onClick={() => put({ ...sc, states: snapshot(lights) }, `Lichtszene «${sc.name}» aktualisieren`)}>
              ↻
            </button>
            <button className="ghost" title="Umbenennen" onClick={() => startEdit(sc.id, sc.name)}>
              ✎
            </button>
            <button className="ghost" title="Löschen" onClick={() => commit([{ op: 'DeleteLightScene', id: sc.id }], `Lichtszene «${sc.name}» löschen`)}>
              🗑
            </button>
          </div>
        ),
      )}
      {editing === '' ? (
        editor
      ) : (
        <button className="link v3d-small" onClick={() => startEdit('', '')}>
          + Aktuellen Zustand speichern
        </button>
      )}
    </div>
  )
}
