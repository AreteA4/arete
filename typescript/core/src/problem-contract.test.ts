import { describe, expect, it } from 'vitest';
import {
  isClaimAgentRecoveryAction,
  isSafeClaimActionUrl,
  parseApiProblem,
} from './types';

describe('API problem v1', () => {
  it('accepts legacy errors and ignores unknown future fields', () => {
    expect(parseApiProblem({ error: 'Nope', code: 'legacy-code' })).toMatchObject({
      error: 'Nope',
      code: 'legacy-code',
    });
    expect(parseApiProblem({
      schemaVersion: 2,
      error: 'Future',
      futureField: { value: true },
      action: { type: 'future_action', payload: 1 },
    })?.action?.type).toBe('future_action');
  });

  it('validates the closed claim materializer contract', () => {
    const problem = parseApiProblem({
      schemaVersion: 1,
      error: 'Claim required',
      code: 'agent-claim-required',
      retryable: false,
      usage: { unit: 'bytes', used: 4, limit: 10, resetsAt: null },
      action: {
        type: 'claim_agent',
        label: 'Claim this agent',
        method: 'POST',
        path: '/api/agents/me/claim-links',
      },
    });
    expect(problem?.usage).toEqual({ unit: 'bytes', used: 4, limit: 10, resetsAt: null });
    expect(isClaimAgentRecoveryAction(problem?.action)).toBe(true);
    expect(isClaimAgentRecoveryAction({
      type: 'claim_agent',
      method: 'GET',
      path: '/api/agents/me/claim-links',
    })).toBe(false);
  });

  it('rejects unsafe or malformed numeric/action fields', () => {
    expect(parseApiProblem({
      error: 'Unsafe',
      usage: { unit: 'bytes', used: Number.MAX_SAFE_INTEGER + 1, limit: 10 },
    })).toBeUndefined();
    expect(parseApiProblem({
      error: 'Malformed',
      action: { type: 'claim_agent', method: 12 },
    })).toBeUndefined();
  });

  it('accepts only an exact, fragment-bearing claim URL on the configured origin', () => {
    const action = {
      type: 'claim_agent',
      url: 'https://app.arete.run/claim#opaque-token',
      elicitationId: 'claim-id',
      expiresAt: '2026-09-22T12:30:00Z',
    };
    expect(isSafeClaimActionUrl(action, 'https://app.arete.run')).toBe(true);
    expect(isSafeClaimActionUrl({ ...action, url: 'https://evil.test/claim#opaque-token' }, 'https://app.arete.run')).toBe(false);
    expect(isSafeClaimActionUrl({ ...action, url: 'https://app.arete.run/claim?token=x#y' }, 'https://app.arete.run')).toBe(false);
  });
});
