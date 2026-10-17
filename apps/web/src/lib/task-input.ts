import type { TaskUploadRequest } from './protocol.generated';

/** Preserve every valid UTF-8 byte, including an initial BOM, in captured text. */
export async function captureTextRequest(file: File, limit: number): Promise<TaskUploadRequest> {
  if (file.size > limit) throw new Error(`This file exceeds the daemon upload bound of ${limit.toLocaleString()} bytes. Supply a smaller text file.`);
  const text = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(await file.arrayBuffer());
  return { request_id: crypto.randomUUID(), name: file.name, media_type: 'text/plain', text };
}
