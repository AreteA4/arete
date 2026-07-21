import { act, render, screen } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { StatsPanel } from './StatsPanel';

function makeRound(overrides: {
  roundId?: bigint;
  estimatedExpiresAtUnix?: bigint | null;
  totalDeployed?: number | null;
  motherlode?: number | null;
  totalMiners?: bigint | null;
} = {}): ComponentProps<typeof StatsPanel> {
  return {
    roundId: overrides.roundId?.toString(),
    estimatedExpiresAtUnix: overrides.estimatedExpiresAtUnix == null
      ? undefined
      : Number(overrides.estimatedExpiresAtUnix),
    totalDeployed: overrides.totalDeployed ?? undefined,
    motherlode: overrides.motherlode ?? undefined,
    totalMiners: overrides.totalMiners?.toString(),
    myDeploymentTotal: undefined,
  };
}

describe('StatsPanel', () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it('only marks the final ten seconds as urgent', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-01-01T00:00:00.000Z'));
    const now = BigInt(Math.floor(Date.now() / 1_000));
    const { rerender } = render(
      <StatsPanel {...makeRound({ roundId: 1n, estimatedExpiresAtUnix: now + 30n })} />,
    );

    expect(screen.getByText('00:30')).toHaveClass('text-stone-800');

    rerender(
      <StatsPanel {...makeRound({ roundId: 1n, estimatedExpiresAtUnix: now + 10n })} />,
    );
    expect(screen.getByText('00:10')).toHaveClass('text-red-500');
  });

  it('holds the display until a later estimate catches up', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-01-01T00:00:00.000Z'));
    const now = BigInt(Math.floor(Date.now() / 1_000));
    const { rerender } = render(
      <StatsPanel {...makeRound({ roundId: 1n, estimatedExpiresAtUnix: now + 30n })} />,
    );

    act(() => vi.advanceTimersByTime(5_000));
    expect(screen.getByText('00:25')).toBeInTheDocument();

    const later = BigInt(Math.floor(Date.now() / 1_000)) + 60n;
    rerender(
      <StatsPanel {...makeRound({ roundId: 1n, estimatedExpiresAtUnix: later })} />,
    );
    expect(screen.getByText('00:25')).toBeInTheDocument();

    act(() => vi.advanceTimersByTime(1_000));
    expect(screen.getByText('00:25')).toBeInTheDocument();

    act(() => vi.advanceTimersByTime(34_000));
    expect(screen.getByText('00:25')).toBeInTheDocument();

    act(() => vi.advanceTimersByTime(1_000));
    expect(screen.getByText('00:24')).toBeInTheDocument();
  });

  it('applies an earlier estimate for the same round immediately', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-01-01T00:00:00.000Z'));
    const now = BigInt(Math.floor(Date.now() / 1_000));
    const { rerender } = render(
      <StatsPanel {...makeRound({ roundId: 1n, estimatedExpiresAtUnix: now + 30n })} />,
    );

    rerender(
      <StatsPanel {...makeRound({ roundId: 1n, estimatedExpiresAtUnix: now + 10n })} />,
    );
    expect(screen.getByText('00:10')).toBeInTheDocument();

    act(() => vi.advanceTimersByTime(1_000));
    expect(screen.getByText('00:09')).toBeInTheDocument();
  });

  it('allows a new round to reset the countdown upward', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-01-01T00:00:00.000Z'));
    const now = BigInt(Math.floor(Date.now() / 1_000));
    const { rerender } = render(
      <StatsPanel {...makeRound({ roundId: 1n, estimatedExpiresAtUnix: now + 5n })} />,
    );

    act(() => vi.advanceTimersByTime(2_000));
    expect(screen.getByText('00:03')).toBeInTheDocument();

    const next = BigInt(Math.floor(Date.now() / 1_000)) + 60n;
    rerender(
      <StatsPanel {...makeRound({ roundId: 2n, estimatedExpiresAtUnix: next })} />,
    );
    expect(screen.getByText('01:00')).toBeInTheDocument();

    act(() => vi.advanceTimersByTime(1_000));
    expect(screen.getByText('00:59')).toBeInTheDocument();
  });

  it('continues a same-round countdown through a missing estimate', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-01-01T00:00:00.000Z'));
    const now = BigInt(Math.floor(Date.now() / 1_000));
    const { rerender } = render(
      <StatsPanel {...makeRound({ roundId: 1n, estimatedExpiresAtUnix: now + 30n })} />,
    );

    act(() => vi.advanceTimersByTime(5_000));
    rerender(<StatsPanel {...makeRound({ roundId: 1n, estimatedExpiresAtUnix: null })} />);
    expect(screen.getByText('00:25')).toBeInTheDocument();

    act(() => vi.advanceTimersByTime(1_000));
    expect(screen.getByText('00:24')).toBeInTheDocument();

    const later = BigInt(Math.floor(Date.now() / 1_000)) + 60n;
    rerender(
      <StatsPanel {...makeRound({ roundId: 1n, estimatedExpiresAtUnix: later })} />,
    );
    expect(screen.getByText('00:24')).toBeInTheDocument();
  });

  it('keeps an expired countdown red while the next estimate is unavailable', () => {
    const now = BigInt(Math.floor(Date.now() / 1_000));
    const { rerender } = render(
      <StatsPanel {...makeRound({ roundId: 1n, estimatedExpiresAtUnix: now })} />,
    );

    expect(screen.getByText('00:00')).toHaveClass('text-red-500');

    rerender(<StatsPanel {...makeRound({ roundId: 1n, estimatedExpiresAtUnix: null })} />);
    expect(screen.getByText('00:00')).toHaveClass('text-red-500');
  });

  it('hides an empty user deployment and labels a non-zero deployment', () => {
    const { rerender } = render(<StatsPanel {...makeRound()} />);

    expect(screen.queryByRole('tooltip')).not.toBeInTheDocument();

    rerender(<StatsPanel {...makeRound()} myDeploymentTotal={1.25} />);
    expect(screen.getByLabelText('1.25 SOL deployed by you this round')).toBeInTheDocument();
    expect(screen.getByRole('tooltip')).toHaveTextContent('Total deployed by you this round');
  });

  it('does not present missing live values as zero', () => {
    render(<StatsPanel {...makeRound()} />);

    expect(screen.getAllByText('–').length).toBeGreaterThanOrEqual(3);
    expect(screen.queryByText('0.00')).not.toBeInTheDocument();
    expect(screen.queryByText(/0 miners/)).not.toBeInTheDocument();
  });
});
