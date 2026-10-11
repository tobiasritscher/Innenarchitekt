import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// Backend (axum) runs on :8090 in dev; Vite proxies /api.
export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: { '/api': { target: 'http://127.0.0.1:8090', changeOrigin: false } },
  },
  build: { target: 'es2022', chunkSizeWarningLimit: 4000 },
})
