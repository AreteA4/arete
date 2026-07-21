import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { OreDashboard } from './OreDashboard';

const mocks = vi.hoisted(() => ({
  board: undefined as Record<string, unknown> | undefined,
  round: undefined as Record<string, unknown> | undefined,
  treasury: undefined as Record<string, unknown> | undefined,
  miner: undefined as Record<string, unknown> | undefined,
  wallet: { connected: false, publicKey: null } as {
    connected: boolean;
    publicKey: { toBase58(): string } | null;
  },
  boardUse: vi.fn(),
  roundUse: vi.fn(),
  treasuryUse: vi.fn(),
  minerUse: vi.fn(),
  quoteUse: vi.fn(),
  retry: vi.fn(),
  useArete: vi.fn(),
  isConnected: true,
  connectionState: 'connected',
  connectionError: null as Error | null,
  socketIssue: null as { message: string } | null,
  boardLoading: false,
  boardRefreshing: false,
  boardError: null as Error | null,
  roundLoading: false,
  roundRefreshing: false,
  roundError: null as Error | null,
  treasuryLoading: false,
  treasuryRefreshing: false,
  treasuryError: null as Error | null,
  minerLoading: false,
  minerRefreshing: false,
  minerError: null as Error | null,
}));

vi.mock('@solana/wallet-adapter-react', () => ({
  useWallet: () => mocks.wallet,
}));

vi.mock('@solana/wallet-adapter-react-ui', () => ({
  useWalletModal: () => ({ setVisible: vi.fn() }),
}));

// Only `useArete` is mocked; pure helpers such as summarizeStatuses run their
// real implementations.
vi.mock('@usearete/react', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@usearete/react')>()),
  useArete: mocks.useArete,
}));

vi.mock('./ThemeToggle', () => ({
  ThemeToggle: () => null,
}));

function mutation() {
  return {
    mutate: vi.fn(),
    submit: vi.fn(),
    status: 'idle',
    phase: 'idle',
    isLoading: false,
    isAwaitingWallet: false,
    isReconciling: false,
    canRetryReconciliation: false,
    displayError: null,
    reconciliationError: null,
    signature: null,
    retryReconciliation: vi.fn(),
    reset: vi.fn(),
  };
}

