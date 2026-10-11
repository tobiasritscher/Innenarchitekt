// Minimal stroke icons for the 2D toolbar (24×24, currentColor).
import type { ReactNode } from 'react'

const I = ({ children }: { children: ReactNode }) => (
  <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    {children}
  </svg>
)

export const icons = {
  select: (
    <I>
      <path d="M5 3l14 8-6 2-2 6z" />
    </I>
  ),
  wall: (
    <I>
      <path d="M3 18V6h12v6h6" />
      <circle cx="3" cy="18" r="1.4" />
      <circle cx="21" cy="12" r="1.4" />
    </I>
  ),
  measure: (
    <I>
      <path d="M3 8v8M21 8v8M3 12h18" />
      <path d="M6 10l-3 2 3 2M18 10l3 2-3 2" />
    </I>
  ),
  opening: (
    <I>
      <path d="M3 20h4M17 20h4M7 20V9" />
      <path d="M7 9a10 10 0 0 1 10 11" strokeDasharray="2 2" />
    </I>
  ),
  room: (
    <I>
      <rect x="3" y="4" width="18" height="16" rx="1" />
      <path d="M8 13h8M10 10h4" />
    </I>
  ),
  material: (
    <I>
      <path d="M4 15l8-11 8 11-8 5z" />
      <path d="M4 15l8 1 8-1" />
    </I>
  ),
  light: (
    <I>
      <path d="M9 18h6M10 21h4" />
      <path d="M12 3a6 6 0 0 1 4 10.5c-.8.8-1 1.5-1 2.5H9c0-1-.2-1.7-1-2.5A6 6 0 0 1 12 3z" />
    </I>
  ),
  refimage: (
    <I>
      <rect x="3" y="4" width="18" height="16" rx="1.5" />
      <path d="M3 16l5-5 4 4 3-3 6 6" />
      <circle cx="16" cy="8.5" r="1.5" />
    </I>
  ),
  snap: (
    <I>
      <path d="M6 4v7a6 6 0 0 0 12 0V4" />
      <path d="M6 7h4M14 7h4" />
    </I>
  ),
  fit: (
    <I>
      <path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5" />
    </I>
  ),
  furniture: (
    <I>
      <path d="M4 11V8a2 2 0 0 1 2-2h12a2 2 0 0 1 2 2v3" />
      <path d="M3 11h18v5H3zM5 16v3M19 16v3" />
    </I>
  ),
}
