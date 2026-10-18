<script lang="ts">
  import type { InputBinding, TaskCapability, TaskDefinitionDto } from '../lib/protocol.generated';
  import TaskModelOptions from './TaskModelOptions.svelte';
  import { capabilityKey, createDefinitionEditorDraft, type DefinitionEditorDraft } from '../lib/task-composer';

  let { definition = $bindable(), capabilities = [], compact = false, editor = $bindable(createDefinitionEditorDraft()) }: {
    definition: TaskDefinitionDto;
    capabilities?: TaskCapability[];
    compact?: boolean;
    editor?: DefinitionEditorDraft;
  } = $props();

  function clearNonModelChoices(operation: string) {
    if (operation !== 'model.generate') {
      definition.actions = null;
      definition.tools = [];
    }
  }

  function addInput() {
    if (!editor.inputName.trim()) return;
    let binding: InputBinding;
    switch (editor.inputKind) {
      case 'inline': binding = { kind: 'inline', value: editor.inputAbsent ? { kind: 'absent' } : { kind: 'text', text: editor.inputSource } }; break;
      case 'supplied': binding = { kind: 'supplied', name: editor.inputSource, optional: editor.inputOptional }; break;
      case 'task_output': binding = { kind: 'task_output', task: editor.inputSource, output: editor.inputOutput, optional: editor.inputOptional }; break;
      case 'settled_outcome': binding = { kind: 'settled_outcome', task: editor.inputSource }; break;
    }
    definition.inputs = { ...definition.inputs, [editor.inputName.trim()]: binding };
    editor.inputName = ''; editor.inputSource = '';
  }
</script>

