// Sidebar «KI»: questions and commands in natural language, material suggestions from photos.
// The answer's changes are shown as a live preview (transient ops) and only applied on «Übernehmen».
import { useEffect, useRef, useState } from 'react'
import { api, type AiResponse } from '../core/api'
import { MATERIALS } from '../core/materials'
import type { Op } from '../core/generated/Op'
import { useEditor, type Selection } from '../state/editor'
import { useProject } from '../state/project'

type Msg =
  | { role: 'user'; text: string; photos: number }
  | { role: 'ai'; res: AiResponse; status: 'preview' | 'applied' | 'discarded' | 'info' }
  | { role: 'error'; text: string }

const EXAMPLES = [
  'Wie gross ist das Wohnzimmer?',
  'Mach das Schlafzimmer 50 cm breiter',
  'Streich die Wände im Bad salbeigrün',
  'Passt ein 2.20 m Sofa an die längste Wand im Wohnzimmer?',
]

const PHOTO_MAX_PX = 1568

async function shrinkPhoto(file: File): Promise<Blob> {
  const bmp = await createImageBitmap(file)
  const s = Math.min(1, PHOTO_MAX_PX / Math.max(bmp.width, bmp.height))
  const c = document.createElement('canvas')
  c.width = Math.round(bmp.width * s)
  c.height = Math.round(bmp.height * s)
  c.getContext('2d')!.drawImage(bmp, 0, 0, c.width, c.height)
  bmp.close()
  return new Promise((res, rej) => c.toBlob((b) => (b ? res(b) : rej(new Error('Foto konnte nicht gelesen werden'))), 'image/jpeg', 0.85))
}

function discardPreview(msgs: Msg[]): Msg[] {
  const st = useProject.getState()
  if (msgs.some((m) => m.role === 'ai' && m.status === 'preview')) st.endTransient(false)
  return msgs.map((m) => (m.role === 'ai' && m.status === 'preview' ? { ...m, status: 'discarded' } : m))
}

