import { deriveHttpEndpoint } from '@usearete/sdk';
import { ORE_STREAM_STACK } from './generated/ore-stack';

export interface OreAppConfig {
  areteWsUrl: string;
  areteHttpUrl: string;
  transactionTransport: 'auto' | 'direct';
  solanaRpcUrl?: string;
  explorerUrl: string;
  publishableKey?: string;
  automatedTestMode: boolean;
}

export type RuntimeEnv = Partial<Record<
  | 'VITE_ARETE_WS_URL'
  | 'VITE_ARETE_HTTP_URL'
  | 'VITE_DERIVE_ARETE_HTTP'
  | 'VITE_ARETE_PUBLISHABLE_KEY'
  | 'VITE_SOLANA_RPC_URL'
  | 'VITE_TRANSACTION_TRANSPORT'
  | 'VITE_SOLANA_EXPLORER_URL'
  | 'VITE_AUTOMATED_TEST_MODE',
  string
>>;

const DEFAULT_EXPLORER_URL = 'https://solscan.io/tx/';

function optional(value: string | undefined): string | undefined {
  const trimmed = value?.trim();
  return trimmed ? trimmed : undefined;
}

function flag(value: string | undefined): boolean {
  return value?.trim().toLowerCase() === 'true';
}

function assertUrl(value: string, label: string, protocols: readonly string[]): string {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    throw new Error(`${label} must be an absolute URL`);
  }
  if (!protocols.includes(url.protocol)) {
    throw new Error(`${label} must use ${protocols.join(' or ')}`);
  }
  return value.replace(/\/$/, '');
}

function assertMainnetRpc(value: string): string {
  const checked = assertUrl(value, 'VITE_SOLANA_RPC_URL', ['https:', 'http:']);
  const url = new URL(checked);
  const explicitCluster = url.searchParams.get('cluster');
  const nonMainnet = /(^|[.-])(devnet|testnet|localhost)([.:/-]|$)/i.test(url.hostname)
    || url.hostname === '127.0.0.1'
    || url.hostname === '0.0.0.0'
    || (explicitCluster !== null && !/^(mainnet|mainnet-beta)$/i.test(explicitCluster));
  if (nonMainnet) {
    throw new Error('ORE writes require a mainnet Solana RPC endpoint');
  }
  return checked;
}

export function resolveConfig(env: RuntimeEnv): OreAppConfig {
  const wsOverride = optional(env.VITE_ARETE_WS_URL);
  const httpOverride = optional(env.VITE_ARETE_HTTP_URL);
  const deriveHttp = flag(env.VITE_DERIVE_ARETE_HTTP);
  const transactionTransport = optional(env.VITE_TRANSACTION_TRANSPORT) ?? 'auto';
  if (transactionTransport !== 'auto' && transactionTransport !== 'direct') {
    throw new Error('VITE_TRANSACTION_TRANSPORT must be auto or direct');
  }
  const rpcOverride = optional(env.VITE_SOLANA_RPC_URL);
  if (transactionTransport === 'direct' && !rpcOverride) {
    throw new Error('VITE_SOLANA_RPC_URL is required when VITE_TRANSACTION_TRANSPORT=direct');
  }

  if (httpOverride && !wsOverride) {
    throw new Error('VITE_ARETE_HTTP_URL requires the paired VITE_ARETE_WS_URL');
  }
  if (wsOverride && !httpOverride && !deriveHttp) {
    throw new Error(
      'VITE_ARETE_WS_URL requires VITE_ARETE_HTTP_URL or VITE_DERIVE_ARETE_HTTP=true',
    );
  }
  if (deriveHttp && (!wsOverride || httpOverride)) {
    throw new Error(
      'VITE_DERIVE_ARETE_HTTP=true requires only VITE_ARETE_WS_URL to be set',
    );
  }

  const areteWsUrl = assertUrl(
    wsOverride ?? ORE_STREAM_STACK.endpoints.ws,
    'Arete WebSocket URL',
    ['wss:', 'ws:'],
  );
  const areteHttpUrl = assertUrl(
    httpOverride ?? (deriveHttp ? deriveHttpEndpoint(areteWsUrl) : ORE_STREAM_STACK.endpoints.http),
    'Arete HTTP URL',
    ['https:', 'http:'],
  );
  const explorerUrl = assertUrl(
    optional(env.VITE_SOLANA_EXPLORER_URL) ?? DEFAULT_EXPLORER_URL,
    'VITE_SOLANA_EXPLORER_URL',
    ['https:', 'http:'],
  );

  return {
    areteWsUrl,
    areteHttpUrl,
    transactionTransport,
    solanaRpcUrl: transactionTransport === 'direct' ? assertMainnetRpc(rpcOverride!) : undefined,
    explorerUrl: `${explorerUrl}/`,
    publishableKey: optional(env.VITE_ARETE_PUBLISHABLE_KEY),
    automatedTestMode: flag(env.VITE_AUTOMATED_TEST_MODE),
  };
}

export function transactionExplorerUrl(config: OreAppConfig, signature: string): string {
  return `${config.explorerUrl}${encodeURIComponent(signature)}`;
}

export const appConfig = resolveConfig(import.meta.env);
