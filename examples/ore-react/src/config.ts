const env = import.meta.env;

function optionalEnv(value: string | undefined): string | undefined {
  const trimmed = value?.trim();
  return trimmed ? trimmed : undefined;
}

const wsUrl = optionalEnv(env.VITE_ARETE_WS_URL as string | undefined);
const httpUrl = optionalEnv(env.VITE_ARETE_HTTP_URL as string | undefined);
const publishableKey = optionalEnv(env.VITE_ARETE_PUBLISHABLE_KEY as string | undefined);

function validateEndpoint(
  name: string,
  url: string | undefined,
  protocols: readonly string[],
): string | null {
  if (url === undefined) return null;
  try {
    const protocol = new URL(url).protocol;
    return protocols.includes(protocol)
      ? null
      : `${name} must use ${protocols.join(' or ')}.`;
  } catch {
    return `${name} must be a valid URL.`;
  }
}

const endpointError = validateEndpoint('VITE_ARETE_WS_URL', wsUrl, ['ws:', 'wss:'])
  ?? validateEndpoint('VITE_ARETE_HTTP_URL', httpUrl, ['http:', 'https:']);

export const appConfig = {
  /**
   * Endpoint overrides for the AreteProvider's default stack. The generated
   * stack carries hosted endpoints, so overrides are only needed for local
   * development or automated tests.
   */
  areteOptions: wsUrl || httpUrl ? { url: wsUrl, httpUrl } : undefined,
  /**
   * Optional publishable key. Without one the app mints anonymous hosted
   * sessions that share low-trust limits per IP/origin; providing a key
   * attributes usage to that key's own quota instead.
   */
  publishableKey,
  configurationError: endpointError,
};

export function transactionExplorerUrl(signature: string): string {
  return `https://solscan.io/tx/${encodeURIComponent(signature)}`;
}
