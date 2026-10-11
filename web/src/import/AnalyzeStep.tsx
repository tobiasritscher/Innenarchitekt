// Step 3: upload + watch the analysis job (SSE, fallback polling).
import { useEffect, useRef, useState } from 'react'
import { api, type Job } from '../core/api'
import type { FloorplanProposal } from '../core/generated/FloorplanProposal'
import type { Corners } from './CropStep'
import type { PreparedImage } from './image'

type Phase = { t: 'uploading' } | { t: 'job'; job: Job } | { t: 'failed'; error: string }

export function AnalyzeStep({
  projectId,
  image,
  corners,
  onBack,
  onManual,
  onDone,
}: {
  projectId: string
  image: PreparedImage
  corners: Corners | null
  onBack: () => void
  onManual: () => void
  onDone: (p: FloorplanProposal) => void
}) {
  const [phase, setPhase] = useState<Phase>({ t: 'uploading' })
  const [attempt, setAttempt] = useState(0)
  const [elapsed, setElapsed] = useState(0)
  const doneRef = useRef(onDone)
  doneRef.current = onDone

  useEffect(() => {
    let cancelled = false
    let unsub: (() => void) | null = null
    let poll: ReturnType<typeof setInterval> | null = null
    const t0 = Date.now()
    const tick = setInterval(() => setElapsed(Math.round((Date.now() - t0) / 1000)), 1000)
    setPhase({ t: 'uploading' })
    setElapsed(0)

    const handle = (job: Job) => {
      if (cancelled) return
      if (job.status === 'done') {
        stop()
        if (job.result) doneRef.current(job.result)
        else setPhase({ t: 'failed', error: 'Die Analyse lieferte kein Ergebnis.' })
      } else if (job.status === 'failed') {
        stop()
        setPhase({ t: 'failed', error: job.error ?? job.message ?? 'Analyse fehlgeschlagen.' })
      } else setPhase({ t: 'job', job })
    }
    const stop = () => {
      unsub?.()
      unsub = null
      if (poll) clearInterval(poll)
      poll = null
      clearInterval(tick)
    }
    const startPolling = (id: string) => {
      if (poll || cancelled) return
      poll = setInterval(() => {
        api.getJob(id).then(handle, (e) => {
          stop()
          if (!cancelled) setPhase({ t: 'failed', error: `Status nicht abrufbar: ${String(e)}` })
        })
      }, 1500)
    }

    api
      .uploadFloorplan(projectId, image.blob, corners ?? undefined)
      .then(({ job_id }) => {
        if (cancelled) return
        setPhase({ t: 'job', job: { id: job_id, kind: 'floorplan', status: 'queued', progress: 0, message: 'In der Warteschlange …', result: null, error: null } })
        unsub = api.watchJob(job_id, handle, () => {
          unsub = null
          startPolling(job_id)
        })
      })
      .catch((e) => {
        stop()
        if (!cancelled) setPhase({ t: 'failed', error: `Upload fehlgeschlagen: ${e instanceof Error ? e.message : String(e)}` })
      })

    return () => {
      cancelled = true
      stop()
    }
  }, [projectId, image, corners, attempt])

  const progress = phase.t === 'job' ? phase.job.progress : 0
  return (
    <div className="import-step analyze-step">
      <div className="card analyze-card">
        <img className="analyze-thumb" src={image.url} alt="" />
        {phase.t === 'failed' ? (
          <>
            <h2>Analyse fehlgeschlagen</h2>
            <p className="error">{phase.error}</p>
            <div className="row actions">
              <button onClick={onBack}>Zurück</button>
              <button onClick={onManual}>Manuell zeichnen</button>
              <button className="primary" onClick={() => setAttempt((a) => a + 1)}>
                Erneut versuchen
              </button>
            </div>
          </>
        ) : (
          <>
            <h2>Grundriss wird erkannt …</h2>
            <div className="progress" role="progressbar" aria-valuenow={Math.round(progress * 100)} aria-valuemin={0} aria-valuemax={100}>
              <div className="bar" style={{ width: `${Math.max(3, progress * 100)}%` }} />
            </div>
            <p className="muted">
              {phase.t === 'uploading' ? 'Lade hoch …' : phase.job.message || 'Analysiere …'} · {elapsed} s
            </p>
            <div className="row actions">
              <button onClick={onBack}>Abbrechen</button>
            </div>
          </>
        )}
      </div>
    </div>
  )
}
