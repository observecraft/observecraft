/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** "live" → Rust API via /api; anything else → in-browser mocks. */
  readonly VITE_API_MODE?: 'live' | 'mock';
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
