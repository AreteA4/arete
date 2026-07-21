import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { RewardsPanel } from './RewardsPanel';

const mocks = vi.hoisted(() => ({
  isConnected: true,
  wallet: { publicKey: { toBase58: () => 'wallet-address' } } as {
    publicKey: { toBase58(): string } | null;
  },
  preview: {} as {
    data: { totalClaimableSol: bigint; totalClaimableSolUi: string; action: string } | null | undefined;
    error: Error | null;
    status: string;
    isPending: boolean;
    isReady: boolean;
    isEmpty: boolean;
    isLoading: boolean;
    isRefreshing: boolean;
    refresh: ReturnType<typeof vi.fn>;
  },
  claim: {} as {
    mutate: ReturnType<typeof vi.fn>;
    submit: ReturnType<typeof vi.fn>;
    reset: ReturnType<typeof vi.fn>;
    retryReconciliation: ReturnType<typeof vi.fn>;
    phase: string;
    isLoading: boolean;
    isReconciling: boolean;
    canRetryReconciliation: boolean;
    displayError: string | null;
    reconciliationError: Error | null;
    signature: string | null;
  },
}));

vi.mock('@solana/wallet-adapter-react', () => ({
  useWallet: () => mocks.wallet,
}));

vi.mock('@usearete/react', () => ({
  useArete: () => ({
    isConnected: mocks.isConnected,
    read: { solClaimPreview: { use: () => mocks.preview } },
    programs: {
      ore: {
        transactions: {
          rewards: { claimSolWithCheckpoint: { useMutation: () => mocks.claim } },
        },
      },
    },
  }),
}));

describe('RewardsPanel', () => {
  beforeEach(() => {
    mocks.isConnected = true;
    mocks.wallet = { publicKey: { toBase58: () => 'wallet-address' } };
    mocks.preview = {
      data: { totalClaimableSol: 0n, totalClaimableSolUi: '0', action: 'none' },
      error: null,
      status: 'ready',
      isPending: false,
      isReady: true,
      isEmpty: false,
      isLoading: false,
      isRefreshing: false,
      refresh: vi.fn().mockResolvedValue(undefined),
    };
    mocks.claim = {
      mutate: vi.fn(),
      submit: vi.fn(),
      reset: vi.fn(),
      retryReconciliation: vi.fn().mockResolvedValue(undefined),
      phase: 'idle',
      isLoading: false,
      isReconciling: false,
      canRetryReconciliation: false,
      displayError: null,
      reconciliationError: null,
      signature: null,
    };
  });

  it('stays hidden until there are SOL rewards to claim', () => {
    mocks.wallet = { publicKey: null };
    mocks.preview = {
      ...mocks.preview,
      data: undefined,
      status: 'disabled',
      isReady: false,
    };

    render(<RewardsPanel />);

    expect(screen.queryByRole('region', { name: 'Claimable SOL rewards' })).not.toBeInTheDocument();
  });

  it('stays hidden when the claimable SOL amount is zero', () => {
    mocks.preview = {
      ...mocks.preview,
      data: { totalClaimableSol: 0n, totalClaimableSolUi: '0', action: 'none' },
    };

    render(<RewardsPanel />);

    expect(screen.queryByRole('region', { name: 'Claimable SOL rewards' })).not.toBeInTheDocument();
  });

  it('shows the card when there are SOL rewards to claim', () => {
    mocks.preview = {
      ...mocks.preview,
      data: { totalClaimableSol: 1n, totalClaimableSolUi: '0.000000001', action: 'claim' },
    };

    render(<RewardsPanel />);

    expect(screen.getByRole('region', { name: 'Claimable SOL rewards' })).toBeInTheDocument();
    expect(screen.getByText('0.000000001 SOL')).toBeInTheDocument();
  });

  it('keeps confirmed-unreconciled claims visible and retries without resubmitting', async () => {
    const user = userEvent.setup();
    mocks.claim = {
      ...mocks.claim,
      phase: 'confirmed-unreconciled',
      reconciliationError: new Error('preview refresh failed'),
      canRetryReconciliation: true,
      signature: 'confirmed-signature',
    };
    mocks.preview = {
      ...mocks.preview,
      data: { totalClaimableSol: 1n, totalClaimableSolUi: '0.000000001', action: 'claim' },
      isRefreshing: true,
    };
    render(<RewardsPanel />);

    expect(screen.getByRole('alert')).toHaveTextContent(
      'Claim confirmed, but live rewards could not be synchronized: preview refresh failed',
    );
    expect(screen.getByRole('link', { name: /View confirme.*on explorer/ })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Claim SOL' })).toBeDisabled();

    const retry = screen.getByRole('button', { name: 'Retry synchronization' });
    expect(retry).toBeEnabled();
    await user.click(retry);
    await waitFor(() => expect(mocks.claim.retryReconciliation).toHaveBeenCalledOnce());
    expect(mocks.claim.submit).not.toHaveBeenCalled();
    expect(mocks.claim.reset).not.toHaveBeenCalled();
    expect(mocks.preview.refresh).not.toHaveBeenCalled();
  });

  it('does not claim from retained preview data after a refresh error', async () => {
    const user = userEvent.setup();
    mocks.preview = {
      ...mocks.preview,
      data: { totalClaimableSol: 1n, totalClaimableSolUi: '0.000000001', action: 'claim' },
      error: new Error('preview refresh failed'),
      status: 'error',
      isReady: false,
    };
    render(<RewardsPanel />);

    const button = screen.getByRole('button', { name: 'Claim SOL' });
    expect(button).toBeDisabled();
    await user.click(button);
    expect(mocks.claim.mutate).not.toHaveBeenCalled();
  });

  it('does not start a claim while the stream is disconnected', () => {
    mocks.isConnected = false;
    mocks.preview = {
      ...mocks.preview,
      data: { totalClaimableSol: 1n, totalClaimableSolUi: '0.000000001', action: 'claim' },
    };
    render(<RewardsPanel />);

    expect(screen.getByRole('button', { name: 'Claim SOL' })).toBeDisabled();
  });
});
