/**
 * Base64URL encoding/decoding utilities
 */

/**
 * Encode Uint8Array to base64url string (RFC 4648)
 */
export function encode(bytes: Uint8Array): string {
  // Convert to regular base64
  const base64 = Buffer.from(bytes).toString('base64');
  // Convert to base64url: replace + with -, / with _, remove padding
  return base64
    .replace(/\+/g, '-')
    .replace(/\//g, '_')
    .replace(/=/g, '');
}

/**
 * Decode base64url string to Uint8Array
 */
export function decode(base64url: string): Uint8Array {
  // Convert from base64url to regular base64
  let base64 = base64url
    .replace(/-/g, '+')
    .replace(/_/g, '/');
  
  // Add padding if needed
  const padding = 4 - (base64.length % 4);
  if (padding !== 4) {
    base64 += '='.repeat(padding);
  }
  
  return new Uint8Array(Buffer.from(base64, 'base64'));
}

export const base64url = {
  encode,
  decode,
};
