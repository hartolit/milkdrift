import { describe, expect, it, vi } from 'vitest';
import { checkpointState, ConditionalCheckpoint } from './checkpoint';
const snapshot = (generation: number, text: string) => ({ generation, digest: String(generation), document: { text } });
const deferred = <T>() => { let resolve!: (value: T) => void; const promise = new Promise<T>((done) => resolve = done); return { promise, resolve }; };
describe('conditional latest canvas checkpoint', () => {
  it('coalesces newer edits behind one write and never labels them saved from an older reply', async () => {
    const first = deferred<ReturnType<typeof snapshot>>(), second = deferred<ReturnType<typeof snapshot>>();
    const writes: string[] = [], state = checkpointState<{ text: string }>();
    const owner = new ConditionalCheckpoint(state, async () => snapshot(0, ''), async (_, doc) => { writes.push(doc.text); return writes.length === 1 ? first.promise : second.promise; });
    await owner.load(); owner.stage({ text: 'a' }); const saving = owner.flush();
    owner.stage({ text: 'b' }); owner.stage({ text: 'latest' });
    expect(writes).toEqual(['a']); first.resolve(snapshot(1, 'a')); await vi.waitFor(() => expect(writes).toHaveLength(2));
    expect(state.status).not.toBe('saved'); expect(writes).toEqual(['a', 'latest']);
    second.resolve(snapshot(2, 'latest')); await saving; expect(state.status).toBe('saved'); expect(state.candidate).toBeNull();
  });
  it('recovers a lost reply by current content without resending, retaining newer edits', async () => {
    let current = snapshot(0, ''); let calls = 0;
    const state = checkpointState<{ text: string }>();
    const owner = new ConditionalCheckpoint(state, async () => current, async (_, doc) => { calls++; current = snapshot(calls, doc.text); throw new Error('reply lost'); });
    await owner.load(); owner.stage({ text: 'saved remotely' }); await owner.flush();
    expect(calls).toBe(1); expect(state.status).toBe('saved'); expect(state.generation).toBe(1);
  });
  it('preserves a local candidate when another client wins and requires an explicit choice', async () => {
    let current = snapshot(0, ''); let calls = 0;
    const state = checkpointState<{ text: string }>();
    const owner = new ConditionalCheckpoint(state, async () => current, async () => { calls++; throw new Error('stale'); });
    await owner.load(); current = snapshot(1, 'theirs'); owner.stage({ text: 'mine' }); await owner.flush();
    expect(state.status).toBe('conflict'); expect(state.candidate).toEqual({ text: 'mine' }); await owner.flush(); expect(calls).toBe(1);
    expect(owner.useSaved()).toEqual({ text: 'theirs' }); expect(state.status).toBe('saved');
  });
  it('saves a newer reversal even when it matches the value before the in-flight write', async () => {
    const held = deferred<ReturnType<typeof snapshot>>(), state = checkpointState<{ text: string }>();
    const writes: string[] = [];
    const owner = new ConditionalCheckpoint(state, async () => snapshot(0, 'original'), async (generation, doc) => { writes.push(doc.text); return writes.length === 1 ? held.promise : snapshot(generation + 1, doc.text); });
    await owner.load(); owner.stage({ text: 'temporary' }); const saving = owner.flush(); owner.stage({ text: 'original' });
    held.resolve(snapshot(1, 'temporary')); await saving;
    expect(writes).toEqual(['temporary', 'original']); expect(state.status).toBe('saved'); expect(state.generation).toBe(2);
  });
  it('keeps a refused oversized candidate editable when the saved content did not change', async () => {
    const state = checkpointState<{ text: string }>();
    const owner = new ConditionalCheckpoint(state, async () => snapshot(0, 'small'), async () => { throw new Error('Canvas capacity reached'); });
    await owner.load(); owner.stage({ text: 'too large' }); await owner.flush();
    expect(state.status).toBe('unsaved'); expect(state.error).toBe('Canvas capacity reached'); expect(state.candidate).toEqual({ text: 'too large' }); expect(state.remote).toBeNull();
  });

});
