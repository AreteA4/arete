import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { BlockGrid } from './BlockGrid';

describe('BlockGrid accessibility', () => {
  it('renders all 1-based labels as buttons with explicit protocol indexes', async () => {
    const user = userEvent.setup();
    const onToggle = vi.fn();
    render(
      <BlockGrid
        selected={[0]}
        onToggle={onToggle}
      />,
    );
    const tiles = screen.getAllByRole('button', { name: /Square \d+/ });
    expect(tiles).toHaveLength(25);
    expect(screen.getByRole('button', { name: /Square 1, protocol index 0/ })).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByRole('button', { name: /Square 25, protocol index 24/ })).toHaveAttribute('aria-pressed', 'false');
    await user.click(screen.getByTestId('tile-25'));
    expect(onToggle).toHaveBeenCalledWith(24);
  });

  it('supports keyboard activation', async () => {
    const user = userEvent.setup();
    const onToggle = vi.fn();
    render(
      <BlockGrid
        selected={[]}
        onToggle={onToggle}
      />,
    );
    screen.getByTestId('tile-1').focus();
    await user.keyboard('[Space]');
    expect(onToggle).toHaveBeenCalledWith(0);
  });

  it('highlights candidate and finalized winning squares with full rings', () => {
    render(
      <BlockGrid
        preRevealWinningSquare={1}
        winningSquare={0}
        selected={[]}
        onToggle={vi.fn()}
      />,
    );

    expect(screen.getByTestId('tile-1')).toHaveClass('outline-emerald-500');
    expect(screen.getByTestId('tile-1')).toHaveAttribute('data-winning-state', 'final');
    expect(screen.getByTestId('tile-2')).toHaveClass('outline-amber-400');
    expect(screen.getByTestId('tile-2')).toHaveAttribute('data-winning-state', 'candidate');
  });

  it('fills positions confirmed on chain in blue', () => {
    const deployedPerSquare = Array<number>(25).fill(0);
    deployedPerSquare[0] = 9;
    const myDeployment = Array<number>(25).fill(0);
    myDeployment[0] = 1.25;
    render(
      <BlockGrid
        deployedPerSquare={deployedPerSquare}
        myDeployment={myDeployment}
        selected={[]}
        onToggle={vi.fn()}
      />,
    );

    expect(screen.getByTestId('tile-1')).toHaveClass('bg-sky-100', 'outline-sky-500');
    expect(screen.getByTestId('tile-1')).toHaveTextContent('9.0000');
    expect(screen.getByTestId('tile-1')).toHaveTextContent('1.2500');
    expect(screen.getByTestId('tile-1')).not.toHaveTextContent('Yours');
    expect(screen.getByRole('button', { name: /Square 1,/ })).toHaveAccessibleName(
      /9 SOL total, 1.25 SOL yours.*your current position/,
    );
  });
});
