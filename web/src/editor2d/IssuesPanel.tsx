// Non-blocking validation hints (bottom of the sidebar). Click = select + zoom to the element.
import { useState } from 'react'
import { useEditor } from '../state/editor'
import { kindOf } from './model'
import { useIssuesOf, usePlanSource } from './source'

export function IssuesPanel() {
  const source = usePlanSource()
  const issues = useIssuesOf(source)
  const [open, setOpen] = useState(false)
  const warnings = issues.filter((i) => i.severity === 'Warning').length
  return (
    <section className={`issues${open ? ' open' : ''}`}>
      <button className="issues-head" onClick={() => setOpen(!open)} aria-expanded={open}>
        <span>Hinweise</span>
        {issues.length > 0 ? <span className={`badge${warnings ? ' warn' : ''}`}>{issues.length}</span> : <span className="muted small">keine</span>}
        <span className="chev">{open ? '▾' : '▸'}</span>
      </button>
      {open && (
        <ul className="issues-list">
          {issues.length === 0 && <li className="muted small">Alles in Ordnung.</li>}
          {issues.map((iss, i) => (
            <li key={`${iss.code}-${i}`}>
              <button
                className={`issue ${iss.severity === 'Warning' ? 'warn' : 'info'}`}
                onClick={() => {
                  if (!source) return
                  const ed = useEditor.getState()
                  const id = iss.ids.find((x) => kindOf(source.project, x))
                  const k = id ? kindOf(source.project, id) : null
                  if (id && k) ed.select({ kind: k, id })
                  if (iss.ids.length) ed.requestFocus(iss.ids)
                }}
              >
                <span className="dot" />
                {iss.message}
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}
