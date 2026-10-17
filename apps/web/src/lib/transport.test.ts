import { describe, expect, it, vi } from 'vitest';
import { DaemonClient, DaemonError, streamEvents } from './transport';
import { PROTOCOL } from './protocol.generated';

describe('authenticated observation transport', () => {
  it('keeps credentials in headers and clears them on disconnect', async () => {
    const fetchMock = vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response(JSON.stringify({ protocol: PROTOCOL, value: { daemon_id: 'a' } })));
    const client = new DaemonClient('http://127.0.0.1:10001', 'private-value');
    await expect(client.request('/v1/session')).resolves.toEqual({ daemon_id: 'a' });
    expect(fetchMock.mock.calls[0][0]).toBe('http://127.0.0.1:10001/v1/session');
    expect(new Headers(fetchMock.mock.calls[0][1]?.headers).get('Authorization')).toBe('Bearer private-value');
    client.disconnect();
    await expect(client.request('/v1/session')).rejects.toThrow('disconnected');
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it('does not borrow another connection credential', async () => {
    const fetchMock = vi.spyOn(globalThis, 'fetch').mockImplementation(async () => new Response(JSON.stringify({ protocol: PROTOCOL, value: {} })));
    const first = new DaemonClient('http://127.0.0.1:10001', 'first');
    const second = new DaemonClient('http://127.0.0.1:10002', 'second');
    await Promise.all([first.request('/v1/session'), second.request('/v1/session')]);
    expect(fetchMock.mock.calls.map(([, init]) => new Headers(init?.headers).get('Authorization'))).toEqual(['Bearer first', 'Bearer second']);
  });

  it('binds SSE cursors and daemon identity in authenticated headers', async () => {
    const fetchMock = vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response('event: workflow_changed\nid: cursor-b\ndata: {}\n\n'));
    const client = new DaemonClient('http://127.0.0.1:10001', 'private-value');
    client.bindIdentity('daemon-a');
    const callback = vi.fn(async () => {});
    await client.stream('/v1/workflows/stream', callback, new AbortController().signal, 'cursor-a');
    const headers = new Headers(fetchMock.mock.calls[0][1]?.headers);
    expect(headers.get('Last-Event-ID')).toBe('cursor-a');
    expect(headers.get('x-milkdrift-daemon')).toBe('daemon-a');
    expect(headers.get('Accept')).toBe('text/event-stream');
    expect(fetchMock.mock.calls[0][0]).toBe('http://127.0.0.1:10001/v1/workflows/stream');
    expect(callback).toHaveBeenCalledWith({ event: 'workflow_changed', id: 'cursor-b', data: '{}' });
  });

  it('retains forbidden and expired failure classifications', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response('{"code":"unauthorized","message":"Scope revoked","request_id":"r1"}', { status: 403 }));
    const client = new DaemonClient('http://127.0.0.1:10001', 'private-value');
    try {
      await client.request('/v1/workflows');
      throw new Error('Expected refusal');
    } catch (error) {
      expect(error).toBeInstanceOf(DaemonError);
      expect((error as DaemonError).authorizationLost).toBe(true);
      expect((error as DaemonError).requestId).toBe('r1');
    }
  });

  it('parses fragmented UTF-8 events and does not emit an unfinished event', async () => {
    const encoded = new TextEncoder().encode(': keepalive\r\nid: cursor-a\r\nevent: snapshot\r\ndata: {"title":"æ"}\r\n\r\ndata: partial');
    const stream = new ReadableStream<Uint8Array>({ start(controller) {
      for (const byte of encoded) controller.enqueue(Uint8Array.of(byte));
      controller.close();
    } });
    const events = [];
    for await (const event of streamEvents(stream)) events.push(event);
    expect(events).toEqual([{ event: 'snapshot', id: 'cursor-a', data: '{"title":"æ"}' }]);
  });
});
