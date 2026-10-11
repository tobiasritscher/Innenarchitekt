// Small shared form controls. Values commit on Enter/blur, not on every keystroke,
// so each edit becomes exactly one undoable batch.
import { useEffect, useState, type ReactNode } from 'react'
import { MATERIALS, resolveMaterial } from '../core/materials'
import { fmtLength, parseLength } from '../core/format'

export function NumberField({ label, value, onCommit, step = 1, min, max, suffix }: { label: ReactNode; value: number; onCommit: (v: number) => void; step?: number; min?: number; max?: number; suffix?: string }) {
  const [text, setText] = useState(String(value))
  useEffect(() => setText(String(value)), [value])
  const commit = () => {
    const v = parseFloat(text.replace(',', '.').replace(/'/g, ''))
    if (isNaN(v)) return setText(String(value))
    let c = v
    if (min !== undefined) c = Math.max(min, c)
    if (max !== undefined) c = Math.min(max, c)
    if (c !== value) onCommit(c)
    else setText(String(value))
  }
  return (
    <label className="field">
      {label}
      <span className="input-suffix">
        <input
          type="number"
          step={step}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === 'Enter') (e.target as HTMLInputElement).blur()
            if (e.key === 'Escape') setText(String(value))
          }}
        />
        {suffix && <span>{suffix}</span>}
      </span>
    </label>
  )
}

/** Length input accepting "3.45 m", "345 cm", "3450". */
export function LengthField({ label, valueMm, onCommit }: { label: ReactNode; valueMm: number; onCommit: (mm: number) => void }) {
  const [text, setText] = useState(fmtLength(valueMm))
  useEffect(() => setText(fmtLength(valueMm)), [valueMm])
  const commit = () => {
    const mm = parseLength(text)
    if (mm === null || mm <= 0) return setText(fmtLength(valueMm))
    if (mm !== Math.round(valueMm)) onCommit(mm)
    else setText(fmtLength(valueMm))
  }
  return (
    <label className="field">
      {label}
      <input
        value={text}
        onChange={(e) => setText(e.target.value)}
        onFocus={(e) => e.target.select()}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === 'Enter') (e.target as HTMLInputElement).blur()
          if (e.key === 'Escape') setText(fmtLength(valueMm))
        }}
      />
    </label>
  )
}

export function MaterialPicker({ label, value, onChange }: { label: ReactNode; value: string; onChange: (ref: string) => void }) {
  const current = resolveMaterial(value)
  const groups = Array.from(new Set(MATERIALS.map((m) => m.group)))
  return (
    <label className="field">
      {label}
      <span className="material-picker">
        <span className="swatch" style={{ background: current.color }} />
        <select value={value.startsWith('#') ? '#custom' : value} onChange={(e) => e.target.value !== '#custom' && onChange(e.target.value)}>
          {groups.map((g) => (
            <optgroup key={g} label={g}>
              {MATERIALS.filter((m) => m.group === g).map((m) => (
                <option key={m.id} value={m.id}>
                  {m.name}
                </option>
              ))}
            </optgroup>
          ))}
          <option value="#custom">Eigene Farbe …</option>
        </select>
        <input type="color" value={current.color} onChange={(e) => onChange(e.target.value)} title="Eigene Farbe" />
      </span>
    </label>
  )
}
