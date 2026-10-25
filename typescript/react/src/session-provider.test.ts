/**
 * `AreteSessionProvider` over the real `createSession`: a standalone program
 * member opens without any network, so the provider's lifecycle (open once,
 * close on unmount, reopen on new auth or options, the wallet without a
 * reopen) is observed on real sessions.
 */

import React from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import type { AuthConfig, SessionDefinition, WalletAdapter } from '@usearete/sdk';

import { AreteProvider } from './provider';
import {
  AreteSessionProvider,
  AreteSessionUnavailableError,
  isAreteSessionUnavailableError,
  useAreteSession,
  type AnySession,
  type AreteSessionProviderProps,
  type UseAreteSessionResult,
} from './session-provider';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const originalConsoleError = console.error;
beforeAll(() => {
  jest.spyOn(console, 'error').mockImplementation((message: unknown, ...args: unknown[]) => {
    if (typeof message === 'string' && message.startsWith('react-test-renderer is deprecated')) return;
    originalConsoleError(message, ...args);
  });
});
afterAll(() => {
  jest.restoreAllMocks();
});

const MEMO = { name: 'memo', programId: 'MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr' };
const DEFINITION = { programs: { memo: MEMO } } satisfies SessionDefinition;
const AUTH_A: AuthConfig = { publishableKey: 'pk-a' };
const AUTH_B: AuthConfig = { publishableKey: 'pk-b' };

function wallet(publicKey: string): WalletAdapter {
  return { publicKey, signAndSend: async () => ({ signature: `sig-${publicKey}` }) };
}

type Props = AreteSessionProviderProps<typeof DEFINITION>;

function renderSession(initial: Props, wrap: (node: React.ReactElement) => React.ReactElement = (node) => node) {
  const seen: UseAreteSessionResult<AnySession>[] = [];
  function Reader() {
    seen.push(useAreteSession());
    return null;
  }
  const element = (props: Props) => wrap(
    React.createElement(AreteSessionProvider, props as AreteSessionProviderProps, React.createElement(Reader))
  );
  let renderer!: ReactTestRenderer;
  act(() => {
    renderer = create(element(initial));
  });
  return {
    get current() {
      return seen[seen.length - 1];
    },
    seen,
    update(props: Props) {
      act(() => {
        renderer.update(element(props));
      });
    },
    unmount() {
      act(() => {
        renderer.unmount();
      });
    },
  };
}

async function settle() {
  await act(async () => {
    for (let i = 0; i < 10; i += 1) await Promise.resolve();
  });
}

