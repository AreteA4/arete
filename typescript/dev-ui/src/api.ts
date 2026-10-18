/**
 * The `a4 dev` server's routes, relative to the page's origin.
 */

import type { DevEvent, InstanceSummary, ProtocolSnapshot } from "./types";

export const EVENT_TYPES = [
  "build.started",
  "build.failed",
  "build.succeeded",
  "deploy.activated",
  "deploy.failed",
  "step.prepared",
  "step.submitted",
  "step.confirmed",
  "step.failed",
  "instance.transitioned",
  "instance.emitted",
  "stack.row",
  "stack.redeployed",
  "chain.clock",
  "feed.degraded",
  "chain.airdrop",
  "env.changed",
  "stream.reset",
] as const;

export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly code?: string,
    readonly body?: unknown,
  ) {
    super(message);
  }
}

async function json<T>(response: Response): Promise<T> {
  const text = await response.text();
  let body: any = null;
  try {
    body = text ? JSON.parse(text) : null;
  } catch {
    body = text;
  }
  if (!response.ok) {
    throw new ApiError(body?.error ?? `${response.status} ${response.statusText}`, response.status, body?.code, body);
  }
  return body as T;
}

export async function getJson<T>(path: string): Promise<T> {
  return json<T>(await fetch(path, { headers: { accept: "application/json" } }));
}

export async function postJson<T>(path: string, body: unknown): Promise<T> {
  return json<T>(
    await fetch(path, {
      method: "POST",
      headers: { "content-type": "application/json", accept: "application/json" },
      body: JSON.stringify(body),
    }),
  );
}

export const fetchProtocol = () => getJson<ProtocolSnapshot>("/dev/v1/protocol");

export const fetchInstances = () =>
  getJson<{ deploymentId: string | null; instances: InstanceSummary[] }>("/dev/v1/instances");

export const fetchInstance = (id: string) => getJson<Record<string, unknown>>(`/dev/v1/instances/${encodeURIComponent(id)}`);

export interface FeedHandlers {
  onEvent(event: DevEvent): void;
  onLive(): void;
  onStatus(status: "connecting" | "live" | "reconnecting" | "offline"): void;
}

/** Subscribe to the event feed; EventSource resumes with Last-Event-ID itself. */
export function subscribe(handlers: FeedHandlers, url = "/dev/v1/events"): () => void {
  const source = new EventSource(url);
  handlers.onStatus("connecting");
  const deliver = (message: MessageEvent<string>) => {
    try {
      handlers.onEvent(JSON.parse(message.data) as DevEvent);
    } catch {
      // A malformed event is skipped.
    }
  };
  for (const type of EVENT_TYPES) source.addEventListener(type, deliver as EventListener);
  source.addEventListener("stream.live", () => {
    handlers.onStatus("live");
    handlers.onLive();
  });
  source.onerror = () => {
    handlers.onStatus(source.readyState === EventSource.CLOSED ? "offline" : "reconnecting");
  };
  return () => source.close();
}
