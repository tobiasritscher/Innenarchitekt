import { useEffect, useState } from 'react'
import { api, type ProjectSummary } from '../core/api'
import { fmtDateTime } from '../core/format'

export function ProjectList({ onOpen }: { onOpen: (id: string) => void }) {
  const [projects, setProjects] = useState<ProjectSummary[] | null>(null)
  const [name, setName] = useState('')
  const [address, setAddress] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const reload = () => api.listProjects().then(setProjects, (e) => setError(String(e)))
  useEffect(() => {
    reload()
  }, [])

  return (
    <div className="page">
      <header className="page-head">
        <h1>Projekte</h1>
      </header>
      <form
        className="card new-project"
        onSubmit={async (e) => {
          e.preventDefault()
          if (!name.trim()) return
          setBusy(true)
          setError(null)
          try {
            const p = await api.createProject(name.trim(), address.trim() || undefined)
            if (address.trim() && !p.location) setError('Adresse nicht gefunden – Sonnenstand später im Projekt setzen.')
            onOpen(p.id)
          } catch (err) {
            setError(String(err))
          } finally {
            setBusy(false)
          }
        }}
      >
        <h2>Neues Projekt</h2>
        <label>
          Name
          <input value={name} onChange={(e) => setName(e.target.value)} placeholder="z. B. Wohnung Seefeld" required />
        </label>
        <label>
          Adresse <span className="muted">(optional, für den Sonnenstand)</span>
          <input value={address} onChange={(e) => setAddress(e.target.value)} placeholder="Strasse, PLZ Ort" />
        </label>
        <button className="primary" disabled={busy}>
          {busy ? 'Lege an …' : 'Anlegen'}
        </button>
        {error && <p className="error">{error}</p>}
      </form>

      <ul className="project-list">
        {projects === null && <li className="muted">Lade …</li>}
        {projects?.length === 0 && <li className="muted">Noch keine Projekte.</li>}
        {projects?.map((p) => (
          <li key={p.id} className="card project-row">
            <button className="link" onClick={() => onOpen(p.id)}>
              <strong>{p.name}</strong>
              <span className="muted">
                {p.wall_count} Wände · geändert {fmtDateTime(p.updated_at)}
              </span>
            </button>
            <button
              className="ghost danger"
              title="Projekt löschen"
              onClick={async () => {
                if (!confirm(`Projekt «${p.name}» endgültig löschen?`)) return
                await api.deleteProject(p.id)
                reload()
              }}
            >
              Löschen
            </button>
          </li>
        ))}
      </ul>
    </div>
  )
}
