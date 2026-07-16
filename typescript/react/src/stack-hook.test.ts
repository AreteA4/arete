jest.mock('react', () => ({
  useEffect: jest.fn(),
  useMemo: jest.fn((factory: () => unknown) => factory()),
  useState: jest.fn((initial: unknown) => [
    typeof initial === 'function' ? (initial as () => unknown)() : initial,
    jest.fn(),
  ]),
}));
jest.mock('./provider', () => ({
  useAreteContext: () => ({
    getClient: jest.fn(() => null),
    getOrCreateClient: jest.fn(() => new Promise(() => undefined)),
  }),
}));
jest.mock('@usearete/sdk', () => ({
  InstructionError: class InstructionError extends Error {},
  OperationExecutionError: class OperationExecutionError extends Error {},
  getTransactionFailureOutcome: jest.fn(() => null),
  unwrapOperationExecutionError: jest.fn((error: unknown) => error),
}));

import { useArete } from './stack';

describe('useArete loading surface', () => {
  it('keeps connected fields nullable and exposes static stack extensions', () => {
    const stack = {
      name: 'extended-stack',
      endpoints: { ws: 'wss://example.invalid', http: 'https://example.invalid' },
      views: {},
      addresses: { board: () => 'board-address' },
      constants: { tileCount: 25 },
      math: { double: (value: number) => value * 2 },
    } as const;

    const result = useArete(stack);

    expect(result.client).toBeNull();
    expect(result.chain).toBeNull();
    expect(result.zustandStore).toBeNull();
    expect(result.read).toBeNull();
    expect(result.addresses.board()).toBe('board-address');
    expect(result.constants.tileCount).toBe(25);
    expect(result.math.double(3)).toBe(6);
  });
});
