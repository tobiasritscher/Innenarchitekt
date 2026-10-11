// «Aus Foto»: photo + real dimensions → 3D model via TRELLIS.2 on the server (several minutes).
// Running jobs live in a module store so they survive tab switches; the result lands in the library.
import { useEffect, useState } from 'react'
import { create } from 'zustand'
import { api, ApiError, type Job } from '../core/api'
import type { Asset } from '../core/generated/Asset'
import { shrinkPhoto } from '../import/image'
import { useAssets } from '../state/assets'

type PhotoJob = { id: string; name: string; progress: number; message: string; status: string; error: string | null; preview: string }

const usePhotoJobs = create<{ jobs: PhotoJob[]; set: (id: string, p: Partial<PhotoJob>) => void; add: (j: PhotoJob) => void; dismiss: (id: string) => void }>((set, get) => ({
  jobs: [],
  add: (j) => set({ jobs: [...get().jobs, j] }),
  set: (id, p) => set({ jobs: get().jobs.map((j) => (j.id === id ? { ...j, ...p } : j)) }),
  dismiss: (id) => set({ jobs: get().jobs.filter((j) => j.id !== id) }),
}))

function follow(id: string) {
  const st = usePhotoJobs.getState()
  const handle = (job: Job) => {
    st.set(id, { progress: job.progress, message: job.message, status: job.status, error: job.error })
    if (job.status === 'done' && job.result) useAssets.getState().add(job.result as unknown as Asset)
  }
  // SSE with polling fallback (proxies sometimes cut long-lived streams)
  let timer: ReturnType<typeof setInterval> | null = null
  const poll = () => {
    if (timer) return
    timer = setInterval(() => {
      api.getJob(id).then(
        (j) => {
          handle(j)
          if (j.status === 'done' || j.status === 'failed') clearInterval(timer!)
        },
        () => {},
      )
    }, 4000)
  }
  api.watchJob(id, handle, poll)
}

const PHOTO_MAX_PX = 1600

export function PhotoTo3D({ enabled }: { enabled: boolean }) {
  const [open, setOpen] = useState(false)
  const [photo, setPhoto] = useState<{ blob: Blob; url: string } | null>(null)
  const [name, setName] = useState('')
  const [dims, setDims] = useState<[number, number, number]>([800, 800, 800])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const jobs = usePhotoJobs((s) => s.jobs)
  useEffect(() => () => void (photo && URL.revokeObjectURL(photo.url)), [photo])

  const start = async () => {
    if (!photo) return
    setBusy(true)
    setError(null)
    try {
      const { job_id } = await api.fromPhoto(photo.blob, name.trim() || 'Möbel aus Foto', dims)
      usePhotoJobs.getState().add({ id: job_id, name: name.trim() || 'Möbel aus Foto', progress: 0, message: 'In Warteschlange', status: 'queued', error: null, preview: URL.createObjectURL(photo.blob) })
      follow(job_id)
      setPhoto(null)
      setName('')
    } catch (e) {
      setError(e instanceof ApiError || e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <section>
      <h3>
        <button className="link v3d-h3btn" onClick={() => setOpen(!open)}>
          {open ? '▾' : '▸'} Aus Foto (KI)
        </button>
      </h3>
      {open && !enabled && <p className="muted v3d-small">Auf dem Server nicht eingerichtet (PHOTO3D_DIR, siehe deploy/photo3d-setup.sh).</p>}
      {open && enabled && (
        <div className="form v3d-subform">
          <p className="muted v3d-small">Ein Foto eines einzelnen Möbels, möglichst ganz im Bild und schräg von vorne. Der Hintergrund wird automatisch entfernt. Dauert ca. 5–10 Minuten.</p>
          <label className="field">
            Foto
            <input
              type="file"
              accept="image/*"
              onChange={async (e) => {
                const f = e.target.files?.[0]
                e.target.value = ''
                if (!f) return
                try {
                  const blob = await shrinkPhoto(f, PHOTO_MAX_PX)
                  setPhoto({ blob, url: URL.createObjectURL(blob) })
                  if (!name) setName(f.name.replace(/\.[^.]+$/, ''))
                } catch (err) {
                  setError(String(err))
                }
              }}
            />
          </label>
          {photo && <img className="v3d-photo-preview" src={photo.url} alt="Foto" />}
          <label className="field">
            Name
            <input value={name} onChange={(e) => setName(e.target.value)} placeholder="z. B. Sessel Oma" />
          </label>
          <PhotoDims dims={dims} onChange={setDims} />
          <p className="muted v3d-small">Masse sind Pflicht – die KI kennt die echte Grösse nicht.</p>
          <button className="primary" onClick={start} disabled={busy || !photo}>
            {busy ? 'Lade hoch …' : '3D-Modell erzeugen'}
          </button>
          {error && <p className="error v3d-small">{error}</p>}
        </div>
      )}
      {jobs.map((j) => (
        <div key={j.id} className="v3d-photo-job">
          <img src={j.preview} alt="" />
          <div>
            <strong>{j.name}</strong>
            {j.status === 'failed' ? (
              <p className="error v3d-small">{j.error ?? 'Fehlgeschlagen'}</p>
            ) : j.status === 'done' ? (
              <p className="v3d-small">✓ In der Bibliothek – zum Prüfen platzieren, bei Nichtgefallen dort löschen.</p>
            ) : (
              <>
                <progress max={1} value={j.progress} />
                <p className="muted v3d-small">{j.message}</p>
              </>
            )}
            {(j.status === 'done' || j.status === 'failed') && (
              <button className="link small" onClick={() => usePhotoJobs.getState().dismiss(j.id)}>
                Ausblenden
              </button>
            )}
          </div>
        </div>
      ))}
    </section>
  )
}

function PhotoDims({ dims, onChange }: { dims: [number, number, number]; onChange: (d: [number, number, number]) => void }) {
  const labels = ['Breite', 'Tiefe', 'Höhe']
  return (
    <div className="v3d-dims">
      {dims.map((v, i) => (
        <label key={i} className="field">
          {labels[i]} (cm)
          <input
            type="number"
            min={1}
            step={1}
            value={Math.round(v / 10)}
            onChange={(e) => {
              const next = [...dims] as [number, number, number]
              next[i] = Math.max(1, Math.round(Number(e.target.value) * 10) || 10)
              onChange(next)
            }}
          />
        </label>
      ))}
    </div>
  )
}
