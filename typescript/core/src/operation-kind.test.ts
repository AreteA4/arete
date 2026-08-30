import { describe, expect, it } from 'vitest';

import { OPERATION_KIND, getOperationKind } from './operation-kind';
import {
  flowOperation,
  instructionOperation,
  transactionOperation,
} from './program-instructions';
import { programAccountRead, programQuery, stackQuery } from './read';

describe('operation kind brands', () => {
  const branded = [
    ['instruction', instructionOperation(async () => ({}) as never)],
    ['transaction', transactionOperation(async () => ({}) as never)],
    ['flow', flowOperation(async () => ({}) as never)],
    ['account-fetch', programAccountRead({ account: 'Config' })],
    ['query', programQuery({ name: 'topHolders', path: '/top-holders' })],
    ['query', stackQuery({ name: 'overview', path: '/overview' })],
  ] as const;

  it.each(branded)('classifies %s factory output by brand', (kind, value) => {
    expect(getOperationKind(value)).toBe(kind);
  });

  it.each(branded)('keeps the %s brand non-enumerable', (_kind, value) => {
    expect(Object.keys(value)).not.toContain(OPERATION_KIND);
    expect(JSON.stringify(value)).not.toContain(OPERATION_KIND);
    expect({ ...value }).not.toHaveProperty(OPERATION_KIND);
  });

  it('returns undefined for unbranded values', () => {
    expect(getOperationKind(undefined)).toBeUndefined();
    expect(getOperationKind(null)).toBeUndefined();
    expect(getOperationKind('instruction')).toBeUndefined();
    expect(getOperationKind({ kind: 'instruction' })).toBeUndefined();
    expect(getOperationKind(() => undefined)).toBeUndefined();
  });
});
