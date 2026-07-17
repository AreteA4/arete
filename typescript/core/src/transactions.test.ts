import { describe, expect, it, vi } from 'vitest';
import { createTransactionTransport, TransactionTransportError } from './transactions';

function json(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

describe('TransactionTransport', () => {
  it('serializes bounded route requests and parses decimal u64 fields', async () => {
    const request = vi.fn(async (url: string, init: RequestInit, scope: string) => {
      expect(url).toBe('https://stack.example/transactions/v1/latest-blockhash');
      expect(scope).toBe('transaction:inspect');
      expect(JSON.parse(String(init.body))).toEqual({
        commitment: 'confirmed',
        minContextSlot: '42',
      });
      return json({
        blockhash: 'blockhash',
        contextSlot: '43',
        lastValidBlockHeight: '99',
      });
    });
    const transport = createTransactionTransport('https://stack.example/', request);

    await expect(transport.getLatestBlockhash({
      commitment: 'confirmed', minContextSlot: 42n,
    })).resolves.toEqual({
      blockhash: 'blockhash', contextSlot: 43n, lastValidBlockHeight: 99n,
    });
  });

  it('marks send as a transaction-scoped request and does not retry internally', async () => {
    const request = vi.fn(async () => json({ signature: 'sig' }));
    const transport = createTransactionTransport('https://stack.example', request);
    await expect(transport.sendTransaction('signed-base64', {
      skipPreflight: true,
    })).resolves.toEqual({ signature: 'sig' });
    expect(request).toHaveBeenCalledTimes(1);
    expect(request.mock.calls[0]?.[2]).toBe('transaction:send');
    expect(JSON.parse(String(request.mock.calls[0]?.[1].body))).toEqual({
      transaction: 'signed-base64', skipPreflight: true,
    });
  });

  it('exposes stable transport error metadata without reflecting raw bodies', async () => {
    const transport = createTransactionTransport('https://stack.example', async () =>
      json({
        code: 'upstream_timeout', message: 'Submission outcome is unknown',
        retryable: false, requestId: 'req-1', submissionState: 'unknown', signature: 'local-sig',
      }, 504)
    );
    const error = await transport.sendTransaction('signed').catch((cause) => cause);
    expect(error).toBeInstanceOf(TransactionTransportError);
    expect(error).toMatchObject({
      code: 'upstream_timeout', retryable: false, requestId: 'req-1',
      submissionState: 'unknown', signature: 'local-sig', status: 504,
    });
  });
});
