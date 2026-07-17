import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { OreDashboard } from './OreDashboard';

const mocks = vi.hoisted(() => ({
  clock: vi.fn(),
  claimPrepare: vi.fn(),
  checkpointPrepare: vi.fn(),
  execute: vi.fn(),
  inspectOperation: vi.fn(),
  readAutomation: vi.fn(),
  readBoard: vi.fn(),
  readMiner: vi.fn(),
  readRound: vi.fn(),
  readSolClaimPreview: vi.fn(),
  waitForProcessedSlot: vi.fn(),
  latestRound: undefined as Record<string, unknown> | undefined,
  miner: undefined as Record<string, unknown> | undefined,
  wallet: { connected: false, publicKey: null } as {
    connected: boolean;
    publicKey: { toBase58(): string } | null;
  },
  useArete: vi.fn(),
}));

vi.mock('@solana/wallet-adapter-react', () => ({
  useWallet: () => mocks.wallet,
}));

vi.mock('@solana/wallet-adapter-react-ui', () => ({
  useWalletModal: () => ({ setVisible: vi.fn() }),
}));

vi.mock('@usearete/react', () => ({
  getTransactionFailureOutcome: vi.fn(),
  useArete: mocks.useArete,
}));

vi.mock('./ThemeToggle', () => ({
  ThemeToggle: () => null,
}));

