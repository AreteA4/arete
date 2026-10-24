/** A duration in milliseconds, or text such as `30s`, `10m`, `24h` or `7d`. */
export type Duration = number | `${number}${'ms' | 's' | 'm' | 'h' | 'd'}`;

const UNITS: Record<string, number> = { ms: 1, s: 1000, m: 60_000, h: 3_600_000, d: 86_400_000 };

export function durationMs(duration: Duration): number {
  if (typeof duration === 'number') {
    if (!Number.isFinite(duration) || duration < 0) throw new RangeError(`invalid duration ${duration}`);
    return duration;
  }
  const match = /^(\d+(?:\.\d+)?)(ms|s|m|h|d)$/.exec(duration);
  if (!match) throw new RangeError(`invalid duration '${duration}'; use for example 30s, 10m, 24h or 7d`);
  return Number(match[1]) * UNITS[match[2]!]!;
}

/** Wall-clock time and waiting, injectable for tests. */
export interface Clock {
  now(): number;
  sleep(ms: number): Promise<void>;
}

export const systemClock: Clock = {
  now: () => Date.now(),
  sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
};

export type SchedulePeriod = 'minute' | 'hour' | 'day' | 'week' | 'month';

function pad(value: number, length = 2): string {
  return String(value).padStart(length, '0');
}

/** The ISO 8601 week of a UTC date, as `2026-W43`. */
function isoWeek(date: Date): string {
  const day = new Date(Date.UTC(date.getUTCFullYear(), date.getUTCMonth(), date.getUTCDate()));
  const weekday = day.getUTCDay() || 7;
  day.setUTCDate(day.getUTCDate() + 4 - weekday);
  const yearStart = Date.UTC(day.getUTCFullYear(), 0, 1);
  const week = Math.ceil(((day.getTime() - yearStart) / 86_400_000 + 1) / 7);
  return `${day.getUTCFullYear()}-W${pad(week)}`;
}

/**
 * A deterministic dispatch id for a scheduled run: the same name, period
 * and time give the same id, so a cron that fires twice in one period is
 * deduplicated. Times are UTC.
 *
 * `scheduleSlot('weekly-buy', 'week')` → `weekly-buy:2026-W43`.
 */
export function scheduleSlot(name: string, period: SchedulePeriod, at: Date | number = Date.now()): string {
  const date = typeof at === 'number' ? new Date(at) : at;
  const day = `${date.getUTCFullYear()}-${pad(date.getUTCMonth() + 1)}-${pad(date.getUTCDate())}`;
  switch (period) {
    case 'minute':
      return `${name}:${day}T${pad(date.getUTCHours())}:${pad(date.getUTCMinutes())}`;
    case 'hour':
      return `${name}:${day}T${pad(date.getUTCHours())}`;
    case 'day':
      return `${name}:${day}`;
    case 'week':
      return `${name}:${isoWeek(date)}`;
    case 'month':
      return `${name}:${date.getUTCFullYear()}-${pad(date.getUTCMonth() + 1)}`;
  }
}

/** How often a schedule runs: a calendar period (UTC), or a fixed interval of at least a second. */
export type ScheduleEvery = SchedulePeriod | Duration;

const PERIODS: readonly string[] = ['minute', 'hour', 'day', 'week', 'month'];

export function isSchedulePeriod(every: ScheduleEvery): every is SchedulePeriod {
  return typeof every === 'string' && PERIODS.includes(every);
}

/** Checks `every`, throwing a readable error for an interval under a second. */
export function checkScheduleEvery(every: ScheduleEvery): void {
  if (isSchedulePeriod(every)) return;
  if (durationMs(every) < 1000) throw new RangeError(`a schedule runs at most once a second, not every ${every}`);
}

/**
 * The slot `at` falls in for a schedule: a calendar period's slot as
 * {@link scheduleSlot} names it, or for an interval the start of the
 * interval since the epoch (`sweep:2026-10-23T12:15:00.000Z` for `15m`).
 */
export function scheduleSlotFor(name: string, every: ScheduleEvery, at: number): string {
  if (isSchedulePeriod(every)) return scheduleSlot(name, every, at);
  const ms = durationMs(every);
  return `${name}:${new Date(Math.floor(at / ms) * ms).toISOString()}`;
}
