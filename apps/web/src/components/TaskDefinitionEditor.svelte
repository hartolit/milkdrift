<script lang="ts">
  import type { InputBinding, TaskCapability, TaskDefinitionDto } from '../lib/protocol.generated';
  import TaskModelOptions from './TaskModelOptions.svelte';

  let { definition = $bindable(), capabilities = [] }: {
    definition: TaskDefinitionDto;
    capabilities?: TaskCapability[];
  } = $props();
  let inputName = $state('');
  let inputKind = $state<InputBinding['kind']>('supplied');
  let inputSource = $state('');
  let inputOutput = $state('result');
  let inputOptional = $state(false);
  let inputAbsent = $state(false);
  let outputName = $state('');
  let dependency = $state('');
  let path = $state('');
  let write = $state(false);

  function clearNonModelChoices(operation: string) {
    if (operation !== 'model.generate') {
      definition.actions = null;
      definition.tools = [];
    }
  }

  function addInput() {
    if (!inputName.trim()) return;
    let binding: InputBinding;
    switch (inputKind) {
      case 'inline': binding = { kind: 'inline', value: inputAbsent ? { kind: 'absent' } : { kind: 'text', text: inputSource } }; break;
      case 'supplied': binding = { kind: 'supplied', name: inputSource, optional: inputOptional }; break;
      case 'task_output': binding = { kind: 'task_output', task: inputSource, output: inputOutput, optional: inputOptional }; break;
      case 'settled_outcome': binding = { kind: 'settled_outcome', task: inputSource }; break;
    }
    definition.inputs = { ...definition.inputs, [inputName.trim()]: binding };
    inputName = ''; inputSource = '';
  }
</script>

