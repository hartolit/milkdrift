/** A conditional latest-value write has no command receipt. Recovery compares current content. */
export interface CheckpointSnapshot<T> { generation: number; digest: string; document: T }
export interface CheckpointState<T> {
  generation: number; digest: string; saved: string; candidate: T | null;
  remote: CheckpointSnapshot<T> | null; status: 'loading' | 'saved' | 'saving' | 'unsaved' | 'conflict'; error: string;
}
export function checkpointState<T>(): CheckpointState<T> { return { generation: 0, digest: '', saved: '', candidate: null, remote: null, status: 'loading', error: '' }; }
export function contentKey(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(contentKey).join(',')}]`;
  if (value !== null && typeof value === 'object') return `{${Object.entries(value).sort(([a], [b]) => a.localeCompare(b)).map(([key, item]) => `${JSON.stringify(key)}:${contentKey(item)}`).join(',')}}`;
  return JSON.stringify(value);
}
export class ConditionalCheckpoint<T> {
  #running: Promise<void> | null = null;
  #closed = false;
  constructor(readonly state: CheckpointState<T>, private readonly read: () => Promise<CheckpointSnapshot<T>>, private readonly write: (generation: number, document: T) => Promise<CheckpointSnapshot<T>>, private readonly onfailure: (failure: unknown) => void = () => {}) {}
  async load(): Promise<T> {
    const snapshot = await this.read();
    if (this.#closed) return snapshot.document;
    this.accept(snapshot); this.state.status = 'saved';
    return snapshot.document;
  }
  private accept(snapshot: CheckpointSnapshot<T>) {
    this.state.generation = snapshot.generation; this.state.digest = snapshot.digest;
    this.state.saved = contentKey(snapshot.document); this.state.remote = null; this.state.error = '';
  }
  stage(document: T) {
    if (this.#closed || this.state.status === 'loading') return;
    const key = contentKey(document);
    this.state.candidate = key === this.state.saved && !this.#running ? null : JSON.parse(JSON.stringify(document)) as T;
    if (this.state.status !== 'conflict') this.state.status = this.#running ? 'saving' : this.state.candidate ? 'unsaved' : 'saved';
  }
  async flush(): Promise<void> {
    if (this.#running) { await this.#running; return; }
    if (this.#closed || this.state.status === 'conflict' || this.state.status === 'loading' || !this.state.candidate) return;
    this.#running = this.drain();
    try { await this.#running; } finally { this.#running = null; }
  }
  private async drain() {
    while (!this.#closed && this.state.candidate && this.state.status !== 'conflict') {
      const submitted = JSON.parse(JSON.stringify(this.state.candidate)) as T, submittedKey = contentKey(submitted), generation = this.state.generation;
      this.state.status = 'saving'; this.state.error = '';
      try {
        let snapshot: CheckpointSnapshot<T>;
        try { snapshot = await this.write(generation, submitted); }
        catch (failure) {
          // Neither a lost reply nor a stale-generation refusal permits overwriting current work.
          this.onfailure(failure);
          snapshot = await this.read();
          if (contentKey(snapshot.document) !== submittedKey) {
            if (!this.#closed) {
              if (contentKey(snapshot.document) === this.state.saved) { this.state.status = 'unsaved'; this.state.error = failure instanceof Error ? failure.message : 'The save could not be confirmed.'; }
              else { this.state.remote = snapshot; this.state.status = 'conflict'; this.state.error = 'The saved canvas differs from this edit. Choose which version to keep.'; }
            }
            return;
          }
        }
        if (this.#closed) return;
        this.accept(snapshot);
        if (this.state.candidate && contentKey(this.state.candidate) === submittedKey) this.state.candidate = null;
        this.state.status = this.state.candidate ? 'unsaved' : 'saved';
      } catch (failure) {
        if (!this.#closed) { this.state.status = 'unsaved'; this.state.error = failure instanceof Error ? failure.message : 'The save could not be confirmed.'; this.onfailure(failure); }
        return;
      }
    }
  }
  useSaved(): T | null {
    const snapshot = this.state.remote;
    if (!snapshot) return null;
    this.accept(snapshot); this.state.candidate = null; this.state.status = 'saved';
    return snapshot.document;
  }
  keepCandidate() {
    const snapshot = this.state.remote;
    if (!snapshot) return;
    this.accept(snapshot); this.state.status = this.state.candidate ? 'unsaved' : 'saved';
  }
  close() { this.#closed = true; }
}
