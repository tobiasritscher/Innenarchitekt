// Floor plan import (spec flow A): upload → crop/rectify → analysis → correction → reference
// measure → north arrow → apply as ONE undoable commit on the main project.
import { useEffect, useMemo, useRef, useState } from 'react'
import type { FloorplanProposal } from '../core/generated/FloorplanProposal'
import type { Op } from '../core/generated/Op'
import type { Project } from '../core/generated/Project'
import { fmtLength, fmtNumber, parseLength } from '../core/format'
import { PlanCanvas } from '../editor2d/PlanCanvas'
import { PropertiesPanel } from '../editor2d/PropertiesPanel'
import { Toolbar } from '../editor2d/Toolbar'
import { dist, type Vec } from '../editor2d/geom'
import { kindOf } from '../editor2d/model'
import { createLocalSession, PlanSourceContext, useLocalSource, type LocalSessionStore, type PlanSource } from '../editor2d/source'
import type { View } from '../editor2d/view'
import { useEditor } from '../state/editor'
import { useProject } from '../state/project'
import { AnalyzeStep } from './AnalyzeStep'
import { Compass } from './Compass'
import { CropStep, type Corners } from './CropStep'
import { makeFixtureProposal } from './fixture'
import { prepareImage, type PreparedImage } from './image'

type Step = 'upload' | 'crop' | 'analyze' | 'correct' | 'scale' | 'north'
const STEPS: { id: Step; label: string }[] = [
  { id: 'upload', label: 'Hochladen' },
  { id: 'crop', label: 'Entzerren' },
  { id: 'analyze', label: 'Erkennen' },
  { id: 'correct', label: 'Korrigieren' },
  { id: 'scale', label: 'Massstab' },
  { id: 'north', label: 'Norden' },
]

const LOW_CONF = 0.6
const KIND_NAMES: Record<string, string> = { wall: 'Wand', opening: 'Öffnung', room: 'Raum', node: 'Ecke', furniture: 'Möbel', light: 'Lampe' }

const devFixture = import.meta.env.DEV && new URLSearchParams(location.search).has('fixture')

/** Ops from the proposal, re-targeted to `floor`, plus the reference image. */
export function proposalOps(p: FloorplanProposal, project: Project, replace: boolean): Op[] {
  const f = project.floors[0]
  const ops: Op[] = []
  if (replace) {
    for (const o of f.openings) ops.push({ op: 'DeleteOpening', floor: f.id, id: o.id })
    for (const r of f.rooms) ops.push({ op: 'DeleteRoom', floor: f.id, id: r.id })
    for (const w of f.walls) ops.push({ op: 'DeleteWall', floor: f.id, id: w.id })
    for (const n of f.nodes) ops.push({ op: 'DeleteNode', floor: f.id, id: n.id })
  }
  for (const op of p.ops) ops.push('floor' in op ? ({ ...op, floor: f.id } as Op) : op)
  ops.push({
    op: 'SetFloor',
    floor: f.id,
    name: f.name,
    ceiling_height_mm: f.ceiling_height_mm,
    reference_image: { ...p.image, opacity: 0.5, visible: true },
  })
  return ops
}

