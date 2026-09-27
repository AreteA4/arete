import type { Subscription, WebSocketFactoryInit } from '../index';
import { frames, type FrameTarget } from './frames';

type Listener = ((event: never) => void) | null;

/**
 * An in-memory WebSocket the SDK connects to through `auth.websocketFactory`.
 * The client side is the standard `WebSocket` surface the SDK uses; the
 * server side is `deliver`, `serverClose` and `fail`.
 */
export class FakeWebSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;

  readonly CONNECTING = 0;
  readonly OPEN = 1;
  readonly CLOSING = 2;
  readonly CLOSED = 3;

  readyState = FakeWebSocket.CONNECTING;
  binaryType: 'blob' | 'arraybuffer' = 'blob';
  onopen: Listener = null;
  onmessage: ((event: { data: unknown }) => void | Promise<void>) | null = null;
  onerror: Listener = null;
  onclose: ((event: { code: number; reason: string; wasClean: boolean }) => void) | null = null;
  /** Raw messages the client sent, in order. */
  readonly sent: string[] = [];

  constructor(
    readonly url: string,
    readonly init?: WebSocketFactoryInit,
    autoOpen = true,
  ) {
    if (autoOpen) {
      queueMicrotask(() => this.open());
    }
  }

  /** Client side: queue an outgoing message. */
  send(data: string): void {
    if (this.readyState !== FakeWebSocket.OPEN) {
      throw new Error('FakeWebSocket is not open');
    }
    this.sent.push(data);
  }

  /** Client side: close the socket. */
  close(code = 1000, reason = ''): void {
    if (this.readyState === FakeWebSocket.CLOSED) return;
    this.readyState = FakeWebSocket.CLOSED;
    this.onclose?.({ code, reason, wasClean: true });
  }

  /** Server side: accept the connection (automatic unless disabled). */
  open(): void {
    if (this.readyState !== FakeWebSocket.CONNECTING) return;
    this.readyState = FakeWebSocket.OPEN;
    (this.onopen as (() => void) | null)?.();
  }

  /** Server side: deliver one message (objects are sent as JSON text). */
  deliver(message: unknown): void {
    if (this.readyState !== FakeWebSocket.OPEN) {
      throw new Error('FakeWebSocket is not open');
    }
    void this.onmessage?.({
      data: typeof message === 'string' ? message : JSON.stringify(message),
    });
  }

  /** Server side: close the connection with a code and reason. */
  serverClose(code = 1011, reason = ''): void {
    if (this.readyState === FakeWebSocket.CLOSED) return;
    this.readyState = FakeWebSocket.CLOSED;
    this.onclose?.({ code, reason, wasClean: false });
  }

  /** Server side: fail the socket as a network error would. */
  fail(): void {
    (this.onerror as (() => void) | null)?.();
    this.serverClose(1006, 'connection failed');
  }

  /** Parsed `subscribe` messages this socket received. */
  subscriptions(): Subscription[] {
    return this.sent.flatMap((raw) => {
      try {
        const message = JSON.parse(raw) as { type?: unknown };
        return message.type === 'subscribe' ? [message as Subscription] : [];
      } catch {
        return [];
      }
    });
  }
}

export interface WebSocketHarnessOptions {
  /** Open sockets automatically on the next microtask (default `true`). */
  readonly autoOpen?: boolean;
  /**
   * Install {@link FakeWebSocket} as `globalThis.WebSocket` when the runtime has
   * none (Node 20 and older), because the SDK reads the standard readyState
   * constants from it. Default `true`; `restore()` removes it again.
   */
  readonly installGlobal?: boolean;
}

export interface WebSocketHarness {
  /** Pass as `auth.websocketFactory` (core) or `<AreteProvider auth={…}>`. */
  readonly websocketFactory: (url: string, init?: WebSocketFactoryInit) => WebSocket;
  /** Every socket the client opened, oldest first. */
  readonly sockets: readonly FakeWebSocket[];
  /** The most recent socket; throws if the client never connected. */
  latest(): FakeWebSocket;
  /** Subscriptions sent on the latest socket, oldest first. */
  subscriptions(): Subscription[];
  /**
   * The latest subscription for a view (or matching a predicate); throws when
   * the client has not subscribed yet.
   */
  subscription(match?: string | ((subscription: Subscription) => boolean)): Subscription;
  /**
   * Resolve with the latest subscription for a view (or matching a
   * predicate) once the client has sent it; rejects after `timeoutMs`
   * (default 1000). Subscribing is asynchronous, so tests usually await this.
   */
  waitForSubscription(
    match?: string | ((subscription: Subscription) => boolean),
    timeoutMs?: number,
  ): Promise<Subscription>;
  /** Deliver a server frame on the latest socket. */
  send(frame: unknown): void;
  /** Acknowledge a subscription and answer it with an authoritative snapshot. */
  serve(
    target: FrameTarget,
    rows: readonly { readonly key: string; readonly data: unknown }[],
    options?: Parameters<typeof frames.subscribed>[1],
  ): void;
  /** Remove the global installed by `installGlobal`, if any. */
  restore(): void;
}

/**
 * A scripted WebSocket server for tests. The SDK connects to it through
 * `auth.websocketFactory`, so views, snapshots, live frames, reconnects and
 * errors run through the real client without a network.
 */
export function createWebSocketHarness(options: WebSocketHarnessOptions = {}): WebSocketHarness {
  const sockets: FakeWebSocket[] = [];
  const globals = globalThis as { WebSocket?: unknown };
  const installed = options.installGlobal !== false && globals.WebSocket === undefined;
  if (installed) {
    globals.WebSocket = FakeWebSocket;
  }

  const latest = (): FakeWebSocket => {
    const socket = sockets[sockets.length - 1];
    if (!socket) {
      throw new Error('The client has not opened a WebSocket yet');
    }
    return socket;
  };

  const find = (
    match: string | ((subscription: Subscription) => boolean) | undefined,
  ): Subscription | undefined => {
    const socket = sockets[sockets.length - 1];
    const candidates = (socket?.subscriptions() ?? []).filter((subscription) =>
      match === undefined
        || (typeof match === 'string' ? subscription.query.view === match : match(subscription))
    );
    return candidates[candidates.length - 1];
  };
  const describe = (match: string | ((subscription: Subscription) => boolean) | undefined) =>
    `No subscription${typeof match === 'string' ? ` for ${match}` : ''} has been sent`;

  return {
    sockets,
    websocketFactory(url, init) {
      const socket = new FakeWebSocket(url, init, options.autoOpen !== false);
      sockets.push(socket);
      return socket as unknown as WebSocket;
    },
    latest,
    subscriptions: () => latest().subscriptions(),
    subscription(match) {
      const found = find(match);
      if (!found) throw new Error(`${describe(match)} yet`);
      return found;
    },
    async waitForSubscription(match, timeoutMs = 1_000) {
      const deadline = Date.now() + timeoutMs;
      for (;;) {
        const found = find(match);
        if (found) return found;
        if (Date.now() > deadline) throw new Error(`${describe(match)} within ${timeoutMs}ms`);
        await new Promise((resolve) => setTimeout(resolve, 0));
      }
    },
    send(frame) {
      latest().deliver(frame);
    },
    serve(target, rows, subscribedOptions) {
      const socket = latest();
      socket.deliver(frames.subscribed(target, subscribedOptions));
      socket.deliver(frames.snapshot(target, rows, { mode: subscribedOptions?.mode }));
    },
    restore() {
      if (installed && globals.WebSocket === FakeWebSocket) {
        delete globals.WebSocket;
      }
    },
  };
}
