import { useState } from 'react'
import { api } from '../core/api'
import { useProject } from '../state/project'
import { NumberField } from './fields'

/** Project-wide settings: name, location (for the sun), north angle, ceiling height. */
export function ProjectSettings() {
  const project = useProject((s) => s.project)!
  const commit = useProject((s) => s.commit)
  const floor = project.floors[0]
  const [address, setAddress] = useState(project.location?.address ?? '')
  const [geoError, setGeoError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const setProject = (patch: Partial<{ name: string; location: typeof project.location; north_angle_deg: number }>, label: string) =>
    commit(
      [
        {
          op: 'SetProject',
          name: patch.name ?? project.name,
          location: patch.location !== undefined ? patch.location : project.location,
          north_angle_deg: patch.north_angle_deg ?? project.north_angle_deg,
        },
      ],
      label,
    )

  return (
    <div className="form">
      <label>
        Projektname
        <input defaultValue={project.name} onBlur={(e) => e.target.value.trim() && e.target.value !== project.name && setProject({ name: e.target.value.trim() }, 'Projekt umbenennen')} />
      </label>
      <label>
        Adresse (Sonnenstand)
        <div className="row">
          <input value={address} onChange={(e) => setAddress(e.target.value)} placeholder="Strasse, PLZ Ort" />
          <button
            disabled={busy || !address.trim()}
            onClick={async () => {
              setBusy(true)
              setGeoError(null)
              try {
                const loc = await api.geocode(address.trim())
                setProject({ location: loc }, 'Standort setzen')
              } catch {
                setGeoError('Adresse nicht gefunden')
              } finally {
                setBusy(false)
              }
            }}
          >
            Suchen
          </button>
        </div>
      </label>
      {geoError && <p className="error">{geoError}</p>}
      {project.location && (
        <p className="muted">
          {project.location.lat.toFixed(4)}° N, {project.location.lon.toFixed(4)}° E
        </p>
      )}
      <div className="row">
        <NumberField label="Breitengrad" value={project.location?.lat ?? 47.37} step={0.0001} onCommit={(lat) => setProject({ location: { lat, lon: project.location?.lon ?? 8.54, address: project.location?.address ?? null } }, 'Standort setzen')} />
        <NumberField label="Längengrad" value={project.location?.lon ?? 8.54} step={0.0001} onCommit={(lon) => setProject({ location: { lat: project.location?.lat ?? 47.37, lon, address: project.location?.address ?? null } }, 'Standort setzen')} />
      </div>
      <NumberField label="Nordausrichtung (° im Uhrzeigersinn)" value={project.north_angle_deg} step={1} onCommit={(v) => setProject({ north_angle_deg: ((v % 360) + 360) % 360 }, 'Nordpfeil')} />
      <NumberField
        label="Raumhöhe (mm)"
        value={floor.ceiling_height_mm}
        step={10}
        min={1000}
        onCommit={(v) =>
          commit([{ op: 'SetFloor', floor: floor.id, name: floor.name, ceiling_height_mm: Math.round(v), reference_image: floor.reference_image }], 'Raumhöhe')
        }
      />
    </div>
  )
}
