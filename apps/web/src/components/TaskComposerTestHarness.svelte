<script lang="ts">
  import { untrack } from 'svelte';
  import TaskComposer from './TaskComposer.svelte';
  import type { TaskDraft } from '../lib/task-composer';
  import type { TaskCapability, TaskSessionDto, WorkflowReceipt } from '../lib/protocol.generated';
  import type { DaemonClient } from '../lib/transport';
  let { initial, client, session, visible = true, disposed = false, onaccepted, onerror = () => {}, oncapabilitychoice }: {
    initial: TaskDraft; client: DaemonClient; session: TaskSessionDto; visible?: boolean; disposed?: boolean;
    onaccepted: (receipt: WorkflowReceipt) => Promise<void>; onerror?: (failure: unknown) => void;
    oncapabilitychoice?: (capability: TaskCapability) => void;
  } = $props();
  let draft = $state(untrack(() => initial));
  $effect(() => { if (disposed) draft.disposed = true; });
</script>
{#if visible}<TaskComposer {client} {session} bind:draft {onaccepted} {onerror} {oncapabilitychoice} onclose={() => {}} ondiscard={() => { draft.disposed = true; }} />{/if}
<output data-testid="task-draft">{JSON.stringify(draft)}</output>
