// Sidebar «Möbel»: IKEA import, primitives, GLB upload, library with place/delete.
import { useEffect, useState, type ReactNode } from 'react'
import { api, ApiError } from '../core/api'
import { fmtNumber } from '../core/format'
import type { Asset } from '../core/generated/Asset'
import type { PrimitiveShape } from '../core/generated/PrimitiveShape'
import { useAssets } from '../state/assets'
import { useEditor } from '../state/editor'
import { useProject } from '../state/project'

const SHAPES: { shape: PrimitiveShape; label: string; dims: [number, number, number] }[] = [
  { shape: 'Box', label: 'Box', dims: [500, 500, 500] },
  { shape: 'Cylinder', label: 'Zylinder', dims: [400, 400, 600] },
  { shape: 'Plate', label: 'Platte', dims: [1200, 600, 30] },
  { shape: 'Sofa', label: 'Sofa', dims: [2200, 950, 850] },
  { shape: 'Bed', label: 'Bett', dims: [1600, 2050, 900] },
  { shape: 'Table', label: 'Tisch', dims: [1600, 900, 750] },
  { shape: 'Chair', label: 'Stuhl', dims: [450, 520, 850] },
  { shape: 'Shelf', label: 'Regal', dims: [800, 350, 1800] },
  { shape: 'Wardrobe', label: 'Schrank', dims: [1500, 600, 2200] },
]

function errText(e: unknown): string {
  if (e instanceof ApiError) return e.message
  if (e instanceof Error) return e.message
  return String(e)
}

export function fmtDims(d: [number, number, number] | number[]): string {
  return `${fmtNumber(d[0] / 10)} × ${fmtNumber(d[1] / 10)} × ${fmtNumber(d[2] / 10)} cm`
}

export function FurniturePanel() {
  const loaded = useAssets((s) => s.loaded)
  const [loadError, setLoadError] = useState<string | null>(null)
  useEffect(() => {
    useAssets
      .getState()
      .load()
      .then(() => setLoadError(null))
      .catch((e) => setLoadError(errText(e)))
  }, [])
  return (
    <div className="form v3d-furniture">
      <IkeaImport />
      <Primitives />
      <GlbUpload />
      <h3>Bibliothek</h3>
      {loadError && <p className="error v3d-small">Bibliothek nicht geladen: {loadError}</p>}
      {!loaded && !loadError ? <p className="muted v3d-small">Lade …</p> : <Library />}
    </div>
  )
}

function IkeaImport() {
  const [url, setUrl] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<Asset | null>(null)
  const submit = async () => {
    const u = url.trim()
    if (!u) return
    setBusy(true)
    setError(null)
    setResult(null)
    try {
      const a = await api.importIkea(u)
      useAssets.getState().add(a)
      setResult(a)
      setUrl('')
    } catch (e) {
      setError(errText(e))
    } finally {
      setBusy(false)
    }
  }
  return (
    <section>
      <h3>IKEA-Link</h3>
      <form
        className="v3d-inline"
        onSubmit={(e) => {
          e.preventDefault()
          submit()
        }}
      >
        <input type="url" placeholder="https://www.ikea.com/ch/de/p/…" value={url} onChange={(e) => setUrl(e.target.value)} disabled={busy} />
        <button className="primary" type="submit" disabled={busy || !url.trim()}>
          {busy ? 'Lade …' : 'Import'}
        </button>
      </form>
      {busy && <p className="muted v3d-small">Produktseite und 3D-Modell werden geladen …</p>}
      {error && (
        <p className="error v3d-small">
          Import fehlgeschlagen: {error}. Nicht jedes Produkt hat ein 3D-Modell, und IKEA ändert die Seite gelegentlich – als Ersatz das GLB unten hochladen oder eine Basisform mit den Produktmassen anlegen.
        </p>
      )}
      {result && (
        <div className="v3d-result">
          <AssetThumb asset={result} />
          <div>
            <strong>{result.name}</strong>
            <div className="muted v3d-small">{fmtDims(result.dims_mm)}</div>
            {result.ikea?.price != null && (
              <div className="v3d-small">
                {result.ikea.currency ?? 'CHF'} {fmtNumber(result.ikea.price, Number.isInteger(result.ikea.price) ? 0 : 2)}
              </div>
            )}
            {!result.model_url && <div className="v3d-warn v3d-small">Kein 3D-Modell – wird als Basisform angezeigt.</div>}
            {result.warnings.map((w, i) => (
              <div key={i} className="v3d-warn v3d-small">
                {w}
              </div>
            ))}
          </div>
        </div>
      )}
    </section>
  )
}

