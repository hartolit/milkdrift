import { expect, it, vi } from 'vitest';
import { captureTextRequest } from './task-input';

it('captures exact UTF-8 bytes including a leading BOM instead of normalizing file content', async () => {
  const bytes = new Uint8Array([0xef, 0xbb, 0xbf, 0x61, 0xc3, 0xa6, 0x0d, 0x0a]);
  const file = { name: 'notes.txt', size: bytes.length, arrayBuffer: async () => bytes.buffer } as File;
  const request = await captureTextRequest(file, bytes.length);
  expect(Array.from(new TextEncoder().encode(request.text))).toEqual(Array.from(bytes));
});

it('refuses excessive size before reading and refuses non-UTF-8 bytes', async () => {
  const read = vi.fn(async () => new Uint8Array([0xff]).buffer);
  const file = { name: 'notes.txt', size: 1, arrayBuffer: read } as unknown as File;
  await expect(captureTextRequest(file, 0)).rejects.toThrow('daemon upload bound');
  expect(read).not.toHaveBeenCalled();
  await expect(captureTextRequest(file, 1)).rejects.toThrow();
});
