import { describe, expect, it } from 'vitest';
import {
  TransactionOptionsError,
  resolveTransactionBuildOptions,
  toWireResourceOptions,
} from './types';

const v0Only = { supportedTransactionVersions: [0] } as const;
const v1Capable = { supportedTransactionVersions: [0, 1] } as const;

describe('resolveTransactionBuildOptions', () => {
  it('defaults to v0 without touching callers who pass nothing', () => {
    expect(resolveTransactionBuildOptions()).toEqual({
      transactionVersion: 0,
      resources: {
        computeUnitLimit: undefined,
        loadedAccountsDataSizeLimit: undefined,
        heapSize: undefined,
        priorityFeeLamports: undefined,
        computeUnitPriceMicroLamports: undefined,
      },
    });
    expect(resolveTransactionBuildOptions({}, {}).transactionVersion).toBe(0);
  });

  it('transaction_v1 request against a v0-only adapter is rejected, never downgraded', () => {
    const error = (() => {
      try {
        resolveTransactionBuildOptions({ transactionVersion: 1 }, v0Only);
        return null;
      } catch (cause) {
        return cause as TransactionOptionsError;
      }
    })();

    expect(error).toBeInstanceOf(TransactionOptionsError);
    expect(error).toMatchObject({
      code: 'unsupported_transaction_version',
      requestedVersion: 1,
      supportedVersions: [0],
    });
  });

  it('treats a missing capability as unknown, failing only an explicit V1 request', () => {
    expect(resolveTransactionBuildOptions({ transactionVersion: 'legacy' }).transactionVersion)
      .toBe('legacy');
    expect(resolveTransactionBuildOptions({ transactionVersion: 0 }).transactionVersion).toBe(0);
    expect(() => resolveTransactionBuildOptions({ transactionVersion: 1 })).toThrow(
      /does not support transaction version 1 \(adapter declares no supported versions\)/
    );
  });

  it('rejects an explicit version a declaring adapter left out', () => {
    expect(() => resolveTransactionBuildOptions({ transactionVersion: 'legacy' }, v0Only))
      .toThrow(/does not support transaction version "legacy"/);
    expect(resolveTransactionBuildOptions({ transactionVersion: 1 }, v1Capable))
      .toMatchObject({ transactionVersion: 1 });
  });

  it('rejects an unknown transaction version outright', () => {
    expect(() => resolveTransactionBuildOptions(
      { transactionVersion: 2 as unknown as 1 }
    )).toThrow(/Unknown transactionVersion 2/);
  });

  it('v1_contract rejects an unknown key inside the closed resources object', () => {
    expect(() => resolveTransactionBuildOptions({
      resources: { computeUnitLimit: 200_000, priorityFee: 5_000n } as never,
    })).toThrow(/Unknown resource option 'priorityFee'/);
    expect(() => resolveTransactionBuildOptions({ resources: 'lots' as never }))
      .toThrow(/'resources' must be an object of canonical resource options/);
  });

  it('binds the two fee fields to their versions instead of converting', () => {
    expect(() => resolveTransactionBuildOptions({ resources: { priorityFeeLamports: 5_000n } }))
      .toThrow(/priorityFeeLamports requires transaction version 1, not 0/);
    expect(() => resolveTransactionBuildOptions({
      transactionVersion: 'legacy',
      resources: { priorityFeeLamports: 5_000n },
    })).toThrow(/requires transaction version 1, not "legacy"/);
    expect(() => resolveTransactionBuildOptions({
      transactionVersion: 1,
      resources: { computeUnitPriceMicroLamports: 7n },
    }, v1Capable)).toThrow(/applies to legacy\/v0 only/);
    expect(() => resolveTransactionBuildOptions({
      transactionVersion: 1,
      resources: { priorityFeeLamports: 1n, computeUnitPriceMicroLamports: 1n },
    }, v1Capable)).toThrow(/mutually exclusive/);
  });

  it('keeps u64 fee quantities exact and refuses lossy JSON numbers', () => {
    expect(resolveTransactionBuildOptions({
      transactionVersion: 1,
      resources: { priorityFeeLamports: '18446744073709551615' },
    }, v1Capable).resources.priorityFeeLamports).toBe(18_446_744_073_709_551_615n);
    expect(resolveTransactionBuildOptions({
      resources: { computeUnitPriceMicroLamports: 9_999_999_999_999_999_999n },
    }).resources.computeUnitPriceMicroLamports).toBe(9_999_999_999_999_999_999n);

    expect(() => resolveTransactionBuildOptions({
      resources: { computeUnitPriceMicroLamports: 1000 as unknown as bigint },
    })).toThrow(/never a number.*may already have lost precision/s);
    expect(() => resolveTransactionBuildOptions({
      resources: { computeUnitPriceMicroLamports: 18_446_744_073_709_551_616n },
    })).toThrow(/exceeds its maximum/);
    expect(() => resolveTransactionBuildOptions({
      resources: { computeUnitPriceMicroLamports: -1n },
    })).toThrow(/must not be negative/);
  });

  it('accepts u32 budgets as numbers, bigints or decimal strings', () => {
    expect(resolveTransactionBuildOptions({
      resources: {
        computeUnitLimit: 200_000,
        loadedAccountsDataSizeLimit: '65536',
        heapSize: 262_144n,
      },
    }).resources).toMatchObject({
      computeUnitLimit: 200_000n,
      loadedAccountsDataSizeLimit: 65_536n,
      heapSize: 262_144n,
    });
    expect(resolveTransactionBuildOptions({ resources: { computeUnitLimit: 0 } })
      .resources.computeUnitLimit).toBe(0n);

    expect(() => resolveTransactionBuildOptions({ resources: { computeUnitLimit: 1.5 } }))
      .toThrow(/must be a non-negative integer or decimal string, received 1.5/);
    expect(() => resolveTransactionBuildOptions({ resources: { heapSize: -1 } }))
      .toThrow(/must be a non-negative integer or decimal string, received -1/);
    expect(() => resolveTransactionBuildOptions({ resources: { heapSize: 'lots' } }))
      .toThrow(/must be a non-negative integer or decimal string/);
    expect(() => resolveTransactionBuildOptions({ resources: { heapSize: -1n } }))
      .toThrow(/must not be negative/);
    expect(() => resolveTransactionBuildOptions({ resources: { heapSize: 4_294_967_296n } }))
      .toThrow(/exceeds its maximum of 4294967295/);
    expect(() => resolveTransactionBuildOptions({ resources: { heapSize: true as never } }))
      .toThrow(/must be a non-negative integer or decimal string/);
  });

  it('v1_contract round-trips a resolved budget through the wire form unchanged', () => {
    const options = {
      transactionVersion: 1,
      resources: {
        computeUnitLimit: 1_400_000,
        heapSize: 0,
        loadedAccountsDataSizeLimit: '65536',
        priorityFeeLamports: 18_446_744_073_709_551_615n,
      },
    } as const;
    const first = resolveTransactionBuildOptions(options, v1Capable);

    const fromWire = resolveTransactionBuildOptions(
      { transactionVersion: 1, resources: toWireResourceOptions(first.resources) },
      v1Capable
    );
    expect(fromWire).toEqual(first);

    // A resolved object's own bigints are valid input again.
    const fromResolved = resolveTransactionBuildOptions(
      { transactionVersion: 1, resources: first.resources },
      v1Capable
    );
    expect(fromResolved).toEqual(first);
  });
});

describe('toWireResourceOptions', () => {
  it('v1_contract matches the cross-language decimal-string reference', () => {
    const { resources } = resolveTransactionBuildOptions({
      transactionVersion: 1,
      resources: {
        computeUnitLimit: 1_400_000,
        heapSize: 0,
        loadedAccountsDataSizeLimit: '65536',
        priorityFeeLamports: 18_446_744_073_709_551_615n,
      },
    }, v1Capable);

    // Keys and decimal-string values are the contract; JSON key order is not.
    expect(JSON.parse(JSON.stringify(toWireResourceOptions(resources)))).toEqual({
      computeUnitLimit: '1400000',
      heapSize: '0',
      loadedAccountsDataSizeLimit: '65536',
      priorityFeeLamports: '18446744073709551615',
    });
  });

  it('omits absent budgets instead of emitting null', () => {
    const { resources } = resolveTransactionBuildOptions({
      resources: { computeUnitPriceMicroLamports: 1_000n },
    });
    expect(toWireResourceOptions(resources)).toEqual({ computeUnitPriceMicroLamports: '1000' });
  });
});
