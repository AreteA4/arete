/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_ARETE_WS_URL?: string
  readonly VITE_ARETE_HTTP_URL?: string
  readonly VITE_ARETE_PUBLISHABLE_KEY?: string
}

interface ImportMeta {
  readonly env: ImportMetaEnv
}
