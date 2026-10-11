// 2D plan of the main project (modelling mode).
import { PlanCanvas } from './PlanCanvas'
import { usePlanSource } from './source'

export function PlanView() {
  const source = usePlanSource()
  if (!source) return <div className="placeholder">Lade Plan …</div>
  return <PlanCanvas source={source} />
}
