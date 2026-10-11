// Photometric helpers: colour temperature, lumen → three.js physical light units, daylight model.
import { Color, SRGBColorSpace } from 'three'
import type { LightKind } from '../core/generated/LightKind'

/** Colour of a black-body radiator (Tanner Helland fit, 1000–40000 K), as linear three.js Color. */
export function kelvinToColor(kelvin: number, target = new Color()): Color {
  const t = Math.min(40000, Math.max(1000, kelvin)) / 100
  let r: number, g: number, b: number
  if (t <= 66) {
    r = 255
    g = 99.4708025861 * Math.log(t) - 161.1195681661
    b = t <= 19 ? 0 : 138.5177312231 * Math.log(t - 10) - 305.0447927307
  } else {
    r = 329.698727446 * Math.pow(t - 60, -0.1332047592)
    g = 288.1221695283 * Math.pow(t - 60, -0.0755148492)
    b = 255
  }
  const c = (v: number) => Math.min(255, Math.max(0, v)) / 255
  return target.setRGB(c(r), c(g), c(b), SRGBColorSpace)
}

/**
 * three.js physical intensity for a lamp of `lumen`:
 * Point → candela (lm / 4π), Spot → candela over its cone (lm / 2π(1 − cos θ/2)),
 * Area (RectAreaLight) → luminance in nits (lm / (π · A)).
 */
export function lumenToIntensity(kind: LightKind, lumen: number): number {
  switch (kind.type) {
    case 'Point':
      return lumen / (4 * Math.PI)
    case 'Spot': {
      const half = (Math.max(5, Math.min(170, kind.angle_deg)) / 2) * (Math.PI / 180)
      return lumen / (2 * Math.PI * (1 - Math.cos(half)))
    }
    // Surface: the real intensity needs the object's size (lightGeom.surfaceIntensity); this keeps the switch total.
    case 'Surface':
      return lumen / (4 * Math.PI)
    case 'Area': {
      const a = Math.max(0.0025, (kind.w_mm / 1000) * (kind.h_mm / 1000))
      return lumen / (Math.PI * a)
    }
  }
}

export type Daylight = {
  /** Direct normal sun illuminance (lux). */
  sunLux: number
  /** Diffuse horizontal sky illuminance (lux). */
  skyLux: number
  /** 0 = full night … 1 = sun above the horizon. */
  day: number
}

/** Simple clear/overcast sky model from solar elevation (deg) and clearness (0 overcast … 1 clear). */
export function daylight(elevationDeg: number, clearness: number): Daylight {
  const el = elevationDeg
  const s = Math.sin((Math.max(0, el) * Math.PI) / 180)
  // Kasten–Young air mass; extraterrestrial ~127'500 lux; clear-air extinction.
  let sunLux = 0
  if (el > -0.5) {
    const h = Math.max(0, el)
    const airmass = 1 / (Math.sin((h * Math.PI) / 180) + 0.50572 * Math.pow(h + 6.07995, -1.6364))
    sunLux = 127500 * Math.exp(-0.21 * airmass) * Math.pow(clearness, 2.2)
    if (el < 0.5) sunLux *= Math.max(0, (el + 0.5) / 1)
  }
  const skyClear = 16000 * Math.pow(s, 0.65)
  const skyOvercast = 26000 * s
  let skyLux = skyClear * clearness + skyOvercast * (1 - clearness)
  // civil twilight
  const twilight = el > 0 ? 0 : el > -6 ? 400 * Math.pow((el + 6) / 6, 2) : 0
  skyLux += twilight * (el > 0 ? 0 : 1) + (el > 0 && el < 3 ? 400 * (1 - el / 3) : 0)
  skyLux = Math.max(skyLux, 0.08) // moon/starlight floor
  const day = el >= 0 ? 1 : el <= -6 ? 0 : (el + 6) / 6
  return { sunLux, skyLux, day }
}
