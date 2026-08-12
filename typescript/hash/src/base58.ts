const BASE58_ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

export function isCanonicalBase58_32(value: unknown): value is string {
  if (typeof value !== "string" || !/^[1-9A-HJ-NP-Za-km-z]+$/.test(value)) {
    return false;
  }
  const decoded = decodeBase58(value);
  return decoded.length === 32 && encodeBase58(decoded) === value;
}

function decodeBase58(value: string): Uint8Array {
  let numeric = 0n;
  for (const character of value) {
    numeric = numeric * 58n + BigInt(BASE58_ALPHABET.indexOf(character));
  }

  const suffix: number[] = [];
  while (numeric > 0n) {
    suffix.push(Number(numeric & 0xffn));
    numeric >>= 8n;
  }
  suffix.reverse();

  const leadingZeroes = value.match(/^1*/)?.[0].length ?? 0;
  const decoded = new Uint8Array(leadingZeroes + suffix.length);
  decoded.set(suffix, leadingZeroes);
  return decoded;
}

function encodeBase58(value: Uint8Array): string {
  let leadingZeroes = 0;
  while (leadingZeroes < value.length && value[leadingZeroes] === 0) {
    leadingZeroes += 1;
  }

  let numeric = 0n;
  for (const byte of value) {
    numeric = numeric * 256n + BigInt(byte);
  }

  let suffix = "";
  while (numeric > 0n) {
    suffix = BASE58_ALPHABET[Number(numeric % 58n)]! + suffix;
    numeric /= 58n;
  }
  return "1".repeat(leadingZeroes) + suffix;
}
