<script lang="ts">
  import { untrack } from 'svelte';
  import TaskDefinitionEditor from './TaskDefinitionEditor.svelte';
  import type { TaskCapability, TaskDefinitionDto } from '../lib/protocol.generated';

  let { initial, capabilities = [] }: { initial: TaskDefinitionDto; capabilities?: TaskCapability[] } = $props();
  let definition = $state(structuredClone(untrack(() => initial)));
</script>

<TaskDefinitionEditor bind:definition {capabilities} />
<output data-testid="edited-definition">{JSON.stringify(definition)}</output>
