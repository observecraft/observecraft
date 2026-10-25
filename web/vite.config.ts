import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    // Dev: the API runs on :8080 (make server). Same-origin /api keeps cookies/SSE simple.
    proxy: { '/api': { target: 'http://localhost:8080', changeOrigin: false } },
  },
});
