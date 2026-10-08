import { agentKeys } from './env.js';

export interface KeyLease {
  key?: string;
  /**
   * `exclusive` while no other run has held this key during the lease, so
   * the key's usage meters belong to this run alone.
   */
  attribution(): 'exclusive' | 'shared' | 'unknown';
  release(): void;
}

interface ActiveLease {
  shared: boolean;
}

/**
 * Hands out Arete agent keys to concurrent runs. Each run gets its own key
 * while the pool lasts so per-run usage meters stay attributable; once every
 * key is busy, runs share the least-used one and usage is marked `shared`.
 */
export class KeyPool {
  private readonly active = new Map<string, Set<ActiveLease>>();

  constructor(private readonly keys: string[] = agentKeys()) {
    for (const key of keys) this.active.set(key, new Set());
  }

  get size(): number {
    return this.keys.length;
  }

  lease(): KeyLease {
    if (this.keys.length === 0) {
      return { attribution: () => 'unknown', release: () => {} };
    }
    let key = this.keys[0]!;
    for (const candidate of this.keys) {
      if (this.active.get(candidate)!.size < this.active.get(key)!.size) key = candidate;
    }
    const holders = this.active.get(key)!;
    const lease: ActiveLease = { shared: holders.size > 0 };
    for (const other of holders) other.shared = true;
    holders.add(lease);
    return {
      key,
      attribution: () => (lease.shared ? 'shared' : 'exclusive'),
      release: () => {
        holders.delete(lease);
      },
    };
  }
}
