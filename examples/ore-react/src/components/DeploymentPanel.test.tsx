import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ManualDeploymentQuote, SquareIndex } from '../generated/ore-stack';
import { DeploymentPanel } from './DeploymentPanel';

const mocks = vi.hoisted(() => ({
  wallet: { publicKey: { toBase58: () => 'wallet-address' } } as {
    publicKey: { toBase58(): string } | null;
  },
  showWalletModal: vi.fn(),
  quoteUse: vi.fn(),
  quoteRoundId: 42n as bigint,
  quoteResult: {} as {
    data: ManualDeploymentQuote | undefined;
    error: Error | null;
    isLoading: boolean;
    isRefreshing: boolean;
    isPending: boolean;
    isReady: boolean;
    isEmpty: boolean;
    status: string;
    refresh: ReturnType<typeof vi.fn>;
  },
  boardResult: { data: undefined as unknown, refresh: vi.fn() },
  roundResult: { data: undefined as unknown, refresh: vi.fn() },
  minerResult: { data: undefined as unknown, refresh: vi.fn() },
  boardView: { use: vi.fn(), refresh: vi.fn() },
  roundView: { use: vi.fn(), refresh: vi.fn() },
  minerView: { use: vi.fn(), refresh: vi.fn() },
  submit: vi.fn(),
  reset: vi.fn(),
  retryReconciliation: vi.fn(),
  mutation: {} as {
    mutate: ReturnType<typeof vi.fn>;
    submit: ReturnType<typeof vi.fn>;
    reset: ReturnType<typeof vi.fn>;
    isLoading: boolean;
    phase: string;
    isAwaitingWallet: boolean;
    isReconciling: boolean;
    canRetryReconciliation: boolean;
    displayError: string | null;
    reconciliationError: Error | null;
    signature: string | null;
    retryReconciliation: ReturnType<typeof vi.fn>;
  },
  useArete: vi.fn(),
  isConnected: true,
}));

vi.mock('@solana/wallet-adapter-react', () => ({
  useWallet: () => mocks.wallet,
}));

vi.mock('@solana/wallet-adapter-react-ui', () => ({
  useWalletModal: () => ({ setVisible: mocks.showWalletModal }),
}));

vi.mock('@usearete/react', () => ({
  safeToRawAmount: (amount: { ui: string | number }, decimals: number) => {
    const value = String(amount.ui);
    if (!/^\d+(?:\.\d+)?$/.test(value)) {
      return { success: false, error: new Error(`Invalid UI amount: ${value}`) };
    }
    const [whole, fraction = ''] = value.split('.');
    return {
      success: true,
      data: BigInt(whole) * 10n ** BigInt(decimals)
        + BigInt(fraction.padEnd(decimals, '0').slice(0, decimals) || '0'),
    };
  },
  useArete: mocks.useArete,
}));

function quote(overrides: Partial<ManualDeploymentQuote> = {}): ManualDeploymentQuote {
  return {
    roundId: 42n,
    totalPrincipal: 10_000_000n,
    requestedSquares: [0, 2],
    effectiveSquares: [0, 2],
    alreadyDeployedSquares: [],
    existingMinerDeployment: Array<bigint>(25).fill(0n),
    requestedSquareMask: 5n,
    effectiveSquareMask: 5n,
    requestedSquareCount: 2,
    effectiveSquareCount: 2,
    amountPerSquare: 5_000_000n,
    allocatedPrincipal: 10_000_000n,
    roundingRemainder: 0n,
    maximumDeploymentTransfer: 10_000_000n,
    unspentPrincipal: 0n,
    checkpointReserve: 10_000n,
    checkpointReserveUi: '0.00001',
    maximumWalletDebit: 10_010_000n,
    hasActiveAutomation: false,
    requiresDisableBeforeDeployment: false,
    includesNetworkFee: false,
    includesAccountRent: false,
    ...overrides,
  };
}

function renderPanel(overrides: Partial<{
  currentRoundId: bigint | undefined;
  selected: readonly SquareIndex[];
  onDeployed: () => void;
}> = {}) {
  const props = {
    currentRoundId: 42n as bigint | undefined,
    selected: [0, 2] as readonly SquareIndex[],
    onDeployed: vi.fn(),
    ...overrides,
  };
  return { ...render(<DeploymentPanel {...props} />), props };
}

