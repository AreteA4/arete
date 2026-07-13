jest.mock('react', () => ({
  useState: jest.fn(),
  useCallback: jest.fn((callback: unknown) => callback),
}));
jest.mock('@usearete/sdk', () => ({
  InstructionError: class InstructionError extends Error {},
}));

import { useState } from 'react';

import { useInstructionMutation } from './use-mutation';

const mockUseState = useState as jest.Mock;

function renderMutation<TResult>(execute: () => Promise<TResult>) {
  const setStatus = jest.fn();
  const setError = jest.fn();
  const setSignatures = jest.fn();
  const setSignature = jest.fn();

  mockUseState
    .mockImplementationOnce((value: unknown) => [value, setStatus])
    .mockImplementationOnce((value: unknown) => [value, setError])
    .mockImplementationOnce((value: unknown) => [value, setSignatures])
    .mockImplementationOnce((value: unknown) => [value, setSignature]);

  return {
    mutation: useInstructionMutation(execute),
    setSignatures,
    setSignature,
  };
}

describe('useInstructionMutation signatures', () => {
  beforeEach(() => {
    mockUseState.mockReset();
  });

  it('normalizes ordered operation signatures and only exposes a singular signature for one', async () => {
    const multi = renderMutation(async () => ({ signatures: ['sig-1', 'sig-2'] }));
    await multi.mutation.submit(undefined);

    expect(multi.setSignatures).toHaveBeenLastCalledWith(['sig-1', 'sig-2']);
    expect(multi.setSignature).toHaveBeenLastCalledWith(null);

    const single = renderMutation(async () => ({ signatures: ['sig-only'] }));
    await single.mutation.submit(undefined);
    expect(single.setSignatures).toHaveBeenLastCalledWith(['sig-only']);
    expect(single.setSignature).toHaveBeenLastCalledWith('sig-only');
  });

  it('normalizes raw transaction signatures and resets both forms', async () => {
    const { mutation, setSignatures, setSignature } = renderMutation(async () => ({
      signature: 'raw-signature',
    }));
    await mutation.submit(undefined);

    expect(setSignatures).toHaveBeenLastCalledWith(['raw-signature']);
    expect(setSignature).toHaveBeenLastCalledWith('raw-signature');

    mutation.reset();
    expect(setSignatures).toHaveBeenLastCalledWith([]);
    expect(setSignature).toHaveBeenLastCalledWith(null);
  });
});
