<script lang="ts">
  import type { TaskActionPolicyDto, TaskCapability, TaskToolDto } from '../lib/protocol.generated';
  let { actions = $bindable(), tools = $bindable(), capabilities = [] }: {
    actions: TaskActionPolicyDto | null;
    tools: TaskToolDto[];
    capabilities?: TaskCapability[];
  } = $props();
  let processSelection = $state('');

  function nativeTool(kind: 'read_file' | 'write_file', allowed: boolean) {
    tools = [...tools.filter((tool) => tool.kind !== kind), ...(allowed ? [{ kind }] : [])];
  }
</script>

<details><summary>Model continuation and permitted tools</summary>
  <label class="checkbox-label"><input type="checkbox" checked={actions !== null} onchange={(event) => actions = event.currentTarget.checked ? { automatic_repair: false } : null} /> Allow model to propose future work</label>
  {#if actions}<label class="checkbox-label"><input type="checkbox" bind:checked={actions.automatic_repair} /> Allow automatic feedback after refused proposals</label>{/if}
  <p class="field-hint">The daemon validates every proposed batch with your existing authority and the workflow's shared limits. Refused proposals wait for a decision unless automatic feedback was granted.</p>
  <label class="checkbox-label"><input type="checkbox" checked={tools.some((tool) => tool.kind === 'read_file')} onchange={(event) => nativeTool('read_file', event.currentTarget.checked)} /> Permit bounded file reads in declared paths</label>
  <label class="checkbox-label"><input type="checkbox" checked={tools.some((tool) => tool.kind === 'write_file')} onchange={(event) => nativeTool('write_file', event.currentTarget.checked)} /> Permit bounded file writes in declared write paths</label>
  <p class="field-hint">Declare the project paths below. File tools operate only inside those granted roots; captured uploads remain exact text inputs.</p>
  {#each tools.filter((tool) => tool.kind === 'process') as tool (`${tool.capability.id}/${tool.capability.operation}`)}<div class="draft-row"><span>Process tool: {tool.capability.id} · {tool.capability.operation}</span><button type="button" onclick={() => tools = tools.filter((entry) => entry.kind !== 'process' || entry.capability.id !== tool.capability.id || entry.capability.operation !== tool.capability.operation)}>Remove process tool</button></div>{/each}
  <label>Configured process tool<select bind:value={processSelection}><option value="">Choose a configured process capability</option>{#each capabilities as capability (`${capability.id}/${capability.operation}`)}<option value={`${capability.id}/${capability.operation}`}>{capability.id} · {capability.operation}</option>{/each}</select></label>
  <p class="field-hint">Select an operation backed by a process profile. The daemon checks that the selected capability is a process and is permitted by your grant.</p>
  <button type="button" disabled={!processSelection} onclick={() => {
    const capability = capabilities.find((entry) => `${entry.id}/${entry.operation}` === processSelection);
    if (capability && !tools.some((tool) => tool.kind === 'process' && tool.capability.id === capability.id && tool.capability.operation === capability.operation)) tools = [...tools, { kind: 'process', capability: { ...capability } }];
    processSelection = '';
  }}>Permit process tool</button>
</details>