{#if !compact}<fieldset class="form-section"><legend>Task instructions</legend>
<label>Task title<input bind:value={definition.title} required maxlength="256" /></label>
<label>Short description<input bind:value={definition.description} maxlength="4096" /></label>
<label>Instructions<textarea bind:value={definition.instructions} required maxlength="16384" rows="5"></textarea></label>
</fieldset>{/if}
<fieldset class="form-section"><legend>Execution</legend>
{#if !compact}{#if capabilities.length}
  <label>Configured capability<select value={capabilityKey(definition.capability)} onchange={(event) => {
    const choice = capabilities.find((item) => capabilityKey(item) === event.currentTarget.value);
    if (choice) { definition.capability = { ...choice }; clearNonModelChoices(choice.operation); }
  }}>{#each capabilities as capability (capabilityKey(capability))}<option value={capabilityKey(capability)}>{capability.id} · {capability.operation}</option>{/each}</select></label>
{:else}
  <div class="form-grid"><label>Capability identity<input bind:value={definition.capability.id} maxlength="256" required /></label><label>Operation<input value={definition.capability.operation} oninput={(event) => { definition.capability.operation = event.currentTarget.value; clearNonModelChoices(event.currentTarget.value); }} maxlength="256" required /></label></div>
{/if}{/if}
<label class="checkbox-label"><input type="checkbox" bind:checked={definition.required} /> Required for workflow completion</label>
<details class="advanced-options" open><summary>Corrective link</summary><p class="field-hint">Link new work to the task it addresses. The original instructions and result remain unchanged.</p>
<label>Corrects task<input value={definition.corrective_of ?? ''} oninput={(event) => definition.corrective_of = event.currentTarget.value || null} maxlength="256" placeholder="Prior task identity, or leave empty" /></label>
</details>
{#if definition.capability.operation === 'model.generate'}<TaskModelOptions bind:actions={definition.actions} bind:tools={definition.tools} bind:processSelection={editor.processSelection} {capabilities} />{/if}
</fieldset>

<fieldset class="form-section"><legend>Named inputs</legend>
<p class="field-hint">Required outputs need a successful producer. Optional outputs still wait for settlement. A settled outcome explicitly includes failure. Supplying a new value affects future starts.</p>
{#each Object.entries(definition.inputs) as [name, input] (name)}
  <div class="draft-row"><strong>{name}</strong><span>{input.kind === 'supplied' ? `Workflow input ${input.name} · ${input.optional ? 'optional' : 'required'}` : input.kind === 'task_output' ? `${input.task} / ${input.output} · ${input.optional ? 'optional' : 'required success'}` : input.kind === 'settled_outcome' ? `Settled outcome of ${input.task}, including failure` : `Inline ${input.value.kind}`}</span><button type="button" onclick={() => { const next = { ...definition.inputs }; delete next[name]; definition.inputs = next; }} aria-label={`Remove input ${name}`}>Remove</button></div>
{/each}
<div class="form-grid"><label>Input binding name<input bind:value={editor.inputName} maxlength="128" /></label><label>Input policy<select bind:value={editor.inputKind}><option value="supplied">Supplied workflow input</option><option value="inline">Inline text or absence</option><option value="task_output">Successful task output</option><option value="settled_outcome">Settled task outcome (including failure)</option></select></label></div>
<label>{editor.inputKind === 'inline' ? 'Inline text' : editor.inputKind === 'supplied' ? 'Supplied input name' : 'Producer task identity or batch alias'}<input bind:value={editor.inputSource} maxlength="16384" /></label>
{#if editor.inputKind === 'task_output'}<label>Producer output name<input bind:value={editor.inputOutput} maxlength="128" /></label>{/if}
{#if editor.inputKind === 'task_output' || editor.inputKind === 'supplied'}<label class="checkbox-label"><input type="checkbox" bind:checked={editor.inputOptional} /> {editor.inputKind === 'task_output' ? 'Accept absent input after settlement' : 'Accept an absent supplied value'}</label>{/if}
{#if editor.inputKind === 'inline'}<label class="checkbox-label"><input type="checkbox" bind:checked={editor.inputAbsent} /> Explicit absence</label>{/if}
<button type="button" onclick={addInput} disabled={!editor.inputName.trim() || (Object.keys(definition.inputs).length >= 64 && !(editor.inputName.trim() in definition.inputs))}>Set named input</button>
{#if Object.keys(definition.inputs).length >= 64}<p class="field-hint">This task has reached its limit of 64 inputs. Remove one to add another.</p>{/if}
</fieldset>

<details class="advanced-options" open><summary>Explicit dependencies</summary>
<p class="field-hint">Add prerequisite tasks for this work. Input bindings also establish their own dependencies.</p>
{#each definition.dependencies as id (id)}<div class="draft-row"><code>{id}</code><button type="button" aria-label={`Remove dependency ${id}`} onclick={() => definition.dependencies = definition.dependencies.filter((item) => item !== id)}>Remove</button></div>{/each}
<label>Dependency task identity or batch alias<input bind:value={editor.dependency} maxlength="256" /></label><button type="button" disabled={!editor.dependency.trim() || definition.dependencies.includes(editor.dependency.trim()) || definition.dependencies.length >= 256} onclick={() => { definition.dependencies = [...definition.dependencies, editor.dependency.trim()]; editor.dependency = ''; }}>Add dependency</button>
{#if definition.dependencies.length >= 256}<p class="field-hint">This task has reached its limit of 256 dependencies. Remove one to add another.</p>{/if}
</details>

<details class="advanced-options"><summary>Declared outputs</summary>
<p class="field-hint">Name the outputs that future tasks may use and bound their retained content.</p>
{#each Object.entries(definition.outputs) as [name, output] (name)}
  <fieldset class="nested-fields"><legend>{name}</legend><label class="checkbox-label"><input type="checkbox" bind:checked={output.required} /> Required output</label><label>Media type for {name}<input value={output.media_type ?? ''} oninput={(event) => output.media_type = event.currentTarget.value || null} maxlength="256" /></label><label>Maximum bytes for {name}<input type="number" min="1" step="1" bind:value={output.max_bytes} required /></label><button type="button" onclick={() => { const next = { ...definition.outputs }; delete next[name]; definition.outputs = next; }} aria-label={`Remove output ${name}`}>Remove output</button></fieldset>
{/each}
<label>New output name<input bind:value={editor.outputName} maxlength="128" /></label><button type="button" disabled={!editor.outputName.trim() || editor.outputName in definition.outputs || Object.keys(definition.outputs).length >= 64} onclick={() => { definition.outputs = { ...definition.outputs, [editor.outputName.trim()]: { required: true, media_type: 'text/plain', max_bytes: 16384 } }; editor.outputName = ''; }}>Add output</button>
{#if Object.keys(definition.outputs).length >= 64}<p class="field-hint">This task has reached its limit of 64 outputs. Remove one to add another.</p>{/if}
</details>

<details class="advanced-options"><summary>Scoped project paths</summary>
<p class="field-hint">Choose filesystem roots that this task may access, within your daemon grant.</p>
{#each definition.paths as claim, index (`${index}/${claim.path}`)}<div class="draft-row"><code>{claim.path}</code><span>{claim.write ? 'Read and write' : 'Read only'}</span><button type="button" aria-label={`Remove path ${claim.path}`} onclick={() => definition.paths = definition.paths.filter((_, current) => current !== index)}>Remove</button></div>{/each}
<label>Project path<input bind:value={editor.path} maxlength="4096" placeholder="Absolute path within your grant" /></label><label class="checkbox-label"><input type="checkbox" bind:checked={editor.write} /> Permit writes in this path</label><button type="button" disabled={!editor.path.trim() || definition.paths.length >= 32} onclick={() => { definition.paths = [...definition.paths, { path: editor.path.trim(), write: editor.write }]; editor.path = ''; }}>Add path</button>
{#if definition.paths.length >= 32}<p class="field-hint">This task has reached its limit of 32 paths. Remove one to add another.</p>{/if}
</details>