describe('OreDashboard rollovers', () => {
  beforeEach(() => {
    const refresh = vi.fn();
    mocks.clock.mockReset();
    mocks.claimPrepare.mockReset();
    mocks.checkpointPrepare.mockReset();
    mocks.execute.mockReset();
    mocks.inspectOperation.mockReset();
    mocks.readAutomation.mockReset();
    mocks.readBoard.mockReset();
    mocks.readMiner.mockReset();
    mocks.readRound.mockReset();
    mocks.readSolClaimPreview.mockReset();
    mocks.waitForProcessedSlot.mockReset();
    mocks.latestRound = { id: { roundId: 2n } };
    mocks.miner = undefined;
    mocks.wallet = { connected: false, publicKey: null };
    mocks.useArete.mockReturnValue({
      addresses: { board: () => 'board-address' },
      chain: { clock: mocks.clock },
      views: {
        OreBoard: {
          state: {
            use: () => ({ data: { state: { roundId: 1n } }, refresh }),
          },
        },
        OreRound: {
          latest: {
            useOne: () => ({ data: mocks.latestRound, refresh }),
          },
        },
        OreMiner: {
          state: {
            use: () => ({ data: mocks.miner, refresh }),
          },
        },
      },
      client: {
        execute: mocks.execute,
        inspectOperation: mocks.inspectOperation,
        waitForProcessedSlot: mocks.waitForProcessedSlot,
      },
      read: {
        automation: mocks.readAutomation,
        board: mocks.readBoard,
        miner: mocks.readMiner,
        round: mocks.readRound,
        solClaimPreview: mocks.readSolClaimPreview,
      },
      math: { miner: {} },
      programs: {
        ore: {
          instructions: {
            miner: {
              checkpoint: { prepare: mocks.checkpointPrepare },
            },
            rewards: {
              claimSol: { prepare: mocks.claimPrepare },
            },
          },
        },
      },
      isConnected: true,
      connectionState: 'connected',
      error: null,
    });
  });

  it('renders a newly promoted round before its sections arrive', () => {
    render(<OreDashboard />);

    expect(screen.getByText(/Round 2/)).toBeInTheDocument();
    expect(screen.getByText('0.00')).toBeInTheDocument();
    expect(screen.getByText('–')).toBeInTheDocument();
  });

  it('counts down the stack-provided estimated expiry without a chain read', async () => {
    const estimatedExpiresAtUnix = Math.floor(Date.now() / 1_000) + 60;
    mocks.latestRound = {
      id: { roundId: 2n },
      state: {
        estimatedExpiresAtUnix: BigInt(estimatedExpiresAtUnix),
        expiresAt: 1_150n,
      },
    };

    render(<OreDashboard />);

    await waitFor(() => {
      expect(screen.getByText(/^(01:00|00:59)$/)).toBeInTheDocument();
    });
    expect(mocks.clock).not.toHaveBeenCalled();
  });

  it('does not render quote or selection text below the principal input', async () => {
    const user = userEvent.setup();
    render(<OreDashboard />);

    await user.click(screen.getByTestId('tile-1'));

    expect(screen.queryByText(/Selected:|Choose squares|lamports\/tile|SOL allocated|remainder/)).not.toBeInTheDocument();
  });

  it('clears deployment form errors when the round changes', async () => {
    const user = userEvent.setup();
    mocks.wallet = {
      connected: true,
      publicKey: { toBase58: () => 'wallet-address' },
    };
    const { rerender } = render(<OreDashboard />);

    await user.click(screen.getByTestId('tile-1'));
    await user.click(screen.getByRole('button', { name: 'Deploy' }));
    expect(await screen.findByText('The ORE Board account is unavailable.')).toBeInTheDocument();

    mocks.latestRound = { id: { roundId: 3n } };
    rerender(<OreDashboard />);

    await waitFor(() => {
      expect(screen.queryByText('The ORE Board account is unavailable.')).not.toBeInTheDocument();
    });
  });

  it('does not apply an old deployment array to a newly streamed miner round', async () => {
    mocks.latestRound = { id: { roundId: 3n } };
    mocks.miner = { state: { roundId: 3n } };
    mocks.wallet = {
      connected: true,
      publicKey: { toBase58: () => 'wallet-address' },
    };
    mocks.readMiner.mockResolvedValue({
      checkpointId: 2n,
      roundId: 2n,
      rewardsSol: 0n,
      deployed: [1_000_000_000n, ...Array<bigint>(24).fill(0n)],
    });
    mocks.readBoard.mockResolvedValue({ roundId: 3n });
    mocks.readAutomation.mockResolvedValue(null);
    mocks.clock.mockResolvedValue({ slot: 100 });

    render(<OreDashboard />);

    await waitFor(() => expect(mocks.readMiner).toHaveBeenCalled());
    expect(screen.getByTestId('tile-1')).not.toHaveClass('bg-sky-100', 'outline-sky-500');
  });

  it('shows and claims pending SOL rewards', async () => {
    const user = userEvent.setup();
    const prepared = { name: 'claimSol' };
    mocks.wallet = {
      connected: true,
      publicKey: { toBase58: () => 'wallet-address' },
    };
    mocks.miner = {
      rewards: { rewardsSol: 1_250_000_000n },
      state: { roundId: 2n },
    };
    mocks.claimPrepare.mockResolvedValue(prepared);
    mocks.inspectOperation.mockResolvedValue({ transaction: {} });
    mocks.execute.mockResolvedValue({
      kind: 'instruction',
      signatures: ['claim-signature'],
      transaction: { slot: 123 },
    });
    mocks.waitForProcessedSlot.mockResolvedValue(123n);

    render(<OreDashboard />);

    expect(screen.getByText('1.25 SOL')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Claim SOL' }));

    await waitFor(() => {
      expect(mocks.claimPrepare).toHaveBeenCalledWith({ authority: 'wallet-address' });
      expect(mocks.execute).toHaveBeenCalledWith(prepared);
      expect(screen.queryByLabelText('Claimable SOL rewards')).not.toBeInTheDocument();
    });
  });

  it('checkpoints an unresolved win before claiming its SOL', async () => {
    const user = userEvent.setup();
    mocks.wallet = {
      connected: true,
      publicKey: { toBase58: () => 'winning-wallet' },
    };
    mocks.readMiner.mockResolvedValue({
      checkpointId: 10n,
      roundId: 11n,
      rewardsSol: 0n,
      deployed: Array(25).fill(1n),
    });
    mocks.readBoard.mockResolvedValue({ roundId: 12n });
    mocks.readAutomation.mockResolvedValue(null);
    mocks.readRound.mockResolvedValue({ id: 11n });
    mocks.clock.mockResolvedValue({ slot: 100 });
    mocks.readSolClaimPreview.mockResolvedValue({
      checkpointedRewardsSol: 0n,
      unresolvedRewardsSol: 500_000_000n,
      totalClaimableSol: 500_000_000n,
      checkpoint: null,
      action: 'checkpointAndClaim',
    });
    mocks.checkpointPrepare.mockResolvedValue({
      kind: 'instruction',
      name: 'checkpoint',
      transaction: {
        instructions: [{ programId: '11111111111111111111111111111111', keys: [], data: new Uint8Array() }],
        requiredSignerAddresses: [],
        errors: [],
      },
    });
    mocks.claimPrepare.mockResolvedValue({
      kind: 'instruction',
      name: 'claimSol',
      transaction: {
        instructions: [{ programId: '11111111111111111111111111111111', keys: [], data: new Uint8Array() }],
        requiredSignerAddresses: [],
        errors: [],
      },
    });
    mocks.inspectOperation.mockResolvedValue({ transaction: {} });
    mocks.execute.mockResolvedValue({
      kind: 'transaction',
      signatures: ['checkpoint-claim-signature'],
      transaction: { slot: 456 },
    });
    mocks.waitForProcessedSlot.mockResolvedValue(456n);

    render(<OreDashboard />);

    expect(await screen.findByText('0.5 SOL')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Claim SOL' }));

    await waitFor(() => {
      expect(mocks.checkpointPrepare).toHaveBeenCalledWith({
        signer: 'winning-wallet',
        authority: 'winning-wallet',
      });
      expect(mocks.claimPrepare).toHaveBeenCalledWith({ authority: 'winning-wallet' });
      expect(mocks.execute.mock.calls[0]?.[0]).toMatchObject({
        kind: 'transaction',
        name: 'checkpointAndClaimSol',
      });
    });
  });
});
