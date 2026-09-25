import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import path from 'path';

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  server: {
    port: 5173,
    proxy: {
      '/api': {
        target: 'http://localhost:8080',
        changeOrigin: true,
      },
      // Liveness/readiness probes live outside /api/v1 (see AGENTS.md); the
      // dev server needs its own proxy entry or the SPA's fetch('/health/ready')
      // hits Vite itself instead of the backend.
      '/health': {
        target: 'http://localhost:8080',
        changeOrigin: true,
      },
    },
  },
  build: {
    outDir: '../backend/web/dist',
    emptyOutDir: true,
  },
});
