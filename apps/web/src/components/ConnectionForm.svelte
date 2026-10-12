<script lang="ts">
  import type { ConnectionProfile } from '../lib/connections';

  let { profile, onconnect, oncancel }: {
    profile?: ConnectionProfile;
    onconnect: (details: { label: string; endpoint: string; daemonId: string; token: string }) => Promise<void>;
    oncancel: () => void;
  } = $props();
  let label = $state('');
  let endpoint = $state('');
  let daemonId = $state('');
  let token = $state('');
  let busy = $state(false);
  let error = $state('');

  $effect(() => {
    label = profile?.label ?? '';
    endpoint = profile?.endpoint ?? '';
    daemonId = profile?.daemonId ?? '';
  });

  async function connect(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    error = '';
    const credential = token;
    token = '';
    try {
      await onconnect({ label: label.trim(), endpoint: endpoint.trim(), daemonId: daemonId.trim(), token: credential });
    } catch (failure) {
      error = failure instanceof Error ? failure.message : 'The daemon connection failed.';
    } finally {
      busy = false;
    }
  }
</script>

<section class="panel connect-panel" aria-labelledby="connection-heading">
  <div class="section-heading">
    <div><p class="eyebrow">Your connection</p><h2 id="connection-heading">{profile ? `Reconnect to ${profile.label}` : 'Connect a daemon'}</h2></div>
    <button class="icon-button" onclick={oncancel} aria-label="Close connection form">×</button>
  </div>
  <p class="muted">Your daemon owns the work. Connect here to submit tasks and return to their results from any device.</p>
  <form onsubmit={connect}>
    <label>Connection name<input bind:value={label} placeholder="My studio" required maxlength="80" autocomplete="off" /></label>
    <label>Daemon URL<input bind:value={endpoint} placeholder="https://milkdrift.example.net" required type="url" autocomplete="url" /></label>
    <label>Installation identity<input bind:value={daemonId} placeholder="From your daemon configuration" required autocomplete="off" /></label>
    <label>Bearer credential<input bind:value={token} type="password" required autocomplete="off" spellcheck="false" /></label>
    <p class="field-hint">Credentials stay in this tab’s memory and clear on disconnect. Only the name, URL and installation identity are saved on this device.</p>
    {#if error}<p class="notice error" role="alert">{error}</p>{/if}
    <div class="form-actions"><button class="primary" disabled={busy}>{busy ? 'Connecting…' : 'Connect'}</button><button type="button" class="quiet" onclick={oncancel}>Cancel</button></div>
  </form>
</section>
