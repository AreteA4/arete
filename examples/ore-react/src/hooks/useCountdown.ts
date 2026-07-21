import { useEffect, useRef, useState } from 'react';

/**
 * Seconds until the current round expires.
 *
 * The stream refines its estimate as the round progresses. The display can
 * move down immediately, but never up: when an estimate moves later, the
 * timer pauses until the estimate catches up.
 */
export function useCountdown(
  roundId: string | undefined,
  estimatedExpiresAtUnix: number | undefined,
): number | undefined {
  const latestEstimate = useRef<{ roundId: string; expiresAt: number } | null>(null);
  const [countdown, setCountdown] = useState<{ roundId: string; remaining: number } | null>(null);

  useEffect(() => {
    if (
      roundId === undefined
      || estimatedExpiresAtUnix === undefined
      || estimatedExpiresAtUnix <= 0
    ) {
      if (latestEstimate.current?.roundId !== roundId) {
        latestEstimate.current = null;
        setCountdown((current) => current?.roundId === roundId ? current : null);
      }
      return;
    }

    latestEstimate.current = { roundId, expiresAt: estimatedExpiresAtUnix };
    const remaining = Math.max(
      0,
      estimatedExpiresAtUnix - Math.floor(Date.now() / 1_000),
    );
    setCountdown((current) => {
      if (!current || current.roundId !== roundId || remaining < current.remaining) {
        return { roundId, remaining };
      }
      return current;
    });
  }, [estimatedExpiresAtUnix, roundId]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      const estimate = latestEstimate.current;
      if (!estimate) return;
      const remaining = Math.max(
        0,
        estimate.expiresAt - Math.floor(Date.now() / 1_000),
      );
      setCountdown((current) => {
        if (!current || current.roundId !== estimate.roundId) {
          return { roundId: estimate.roundId, remaining };
        }
        return remaining < current.remaining
          ? { roundId: current.roundId, remaining }
          : current;
      });
    }, 1_000);
    return () => window.clearInterval(timer);
  }, []);

  return countdown && countdown.roundId === roundId ? countdown.remaining : undefined;
}
