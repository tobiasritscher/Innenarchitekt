import { useEffect, useState } from 'react'
import { IssuesPanel } from '../editor2d/IssuesPanel'
import { PlanView } from '../editor2d/PlanView'
import { PropertiesPanel } from '../editor2d/PropertiesPanel'
import { Toolbar } from '../editor2d/Toolbar'
import { ImportWizard } from '../import/ImportWizard'
import { useAssets } from '../state/assets'
import { useEditor } from '../state/editor'
import { useProject } from '../state/project'
import { FurniturePanel } from '../view3d/FurniturePanel'
import { Preview3D } from '../view3d/Preview3D'
import { RoomMode } from '../view3d/RoomMode'
import { ViewMode } from '../view3d/ViewMode'
import { AiPanel } from './AiPanel'
import { ProjectSettings } from './ProjectSettings'

const SYNC_LABEL = { saved: 'Gespeichert', pending: 'Ungespeichert', saving: 'Speichere …', error: 'Offline – wird wiederholt', conflict: 'Neu geladen' }

export function Editor({ projectId, onExit }: { projectId: string; onExit: () => void }) {
  const project = useProject((s) => s.project)
  const open = useProject((s) => s.open)
  const sync = useProject((s) => s.sync)
  const syncError = useProject((s) => s.syncError)
  const lastError = useProject((s) => s.lastError)
  const clearError = useProject((s) => s.clearError)
  const undoStack = useProject((s) => s.undoStack)
  const redoStack = useProject((s) => s.redoStack)
  const { undo, redo } = useProject.getState()
  const mode = useEditor((s) => s.mode)
  const setMode = useEditor((s) => s.setMode)
  const narrowPane = useEditor((s) => s.narrowPane)
  const setNarrowPane = useEditor((s) => s.setNarrowPane)
  const toast = useEditor((s) => s.toast)
  const showToast = useEditor((s) => s.showToast)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [sideTab, setSideTab] = useState<'props' | 'furniture' | 'ai' | 'project'>('props')

  useEffect(() => {
    setLoadError(null)
    open(projectId).catch((e) => setLoadError(String(e)))
    useAssets.getState().load().catch(() => {})
  }, [projectId, open])

  // New, empty project → offer import first.
  useEffect(() => {
    if (project && project.version === 0 && project.floors[0].walls.length === 0) setMode('import')
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project?.id])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // The import correction view has its own history.
      if (useEditor.getState().mode === 'import') return
      const t = e.target as HTMLElement
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT')) return
      const mod = e.metaKey || e.ctrlKey
      if (mod && e.key.toLowerCase() === 'z') {
        e.preventDefault()
        if (e.shiftKey) redo()
        else undo()
      } else if (mod && e.key.toLowerCase() === 'y') {
        e.preventDefault()
        redo()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [undo, redo])

  useEffect(() => {
    if (!toast) return
    const t = setTimeout(() => showToast(null), 4000)
    return () => clearTimeout(t)
  }, [toast, showToast])

  useEffect(() => {
    if (!lastError) return
    const t = setTimeout(clearError, 5000)
    return () => clearTimeout(t)
  }, [lastError, clearError])

  if (loadError) return <div className="splash error">Projekt konnte nicht geladen werden: {loadError}</div>
  if (!project) return <div className="splash">Lade Projekt …</div>

  return (
    <div className={`editor mode-${mode} pane-${narrowPane}`}>
      <header className="topbar">
        <button className="ghost" onClick={onExit} title="Zur Projektliste">
          ←
        </button>
        <strong className="project-name">{project.name}</strong>
        <nav className="mode-switch" role="tablist">
          <button role="tab" aria-selected={mode === 'model'} onClick={() => setMode('model')}>
            Modellieren
          </button>
          <button role="tab" aria-selected={mode === 'room'} onClick={() => setMode('room')}>
            Einrichten
          </button>
          <button role="tab" aria-selected={mode === 'view'} onClick={() => setMode('view')}>
            Begehen
          </button>
          <button role="tab" aria-selected={mode === 'import'} onClick={() => setMode('import')}>
            Grundriss-Import
          </button>
        </nav>
        <div className="spacer" />
        {mode !== 'import' && (<>
        <button className="ghost" onClick={undo} disabled={!undoStack.length} title={undoStack.length ? `Rückgängig: ${undoStack[undoStack.length - 1].label} (⌘Z)` : 'Rückgängig'}>
          ↶
        </button>
        <button className="ghost" onClick={redo} disabled={!redoStack.length} title={redoStack.length ? `Wiederholen: ${redoStack[redoStack.length - 1].label} (⇧⌘Z)` : 'Wiederholen'}>
          ↷
        </button>
        </>)}
        <span className={`sync sync-${sync}`} title={syncError ?? undefined}>
          {SYNC_LABEL[sync]}
        </span>
      </header>

      {mode === 'import' && <ImportWizard onClose={() => setMode('model')} />}
      {mode === 'room' && <RoomMode />}
      {mode === 'view' && <ViewMode />}
      {mode === 'model' && (
        <div className="workspace">
          <Toolbar />
          <div className="panes">
            <div className="pane-switch">
              <button aria-pressed={narrowPane === '2d'} onClick={() => setNarrowPane('2d')}>
                2D
              </button>
              <button aria-pressed={narrowPane === '3d'} onClick={() => setNarrowPane('3d')}>
                3D
              </button>
            </div>
            <section className="pane pane-plan">
              <PlanView />
            </section>
            <section className="pane pane-3d">
              <Preview3D />
            </section>
          </div>
          <aside className="sidebar">
            <nav className="tabs">
              <button aria-selected={sideTab === 'props'} onClick={() => setSideTab('props')}>
                Eigenschaften
              </button>
              <button aria-selected={sideTab === 'furniture'} onClick={() => setSideTab('furniture')}>
                Möbel
              </button>
              <button aria-selected={sideTab === 'ai'} onClick={() => setSideTab('ai')}>
                KI
              </button>
              <button aria-selected={sideTab === 'project'} onClick={() => setSideTab('project')}>
                Projekt
              </button>
            </nav>
            <div className="sidebar-body">
              {sideTab === 'props' && <PropertiesPanel />}
              {sideTab === 'furniture' && <FurniturePanel />}
              {sideTab === 'ai' && <AiPanel />}
              {sideTab === 'project' && <ProjectSettings />}
            </div>
            <IssuesPanel />
          </aside>
        </div>
      )}
      {(toast || lastError) && <div className={`toast ${lastError ? 'toast-error' : ''}`}>{lastError ?? toast}</div>}
    </div>
  )
}
