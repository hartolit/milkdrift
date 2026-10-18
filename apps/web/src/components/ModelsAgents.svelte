<script lang="ts">
  import { onDestroy } from 'svelte';
  import type { TaskSessionDto } from '../lib/protocol.generated';
  import type { DaemonClient } from '../lib/transport';

  let { session, client, onrefresh, onerror, onback }: {
    session: TaskSessionDto;
    client: DaemonClient;
    onrefresh: () => Promise<void>;
    onerror: (failure: unknown) => void;
    onback: () => void;
  } = $props();
  let setup = $state<'model' | 'agent' | null>(null);
  let refreshing = $state(false);
  let message = $state('');
  let disposed = false;
  onDestroy(() => { disposed = true; });
  const choices = $derived(session.capabilities.map((capability) => ({
    capability,
    contract: session.authoring.find((item) => item.capability.id === capability.id && item.capability.operation === capability.operation),
  })));
  const models = $derived(choices.filter((item) => item.capability.operation === 'model.generate'));
  const agents = $derived(choices.filter((item) => item.capability.operation !== 'model.generate'));

  async function refresh() {
    if (refreshing) return;
    refreshing = true;
    message = '';
    try {
      await onrefresh();
      if (!disposed) message = 'Available choices refreshed from this daemon.';
    } catch (failure) {
      if (!disposed) onerror(failure);
    } finally {
      if (!disposed) refreshing = false;
    }
  }
</script>

<section class="panel models-agents" aria-labelledby="models-agents-heading">
  <div class="section-heading">
    <div><p class="eyebrow">{session.daemon_name}</p><h1 id="models-agents-heading">Models &amp; agents</h1></div>
    <button class="quiet" onclick={onback}>Back to canvas</button>
  </div>
  <p>Select a task on the canvas, then choose its <strong>Model or agent</strong>. Each task can use a different one.</p>
  <div class="form-actions">
    <button class="primary" onclick={() => { setup = setup ? null : 'model'; }} aria-expanded={setup !== null} aria-controls="connection-setup">Add model or agent</button>
    <button onclick={refresh} disabled={refreshing}>{refreshing ? 'Refreshing…' : 'Refresh choices'}</button>
  </div>

  {#if setup}
    <section class="connection-setup" id="connection-setup" aria-labelledby="setup-heading">
      <div class="section-heading"><h2 id="setup-heading">Connect it to {session.daemon_name}</h2><button class="quiet" onclick={() => { setup = null; }}>Close setup</button></div>
      <p>A connection is configured once on the computer running this daemon and becomes available to its permitted users. This browser currently selects configured connections; installation is an operator step.</p>
      <div class="form-actions" role="group" aria-label="Connection kind">
        <button aria-pressed={setup === 'model'} onclick={() => { setup = 'model'; }}>Model endpoint</button>
        <button aria-pressed={setup === 'agent'} onclick={() => { setup = 'agent'; }}>Command-line agent or tool</button>
      </div>
      {#if setup === 'model'}
        <ol>
          <li>Use a separately running model server or a supported hosted endpoint.</li>
          <li>On the daemon computer, configure its endpoint profile, credential reference and enforceable usage limits using the model setup guide.</li>
          <li>Add that profile to the daemon configuration and restart the daemon. Reconnect here, then choose the model on a task.</li>
        </ol>
        <a href="https://github.com/hartolit/milkdrift/blob/main/docs/guides/local-model-endpoint.md" target="_blank" rel="noreferrer">Open model setup guide ↗</a>
      {:else}
        <ol>
          <li>Install the agent or tool on the computer running the daemon.</li>
          <li>Configure an approved process profile with its executable, inputs, outputs and permissions using the process setup guide.</li>
          <li>Add the profile to the daemon configuration and restart the daemon. Reconnect here, then choose it on a task.</li>
        </ol>
        <a href="https://github.com/hartolit/milkdrift/blob/main/docs/guides/local-process.md" target="_blank" rel="noreferrer">Open process setup guide ↗</a>
      {/if}
      <p class="muted">If someone else runs this daemon, ask them to add the connection. Credentials belong in the daemon’s secret configuration.</p>
    </section>
  {/if}

  {#each [{ title: 'Models', items: models }, { title: 'Agents & tools', items: agents }] as group (group.title)}
    <section class="choice-group" aria-label={group.title}>
      <h2>{group.title}</h2>
      {#if group.items.length}
        <ul class="connection-choices">
          {#each group.items as item (`${item.capability.id}:${item.capability.operation}`)}
            <li>
              <div class="section-heading"><h3>{item.contract?.label ?? item.capability.id}</h3><span class="choice-state">Available to select</span></div>
              {#if item.contract}
                <p class="muted">{item.contract.inputs.filter((input) => input.required).length ? `${item.contract.inputs.filter((input) => input.required).length} required input${item.contract.inputs.filter((input) => input.required).length === 1 ? '' : 's'}` : 'No required file or text input'}{item.contract.allows_additional_inputs ? ' · Accepts connected context' : ''}</p>
              {:else}
                <p class="notice warning">Connection details are unavailable. Refresh before adding new work.</p>
              {/if}
              <details><summary>Connection details</summary><dl class="facts"><dt>Identity</dt><dd>{item.capability.id}</dd><dt>Operation</dt><dd>{item.capability.operation}</dd>{#if item.contract}<dt>Configuration generation</dt><dd>{item.contract.generation}</dd>{/if}</dl></details>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="empty">No {group.title.toLowerCase()} are available to this account on {session.daemon_name}.</p>
      {/if}
    </section>
  {/each}
  <p class="muted">These are the daemon’s current permitted choices. Availability does not verify a remote model server or start a paid request.</p>
  <details><summary>Daemon connection</summary><p>{client.endpoint}</p></details>
  {#if message}<p role="status">{message}</p>{/if}
</section>

<style>
  .models-agents { max-width: 66rem; margin-inline: auto; }
  .choice-group { margin-block: 1.7rem; }
  .connection-choices { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 21rem), 1fr)); gap: .8rem; list-style: none; padding: 0; }
  .connection-choices li, .connection-setup { padding: 1rem; border: 1px solid var(--border, #303044); border-radius: .75rem; background: var(--panel, #11121e); min-width: 0; overflow-wrap: anywhere; }
  .connection-setup { margin-block: 1rem; }
  .connection-setup ol { padding-left: 1.4rem; }
  .connection-setup li { margin-block: .65rem; }
  .connection-choices h3 { margin: 0; }
  .choice-state { font-size: .78rem; color: var(--muted, #b5b5c6); }
  .connection-choices .section-heading { flex-wrap: wrap; gap: .5rem; }
</style>
