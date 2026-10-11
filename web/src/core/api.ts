// Typed REST client for the backend (see docs/api.md).
import type { Asset } from './generated/Asset'
import type { FloorplanProposal } from './generated/FloorplanProposal'
import type { GeoLocation } from './generated/GeoLocation'
import type { Op } from './generated/Op'
import type { PrimitiveShape } from './generated/PrimitiveShape'
import type { Project } from './generated/Project'

export class ApiError extends Error {
  status: number
  body: any
  constructor(status: number, body: any) {
    super(body?.error ?? `HTTP ${status}`)
    this.status = status
    this.body = body
  }
}

async function req<T>(method: string, path: string, body?: unknown): Promise<T> {
  const init: RequestInit = { method, credentials: 'same-origin', headers: {} }
  if (body instanceof FormData) {
    init.body = body
  } else if (body !== undefined) {
    init.body = JSON.stringify(body)
    ;(init.headers as Record<string, string>)['Content-Type'] = 'application/json'
  }
  const res = await fetch(`/api${path}`, init)
  if (res.status === 204) return undefined as T
  const text = await res.text()
  let data: any = null
  try {
    data = text ? JSON.parse(text) : null
  } catch {
    data = { error: text }
  }
  if (!res.ok) {
    if (res.status === 401) window.dispatchEvent(new Event('ia:unauthorized'))
    throw new ApiError(res.status, data)
  }
  return data as T
}

export type ProjectSummary = { id: string; name: string; version: number; updated_at: string; wall_count: number }
export type JobStatus = 'queued' | 'running' | 'done' | 'failed'
export type Job = {
  id: string
  kind: 'floorplan'
  status: JobStatus
  progress: number
  message: string
  result: FloorplanProposal | null
  error: string | null
}
export type AppConfig = { llm_enabled: boolean; llm_model: string; photo3d_enabled?: boolean }

export const api = {
  login: (token: string) => req<void>('POST', '/auth', { token }),
  checkAuth: () => req<{ ok: boolean }>('GET', '/auth'),
  config: () => req<AppConfig>('GET', '/config'),
  /** KI-Assistent: multipart text, project, materials, history, photo* → answer + proposed ops. */
  ai: (projectId: string, fd: FormData) => req<AiResponse>('POST', `/projects/${projectId}/ai`, fd),

  listProjects: () => req<ProjectSummary[]>('GET', '/projects'),
  createProject: (name: string, address?: string) => req<Project>('POST', '/projects', { name, address: address || null }),
  getProject: (id: string) => req<Project>('GET', `/projects/${id}`),
  deleteProject: (id: string) => req<void>('DELETE', `/projects/${id}`),
  pushOps: (id: string, base_version: number, ops: Op[]) =>
    req<{ version: number }>('POST', `/projects/${id}/ops`, { base_version, ops }),
  geocode: (address: string) => req<GeoLocation>('POST', '/geocode', { address }),

  uploadFloorplan: (projectId: string, file: Blob, corners?: [number, number][], paperMmPerPx?: number) => {
    const fd = new FormData()
    fd.append('file', file, 'grundriss.png')
    if (corners) fd.append('corners', JSON.stringify(corners))
    if (paperMmPerPx) fd.append('paper_mm_per_px', String(paperMmPerPx))
    return req<{ job_id: string }>('POST', `/projects/${projectId}/floorplan`, fd)
  },
  getJob: (id: string) => req<Job>('GET', `/jobs/${id}`),
  /** Subscribe to job progress via SSE. Returns an unsubscribe function. */
  watchJob: (id: string, onJob: (job: Job) => void, onError?: (e: unknown) => void) => {
    const es = new EventSource(`/api/jobs/${id}/events`)
    es.addEventListener('job', (ev) => {
      const job = JSON.parse((ev as MessageEvent).data) as Job
      onJob(job)
      if (job.status === 'done' || job.status === 'failed') es.close()
    })
    es.onerror = (e) => {
      es.close()
      onError?.(e)
    }
    return () => es.close()
  },

  listAssets: () => req<Asset[]>('GET', '/assets'),
  deleteAsset: (id: string) => req<void>('DELETE', `/assets/${id}`),
  importIkea: (url: string) => req<Asset>('POST', '/assets/ikea', { url }),
  /** «Möbel aus Foto»: starts a job (TRELLIS.2, several minutes); the job result is the new asset. */
  fromPhoto: (photo: Blob, name: string, dims: [number, number, number]) => {
    const fd = new FormData()
    fd.append('photo', photo, 'foto.jpg')
    fd.append('name', name)
    fd.append('w_mm', String(dims[0]))
    fd.append('d_mm', String(dims[1]))
    fd.append('h_mm', String(dims[2]))
    return req<{ job_id: string }>('POST', '/assets/from-photo', fd)
  },
  uploadGlb: (file: File, name: string, dims: [number, number, number]) => {
    const fd = new FormData()
    fd.append('file', file)
    fd.append('name', name)
    fd.append('w_mm', String(dims[0]))
    fd.append('d_mm', String(dims[1]))
    fd.append('h_mm', String(dims[2]))
    return req<Asset>('POST', '/assets/upload', fd)
  },
  createPrimitive: (shape: PrimitiveShape, name?: string, dims_mm?: [number, number, number]) =>
    req<Asset>('POST', '/assets/primitive', { shape, name: name ?? null, dims_mm: dims_mm ?? null }),
}

export type AiResponse = {
  answer: string
  ops: Op[]
  changes: string[]
  warnings: string[]
  refs: { kind: 'node' | 'wall' | 'opening' | 'room' | 'furniture' | 'light'; id: string; label?: string }[]
}