function DimsInputs({ dims, onChange }: { dims: [number, number, number]; onChange: (d: [number, number, number]) => void }) {
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
              const n = Math.max(1, Math.round(Number(e.target.value) * 10) || 10)
              const next = [...dims] as [number, number, number]
              next[i] = n
              onChange(next)
            }}
          />
        </label>
      ))}
    </div>
  )
}

function Primitives() {
  const [sel, setSel] = useState<PrimitiveShape | null>(null)
  const [dims, setDims] = useState<[number, number, number]>([500, 500, 500])
  const [name, setName] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const pick = (s: (typeof SHAPES)[number]) => {
    setSel(s.shape === sel ? null : s.shape)
    setDims(s.dims)
    setName(s.label)
    setError(null)
  }
  const create = async () => {
    if (!sel) return
    setBusy(true)
    setError(null)
    try {
      const a = await api.createPrimitive(sel, name.trim() || undefined, dims)
      useAssets.getState().add(a)
      setSel(null)
    } catch (e) {
      setError(errText(e))
    } finally {
      setBusy(false)
    }
  }
  return (
    <section>
      <h3>Basisformen</h3>
      <div className="v3d-shapes">
        {SHAPES.map((s) => (
          <button key={s.shape} aria-pressed={sel === s.shape} onClick={() => pick(s)} title={`${s.label} ${fmtDims(s.dims)}`}>
            <ShapeIcon shape={s.shape} />
            <span>{s.label}</span>
          </button>
        ))}
      </div>
      {sel && (
        <div className="form v3d-subform">
          <label className="field">
            Name
            <input value={name} onChange={(e) => setName(e.target.value)} />
          </label>
          <DimsInputs dims={dims} onChange={setDims} />
          <button className="primary" onClick={create} disabled={busy}>
            {busy ? 'Speichere …' : 'Zur Bibliothek'}
          </button>
          {error && <p className="error v3d-small">{error}</p>}
        </div>
      )}
    </section>
  )
}

function GlbUpload() {
  const [open, setOpen] = useState(false)
  const [file, setFile] = useState<File | null>(null)
  const [name, setName] = useState('')
  const [dims, setDims] = useState<[number, number, number]>([1000, 500, 800])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const upload = async () => {
    if (!file) return
    setBusy(true)
    setError(null)
    try {
      const a = await api.uploadGlb(file, name.trim() || file.name.replace(/\.glb$/i, ''), dims)
      useAssets.getState().add(a)
      setFile(null)
      setName('')
      setOpen(false)
    } catch (e) {
      setError(errText(e))
    } finally {
      setBusy(false)
    }
  }
  return (
    <section>
      <h3>
        <button className="link v3d-h3btn" onClick={() => setOpen(!open)}>
          {open ? '▾' : '▸'} Eigenes GLB hochladen
        </button>
      </h3>
      {open && (
        <div className="form v3d-subform">
          <label className="field">
            Datei (.glb)
            <input
              type="file"
              accept=".glb,model/gltf-binary"
              onChange={(e) => {
                const f = e.target.files?.[0] ?? null
                setFile(f)
                if (f && !name) setName(f.name.replace(/\.glb$/i, ''))
              }}
            />
          </label>
          <label className="field">
            Name
            <input value={name} onChange={(e) => setName(e.target.value)} />
          </label>
          <DimsInputs dims={dims} onChange={setDims} />
          <p className="muted v3d-small">Das Modell wird auf diese Masse skaliert (Breite = x, Tiefe = Vorderseite → Rückseite).</p>
          <button className="primary" onClick={upload} disabled={busy || !file}>
            {busy ? 'Lade hoch …' : 'Hochladen'}
          </button>
          {error && <p className="error v3d-small">{error}</p>}
        </div>
      )}
    </section>
  )
}