describe('OreDashboard', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    const boardRefresh = vi.fn();
    const roundRefresh = vi.fn();
    const minerRefresh = vi.fn();
    mocks.board = { state: { roundId: 2n } };
    mocks.round = { id: { roundId: 2n } };
    mocks.treasury = undefined;
    mocks.miner = undefined;
    mocks.wallet = { connected: false, publicKey: null };
    mocks.isConnected = true;
    mocks.connectionState = 'connected';
    mocks.connectionError = null;
    mocks.socketIssue = null;
    mocks.boardLoading = false;
    mocks.boardRefreshing = false;
    mocks.boardError = null;
    mocks.roundLoading = false;
    mocks.roundRefreshing = false;
    mocks.roundError = null;
    mocks.treasuryLoading = false;
    mocks.treasuryRefreshing = false;
    mocks.treasuryError = null;
    mocks.minerLoading = false;
    mocks.minerRefreshing = false;
    mocks.minerError = null;
    mocks.retry.mockResolvedValue(undefined);
    mocks.boardUse.mockImplementation(() => ({
      data: mocks.board,
      status: mocks.boardError ? 'error' : mocks.boardLoading ? 'subscribing' : 'ready',
      isPending: mocks.boardLoading,
      isReady: !mocks.boardError && !mocks.boardLoading,
      isEmpty: !mocks.boardError && !mocks.boardLoading && mocks.board === undefined,
      isLoading: mocks.boardLoading,
      isRefreshing: mocks.boardRefreshing,
      error: mocks.boardError,
      refresh: boardRefresh,
    }));
    mocks.roundUse.mockImplementation((key?: { roundId: bigint }) => ({
      data: key?.roundId === (mocks.round?.id as { roundId?: bigint } | undefined)?.roundId
        ? mocks.round
        : undefined,
      status: key === undefined
        ? 'disabled'
        : mocks.roundError ? 'error' : mocks.roundLoading ? 'subscribing' : 'ready',
      isPending: key !== undefined && mocks.roundLoading,
      isReady: key !== undefined && !mocks.roundError && !mocks.roundLoading,
      isEmpty: key !== undefined && !mocks.roundError && !mocks.roundLoading && mocks.round === undefined,
      isLoading: mocks.roundLoading,
      isRefreshing: mocks.roundRefreshing,
      error: mocks.roundError,
      refresh: roundRefresh,
    }));
    mocks.treasuryUse.mockImplementation(() => ({
      data: mocks.treasury,
      status: mocks.treasuryError ? 'error' : mocks.treasuryLoading ? 'subscribing' : 'ready',
      isPending: mocks.treasuryLoading,
      isReady: !mocks.treasuryError && !mocks.treasuryLoading,
      isEmpty: !mocks.treasuryError && !mocks.treasuryLoading && mocks.treasury === undefined,
      isLoading: mocks.treasuryLoading,
      isRefreshing: mocks.treasuryRefreshing,
      error: mocks.treasuryError,
      refresh: vi.fn(),
    }));
    mocks.minerUse.mockImplementation(() => ({
      data: mocks.miner,
      status: mocks.minerError ? 'error' : mocks.minerLoading ? 'subscribing' : 'ready',
      isPending: mocks.minerLoading,
      isReady: !mocks.minerError && !mocks.minerLoading,
      isEmpty: !mocks.minerError && !mocks.minerLoading && mocks.miner === undefined,
      isLoading: mocks.minerLoading,
      isRefreshing: mocks.minerRefreshing,
      error: mocks.minerError,
      refresh: minerRefresh,
    }));
    mocks.quoteUse.mockReturnValue({
      data: undefined,
      error: null,
      status: 'disabled',
      isPending: false,
      isReady: false,
      isEmpty: false,
      isLoading: false,
      isRefreshing: false,
      refresh: vi.fn(),
    });
    mocks.useArete.mockImplementation(() => ({
      addresses: { board: () => 'board-address', treasury: () => 'treasury-address' },
      views: {
        OreBoard: { state: { use: mocks.boardUse } },
        OreRound: {
          state: { use: mocks.roundUse },
          latest: {
            use: () => ({
              data: [],
              error: null,
              isEmpty: true,
              isLoading: false,
              isRefreshing: false,
              refresh: vi.fn(),
            }),
          },
        },
        OreTreasury: { state: { use: mocks.treasuryUse } },
        OreMiner: { state: { use: mocks.minerUse } },
      },
      programs: {
        ore: {
          transactions: {
            mining: { deployWithCheckpoint: { useMutation: mutation } },
            rewards: { claimSolWithCheckpoint: { useMutation: mutation } },
          },
        },
      },
      read: {
        quoteManualDeployment: { use: mocks.quoteUse },
        solClaimPreview: {
          use: () => ({
            data: undefined,
            error: null,
            status: 'disabled',
            isPending: false,
            isReady: false,
            isEmpty: false,
            isLoading: false,
            isRefreshing: false,
            refresh: vi.fn(),
          }),
        },
      },
      reads: {},
      client: {},
      isConnected: mocks.isConnected,
      isLoading: mocks.connectionState === 'connecting',
      connectionState: mocks.connectionState,
      status: mocks.connectionState,
      canRetry: mocks.connectionState === 'error' || mocks.connectionState === 'disconnected',
      error: mocks.connectionError,
      socketIssue: mocks.socketIssue,
      retry: mocks.retry,
    }));
  });

  it('uses Board state as the sole authority for the current round subscription', () => {
    render(<OreDashboard />);

    expect(mocks.boardUse).toHaveBeenCalledWith({ address: 'board-address' });
    expect(mocks.roundUse).toHaveBeenCalledWith({ roundId: 2n });
    expect(screen.getByText(/Round 2/)).toBeInTheDocument();
  });

  it('keeps the round subscription disabled until Board state is available', () => {
    mocks.board = undefined;

    render(<OreDashboard />);

    expect(mocks.roundUse).toHaveBeenCalledWith(undefined);
    expect(screen.getByRole('status')).toHaveTextContent('Board state is unavailable.');
    expect(screen.getByText(/Round –/)).toBeInTheDocument();
  });

  it('distinguishes a missing Board-selected round from loading', () => {
    mocks.round = undefined;

    render(<OreDashboard />);

    expect(screen.getByRole('status')).toHaveTextContent('Round 2 is not indexed yet.');
  });

  it('surfaces a Board snapshot without an active round pointer', () => {
    mocks.board = { state: { roundId: null } };

    render(<OreDashboard />);

    expect(mocks.roundUse).toHaveBeenCalledWith(undefined);
    expect(screen.getByRole('status')).toHaveTextContent(
      'Board state does not identify an active round.',
    );
  });

  it('surfaces query errors without showing transient loading callouts', () => {
    mocks.boardLoading = true;
    mocks.treasuryError = new Error('treasury unavailable');

    render(<OreDashboard />);

    expect(screen.queryByText('Loading live state: Board.')).not.toBeInTheDocument();
    expect(screen.getByRole('alert')).toHaveTextContent(
      'Live data error: Treasury: treasury unavailable',
    );
    expect(screen.getByRole('button', { name: 'Connect wallet to continue' })).toBeInTheDocument();
  });

  it('moves the keyed round subscription when Board rolls over', () => {
    const { rerender } = render(<OreDashboard />);
    expect(screen.getByText(/Round 2/)).toBeInTheDocument();

    mocks.board = { state: { roundId: 3n } };
    mocks.round = { id: { roundId: 3n } };
    rerender(<OreDashboard />);

    expect(mocks.roundUse).toHaveBeenLastCalledWith({ roundId: 3n });
    expect(screen.getByText(/Round 3/)).toBeInTheDocument();
  });

  it('keeps motherlode live from Treasury when the new round join is empty', () => {
    mocks.round = {
      id: { roundId: 2n },
      treasury: { motherlode: 146.8 },
    };
    mocks.treasury = { state: { motherlode: 146.8 } };
    const { rerender } = render(<OreDashboard />);

    expect(mocks.treasuryUse).toHaveBeenCalledWith({ address: 'treasury-address' });
    expect(screen.getByText('146.8')).toBeInTheDocument();

    mocks.board = { state: { roundId: 3n } };
    mocks.round = {
      id: { roundId: 3n },
      treasury: { motherlode: null },
    };
    mocks.treasury = { state: { motherlode: 147.2 } };
    rerender(<OreDashboard />);

    expect(screen.getByText(/Round 3/)).toBeInTheDocument();
    expect(screen.getByText('147.2')).toBeInTheDocument();
  });

  it('counts down using the Board-selected round expiry', () => {
    const estimatedExpiresAtUnix = Math.floor(Date.now() / 1_000) + 60;
    mocks.round = {
      id: { roundId: 2n },
      state: { estimatedExpiresAtUnix: BigInt(estimatedExpiresAtUnix) },
    };

    render(<OreDashboard />);

    expect(screen.getByText(/^(01:00|00:59)$/)).toBeInTheDocument();
  });

  it('uses stack-provided per-square UI values from live round patches', () => {
    mocks.round = {
      id: { roundId: 2n },
      state: {
        deployedPerSquare: Array<bigint>(25).fill(0n),
        deployedPerSquareUi: Array<number>(25).fill(0),
      },
    };
    const { rerender } = render(<OreDashboard />);
    expect(screen.getByTestId('tile-1')).toHaveTextContent('0');

    mocks.round = {
      id: { roundId: 2n },
      state: {
        // The raw value deliberately differs so display cannot silently use it.
        deployedPerSquare: [9_000_000_000n, ...Array<bigint>(24).fill(0n)],
        deployedPerSquareUi: [1.25, ...Array<number>(24).fill(0)],
      },
    };
    rerender(<OreDashboard />);

    expect(screen.getByTestId('tile-1')).toHaveTextContent('1.2500');
  });

  it('prompts wallet connection when disconnected', () => {
    render(<OreDashboard />);
    expect(screen.getByRole('button', { name: 'Connect wallet to continue' })).toBeInTheDocument();
    expect(mocks.minerUse).toHaveBeenCalledWith(undefined);
  });

  it('highlights squares where the miner has a position in the Board round', () => {
    mocks.wallet = {
      connected: true,
      publicKey: { toBase58: () => 'wallet-address' },
    };
    mocks.board = { state: { roundId: 3n } };
    mocks.round = { id: { roundId: 3n } };
    mocks.miner = {
      state: {
        roundId: 3n,
        deployedPerSquareUi: [1, ...Array<number>(24).fill(0)],
        totalDeployed: 1,
      },
    };

    render(<OreDashboard />);

    expect(mocks.minerUse).toHaveBeenCalledWith({ authority: 'wallet-address' });
    expect(screen.getByTestId('tile-1')).toHaveClass('bg-sky-100', 'outline-sky-500');
  });

  it('highlights a square when the current-round miner snapshot updates live', () => {
    mocks.wallet = {
      connected: true,
      publicKey: { toBase58: () => 'wallet-address' },
    };
    const { rerender } = render(<OreDashboard />);
    expect(screen.getByTestId('tile-1')).not.toHaveClass('bg-sky-100');

    mocks.miner = {
      state: {
        roundId: 2n,
        // The raw value deliberately differs so grid positions use the UI field.
        deployedPerSquare: [9_000_000_000n, ...Array<bigint>(24).fill(0n)],
        deployedPerSquareUi: [1.25, ...Array<number>(24).fill(0)],
        totalDeployed: 1.25,
      },
    };
    rerender(<OreDashboard />);

    expect(screen.getByTestId('tile-1')).toHaveClass('bg-sky-100', 'outline-sky-500');
    expect(screen.getByRole('button', { name: /Square 1,/ })).toHaveAccessibleName(
      /1.25 SOL yours.*your current position/,
    );
  });

  it('does not render a miner deployment from a previous Board round', () => {
    mocks.wallet = {
      connected: true,
      publicKey: { toBase58: () => 'wallet-address' },
    };
    mocks.board = { state: { roundId: 3n } };
    mocks.round = { id: { roundId: 3n } };
    mocks.miner = {
      state: {
        roundId: 2n,
        deployedPerSquareUi: [1, ...Array<number>(24).fill(0)],
        totalDeployed: 1,
      },
    };

    render(<OreDashboard />);

    expect(screen.getByTestId('tile-1')).not.toHaveClass('bg-sky-100');
  });

  it('surfaces a connection retry without changing the badge status', async () => {
    const user = userEvent.setup();
    mocks.isConnected = false;
    mocks.connectionState = 'error';
    mocks.connectionError = new Error('socket unavailable');

    render(<OreDashboard />);
    expect(screen.getByText('Offline')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Retry' }));
    expect(mocks.retry).toHaveBeenCalledOnce();
  });

  it('shows automatic reconnection without offering a competing manual retry', () => {
    mocks.isConnected = false;
    mocks.connectionState = 'reconnecting';

    render(<OreDashboard />);

    expect(screen.getByText('Reconnecting')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Retry' })).not.toBeInTheDocument();
  });

  it('keeps background stream resynchronization visually quiet', () => {
    mocks.boardRefreshing = true;
    mocks.roundRefreshing = true;

    render(<OreDashboard />);

    expect(screen.queryByText(/Resynchronizing live state/)).not.toBeInTheDocument();
  });

  it('surfaces non-fatal socket issues while the connection remains active', () => {
    mocks.socketIssue = { message: 'subscription limit reached' };

    render(<OreDashboard />);

    expect(screen.getByText('Connected with issue')).toBeInTheDocument();
    expect(screen.getByRole('alert')).toHaveTextContent(
      'Stream issue: subscription limit reached',
    );
  });
});
