// Preparing the uploaded floor plan in the browser: PDF → PNG (page 1), photos → upright, ≤ 2400 px.
import workerUrl from 'pdfjs-dist/build/pdf.worker.min.mjs?url'

/** `paperMmPerPx`: paper size of one pixel (PDF only) – with a printed scale like 1:100 the exact plan scale. */
export type PreparedImage = { blob: Blob; url: string; width: number; height: number; name: string; paperMmPerPx?: number }

const MAX_PX = 2400
const PDF_TARGET_PX = 2000

function canvasToBlob(c: HTMLCanvasElement, type: string, quality?: number): Promise<Blob> {
  return new Promise((res, rej) => c.toBlob((b) => (b ? res(b) : rej(new Error('Bild konnte nicht erzeugt werden'))), type, quality))
}

export async function renderPdfFirstPage(file: Blob): Promise<{ blob: Blob; paperMmPerPx: number }> {
  const pdfjs = await import('pdfjs-dist')
  pdfjs.GlobalWorkerOptions.workerSrc = workerUrl
  const data = new Uint8Array(await file.arrayBuffer())
  const task = pdfjs.getDocument({ data })
  const doc = await task.promise
  try {
    const page = await doc.getPage(1)
    const vp1 = page.getViewport({ scale: 1 })
    const scale = PDF_TARGET_PX / Math.max(vp1.width, vp1.height)
    const vp = page.getViewport({ scale })
    const canvas = document.createElement('canvas')
    canvas.width = Math.round(vp.width)
    canvas.height = Math.round(vp.height)
    const ctx = canvas.getContext('2d')!
    ctx.fillStyle = '#ffffff'
    ctx.fillRect(0, 0, canvas.width, canvas.height)
    await page.render({ canvas, canvasContext: ctx, viewport: vp }).promise
    // PDF user space: 1 unit = 1/72 inch at viewport scale 1
    const paperMmPerPx = (25.4 / 72) * (vp1.width / canvas.width)
    return { blob: await canvasToBlob(canvas, 'image/png'), paperMmPerPx }
  } finally {
    void task.destroy()
  }
}

/** Normalise any supported input to an upright raster image we can show and upload. */
export async function prepareImage(file: File): Promise<PreparedImage> {
  const isPdf = file.type === 'application/pdf' || file.name.toLowerCase().endsWith('.pdf')
  const pdf = isPdf ? await renderPdfFirstPage(file) : null
  let blob: Blob = pdf ? pdf.blob : file
  if (!isPdf && !/^image\/(png|jpeg|jpg|webp|heic|heif)$/i.test(file.type) && !/\.(png|jpe?g|webp)$/i.test(file.name)) {
    throw new Error('Bitte ein JPG, PNG oder PDF wählen.')
  }
  // createImageBitmap applies EXIF orientation, so the corners we send match what the user sees.
  const bmp = await createImageBitmap(blob)
  let { width, height } = bmp
  const needsReencode = !isPdf && (file.type !== 'image/png' || Math.max(width, height) > MAX_PX)
  if (needsReencode) {
    const s = Math.min(1, MAX_PX / Math.max(width, height))
    const c = document.createElement('canvas')
    c.width = Math.round(width * s)
    c.height = Math.round(height * s)
    c.getContext('2d')!.drawImage(bmp, 0, 0, c.width, c.height)
    blob = await canvasToBlob(c, 'image/png')
    width = c.width
    height = c.height
  }
  bmp.close()
  return { blob, url: URL.createObjectURL(blob), width, height, name: file.name, paperMmPerPx: pdf?.paperMmPerPx }
}
