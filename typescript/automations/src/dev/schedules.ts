/**
 * Schedules under `a4 dev`: the runner ticks them against the dev
 * environment's clock, read from the fork's Clock sysvar, so `a4 dev warp`
 * moves schedules along with the protocol's own deadlines. A tick that cannot
 * read the clock uses the wall clock.
 */

import type { Simulator } from '../policy/simulator';
import { decodeBase64 } from '../policy/encoding';
import type { Automations } from '../runtime/automations';

const CLOCK_SYSVAR = 'SysvarC1ock11111111111111111111111111111111';
/** `unix_timestamp` in the Clock sysvar: after slot, epoch_start_timestamp, epoch and leader_schedule_epoch. */
const UNIX_TIMESTAMP_OFFSET = 32;

/** The fork's time in milliseconds, from its Clock sysvar; `null` when it cannot be read. */
export async function chainNow(rpc: Simulator): Promise<number | null> {
  try {
    const result = (await rpc.call('getAccountInfo', [CLOCK_SYSVAR, { encoding: 'base64' }])) as {
      value?: { data?: [string, string] } | null;
    };
    const data = result?.value?.data?.[0];
    if (!data) return null;
    const bytes = decodeBase64(data);
    if (!bytes || bytes.length < UNIX_TIMESTAMP_OFFSET + 8) return null;
    const seconds = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).getBigInt64(UNIX_TIMESTAMP_OFFSET, true);
    return Number(seconds) * 1000;
  } catch {
    return null;
  }
}

export interface ScheduleTicker {
  stop(): Promise<void>;
}

/**
 * Tick the definition's schedules every `intervalMs` (jobs are left to the
 * started worker). Logs each schedule dispatched for a new slot.
 */
export function startScheduleTicker(
  automations: Pick<Automations, 'tick'>,
  now: () => Promise<number | null>,
  log: (line: string) => void,
  intervalMs = 2000,
): ScheduleTicker {
  let stopped = false;
  let wake: (() => void) | null = null;
  const loop = (async () => {
    while (!stopped) {
      try {
        const at = (await now()) ?? Date.now();
        const { scheduled } = await automations.tick({ now: at, poll: false, maxJobs: 0 });
        for (const name of scheduled) log(`schedule ${name} dispatched at ${new Date(at).toISOString()} (dev environment clock)`);
      } catch (error) {
        log(`error (schedule): ${(error as Error)?.message ?? error}`);
      }
      if (stopped) break;
      await new Promise<void>((resolve) => {
        wake = resolve;
        setTimeout(resolve, intervalMs);
      });
    }
  })();
  return {
    async stop() {
      stopped = true;
      wake?.();
      await loop;
    },
  };
}