export function ImportWizard({ onClose }: { onClose: () => void }) {
  const project = useProject((s) => s.project)
  const [step, setStep] = useState<Step>('upload')
  const [image, setImage] = useState<PreparedImage | null>(null)
  const [corners, setCorners] = useState<Corners | null>(null)
  const [proposal, setProposal] = useState<FloorplanProposal | null>(null)
  const [pending, setPending] = useState<FloorplanProposal | null>(null)
  const [session, setSession] = useState<LocalSessionStore | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [north, setNorth] = useState(0)

  useEffect(() => () => session?.getState().dispose(), [session])
  useEffect(() => () => {
    if (image) URL.revokeObjectURL(image.url)
  }, [image])
  useEffect(() => {
    const ed = useEditor.getState()
    ed.select(null)
    ed.setHover(null)
    ed.setTool('select')
    return () => {
      useEditor.getState().select(null)
      useEditor.getState().setTool('select')
    }
  }, [])

  if (!project) return null
  const hasExisting = project.floors[0].walls.length > 0

  const startReview = (p: FloorplanProposal, replace: boolean) => {
    try {
      const s = createLocalSession(project)
      s.getState().applyRaw(proposalOps(p, s.getState().project, replace))
      s.getState().resetHistory()
      setSession(() => s) // the store is a function: never pass it as a state updater
      setProposal(p)
      setPending(null)
      setNorth(p.north_angle_deg ?? project.north_angle_deg)
      setStep('correct')
      useEditor.getState().requestFit()
    } catch (e) {
      setError(`Vorschlag konnte nicht angewendet werden: ${e instanceof Error ? e.message : String(e)}`)
    }
  }

  const onProposal = (p: FloorplanProposal) => {
    if (hasExisting) {
      setPending(p)
      setStep('correct')
    } else startReview(p, false)
  }

  const apply = () => {
    if (!session) return
    const main = useProject.getState()
    if (!main.engine) return
    const temp = session.getState().project
    const target: Project = { ...temp, north_angle_deg: Math.round(north * 10) / 10 }
    try {
      const ops = main.engine.diff_to(target) as Op[]
      if (ops.length && !main.commit(ops, 'Grundriss importieren')) {
        setError(useProject.getState().lastError ?? 'Übernehmen fehlgeschlagen')
        return
      }
      useEditor.getState().select(null)
      useEditor.getState().requestFit()
      onClose()
    } catch (e) {
      setError(String(e))
    }
  }

  const stepIdx = STEPS.findIndex((s) => s.id === step)

  return (
    <div className="import-wizard">
      <header className="import-head">
        <ol className="steps">
          {STEPS.map((s, i) => (
            <li key={s.id} className={i < stepIdx ? 'done' : i === stepIdx ? 'current' : ''}>
              <span className="num">{i < stepIdx ? '✓' : i + 1}</span>
              <span className="lbl">{s.label}</span>
            </li>
          ))}
        </ol>
        <div className="spacer" />
        {session && <LocalUndo store={session} />}
        <button className="ghost" onClick={onClose} title="Import abbrechen, Projekt bleibt unverändert">
          {session ? 'Verwerfen' : 'Schliessen'}
        </button>
      </header>
      {error && (
        <div className="import-error" onClick={() => setError(null)}>
          {error}
        </div>
      )}

      {step === 'upload' && (
        <UploadStep
          onFile={async (f) => {
            setError(null)
            try {
              const img = await prepareImage(f)
              setImage(img)
              setCorners(null)
              setStep('crop')
            } catch (e) {
              setError(e instanceof Error ? e.message : String(e))
            }
          }}
          onSkip={onClose}
          onFixture={devFixture ? () => onProposal(makeFixtureProposal(project)) : undefined}
        />
      )}
      {step === 'crop' && image && (
        <CropStep
          image={image}
          initial={corners}
          onBack={() => setStep('upload')}
          onDone={(c) => {
            setCorners(c)
            setStep('analyze')
          }}
        />
      )}
      {step === 'analyze' && image && (
        <AnalyzeStep projectId={project.id} image={image} corners={corners} onBack={() => setStep('crop')} onManual={onClose} onDone={onProposal} />
      )}
      {(step === 'correct' || step === 'scale' || step === 'north') && pending && !session && (
        <div className="import-step center">
          <div className="card choice-card">
            <h2>Bestehender Grundriss</h2>
            <p>
              Das Projekt enthält bereits {project.floors[0].walls.length} Wände. Soll der erkannte Grundriss den bestehenden ersetzen oder ergänzen?
            </p>
            <div className="row actions">
              <button onClick={() => startReview(pending, false)}>Ergänzen</button>
              <button className="primary" onClick={() => startReview(pending, true)}>
                Ersetzen
              </button>
            </div>
            <p className="muted small">Möbel und Lampen bleiben erhalten. Beides lässt sich nach dem Übernehmen rückgängig machen.</p>
          </div>
        </div>
      )}
      {(step === 'correct' || step === 'scale' || step === 'north') && session && proposal && (
        <Review store={session} proposal={proposal} step={step} setStep={setStep} north={north} setNorth={setNorth} onApply={apply} />
      )}
    </div>
  )
}

