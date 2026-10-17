<script lang="ts">
  import type { ConnectionProfile } from '../lib/connections';

  let { profile, onconnect, oncancel }: {
    profile?: ConnectionProfile;
    onconnect: (details: { endpoint: string; token: string }) => Promise<void>;
    oncancel: () => void;
  } = $props();
  let endpoint = $derived(profile?.endpoint ?? '');
  let token = $state('');
  let busy = $state(false);
  let error = $state('');

  async function connect(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    error = '';
    const credential = token;
    token = '';
    try {
      await onconnect({ endpoint: endpoint.trim(), token: credential });
    } catch (failure) {
      error = failure instanceof Error ? failure.message : 'The daemon connection failed.';
    } finally {
      busy = false;
    }
  }
</script>

<section class="panel connect-panel" aria-labelledby="connection-heading">
  <div class="section-heading">
    <div><p class="eyebrow">Your connection</p><h2 id="connection-heading">{profile ? `Reconnect to ${profile.cachedName}` : 'Connect a daemon'}</h2></div>
    <button class="icon-button" onclick={oncancel} aria-label="Close connection form">×</button>
  </div>
  <p class="muted">Your daemon owns the work. Connect here to submit tasks and return to their results from any device.</p>
  <form onsubmit={connect}>
    <label>Daemon URL<input bind:value={endpoint} readonly={!!profile} placeholder="https://milkdrift.example.net" required type="url" autocomplete="url" /></label>
    <label>Bearer credential<input bind:value={token} type="password" required autocomplete="off" spellcheck="false" /></label>
    <p class="field-hint">Credentials stay in this tab’s memory and clear on disconnect. The daemon supplies its name. This browser remembers its address and checks that you reconnect to the same daemon.</p>
    {#if error}<p class="notice error" role="alert">{error}</p>{/if}
    <div class="form-actions"><button class="primary" disabled={busy}>{busy ? 'Connecting…' : 'Connect'}</button><button type="button" class="quiet" onclick={oncancel}>Cancel</button></div>
  </form>
</section>
