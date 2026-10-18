/** One dispatched Server-Sent Event. */
export interface ServerSentEvent {
  /** `event:` field; `message` when absent. */
  readonly event: string;
  readonly data: string;
  /** The last event id in effect when this event was dispatched. */
  readonly id: string | undefined;
}

/**
 * Incremental `text/event-stream` parser (WHATWG HTML, "Server-sent events").
 * Feed decoded text chunks; complete events are returned as they finish.
 */
export class ServerSentEventParser {
  private buffer = '';
  private eventType = '';
  private data: string[] = [];
  private hasData = false;
  private lastEventIdBuffer: string | undefined;
  /** Reconnection time the server asked for with `retry:`, in milliseconds. */
  retryMs: number | undefined;

  constructor(lastEventId?: string) {
    this.lastEventIdBuffer = lastEventId;
  }

  get lastEventId(): string | undefined {
    return this.lastEventIdBuffer;
  }

  push(chunk: string): ServerSentEvent[] {
    this.buffer += chunk;
    const events: ServerSentEvent[] = [];
    for (;;) {
      const match = /\r\n|\r|\n/.exec(this.buffer);
      if (!match) break;
      // A lone CR at the end of the buffer may be the first half of CRLF.
      if (match[0] === '\r' && match.index === this.buffer.length - 1) break;
      const line = this.buffer.slice(0, match.index);
      this.buffer = this.buffer.slice(match.index + match[0].length);
      const event = this.line(line);
      if (event) events.push(event);
    }
    return events;
  }

  /** Discard a partial event at end of stream, as the specification requires. */
  end(): void {
    this.buffer = '';
    this.eventType = '';
    this.data = [];
    this.hasData = false;
  }

  private line(line: string): ServerSentEvent | undefined {
    if (line === '') return this.dispatch();
    if (line.startsWith(':')) return undefined;
    const colon = line.indexOf(':');
    const field = colon === -1 ? line : line.slice(0, colon);
    let value = colon === -1 ? '' : line.slice(colon + 1);
    if (value.startsWith(' ')) value = value.slice(1);
    switch (field) {
      case 'event':
        this.eventType = value;
        break;
      case 'data':
        this.data.push(value);
        this.hasData = true;
        break;
      case 'id':
        if (!value.includes('\0')) this.lastEventIdBuffer = value;
        break;
      case 'retry':
        if (/^[0-9]+$/.test(value)) this.retryMs = Number(value);
        break;
      default:
        break;
    }
    return undefined;
  }

  private dispatch(): ServerSentEvent | undefined {
    const event = this.hasData
      ? { event: this.eventType || 'message', data: this.data.join('\n'), id: this.lastEventIdBuffer }
      : undefined;
    this.eventType = '';
    this.data = [];
    this.hasData = false;
    return event;
  }
}