describe('AreteSessionProvider', () => {
  it('opens the session once, shares it and closes it on unmount', async () => {
    const view = renderSession({ definition: DEFINITION, options: { auth: AUTH_A }, children: null });
    expect(view.current.status).toBe('opening');
    expect(view.current.session).toBeNull();

    await settle();
    const session = view.current.session;
    expect(view.current.status).toBe('open');
    expect(session?.programs.memo).toBeDefined();
    await expect(view.current.ready()).resolves.toBe(session);

    // The same options written inline again are the same options.
    view.update({ definition: { programs: { memo: MEMO } }, options: { auth: { publishableKey: 'pk-a' } }, children: null });
    await settle();
    expect(view.current.session).toBe(session);

    const close = jest.spyOn(session as AnySession, 'close');
    view.unmount();
    expect(close).toHaveBeenCalledTimes(1);
  });

  it('closes the old session at once and opens a new one when the auth changes', async () => {
    const view = renderSession({ definition: DEFINITION, options: { auth: AUTH_A }, children: null });
    await settle();
    const first = view.current.session as AnySession;
    const closeFirst = jest.spyOn(first, 'close');

    view.update({ definition: DEFINITION, options: { auth: AUTH_B }, children: null });
    expect(closeFirst).toHaveBeenCalledTimes(1);
    expect(view.current.status).toBe('opening');
    expect(view.current.session).toBeNull();

    await settle();
    expect(view.current.status).toBe('open');
    expect(view.current.session).not.toBe(first);
    expect(view.current.session).not.toBeNull();
    view.unmount();
  });

  it('hands a caller waiting through a reopen the new session', async () => {
    const view = renderSession({ definition: DEFINITION, options: { auth: AUTH_A }, children: null });
    // Asked before the first session opened; the auth changes before it does.
    const waiting = view.current.ready();
    view.update({ definition: DEFINITION, options: { auth: AUTH_B }, children: null });
    await settle();

    const opened = view.current.session;
    expect(opened).not.toBeNull();
    await expect(waiting).resolves.toBe(opened);
    view.unmount();
  });

  it('sets a new wallet on the open session without reopening it', async () => {
    const first = wallet('wallet-1');
    const second = wallet('wallet-2');
    const view = renderSession({ definition: DEFINITION, wallet: first, children: null });
    await settle();
    const session = view.current.session as AnySession;
    expect(session.wallet).toBe(first);

    view.update({ definition: DEFINITION, wallet: second, children: null });
    expect(view.current.session).toBe(session);
    expect(session.wallet).toBe(second);

    view.update({ definition: DEFINITION, children: null });
    expect(session.wallet).toBeUndefined();
    view.unmount();
  });

  it("uses the enclosing AreteProvider's auth, and reopens when it changes", async () => {
    let auth = AUTH_A;
    const view = renderSession(
      { definition: DEFINITION, children: null },
      (node) => React.createElement(AreteProvider, { auth, children: node })
    );
    await settle();
    const first = view.current.session;

    auth = AUTH_B;
    view.update({ definition: DEFINITION, children: null });
    await settle();
    expect(view.current.session).not.toBe(first);
    view.unmount();
  });

  it('reports a session that cannot open, and opens it on retry', async () => {
    let refuse = true;
    // A definition that fails the first open only.
    const flaky = {
      get programs() {
        if (refuse) {
          refuse = false;
          return {};
        }
        return { memo: MEMO };
      },
    } as unknown as typeof DEFINITION;
    const view = renderSession({ definition: flaky, children: null });
    const waiting = view.current.ready();
    await settle();

    expect(view.current.status).toBe('error');
    expect(view.current.error?.message).toMatch(/at least one stack or program/);
    await expect(waiting).rejects.toThrow(/at least one stack or program/);

    act(() => view.current.retry());
    expect(view.current.status).toBe('opening');
    await settle();
    expect(view.current.status).toBe('open');
    expect(view.current.error).toBeNull();
    view.unmount();
  });

  it('shares a session it was given and never closes it', async () => {
    const session = { close: jest.fn(), setWallet: jest.fn(), wallet: undefined } as unknown as AnySession;
    const view = renderSession({ session: null, children: null });
    expect(view.current.status).toBe('opening');
    const waiting = view.current.ready();

    view.update({ session, children: null });
    expect(view.current.status).toBe('open');
    expect(view.current.session).toBe(session);
    await expect(waiting).resolves.toBe(session);
    // No `wallet` prop: the app's own wallet on the session is left alone.
    expect(session.setWallet).not.toHaveBeenCalled();

    const signer = wallet('wallet-1');
    view.update({ session, wallet: signer, children: null });
    expect(session.setWallet).toHaveBeenCalledWith(signer);

    view.unmount();
    expect(session.close).not.toHaveBeenCalled();
  });

  it('keeps one open session under StrictMode', async () => {
    const view = renderSession(
      { definition: DEFINITION, children: null },
      (node) => React.createElement(React.StrictMode, null, node)
    );
    await settle();
    const session = view.current.session as AnySession;
    expect(view.current.status).toBe('open');
    await expect(view.current.ready()).resolves.toBe(session);

    const close = jest.spyOn(session, 'close');
    view.unmount();
    expect(close).toHaveBeenCalledTimes(1);
  });

  it('fails a caller waiting on ready() when the provider is removed before the session opens', async () => {
    const view = renderSession({ definition: DEFINITION, options: { auth: AUTH_A }, children: null });
    const { ready } = view.current;
    const waiting = ready();
    view.unmount();
    await settle();

    const failure = await waiting.catch((error: unknown) => error);
    expect(failure).toBeInstanceOf(AreteSessionUnavailableError);
    expect(isAreteSessionUnavailableError(failure)).toBe(true);
    expect(failure).toMatchObject({ code: 'provider_unmounted', name: 'AreteSessionUnavailableError' });
    // Asked after the provider is gone, it fails at once too.
    await expect(ready()).rejects.toBeInstanceOf(AreteSessionUnavailableError);
  });

  it('fails a caller waiting through a reopen when the provider is removed before the new session opens', async () => {
    const view = renderSession({ definition: DEFINITION, options: { auth: AUTH_A }, children: null });
    const waiting = view.current.ready();
    view.update({ definition: DEFINITION, options: { auth: AUTH_B }, children: null });
    view.unmount();
    await settle();
    await expect(waiting).rejects.toMatchObject({ code: 'provider_unmounted' });
  });

  it('fails a caller waiting for a session the app never gave', async () => {
    const view = renderSession({ session: null, children: null });
    const waiting = view.current.ready();
    view.unmount();
    await settle();
    await expect(waiting).rejects.toBeInstanceOf(AreteSessionUnavailableError);
  });

  it('hands a caller waiting from the first render the session StrictMode opens', async () => {
    const view = renderSession(
      { definition: DEFINITION, children: null },
      (node) => React.createElement(React.StrictMode, null, node)
    );
    // Asked in the first render, before StrictMode's simulated remount.
    const waiting = view.seen[0].ready();
    await settle();
    const session = view.current.session as AnySession;
    expect(session).not.toBeNull();
    await expect(waiting).resolves.toBe(session);
    view.unmount();
  });

  it('throws outside a provider', () => {
    function Reader() {
      useAreteSession();
      return null;
    }
    const consoleError = jest.spyOn(console, 'error').mockImplementation(() => undefined);
    expect(() => act(() => {
      create(React.createElement(Reader));
    })).toThrow(/within AreteSessionProvider/);
    consoleError.mockRestore();
  });
});