function Library() {
  const assets = useAssets((s) => s.assets)
  const placing = useEditor((s) => s.placingAsset)
  const furniture = useProject((s) => s.project?.furniture)
  const [confirm, setConfirm] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  if (!assets.length) return <p className="muted v3d-small">Noch keine Möbel. IKEA-Link einfügen oder eine Basisform wählen.</p>
  const del = async (a: Asset) => {
    if (confirm !== a.id) return setConfirm(a.id)
    setConfirm(null)
    try {
      await useAssets.getState().remove(a.id)
      if (useEditor.getState().placingAsset === a.id) useEditor.getState().setPlacingAsset(null)
    } catch (e) {
      setError(errText(e))
    }
  }
  return (
    <>
      {error && <p className="error v3d-small">{error}</p>}
      <ul className="v3d-library">
        {assets.map((a) => {
          const used = furniture?.filter((f) => f.asset === a.id).length ?? 0
          return (
            <li key={a.id} className={placing === a.id ? 'active' : ''}>
              <AssetThumb asset={a} />
              <div className="v3d-lib-info">
                <span className="v3d-lib-name" title={a.name}>
                  {a.name}
                </span>
                <span className="muted v3d-small">{fmtDims(a.dims_mm)}</span>
                <span className="v3d-badges">
                  {a.kind === 'Ikea' && <span className="v3d-badge">IKEA</span>}
                  {a.light && <span className="v3d-badge">Licht</span>}
                  {a.warnings.length > 0 && (
                    <span className="v3d-badge v3d-badge-warn" title={a.warnings.join('\n')}>
                      ⚠ {a.warnings.length}
                    </span>
                  )}
                  {used > 0 && <span className="muted v3d-small">{used}× im Plan</span>}
                </span>
                <span className="v3d-lib-actions">
                  <button
                    className={placing === a.id ? 'primary' : ''}
                    onClick={() => useEditor.getState().setPlacingAsset(placing === a.id ? null : a.id)}
                    title="Im Grundriss oder in der 3D-Vorschau auf den Boden klicken"
                  >
                    {placing === a.id ? 'Abbrechen' : 'Platzieren'}
                  </button>
                  <button className="ghost danger" onClick={() => del(a)} onBlur={() => confirm === a.id && setConfirm(null)} title={used ? `Wird ${used}× verwendet – platzierte Möbel werden als Box angezeigt` : 'Aus der Bibliothek löschen'}>
                    {confirm === a.id ? 'Wirklich?' : 'Löschen'}
                  </button>
                </span>
              </div>
            </li>
          )
        })}
      </ul>
    </>
  )
}

function AssetThumb({ asset }: { asset: Asset }) {
  const [broken, setBroken] = useState(false)
  const src = asset.thumbnail_url ?? asset.ikea?.image_url ?? null
  if (src && !broken) return <img className="v3d-thumb" src={src} alt="" loading="lazy" onError={() => setBroken(true)} />
  return (
    <span className="v3d-thumb v3d-thumb-icon">
      <ShapeIcon shape={asset.primitive ?? 'Box'} model={!!asset.model_url} />
    </span>
  )
}

/** Small line icons for the primitive shapes (isometric-ish). */
export function ShapeIcon({ shape, model = false }: { shape: PrimitiveShape; model?: boolean }) {
  const p: Record<PrimitiveShape, ReactNode> = {
    Box: <path d="M6 9 L16 5 L26 9 L26 21 L16 25 L6 21 Z M6 9 L16 13 L26 9 M16 13 L16 25" />,
    Cylinder: (
      <>
        <ellipse cx="16" cy="8" rx="8" ry="3" />
        <path d="M8 8 V23 A8 3 0 0 0 24 23 V8" />
      </>
    ),
    Plate: <path d="M3 14 L17 9 L29 13 L15 18 Z M3 14 V16 L15 20 L29 15 V13 M15 18 V20" />,
    Sofa: <path d="M4 13 V22 H28 V13 M4 13 H8 V18 H24 V13 H28 M8 18 V11 Q16 8 24 11 V18 M6 22 V25 M26 22 V25" />,
    Bed: <path d="M4 10 V25 M4 18 H28 V25 M4 15 H28 V18 M8 15 V12 H14 V15 M28 15 V13" />,
    Table: <path d="M3 12 H29 V14 H3 Z M6 14 V25 M26 14 V25 M10 14 V22 M22 14 V22" />,
    Chair: <path d="M10 4 V17 H22 V25 M10 17 V25 M10 17 H22 M10 4 H13 V17" />,
    Shelf: <path d="M8 4 H24 V27 H8 Z M8 10 H24 M8 16 H24 M8 22 H24" />,
    Wardrobe: <path d="M6 4 H26 V27 H6 Z M16 4 V27 M13 14 V17 M19 14 V17" />,
  }
  return (
    <svg viewBox="0 0 32 32" className="v3d-icon" aria-hidden>
      {model ? (
        <>
          <path d="M6 9 L16 5 L26 9 L26 21 L16 25 L6 21 Z M6 9 L16 13 L26 9 M16 13 L16 25" />
          <text x="16" y="31" textAnchor="middle" fontSize="7" stroke="none" fill="currentColor">
            3D
          </text>
        </>
      ) : (
        p[shape]
      )}
    </svg>
  )
}
