<script lang="ts">
  import type { ComponentProps } from 'svelte';
  import DaemonSettings, { type RenameDraft } from './DaemonSettings.svelte';

  let props: Omit<ComponentProps<typeof DaemonSettings>, 'draft'> = $props();
  let draft = $state<RenameDraft>({ name: '', dirty: false, pending: null });
</script>

<DaemonSettings {...props} bind:draft />