<label>Task title<input bind:value={definition.title} required maxlength="256" /></label>
<label>Short description<input bind:value={definition.description} maxlength="4096" /></label>
<label>Instructions<textarea bind:value={definition.instructions} required rows="5"></textarea></label>
{#if capabilities.length}
  <label>Configured capability<select value={`${definition.capability.id}/${definition.capability.operation}`} onchange={(event) => {
    const choice = capabilities.find((item) => `${item.id}/${item.operation}` === event.currentTarget.value);
    if (choice) { definition.capability = { ...choice }; clearNonModelChoices(choice.operation); }
  }}>{#each capabilities as capability (`${capability.id}/${capability.operation}`)}<option value={`${capability.id}/${capability.operation}`}>{capability.id} · {capability.operation}</option>{/each}</select></label>
{:else}
  <div class="form-grid"><label>Capability identity<input bind:value={definition.capability.id} required /></label><label>Operation<input value={definition.capability.operation} oninput={(event) => { definition.capability.operation = event.currentTarget.value; clearNonModelChoices(event.currentTarget.value); }} required /></label></div>
{/if}
<label class="checkbox-label"><input type="checkbox" bind:checked={definition.required} /> Required for workflow completion</label>
<label>Corrects task<input value={definition.corrective_of ?? ''} oninput={(event) => definition.corrective_of = event.currentTarget.value || null} placeholder="Prior task identity, or leave empty" /></label>
{#if definition.capability.operation === 'model.generate'}<TaskModelOptions bind:actions={definition.actions} bind:tools={definition.tools} {capabilities} />{/if}

<h3 class="subheading">Named inputs</h3>
<p class="field-hint">Required outputs need a successful producer. Optional outputs still wait for settlement. A settled outcome explicitly includes failure. Supplying a new value affects future starts.</p>
{#each Object.entries(definition.inputs) as [name, input] (name)}
  <div class="draft-row"><strong>{name}</strong><span>{input.kind === 'supplied' ? `Workflow input ${input.name} · ${input.optional ? 'optional' : 'required'}` : input.kind === 'task_output' ? `${input.task} / ${input.output} · ${input.optional ? 'optional' : 'required success'}` : input.kind === 'settled_outcome' ? `Settled outcome of ${input.task}, including failure` : `Inline ${input.value.kind}`}</span><button type="button" onclick={() => { const next = { ...definition.inputs }; delete next[name]; definition.inputs = next; }} aria-label={`Remove input ${name}`}>Remove</button></div>
{/each}
<div class="form-grid"><label>Input binding name<input bind:value={inputName} /></label><label>Input policy<select bind:value={inputKind}><option value="supplied">Supplied workflow input</option><option value="inline">Inline text or absence</option><option value="task_output">Successful task output</option><option value="settled_outcome">Settled task outcome (including failure)</option></select></label></div>
<label>{inputKind === 'inline' ? 'Inline text' : inputKind === 'supplied' ? 'Supplied input name' : 'Producer task identity or batch alias'}<input bind:value={inputSource} /></label>
{#if inputKind === 'task_output'}<label>Producer output name<input bind:value={inputOutput} /></label>{/if}
{#if inputKind === 'task_output' || inputKind === 'supplied'}<label class="checkbox-label"><input type="checkbox" bind:checked={inputOptional} /> {inputKind === 'task_output' ? 'Accept absent input after settlement' : 'Accept an absent supplied value'}</label>{/if}
{#if inputKind === 'inline'}<label class="checkbox-label"><input type="checkbox" bind:checked={inputAbsent} /> Explicit absence</label>{/if}
<button type="button" onclick={addInput} disabled={!inputName.trim()}>Set named input</button>

<h3 class="subheading">Explicit dependencies</h3>
{#each definition.dependencies as id (id)}<div class="draft-row"><code>{id}</code><button type="button" aria-label={`Remove dependency ${id}`} onclick={() => definition.dependencies = definition.dependencies.filter((item) => item !== id)}>Remove</button></div>{/each}
<label>Dependency task identity or batch alias<input bind:value={dependency} /></label><button type="button" disabled={!dependency.trim() || definition.dependencies.includes(dependency.trim())} onclick={() => { definition.dependencies = [...definition.dependencies, dependency.trim()]; dependency = ''; }}>Add dependency</button>

<h3 class="subheading">Declared outputs</h3>
{#each Object.entries(definition.outputs) as [name, output] (name)}
  <fieldset class="nested-fields"><legend>{name}</legend><label class="checkbox-label"><input type="checkbox" bind:checked={output.required} /> Required output</label><label>Media type for {name}<input value={output.media_type ?? ''} oninput={(event) => output.media_type = event.currentTarget.value || null} /></label><label>Maximum bytes for {name}<input type="number" min="1" step="1" bind:value={output.max_bytes} required /></label><button type="button" onclick={() => { const next = { ...definition.outputs }; delete next[name]; definition.outputs = next; }} aria-label={`Remove output ${name}`}>Remove output</button></fieldset>
{/each}
<label>New output name<input bind:value={outputName} /></label><button type="button" disabled={!outputName.trim() || outputName in definition.outputs} onclick={() => { definition.outputs = { ...definition.outputs, [outputName.trim()]: { required: true, media_type: 'text/plain', max_bytes: 16384 } }; outputName = ''; }}>Add output</button>

<h3 class="subheading">Scoped project paths</h3>
{#each definition.paths as claim, index (`${index}/${claim.path}`)}<div class="draft-row"><code>{claim.path}</code><span>{claim.write ? 'Read and write' : 'Read only'}</span><button type="button" aria-label={`Remove path ${claim.path}`} onclick={() => definition.paths = definition.paths.filter((_, current) => current !== index)}>Remove</button></div>{/each}
<label>Project path<input bind:value={path} placeholder="Absolute path within your grant" /></label><label class="checkbox-label"><input type="checkbox" bind:checked={write} /> Permit writes in this path</label><button type="button" disabled={!path.trim()} onclick={() => { definition.paths = [...definition.paths, { path: path.trim(), write }]; path = ''; }}>Add path</button>
