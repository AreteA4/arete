import { act, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { StatsPanel } from './StatsPanel';

describe('StatsPanel', () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it('only marks the final ten seconds as urgent', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-01-01T00:00:00.000Z'));
    const now = Math.floor(Date.now() / 1_000);
    const { rerender } = render(
      <StatsPanel roundId="1" estimatedExpiresAtUnix={now + 30} youDeployed={undefined} />,
    );

    expect(screen.getByText('00:30')).toHaveClass('text-stone-800');

    rerender(
      <StatsPanel roundId="1" estimatedExpiresAtUnix={now + 10} youDeployed={undefined} />,
    );
    expect(screen.getByText('00:10')).toHaveClass('text-red-500');
  });

  it('ignores a later estimate for the same round without freezing the countdown', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-01-01T00:00:00.000Z'));
    const now = Math.floor(Date.now() / 1_000);
    const { rerender } = render(
      <StatsPanel roundId="1" estimatedExpiresAtUnix={now + 30} />,
    );

    act(() => vi.advanceTimersByTime(5_000));
    expect(screen.getByText('00:25')).toBeInTheDocument();

    rerender(
      <StatsPanel
        roundId="1"
        estimatedExpiresAtUnix={Math.floor(Date.now() / 1_000) + 60}
      />,
    );
    expect(screen.getByText('00:25')).toBeInTheDocument();

    act(() => vi.advanceTimersByTime(1_000));
    expect(screen.getByText('00:24')).toBeInTheDocument();
  });

  it('applies an earlier estimate for the same round immediately', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-01-01T00:00:00.000Z'));
    const now = Math.floor(Date.now() / 1_000);
    const { rerender } = render(
      <StatsPanel roundId="1" estimatedExpiresAtUnix={now + 30} />,
    );

    rerender(
      <StatsPanel
        roundId="1"
        estimatedExpiresAtUnix={Math.floor(Date.now() / 1_000) + 10}
      />,
    );
    expect(screen.getByText('00:10')).toBeInTheDocument();

    act(() => vi.advanceTimersByTime(1_000));
    expect(screen.getByText('00:09')).toBeInTheDocument();
  });

  it('allows a new round to reset the countdown upward', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-01-01T00:00:00.000Z'));
    const now = Math.floor(Date.now() / 1_000);
    const { rerender } = render(
      <StatsPanel roundId="1" estimatedExpiresAtUnix={now + 5} />,
    );

    act(() => vi.advanceTimersByTime(2_000));
    expect(screen.getByText('00:03')).toBeInTheDocument();

    rerender(
      <StatsPanel
        roundId="2"
        estimatedExpiresAtUnix={Math.floor(Date.now() / 1_000) + 60}
      />,
    );
    expect(screen.getByText('01:00')).toBeInTheDocument();

    act(() => vi.advanceTimersByTime(1_000));
    expect(screen.getByText('00:59')).toBeInTheDocument();
  });

  it('continues a same-round countdown through a missing estimate', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-01-01T00:00:00.000Z'));
    const now = Math.floor(Date.now() / 1_000);
    const { rerender } = render(
      <StatsPanel roundId="1" estimatedExpiresAtUnix={now + 30} />,
    );

    act(() => vi.advanceTimersByTime(5_000));
    rerender(<StatsPanel roundId="1" estimatedExpiresAtUnix={undefined} />);
    expect(screen.getByText('00:25')).toBeInTheDocument();

    act(() => vi.advanceTimersByTime(1_000));
    expect(screen.getByText('00:24')).toBeInTheDocument();

    rerender(
      <StatsPanel
        roundId="1"
        estimatedExpiresAtUnix={Math.floor(Date.now() / 1_000) + 60}
      />,
    );
    expect(screen.getByText('00:24')).toBeInTheDocument();
  });

  it('keeps an expired countdown red while the next estimate is unavailable', () => {
    const now = Math.floor(Date.now() / 1_000);
    const { rerender } = render(
      <StatsPanel estimatedExpiresAtUnix={now} youDeployed={undefined} />,
    );

    expect(screen.getByText('00:00')).toHaveClass('text-red-500');

    rerender(<StatsPanel estimatedExpiresAtUnix={undefined} youDeployed={undefined} />);
    expect(screen.getByText('00:00')).toHaveClass('text-red-500');
  });

  it('hides an empty user deployment and labels a non-zero deployment', () => {
    const { rerender } = render(<StatsPanel youDeployed={undefined} />);

    expect(screen.queryByRole('tooltip')).not.toBeInTheDocument();

    rerender(<StatsPanel youDeployed="1.25" />);
    expect(screen.getByLabelText('1.25 SOL deployed by you this round')).toBeInTheDocument();
    expect(screen.getByRole('tooltip')).toHaveTextContent('Total deployed by you this round');
  });
});
