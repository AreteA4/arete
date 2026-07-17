/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_ARETE_WS_URL?: string
  readonly VITE_ARETE_HTTP_URL?: string
  readonly VITE_DERIVE_ARETE_HTTP?: string
  readonly VITE_ARETE_PUBLISHABLE_KEY?: string
  readonly VITE_SOLANA_RPC_URL?: string
  readonly VITE_TRANSACTION_TRANSPORT?: 'auto' | 'direct'
  readonly VITE_SOLANA_EXPLORER_URL?: string
  readonly VITE_AUTOMATED_TEST_MODE?: string
}

interface ImportMeta {
  readonly env: ImportMetaEnv
}
