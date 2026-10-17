<script module lang="ts">
  import type { RenameDaemonRequest } from '../lib/protocol.generated';

  /** An uncertain rename stays with its authenticated connection when the user navigates away. */
  export interface RenameDraft {
    name: string;
    dirty: boolean;
    pending: RenameDaemonRequest | null;
  }
</script>

<script lang="ts">
  import { onDestroy } from 'svelte';
  import type { RenameDaemonReceipt, TaskSessionDto } from '../lib/protocol.generated';
  import { DaemonClient, DaemonError } from '../lib/transport';

  let { client, session, draft = $bindable(), onrenamed, onrefresh, onerror, onback }: {
    client: DaemonClient;
    session: TaskSessionDto;
    draft: RenameDraft;
    onrenamed: (receipt: RenameDaemonReceipt) => void;
    onrefresh: () => Promise<void>;
    onerror: (failure: unknown) => void;
    onback: () => void;
  } = $props();
  let busy = $state(false);
  let error = $state('');
  let confirmation = $state('');
  const lifetime = new AbortController();

  $effect(() => {
    if (!draft.dirty && !draft.pending && draft.name !== session.daemon_name) draft = { ...draft, name: session.daemon_name };
  });
  onDestroy(() => lifetime.abort());

  async function send(lookup = false) {
    const request = draft.pending;
    if (!request) return;
    busy = true;
    error = '';
    confirmation = '';
    try {
      const receipt = await client.request<RenameDaemonReceipt>(
        lookup ? `/v1/daemon/rename-requests/${encodeURIComponent(request.request_id)}` : '/v1/daemon/rename',
        lookup ? undefined : request,
        lifetime.signal,
      );
      if (lifetime.signal.aborted) return;
      if (receipt.daemon_id !== session.daemon_id || receipt.request_id !== request.request_id) {
        throw new DaemonError('daemon_identity_mismatch', 'The rename reply belongs to a different daemon or request. Reconnect before continuing.', 409);
      }
      draft = { ...draft, pending: null, dirty: false };
      onrenamed(receipt);
      confirmation = 'Name saved on this daemon. Its other connected clients will refresh shortly.';
      // A replay can describe an earlier accepted rename. Read the current name as well.
      await onrefresh();
    } catch (failure) {
      if (lifetime.signal.aborted) return;
      error = failure instanceof Error ? failure.message : 'The rename could not be confirmed.';
      if (failure instanceof DaemonError) {
        if (failure.status === 401 || failure.code === 'daemon_identity_mismatch') onerror(failure);
        if (!lookup && failure.status < 500) {
          draft = { ...draft, pending: null };
          if (failure.status === 409 || failure.status === 403) await onrefresh();
        }
      }
    } finally { if (!lifetime.signal.aborted) busy = false; }
  }

  function rename(event: SubmitEvent) {
    event.preventDefault();
    if (busy || draft.pending || !session.can_rename_daemon) return;
    draft = { ...draft, pending: { request_id: crypto.randomUUID(), expected_generation: session.daemon_name_generation, name: draft.name } };
    void send();
  }
</script>

<section class="panel" aria-labelledby="daemon-settings-heading">
  <div class="section-heading"><div><p class="eyebrow">{session.daemon_name}</p><h1 id="daemon-settings-heading">Daemon settings</h1></div><button class="quiet" onclick={onback}>Back to work</button></div>
  <p>The daemon owns this name. Changing it affects every client connected to this daemon.</p>
  <form onsubmit={rename}>
    <label>Daemon name<input value={draft.name} oninput={(event) => { draft = { ...draft, name: event.currentTarget.value, dirty: true }; confirmation = ''; }} readonly={!session.can_rename_daemon || !!draft.pending} autocomplete="off" /></label>
    {#if session.can_rename_daemon}<button class="primary" disabled={busy || !!draft.pending}>{busy ? 'Saving…' : 'Save daemon name'}</button>
    {:else}<p class="notice">Your credential can read this name but cannot change it. Ask a daemon administrator to rename it.</p>{/if}
  </form>
  {#if error}<p class="notice error" role="alert">{error} {!draft.pending ? 'Your draft is still here.' : ''}</p>{/if}
  {#if draft.pending && !busy}<p class="notice warning">The rename may have been accepted. Look up its receipt or retry this exact request before making another change.</p><div class="form-actions"><button onclick={() => send(true)}>Find rename receipt</button><button onclick={() => send()}>Replay exact rename</button></div>{/if}
  {#if confirmation}<p class="notice" role="status">{confirmation}</p>{/if}
  <details><summary>Connection details</summary><dl class="facts"><dt>Daemon address</dt><dd>{client.endpoint}</dd><dt>Stable daemon identity</dt><dd>{session.daemon_id}</dd><dt>Signed in as</dt><dd>{session.actor_id}</dd></dl><p>This identity identifies the daemon, not this browser. Milkdrift remembers it to detect when a saved address points to a different daemon. The display name does not establish trust or permissions.</p></details>
</section>
