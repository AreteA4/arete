import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { RecentRounds } from './RecentRounds';

const mocks = vi.hoisted(() => ({
  result: {} as {
    data: Array<Record<string, Record<string, unknown>>> | undefined;
    isLoading: boolean;
    isRefreshing: boolean;
    error: Error | null;
    isEmpty: boolean;
    refresh: ReturnType<typeof vi.fn>;
  },
}));

vi.mock('@usearete/react', () => ({
  useArete: () => ({
    views: { OreRound: { latest: { use: () => mocks.result } } },
  }),
}));

describe('RecentRounds', () => {
  beforeEach(() => {
    mocks.result = {
      data: undefined,
      isLoading: true,
      isRefreshing: false,
      error: null,
      isEmpty: false,
      refresh: vi.fn().mockResolvedValue(undefined),
    };
  });

  it('distinguishes loading, errors, empty snapshots, and populated history', async () => {
    const user = userEvent.setup();
    const { rerender } = render(<RecentRounds />);
    expect(screen.getByRole('status')).toHaveTextContent('Loading recent rounds');

    mocks.result = {
      ...mocks.result,
      data: undefined,
      isLoading: false,
      error: new Error('history unavailable'),
      isEmpty: false,
    };
    rerender(<RecentRounds />);
    expect(screen.getByRole('alert')).toHaveTextContent(
      'Unable to load recent rounds: history unavailable',
    );
    await user.click(screen.getByRole('button', { name: 'Retry recent rounds' }));
    expect(mocks.result.refresh).toHaveBeenCalledOnce();

    mocks.result = { ...mocks.result, data: [], isLoading: false, error: null, isEmpty: true };
    rerender(<RecentRounds />);
    expect(screen.getByText('No recent rounds yet.')).toBeInTheDocument();

    mocks.result = {
      data: [{
        id: { roundId: 42n },
        results: { winningSquare: 3n },
        state: { totalDeployed: 1.5 },
      }],
      isLoading: false,
      isRefreshing: false,
      error: null,
      isEmpty: false,
      refresh: mocks.result.refresh,
    };
    rerender(<RecentRounds />);
    expect(screen.getByText('Round 42')).toBeInTheDocument();
    expect(screen.getByText('Square 4 won')).toBeInTheDocument();
    expect(screen.getByText('1.50 SOL')).toBeInTheDocument();
  });
});
