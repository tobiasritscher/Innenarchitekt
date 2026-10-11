// Swiss formatting: apostrophe thousands separator, dd.mm.yyyy dates.

export function fmtNumber(n: number, digits = 0): string {
  const s = n.toFixed(digits)
  const [int, frac] = s.split('.')
  const neg = int.startsWith('-')
  const body = (neg ? int.slice(1) : int).replace(/\B(?=(\d{3})+(?!\d))/g, "'")
  return (neg ? '-' : '') + body + (frac ? '.' + frac : '')
}

/** 3450 → "3.45 m"; below 1 m → "85 cm". */
export function fmtLength(mm: number): string {
  if (Math.abs(mm) < 1000) return `${fmtNumber(mm / 10, mm % 10 === 0 ? 0 : 1)} cm`
  return `${fmtNumber(mm / 1000, 2)} m`
}

export function fmtArea(m2: number): string {
  return `${fmtNumber(m2, 1)} m²`
}

export function fmtDate(iso: string): string {
  const d = new Date(iso)
  if (isNaN(d.getTime())) return iso
  return `${String(d.getDate()).padStart(2, '0')}.${String(d.getMonth() + 1).padStart(2, '0')}.${d.getFullYear()}`
}

export function fmtDateTime(iso: string): string {
  const d = new Date(iso)
  if (isNaN(d.getTime())) return iso
  return `${fmtDate(iso)} ${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
}

export function fmtTime(minutes: number): string {
  const h = Math.floor(minutes / 60)
  const m = Math.round(minutes % 60)
  return `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}`
}

/** Parse user length input in mm; accepts "3450", "3.45m", "345cm", "3,45 m". */
export function parseLength(input: string): number | null {
  const s = input.trim().toLowerCase().replace(',', '.').replace(/'/g, '')
  const m = s.match(/^(-?\d+(?:\.\d+)?)\s*(mm|cm|m)?$/)
  if (!m) return null
  const v = parseFloat(m[1])
  const unit = m[2] ?? (s.includes('.') && v < 50 ? 'm' : 'mm')
  const mm = unit === 'm' ? v * 1000 : unit === 'cm' ? v * 10 : v
  return Math.round(mm)
}
