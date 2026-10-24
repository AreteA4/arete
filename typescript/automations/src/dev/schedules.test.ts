import { describe, expect, it } from 'vitest';

import { harness } from '../test-support/harness';
import { chainNow, startScheduleTicker } from './schedules';

/** A Clock sysvar whose unix_timestamp is `seconds`. */
function clockAccount(seconds: number): string {
  const bytes = new Uint8Array(40);
  new DataView(bytes.buffer).setBigInt64(32, BigInt(seconds), true);
  return Buffer.from(bytes).toString('base64');
}

describe('schedules under a4 dev', () => {
  it('reads the fork’s time from the Clock sysvar', async () => {
    const calls: unknown[][] = [];
    const rpc = {
      call: async (method: string, params: readonly unknown[]) => {
        calls.push([method, ...params]);
        return { value: { data: [clockAccount(1_792_000_000), 'base64'] } };
      },
    };
    expect(await chainNow(rpc)).toBe(1_792_000_000_000);
    expect(calls[0]).toEqual(['getAccountInfo', 'SysvarC1ock11111111111111111111111111111111', { encoding: 'base64' }]);
    expect(await chainNow({ call: async () => ({ value: null }) })).toBeNull();
    expect(await chainNow({ call: async () => Promise.reject(new Error('down')) })).toBeNull();
  });

  it('dispatches on the fork’s clock, so a warp moves the schedule', async () => {
    const h = harness();
    h.automations.schedule('daily', { every: 'day' });
    h.automations.on('daily', () => []);
    const lines: string[] = [];
    let now = Date.UTC(2026, 9, 19, 12);
    let ticks = 0;
    const ticker = startScheduleTicker(
      h.automations,
      async () => {
        ticks += 1;
        if (ticks === 3) now += 86_400_000; // a4 dev warp +1d
        return now;
      },
      (line) => lines.push(line),
      1,
    );
    while (ticks < 5) await new Promise((resolve) => setTimeout(resolve, 2));
    await ticker.stop();
    expect(lines).toEqual([
      'schedule daily dispatched at 2026-10-19T12:00:00.000Z (dev environment clock)',
      'schedule daily dispatched at 2026-10-20T12:00:00.000Z (dev environment clock)',
    ]);
    // The ticker leaves jobs to the started worker.
    expect(await h.store.jobs.list({ kind: 'delivery' })).toHaveLength(2);
  });
});