describe('DeploymentPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.wallet = { publicKey: { toBase58: () => 'wallet-address' } };
    mocks.isConnected = true;
    mocks.quoteRoundId = 42n;
    mocks.quoteResult = {
      data: quote(),
      error: null,
      isLoading: false,
      isRefreshing: false,
      isPending: false,
      isReady: true,
      isEmpty: false,
      status: 'ready',
      refresh: vi.fn().mockResolvedValue(undefined),
    };
    mocks.mutation = {
      mutate: mocks.submit,
      submit: mocks.submit,
      reset: mocks.reset,
      isLoading: false,
      phase: 'idle',
      isAwaitingWallet: false,
      isReconciling: false,
      canRetryReconciliation: false,
      displayError: null,
      reconciliationError: null,
      signature: null,
      retryReconciliation: mocks.retryReconciliation,
    };
    mocks.submit.mockResolvedValue({});
    mocks.retryReconciliation.mockResolvedValue(undefined);
    mocks.boardResult = { data: undefined, refresh: vi.fn() };
    mocks.roundResult = { data: undefined, refresh: vi.fn() };
    mocks.minerResult = { data: undefined, refresh: vi.fn() };
    mocks.boardView.use.mockImplementation(() => mocks.boardResult);
    mocks.roundView.use.mockImplementation(() => mocks.roundResult);
    mocks.minerView.use.mockImplementation(() => mocks.minerResult);
    // Mimic the SDK contract: a read only exposes data fetched for the
    // arguments it was called with; anything else is reported as loading.
    mocks.quoteUse.mockImplementation((
      input: { roundId?: bigint } | null,
      _options: { debounceMs: number },
    ) => {
      if (input == null) {
        return {
          ...mocks.quoteResult,
          data: undefined,
          isEmpty: false,
          isPending: false,
          isReady: false,
          isLoading: false,
          status: 'disabled',
        };
      }
      return input.roundId === mocks.quoteRoundId
        ? mocks.quoteResult
        : {
            ...mocks.quoteResult,
            data: undefined,
            isEmpty: false,
            isPending: true,
            isReady: false,
            isLoading: true,
            status: 'loading',
          };
    });
    mocks.useArete.mockImplementation(() => ({
      addresses: { board: () => 'board-address' },
      views: {
        OreBoard: { state: mocks.boardView },
        OreRound: { state: mocks.roundView },
        OreMiner: { state: mocks.minerView },
      },
      read: { quoteManualDeployment: { use: mocks.quoteUse } },
      programs: {
        ore: {
          transactions: {
            mining: {
              deployWithCheckpoint: {
                useMutation: () => mocks.mutation,
              },
            },
          },
        },
      },
      isConnected: mocks.isConnected,
    }));
  });

  it('quotes the Board-authoritative round with wallet, principal, and selected squares', () => {
    renderPanel();

    expect(mocks.quoteUse).toHaveBeenCalledWith(
      {
        authority: 'wallet-address',
        roundId: 42n,
        totalPrincipal: 10_000_000n,
        selectedSquares: [0, 2],
      },
      { debounceMs: 300 },
    );
    expect(screen.getByText('2 of 2 selected squares available for this deployment.')).toBeInTheDocument();
    expect(screen.getByText(/0.00001 SOL for the checkpoint reserve/)).toBeInTheDocument();
  });

  it('invalidates the old quote and requests the new Board round on rollover', () => {
    const { rerender, props } = renderPanel();

    rerender(<DeploymentPanel {...props} currentRoundId={43n} />);

    expect(mocks.quoteUse).toHaveBeenLastCalledWith(
      expect.objectContaining({ roundId: 43n }),
      { debounceMs: 300 },
    );
    expect(screen.getByRole('button', { name: 'Checking deployment…' })).toBeDisabled();
  });

  it('uses effective squares and an explicit quoted round, then clears only on reconciled success', async () => {
    const user = userEvent.setup();
    mocks.quoteResult.data = quote({
      effectiveSquares: [2],
      alreadyDeployedSquares: [0],
      effectiveSquareMask: 4n,
      effectiveSquareCount: 1,
      maximumDeploymentTransfer: 5_000_000n,
      unspentPrincipal: 5_000_000n,
      checkpointReserve: 0n,
      checkpointReserveUi: '0',
      maximumWalletDebit: 5_000_000n,
    });
    const { props } = renderPanel();

    expect(screen.getByText(/Existing positions on 1 selected square are excluded/)).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Deploy' }));

    expect(mocks.submit).toHaveBeenCalledWith(
      {
        signer: 'wallet-address',
        roundId: 42n,
        squares: [2],
        amountPerSquare: 5_000_000n,
      },
      expect.objectContaining({
        reconcile: {
          refresh: [mocks.boardView, mocks.roundView, mocks.minerView, mocks.quoteResult],
        },
        onSuccess: props.onDeployed,
      }),
    );
    expect(props.onDeployed).not.toHaveBeenCalled();

    const options = mocks.submit.mock.calls[0]?.[1] as { onSuccess(): void };
    options.onSuccess();
    expect(props.onDeployed).toHaveBeenCalledOnce();
  });

  it('blocks manual deployment while active automation requires disabling', async () => {
    const user = userEvent.setup();
    mocks.quoteResult.data = quote({
      hasActiveAutomation: true,
      requiresDisableBeforeDeployment: true,
    });
    renderPanel();

    expect(screen.getByRole('alert')).toHaveTextContent(
      'Disable active automation before deploying manually.',
    );
    const button = screen.getByRole('button', { name: 'Deploy' });
    expect(button).toBeDisabled();
    await user.click(button);
    expect(mocks.submit).not.toHaveBeenCalled();
  });

  it('disables submission while the quote is loading and surfaces quote errors', () => {
    mocks.quoteResult = {
      data: undefined,
      error: null,
      isLoading: true,
      isRefreshing: false,
      isPending: true,
      isReady: false,
      isEmpty: false,
      status: 'loading',
      refresh: vi.fn(),
    };
    const { rerender, props } = renderPanel();

    expect(screen.getByRole('button', { name: 'Checking deployment…' })).toBeDisabled();
    expect(screen.getByText('Checking current deployment…')).toBeInTheDocument();

    mocks.quoteResult = {
      ...mocks.quoteResult,
      error: new Error('quote unavailable'),
      isLoading: false,
      status: 'error',
      isPending: false,
      isReady: false,
    };
    rerender(<DeploymentPanel {...props} />);
    expect(screen.getByRole('alert')).toHaveTextContent('quote unavailable');
    expect(screen.getByRole('button', { name: 'Deploy' })).toBeDisabled();
  });

  it('does not submit retained quote data after a refresh error', async () => {
    const user = userEvent.setup();
    mocks.quoteResult = {
      ...mocks.quoteResult,
      data: quote(),
      error: new Error('quote refresh failed'),
      status: 'error',
      isReady: false,
    };
    renderPanel();

    const button = screen.getByRole('button', { name: 'Deploy' });
    expect(button).toBeDisabled();
    await user.click(button);
    expect(mocks.submit).not.toHaveBeenCalled();
  });

  it('shows reconciliation as busy and requires a fresh quote after synchronization errors', async () => {
    const user = userEvent.setup();
    const onDeployed = vi.fn();
    mocks.mutation = {
      ...mocks.mutation,
      isLoading: true,
      phase: 'reconciling',
      isReconciling: true,
    };
    const { rerender, props } = renderPanel({ onDeployed });
    expect(screen.getByRole('button', { name: 'Syncing…' })).toBeDisabled();

    mocks.mutation = {
      ...mocks.mutation,
      isLoading: false,
      phase: 'confirmed-unreconciled',
      isReconciling: false,
      canRetryReconciliation: true,
      reconciliationError: new Error('stream timeout'),
      signature: 'confirmed-signature',
    };
    mocks.quoteResult = { ...mocks.quoteResult, isRefreshing: true };
    rerender(<DeploymentPanel {...props} />);

    expect(screen.getByRole('alert')).toHaveTextContent(
      'Deployment confirmed, but live data could not be synchronized: stream timeout',
    );
    expect(screen.getByRole('link', { name: /View confirme.*on explorer/ })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Checking deployment…' })).toBeDisabled();
    expect(onDeployed).not.toHaveBeenCalled();

    await user.clear(screen.getByLabelText('Total principal in SOL'));
    await user.type(screen.getByLabelText('Total principal in SOL'), '0.010');
    expect(mocks.reset).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: 'Checking deployment…' })).toBeDisabled();

    const retry = screen.getByRole('button', { name: 'Retry synchronization' });
    expect(retry).toBeEnabled();
    await user.click(retry);
    expect(mocks.retryReconciliation).toHaveBeenCalledOnce();
    expect(mocks.quoteResult.refresh).not.toHaveBeenCalled();
    expect(mocks.reset).not.toHaveBeenCalled();
    expect(mocks.submit).not.toHaveBeenCalled();
  });

  it('surfaces deployment errors without clearing selected squares', () => {
    const onDeployed = vi.fn();
    mocks.mutation = {
      ...mocks.mutation,
      displayError: 'wallet rejected transaction',
    };
    renderPanel({ onDeployed });

    expect(screen.getByRole('alert')).toHaveTextContent('wallet rejected transaction');
    expect(onDeployed).not.toHaveBeenCalled();
  });
});
