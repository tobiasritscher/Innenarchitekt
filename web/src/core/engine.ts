// WASM loader for plan-core.
import init, { PlanEngine, new_id, primitive, sun } from './wasm/plan_wasm.js'

let ready: Promise<void> | null = null

export function loadWasm(): Promise<void> {
  if (!ready) ready = init().then(() => undefined)
  return ready
}

export { PlanEngine, new_id as newId, primitive as primitiveMesh, sun as sunPosition }
