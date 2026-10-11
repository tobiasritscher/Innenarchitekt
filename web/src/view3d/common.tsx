// Shared canvas setup and quality presets.
import { useLayoutEffect } from 'react'
import { useThree } from '@react-three/fiber'
import { AgXToneMapping, NoToneMapping, SRGBColorSpace, Vector3, type PerspectiveCamera } from 'three'
import type { OrbitControls as OrbitControlsImpl } from 'three-stdlib'
import { RectAreaLightUniformsLib } from 'three/examples/jsm/lights/RectAreaLightUniformsLib.js'
import type { Quality } from '../state/editor'

export type QualityPreset = { dpr: number; sunMap: number; budget: number; lampMap: number; ao: boolean }

export const QUALITY: Record<Quality, QualityPreset> = {
  performance: { dpr: 1, sunMap: 1024, budget: 2, lampMap: 256, ao: false },
  balanced: { dpr: 1.5, sunMap: 2048, budget: 4, lampMap: 512, ao: false },
  beautiful: { dpr: 2, sunMap: 4096, budget: 6, lampMap: 1024, ao: true },
}

export const QUALITY_LABEL: Record<Quality, string> = { performance: 'Leistung', balanced: 'Ausgewogen', beautiful: 'Schön' }

let areaLib = false

/** AgX tone mapping, sRGB output, local clipping (wall cut). `postTone` = tone mapping happens in the post chain. */
export function RendererSetup({ postTone = false }: { postTone?: boolean }) {
  const gl = useThree((s) => s.gl)
  const invalidate = useThree((s) => s.invalidate)
  useLayoutEffect(() => {
    if (!areaLib) {
      RectAreaLightUniformsLib.init()
      areaLib = true
    }
    gl.toneMapping = postTone ? NoToneMapping : AgXToneMapping
    gl.toneMappingExposure = 1
    gl.outputColorSpace = SRGBColorSpace
    gl.localClippingEnabled = true
    invalidate()
  }, [gl, postTone, invalidate])
  return null
}

/** Places the camera obliquely above the centre so a sphere of radius r fits the view. */
export function fitCamera(camera: PerspectiveCamera, controls: OrbitControlsImpl, c: Vector3, r: number) {
  const fov = (camera.fov * Math.PI) / 180
  const d = (r * 1.1) / Math.tan(fov / 2) / Math.min(1, camera.aspect)
  const dir = new Vector3(0.25, 1.1, 0.9).normalize()
  camera.position.copy(c).addScaledVector(dir, d)
  controls.target.copy(c)
}


/** Value of a CSS custom property on :root (theme colours for the canvas). */
export function cssVar(name: string, fallback: string) {
  if (typeof document === 'undefined') return fallback
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim()
  return v || fallback
}