export function AiPanel() {
  const project = useProject((s) => s.project)
  const [enabled, setEnabled] = useState<boolean | null>(null)
  const [msgs, setMsgs] = useState<Msg[]>([])
  const [text, setText] = useState('')
  const [photos, setPhotos] = useState<{ blob: Blob; url: string }[]>([])
  const [busy, setBusy] = useState(false)
  const fileRef = useRef<HTMLInputElement>(null)
  const endRef = useRef<HTMLDivElement>(null)
  const msgsRef = useRef(msgs)
  msgsRef.current = msgs

  useEffect(() => {
    api.config().then((c) => setEnabled(c.llm_enabled), () => setEnabled(false))
  }, [])
  // leaving the tab discards an open preview
  useEffect(() => () => void discardPreview(msgsRef.current), [])
  useEffect(() => endRef.current?.scrollIntoView({ block: 'end' }), [msgs, busy])

  const send = async (q = text) => {
    if (!project || busy || (!q.trim() && photos.length === 0)) return
    const prev = discardPreview(msgs)
    const history = prev.flatMap((m) =>
      m.role === 'user' ? [{ role: 'user', text: m.text }] : m.role === 'ai' ? [{ role: 'assistant', text: `${m.res.answer}${m.res.changes.length ? ` [Vorschlag ${m.status === 'applied' ? 'übernommen' : 'verworfen'}: ${m.res.changes.join('; ')}]` : ''}` }] : [],
    )
    setMsgs([...prev, { role: 'user', text: q.trim(), photos: photos.length }])
    setText('')
    setBusy(true)
    const fd = new FormData()
    fd.append('text', q.trim())
    fd.append('project', JSON.stringify(useProject.getState().project))
    fd.append('materials', JSON.stringify(MATERIALS.map((m) => [m.id, `${m.name} (${m.group})`])))
    fd.append('history', JSON.stringify(history))
    photos.forEach((p, i) => fd.append('photo', p.blob, `foto${i + 1}.jpg`))
    photos.forEach((p) => URL.revokeObjectURL(p.url))
    setPhotos([])
    try {
      const res = await api.ai(project.id, fd)
      let status: 'preview' | 'info' = 'info'
      if (res.ops.length) {
        if (useProject.getState().setTransient(res.ops as Op[])) status = 'preview'
        else res.warnings = [...res.warnings, 'Vorschlag liess sich nicht auf den aktuellen Stand anwenden.']
      }
      setMsgs((m) => [...m, { role: 'ai', res, status }])
    } catch (e) {
      setMsgs((m) => [...m, { role: 'error', text: e instanceof Error ? e.message : String(e) }])
    } finally {
      setBusy(false)
    }
  }

  const accept = (i: number) => {
    const m = msgs[i]
    if (m.role !== 'ai') return
    const st = useProject.getState()
    // re-apply from scratch: robust even if a drag replaced the preview in the meantime
    st.endTransient(false)
    const ok = st.commit(m.res.ops as Op[], `KI: ${m.res.changes[0] ?? 'Vorschlag'}${m.res.changes.length > 1 ? ` (+${m.res.changes.length - 1})` : ''}`)
    setMsgs(msgs.map((x, j) => (j === i && x.role === 'ai' ? { ...x, status: ok ? 'applied' : 'discarded' } : x)))
  }
  const discard = () => setMsgs(discardPreview(msgs))

  const addPhotos = async (files: FileList | null) => {
    if (!files) return
    const add: { blob: Blob; url: string }[] = []
    for (const f of Array.from(files).slice(0, 6 - photos.length)) {
      try {
        const blob = await shrinkPhoto(f)
        add.push({ blob, url: URL.createObjectURL(blob) })
      } catch (e) {
        useEditor.getState().showToast(String(e))
      }
    }
    setPhotos((p) => [...p, ...add])
  }

  if (enabled === false) return <p className="muted">KI ist auf dem Server nicht konfiguriert (LLM_API_KEY fehlt).</p>

  return (
    <div className="ai-panel">
      <div className="ai-log">
        {msgs.length === 0 && (
          <div className="ai-intro">
            <p className="muted small">Frag etwas zum Grundriss oder gib einen Befehl. Änderungen erscheinen als Vorschlag – übernommen wird erst auf Klick. Mit Fotos der Wohnung schlägt die KI Materialien vor.</p>
            {EXAMPLES.map((e) => (
              <button key={e} className="ai-example" onClick={() => void send(e)} disabled={busy}>
                {e}
              </button>
            ))}
          </div>
        )}
        {msgs.map((m, i) =>
          m.role === 'user' ? (
            <div key={i} className="ai-msg ai-user">
              {m.text || <em>(nur Fotos)</em>}
              {m.photos > 0 && <span className="muted small"> · {m.photos} Foto(s)</span>}
            </div>
          ) : m.role === 'error' ? (
            <div key={i} className="ai-msg ai-error">
              {m.text}
            </div>
          ) : (
            <div key={i} className="ai-msg ai-answer">
              <div className="ai-text">{m.res.answer}</div>
              {m.res.refs.length > 0 && (
                <div className="ai-refs">
                  {m.res.refs.map((r, k) => (
                    <button key={k} className="ai-ref" onClick={() => useEditor.getState().select(r as NonNullable<Selection>)}>
                      {REF_NAMES[r.kind] ?? r.kind} {k + 1}
                    </button>
                  ))}
                </div>
              )}
              {m.res.changes.length > 0 && (
                <ul className="ai-changes">
                  {m.res.changes.map((c, k) => (
                    <li key={k}>{c}</li>
                  ))}
                </ul>
              )}
              {m.res.warnings.length > 0 && (
                <ul className="ai-warnings">
                  {m.res.warnings.map((c, k) => (
                    <li key={k}>{c}</li>
                  ))}
                </ul>
              )}
              {m.status === 'preview' && (
                <div className="row">
                  <button className="primary" onClick={() => accept(i)}>
                    Übernehmen
                  </button>
                  <button onClick={discard}>Verwerfen</button>
                </div>
              )}
              {m.status === 'applied' && <p className="muted small">✓ Übernommen (rückgängig mit ⌘Z)</p>}
              {m.status === 'discarded' && m.res.ops.length > 0 && <p className="muted small">Verworfen</p>}
            </div>
          ),
        )}
        {busy && <div className="ai-msg ai-busy muted">KI denkt nach … (ca. 10–40 s)</div>}
        <div ref={endRef} />
      </div>
      {photos.length > 0 && (
        <div className="ai-photos">
          {photos.map((p, i) => (
            <button
              key={p.url}
              className="ai-photo"
              title="Entfernen"
              onClick={() => {
                URL.revokeObjectURL(p.url)
                setPhotos(photos.filter((_, j) => j !== i))
              }}
            >
              <img src={p.url} alt="" />
            </button>
          ))}
        </div>
      )}
      <div className="ai-input">
        <textarea
          rows={3}
          value={text}
          placeholder={photos.length ? 'Optional: welcher Raum ist auf den Fotos?' : 'Frage oder Befehl … (Enter = senden)'}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey) {
              e.preventDefault()
              void send()
            }
          }}
          disabled={busy}
        />
        <div className="row">
          <button onClick={() => fileRef.current?.click()} disabled={busy || photos.length >= 6} title="Fotos der Wohnung für Material-Vorschläge">
            Fotos …
          </button>
          <button className="primary" onClick={() => void send()} disabled={busy || (!text.trim() && photos.length === 0)}>
            Senden
          </button>
        </div>
        <input
          ref={fileRef}
          type="file"
          accept="image/*"
          multiple
          hidden
          onChange={(e) => {
            void addPhotos(e.target.files)
            e.target.value = ''
          }}
        />
      </div>
    </div>
  )
}

const REF_NAMES: Record<string, string> = { wall: 'Wand', opening: 'Öffnung', room: 'Raum', furniture: 'Möbel', light: 'Lampe', node: 'Ecke' }