function LocalUndo({ store }: { store: LocalSessionStore }) {
  const undoStack = store((s) => s.undoStack)
  const redoStack = store((s) => s.redoStack)
  return (
    <>
      <button className="ghost" disabled={!undoStack.length} onClick={() => store.getState().undo()} title={undoStack.length ? `Rückgängig: ${undoStack[undoStack.length - 1].label} (⌘Z)` : 'Rückgängig'}>
        ↶
      </button>
      <button className="ghost" disabled={!redoStack.length} onClick={() => store.getState().redo()} title={redoStack.length ? `Wiederholen: ${redoStack[redoStack.length - 1].label}` : 'Wiederholen'}>
        ↷
      </button>
    </>
  )
}

// ------------------------------------------------------------------ step 1

function UploadStep({ onFile, onSkip, onFixture }: { onFile: (f: File) => void; onSkip: () => void; onFixture?: () => void }) {
  const [over, setOver] = useState(false)
  const [busy, setBusy] = useState(false)
  const fileRef = useRef<HTMLInputElement>(null)
  const camRef = useRef<HTMLInputElement>(null)
  const take = async (f: File | undefined | null) => {
    if (!f) return
    setBusy(true)
    try {
      await onFile(f)
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="import-step center">
      <div
        className={`dropzone card${over ? ' over' : ''}`}
        onDragOver={(e) => {
          e.preventDefault()
          setOver(true)
        }}
        onDragLeave={() => setOver(false)}
        onDrop={(e) => {
          e.preventDefault()
          setOver(false)
          void take(e.dataTransfer.files[0])
        }}
      >
        <h2>Grundriss hochladen</h2>
        <p className="muted">JPG, PNG oder PDF – auch ein schiefes Handyfoto. Datei hierher ziehen oder auswählen.</p>
        {busy ? (
          <p>Bereite Bild vor …</p>
        ) : (
          <div className="row actions">
            <button className="primary" onClick={() => fileRef.current?.click()}>
              Datei wählen
            </button>
            <button onClick={() => camRef.current?.click()}>Foto aufnehmen</button>
          </div>
        )}
        <input ref={fileRef} type="file" accept="image/png,image/jpeg,application/pdf,.pdf" hidden onChange={(e) => void take(e.target.files?.[0])} />
        <input ref={camRef} type="file" accept="image/*" capture="environment" hidden onChange={(e) => void take(e.target.files?.[0])} />
        <div className="divider" />
        <button className="link" onClick={onSkip}>
          Ohne Grundriss starten – Räume von Hand zeichnen →
        </button>
        {onFixture && (
          <button className="link small" onClick={onFixture}>
            [Dev] Test-Vorschlag laden (Fixture)
          </button>
        )}
      </div>
    </div>
  )
}

// ------------------------------------------------------------------ steps 4–6

function Review({
  store,
  proposal,
  step,
  setStep,
  north,
  setNorth,
  onApply,
}: {
  store: LocalSessionStore
  proposal: FloorplanProposal
  step: Step
  setStep: (s: Step) => void
  north: number
  setNorth: (n: number) => void
  onApply: () => void
}) {
  const src = useLocalSource(store)
  const [pickPts, setPickPts] = useState<Vec[] | null>(null)
  const [scaleDone, setScaleDone] = useState(false)

  const pick = step === 'scale' && pickPts && pickPts.length < 2 ? { onPick: (p: Vec) => setPickPts([...(pickPts ?? []), p]), hint: `Punkt ${pickPts.length + 1} von 2 im Plan anklicken` } : null

  const world = (view: View) => {
    if (step !== 'scale') return null
    const fs = 12 / view.scale
    const seg = (a: Vec, b: Vec, label: string, cls: string) => (
      <g className={`ref-line ${cls}`} pointerEvents="none">
        <line x1={a[0]} y1={a[1]} x2={b[0]} y2={b[1]} />
        <circle cx={a[0]} cy={a[1]} r={5 / view.scale} />
        <circle cx={b[0]} cy={b[1]} r={5 / view.scale} />
        <text x={(a[0] + b[0]) / 2} y={(a[1] + b[1]) / 2 - 10 / view.scale} fontSize={fs} textAnchor="middle">
          {label}
        </text>
      </g>
    )
    return (
      <>
        {proposal.reference && !scaleDone && !pickPts && seg(proposal.reference.a, proposal.reference.b, `«${proposal.reference.label}» = ${fmtLength(proposal.reference.length_mm)}`, 'suggested')}
        {pickPts && pickPts.length === 1 && (
          <circle className="ref-line picked" cx={pickPts[0][0]} cy={pickPts[0][1]} r={5 / view.scale} pointerEvents="none" />
        )}
        {pickPts && pickPts.length === 2 && seg(pickPts[0], pickPts[1], fmtLength(Math.round(dist(pickPts[0], pickPts[1]))), 'picked')}
      </>
    )
  }

  return (
    <PlanSourceContext.Provider value={src}>
      <div className="import-review">
        <Toolbar />
        <div className="review-plan">
          <PlanCanvas source={src} confidence={step === 'correct' ? proposal.confidence : undefined} pick={pick} world={world} />
        </div>
        <aside className="review-side">
          <div className="review-body">
            {step === 'correct' && <CorrectPanel src={src} proposal={proposal} />}
            {step === 'scale' && (
              <ScalePanel
                src={src}
                proposal={proposal}
                pickPts={pickPts}
                setPickPts={setPickPts}
                scaleDone={scaleDone}
                onScaled={() => setScaleDone(true)}
              />
            )}
            {step === 'north' && (
              <div className="form">
                <h2>Nordpfeil</h2>
                <p className="muted">
                  Drehe die Nadel, bis sie auf dem Plan nach Norden zeigt. {proposal.north_angle_deg !== null ? 'Vorschlag aus dem Plan übernommen.' : 'Im Plan wurde kein Nordpfeil erkannt.'}
                </p>
                <div className="compass-wrap">
                  <span className="muted small">▲ Plan oben</span>
                  <Compass value={north} onChange={setNorth} />
                </div>
                <label>
                  Winkel
                  <span className="input-suffix">
                    <input type="number" min={0} max={359} step={1} value={Math.round(north)} onChange={(e) => setNorth(((parseFloat(e.target.value) || 0) % 360 + 360) % 360)} />
                    <span>°</span>
                  </span>
                </label>
                <p className="muted small">Wird für den Sonnenstand im Ansichtsmodus verwendet.</p>
              </div>
            )}
            <details className="review-props">
              <summary>Eigenschaften der Auswahl</summary>
              <PropertiesPanel />
            </details>
          </div>
          <div className="review-nav">
            {step === 'correct' && (
              <button className="primary" onClick={() => setStep('scale')}>
                Weiter: Massstab
              </button>
            )}
            {step === 'scale' && (
              <>
                <button onClick={() => setStep('correct')}>Zurück</button>
                <button className="primary" onClick={() => setStep('north')}>
                  Weiter: Norden
                </button>
              </>
            )}
            {step === 'north' && (
              <>
                <button onClick={() => setStep('scale')}>Zurück</button>
                <button className="primary" onClick={onApply}>
                  Übernehmen
                </button>
              </>
            )}
          </div>
        </aside>
      </div>
    </PlanSourceContext.Provider>
  )
}

function CorrectPanel({ src, proposal }: { src: PlanSource; proposal: FloorplanProposal }) {
  const [idx, setIdx] = useState(-1)
  const low = useMemo(
    () =>
      Object.entries(proposal.confidence)
        .filter(([id, c]) => c < LOW_CONF && kindOf(src.project, id))
        .sort((a, b) => a[1] - b[1])
        .map(([id, c]) => ({ id, c, kind: kindOf(src.project, id)! })),
    [proposal.confidence, src.project],
  )
  const go = (i: number) => {
    if (!low.length) return
    const n = ((i % low.length) + low.length) % low.length
    setIdx(n)
    const it = low[n]
    useEditor.getState().select({ kind: it.kind, id: it.id })
    useEditor.getState().requestFocus([it.id])
  }
  const f = src.project.floors[0]
  const nameOf = (id: string, kind: string) => {
    if (kind === 'room') return f.rooms.find((r) => r.id === id)?.name ?? 'Raum'
    if (kind === 'opening') {
      const o = f.openings.find((x) => x.id === id)
      return o ? (o.kind.type === 'Door' ? 'Tür' : o.kind.type === 'Window' ? 'Fenster' : 'Durchgang') : 'Öffnung'
    }
    return KIND_NAMES[kind] ?? kind
  }
  return (
    <div className="form">
      <h2>Erkanntes Modell prüfen</h2>
      <p className="muted">
        {f.walls.length} Wände, {f.openings.length} Öffnungen, {f.rooms.length} Räume erkannt
        {proposal.used_llm ? '' : ' (nur Bildverarbeitung)'}. Korrigiere mit den Werkzeugen links; das Original liegt halbtransparent darunter.
      </p>
      {proposal.notes.length > 0 && (
        <ul className="notes">
          {proposal.notes.map((n, i) => (
            <li key={i}>{n}</li>
          ))}
        </ul>
      )}
      <h3>
        Unsicher ({low.length}) <span className="lowconf-swatch" />
      </h3>
      {low.length === 0 ? (
        <p className="muted small">Keine unsicheren Elemente.</p>
      ) : (
        <>
          <div className="row">
            <button onClick={() => go(idx - 1)}>‹ Vorheriges</button>
            <button onClick={() => go(idx + 1)}>Nächstes ›</button>
          </div>
          <ul className="lowconf-list">
            {low.map((it, i) => (
              <li key={it.id}>
                <button className={`link${i === idx ? ' current' : ''}`} onClick={() => go(i)}>
                  {nameOf(it.id, it.kind)} <span className="muted">· {fmtNumber(it.c * 100)} %</span>
                </button>
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  )
}

function ScalePanel({
  src,
  proposal,
  pickPts,
  setPickPts,
  scaleDone,
  onScaled,
}: {
  src: PlanSource
  proposal: FloorplanProposal
  pickPts: Vec[] | null
  setPickPts: (p: Vec[] | null) => void
  scaleDone: boolean
  onScaled: () => void
}) {
  const [len, setLen] = useState('')
  const ref = proposal.reference
  const floor = src.project.floors[0].id
  const applyFactor = (factor: number) => {
    if (!isFinite(factor) || factor <= 0) return
    if (Math.abs(factor - 1) > 1e-4) {
      try {
        src.commit(src.engine.cmd_transform_floor(floor, factor, 0, 0, 0, 0) as Op[], 'Massstab anpassen')
      } catch (e) {
        useEditor.getState().showToast(String(e))
        return
      }
    }
    onScaled()
    setPickPts(null)
    setLen('')
    useEditor.getState().requestFit()
  }
  const measured = pickPts && pickPts.length === 2 ? dist(pickPts[0], pickPts[1]) : null
  const real = parseLength(len)
  return (
    <div className="form">
      <h2>Referenzmass</h2>
      {scaleDone && <p className="ok-text">Massstab angepasst. Du kannst ihn erneut messen oder weitergehen.</p>}
      {ref && !scaleDone && !pickPts && (
        <div className="card inset">
          <p>
            Vorschlag aus der Masslinie «{ref.label}»: {fmtLength(ref.length_mm)}
          </p>
          <p className="muted small">
            Im Plan aktuell {fmtLength(Math.round(dist(ref.a, ref.b)))} → Faktor {fmtNumber(ref.length_mm / dist(ref.a, ref.b), 3)}
          </p>
          <button className="primary" onClick={() => applyFactor(ref.length_mm / dist(ref.a, ref.b))}>
            Bestätigen
          </button>
        </div>
      )}
      {!ref && !pickPts && !scaleDone && <p className="muted">Kein Mass erkannt. Miss eine bekannte Strecke nach.</p>}
      {!pickPts ? (
        <button onClick={() => setPickPts([])}>Zwei Punkte klicken und Länge eingeben</button>
      ) : (
        <div className="card inset form">
          <p className="small">{pickPts.length < 2 ? `Klicke Punkt ${pickPts.length + 1} von 2 im Plan (rastet an Ecken ein).` : `Gemessen: ${fmtLength(Math.round(measured!))}`}</p>
          {pickPts.length === 2 && (
            <label>
              Tatsächliche Länge
              <input
                autoFocus
                value={len}
                placeholder="z. B. 4.20 m"
                onChange={(e) => setLen(e.target.value)}
                onKeyDown={(e) => e.key === 'Enter' && real && measured && applyFactor(real / measured)}
              />
            </label>
          )}
          <div className="row">
            <button onClick={() => setPickPts(null)}>Abbrechen</button>
            {pickPts.length === 2 && (
              <button className="primary" disabled={!real || !measured} onClick={() => real && measured && applyFactor(real / measured)}>
                Anwenden{real && measured ? ` (×${fmtNumber(real / measured, 3)})` : ''}
              </button>
            )}
          </div>
        </div>
      )}
    </div>
  )
}
