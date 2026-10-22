const LAMPORTS_PER_SOL = 1_000_000_000n;

/** Lamports as a decimal SOL string. */
export function lamportsToSol(lamports: bigint): string {
  const sign = lamports < 0n ? '-' : '';
  const abs = lamports < 0n ? -lamports : lamports;
  const whole = abs / LAMPORTS_PER_SOL;
  const fraction = abs % LAMPORTS_PER_SOL;
  if (fraction === 0n) return `${sign}${whole}`;
  return `${sign}${whole}.${fraction.toString().padStart(9, '0').replace(/0+$/, '')}`;
}

/**
 * A JavaScript number in plain decimal notation, with the digits of its
 * shortest round-trip form (what `String(value)` uses, without an
 * exponent).
 */
export function plainDecimal(value: number): string {
  const text = String(value);
  const match = /^(-?)(\d+)(?:\.(\d+))?e([+-]\d+)$/.exec(text);
  if (!match) return text;
  const [, sign, whole, fraction = '', exponentText] = match;
  const digits = `${whole}${fraction}`;
  const point = whole!.length + Number(exponentText);
  if (point <= 0) return `${sign}0.${'0'.repeat(-point)}${digits}`;
  if (point >= digits.length) return `${sign}${digits}${'0'.repeat(point - digits.length)}`;
  return `${sign}${digits.slice(0, point)}.${digits.slice(point)}`;
}
