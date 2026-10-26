import { endpointUrl } from './connections';
import { decodeResponse } from './protocol.generated';

export class DaemonError extends Error {
  constructor(
    public readonly code: string,
    message: string,
    public readonly status: number,
    public readonly requestId?: string,
    public readonly retryable = false,
    public readonly details: unknown = null,
  ) {
    super(message);
    this.name = 'DaemonError';
  }

  get authorizationLost(): boolean {
    return this.status === 401 || this.status === 403;
  }
}

export interface StreamEvent {
  event: string;
  id?: string;
  data: string;
}

/** Network chunks need not end at a UTF-8, line or event boundary. */
export async function* streamEvents(body: ReadableStream<Uint8Array>): AsyncGenerator<StreamEvent> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';
  let data: string[] = [];
  let event = 'message';
  let id: string | undefined;
  let eventBytes = 0;
  try {
    while (true) {
      const { value, done } = await reader.read();
      buffer += done ? decoder.decode() : decoder.decode(value, { stream: true });
      if (buffer.length + eventBytes > 2 * 1024 * 1024) {
        throw new Error('The daemon stream exceeded the client observation bound. Reconnect to refresh.');
      }
      let newline: number;
      while ((newline = buffer.indexOf('\n')) >= 0) {
        const line = buffer.slice(0, newline).replace(/\r$/, '');
        buffer = buffer.slice(newline + 1);
        if (!line) {
          if (data.length) yield { event, id, data: data.join('\n') };
          data = [];
          event = 'message';
          id = undefined;
          eventBytes = 0;
        } else if (!line.startsWith(':')) {
          const colon = line.indexOf(':');
          const field = colon < 0 ? line : line.slice(0, colon);
          const content = colon < 0 ? '' : line.slice(colon + 1).replace(/^ /, '');
          if (field === 'data') { data.push(content); eventBytes += content.length; }
          else if (field === 'event') event = content;
          else if (field === 'id' && !content.includes('\0')) id = content;
        }
      }
      if (done) break;
    }
  } finally {
    await reader.cancel().catch(() => undefined);
    reader.releaseLock();
  }
}

export class DaemonClient {
  readonly endpoint: string;
  #token: string;
  #daemonId: string | undefined;
  #lifetime = new AbortController();

  constructor(endpoint: string, token: string) {
    this.endpoint = endpointUrl(endpoint);
    this.#token = token.trim();
    if (!this.#token) throw new Error('Enter your scoped bearer credential to connect.');
  }

  disconnect(): void {
    this.#lifetime.abort();
    this.#token = '';
  }

  bindIdentity(daemonId: string): void {
    this.#daemonId = daemonId;
  }

  async #fetch(path: string, init: RequestInit = {}): Promise<Response> {
    if (!this.#token) throw new Error('This connection is disconnected. Authenticate again.');
    const headers = new Headers(init.headers);
    headers.set('Authorization', `Bearer ${this.#token}`);
    if (this.#daemonId) headers.set('x-milkdrift-daemon', this.#daemonId);
    if (!headers.has('Accept')) headers.set('Accept', 'application/json');
    if (init.body) headers.set('Content-Type', 'application/json');
    let response: Response;
    try {
      response = await fetch(`${this.endpoint}${path}`, {
        ...init,
        headers,
        credentials: 'omit',
        cache: 'no-store',
        redirect: 'error',
        signal: init.signal
          ? AbortSignal.any([this.#lifetime.signal, init.signal])
          : this.#lifetime.signal,
      });
    } catch (error) {
      if (error instanceof Error && error.name === 'AbortError') throw error;
      if (error instanceof Error && error.name === 'TimeoutError') throw new Error('The daemon did not reply within 30 seconds. If you submitted a change, look up its receipt or replay the exact request; it may already have been accepted.', { cause: error });
      throw new Error('The daemon could not be reached. Check its address, HTTPS certificate and allowed browser origin. A submitted request may still have been accepted.', { cause: error });
    }
    if (!response.ok) {
      const error = await response.json().catch(() => null) as {
        code?: string; message?: string; request_id?: string; retryable?: boolean; details?: unknown;
      } | null;
      throw new DaemonError(
        error?.code ?? 'transport_error',
        error?.message ?? `The daemon refused this request (HTTP ${response.status}).`,
        response.status,
        error?.request_id,
        error?.retryable ?? false,
        error?.details ?? null,
      );
    }
    return response;
  }

  async request<T>(path: string, body?: unknown, signal?: AbortSignal): Promise<T> {
    const response = await this.#fetch(path, {
      method: body === undefined ? 'GET' : 'POST',
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: signal ? AbortSignal.any([signal, AbortSignal.timeout(30_000)]) : AbortSignal.timeout(30_000),
    });
    return decodeResponse<T>(await response.json());
  }

  async put<T>(path: string, body: unknown): Promise<T> {
    const response = await this.#fetch(path, { method: 'PUT', body: JSON.stringify(body), signal: AbortSignal.timeout(30_000) });
    return decodeResponse<T>(await response.json());
  }

  async download(path: string): Promise<Blob> {
    return (await this.#fetch(path, { signal: AbortSignal.timeout(30_000) })).blob();
  }

  async stream(path: string, onEvent: (event: StreamEvent) => Promise<void>, signal: AbortSignal, cursor?: string): Promise<void> {
    const headers = new Headers({ Accept: 'text/event-stream' });
    if (cursor) headers.set('Last-Event-ID', cursor);
    const response = await this.#fetch(path, { signal, headers });
    if (!response.body) throw new Error('The daemon did not provide an observation stream.');
    for await (const event of streamEvents(response.body)) await onEvent(event);
  }
}
