import { useEffect, useState } from 'react'
import { api, ApiError } from '../core/api'
import { loadWasm } from '../core/engine'
import { useProject } from '../state/project'
import { Editor } from './Editor'
import { ProjectList } from './ProjectList'

type AuthState = 'checking' | 'ok' | 'login'

export function App() {
  const [auth, setAuth] = useState<AuthState>('checking')
  const [wasm, setWasm] = useState<'loading' | 'ok' | string>('loading')
  const [projectId, setProjectId] = useState<string | null>(() => new URLSearchParams(location.search).get('p'))

  useEffect(() => {
    loadWasm().then(
      () => setWasm('ok'),
      (e) => setWasm(String(e)),
    )
    api.checkAuth().then(
      () => setAuth('ok'),
      () => setAuth('login'),
    )
    const onUnauth = () => setAuth('login')
    window.addEventListener('ia:unauthorized', onUnauth)
    const onPop = () => setProjectId(new URLSearchParams(location.search).get('p'))
    window.addEventListener('popstate', onPop)
    return () => {
      window.removeEventListener('ia:unauthorized', onUnauth)
      window.removeEventListener('popstate', onPop)
    }
  }, [])

  const openProject = (id: string | null) => {
    const url = id ? `?p=${id}` : location.pathname
    history.pushState(null, '', url)
    setProjectId(id)
    if (!id) useProject.getState().close()
  }

  if (auth === 'checking' || wasm === 'loading') return <div className="splash">Lade …</div>
  if (wasm !== 'ok') return <div className="splash error">WebAssembly konnte nicht geladen werden: {wasm}</div>
  if (auth === 'login') return <Login onOk={() => setAuth('ok')} />
  if (projectId) return <Editor projectId={projectId} onExit={() => openProject(null)} />
  return <ProjectList onOpen={openProject} />
}

function Login({ onOk }: { onOk: () => void }) {
  const [token, setToken] = useState('')
  const [error, setError] = useState<string | null>(null)
  return (
    <div className="splash">
      <form
        className="card login"
        onSubmit={async (e) => {
          e.preventDefault()
          try {
            await api.login(token)
            onOk()
          } catch (err) {
            setError(err instanceof ApiError && err.status === 401 ? 'Token ungültig' : String(err))
          }
        }}
      >
        <h1>Innenarchitekt</h1>
        <label>
          Zugangs-Token
          <input type="password" autoFocus value={token} onChange={(e) => setToken(e.target.value)} />
        </label>
        {error && <p className="error">{error}</p>}
        <button className="primary" type="submit">
          Anmelden
        </button>
      </form>
    </div>
  )
}
