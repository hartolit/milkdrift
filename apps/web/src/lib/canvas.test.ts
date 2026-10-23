import { describe, expect, it } from 'vitest';
import { addCanvasTask, addSource, cacheTask, capabilityKey, chooseCapability, connectBinding, freezeCanvas, projectGraph, removeConnection, reviseTask, sourceBinding, stageChange, synchronizeDependencies, updateDefinition, updateInstructions, updateSource } from './canvas';

import { acceptedTask, session, workflow, projectCanvas as createCanvas } from './canvas.test-fixtures';

describe('bounded canvas authoring', () => {
  it('shows the current human pause for queued work without rewriting its retained daemon reason', () => {
    const task = acceptedTask(); task.reason = 'missing supplied input source';
    const snapshot = workflow([task]); snapshot.wait_question = 'Wait for my decision.';
    const canvas = createCanvas(session(), { kind: 'existing', workflowId: snapshot.id, title: snapshot.title, generation: snapshot.generation });
    cacheTask(canvas, task);
    expect(projectGraph(canvas, snapshot).nodes.find((node) => node.id === task.id)?.detail).toBe('Workflow paused');
    expect(canvas.details[0].reason).toBe('missing supplied input source');
    expect(snapshot.tasks[0].reason).toBe('missing supplied input source');
    snapshot.wait_question = null;
    expect(projectGraph(canvas, snapshot).nodes.find((node) => node.id === task.id)?.detail).toBe('Waiting for a required input');
  });

  it('keeps an empty canvas unsaved and requires deliberate choice when more than one executor exists', () => {
    const configured = session(); configured.capabilities.push({ id: 'model-b', operation: 'model.generate' });
    const canvas = createCanvas(configured, { kind: 'new' });
    expect(() => freezeCanvas(canvas, configured)).toThrow('Add a task');
    const task = addCanvasTask(canvas, configured);
    updateInstructions(canvas, task, 'Research a question');
    expect(task.definition.capability.id).toBe('');
    expect(() => freezeCanvas(canvas, configured)).toThrow('Choose a model');
    chooseCapability(task, configured, capabilityKey(configured.capabilities[0]));
    expect(freezeCanvas(canvas, configured).request).toMatchObject({ tasks: [{ definition: { authoring_generation: 3 } }] });
  });

  it('submits three research tasks and a connected composition in one exact creation', () => {
    const canvas = createCanvas(session(), { kind: 'new' });
    const research = ['Motion', 'Training', 'Rendering'].map((title) => { const task = addCanvasTask(canvas, session()); updateInstructions(canvas, task, title); task.definition.outputs.model_response = { required: true, media_type: 'application/json', max_bytes: 1048576 }; return task; });
    const compose = addCanvasTask(canvas, session()); updateInstructions(canvas, compose, 'Compare these approaches');
    for (const source of research) connectBinding(canvas, compose, { kind: 'task_output', task: source.id, output: 'model_response', optional: false }, session());
    const frozen = freezeCanvas(canvas, session());
    expect(frozen.path).toBe('/v1/workflows');
    expect('tasks' in frozen.request && frozen.request.tasks).toHaveLength(4);
    expect(compose.definition.dependencies).toEqual(research.map((task) => task.id));
    const request = JSON.stringify(frozen.request);
    updateInstructions(canvas, research[0], 'Changed later');
    expect(JSON.stringify(frozen.request)).toBe(request);
  });

  it('keeps exact workflow identity and generation in a pending edit', () => {
    const canvas = createCanvas(session(), { kind: 'existing', workflowId: 'original/workflow', title: 'Original', generation: 7 });
    const task = addCanvasTask(canvas, session()); updateInstructions(canvas, task, 'Only add here');
    const frozen = freezeCanvas(canvas, session()); canvas.target = { kind: 'existing', workflowId: 'different', title: 'Different', generation: 9 };
    expect(frozen.path).toBe('/v1/workflows/original%2Fworkflow/changes');
    expect(frozen.request).toMatchObject({ expected_generation: 7, changes: [{ kind: 'add', definition: { instructions: 'Only add here' } }] });
  });

  it('uses a configured process slot without asking for its mechanical name', () => {
    const configured = session(); const capability = { id: 'cargo-reader', operation: 'process.run' };
    configured.capabilities = [capability]; configured.authoring = [{ capability, category: 'process', generation: 9, label: 'Read Cargo manifest', inputs: [{ name: 'manifest', label: 'Cargo manifest', required: true, media_type: 'text/plain', max_bytes: 16384, accepts_artifacts: true }], outputs: [], allows_additional_inputs: false, default_output: null }];
    const canvas = createCanvas(configured, { kind: 'new' }), task = addCanvasTask(canvas, configured);
    updateInstructions(canvas, task, 'Inspect this manifest');
    expect(() => freezeCanvas(canvas, configured)).toThrow('needs Cargo manifest');
    const source = addSource(canvas, 'Cargo.toml', { kind: 'uploaded', input_id: 'captured-input' });
    expect(connectBinding(canvas, task, sourceBinding(source), configured, undefined, source.id)).toBe('manifest');
    expect(task.definition.inputs).toEqual({ manifest: { kind: 'inline', value: { kind: 'uploaded', input_id: 'captured-input' } } });
    expect(() => freezeCanvas(canvas, configured)).not.toThrow();
    task.definition.inputs.other = { kind: 'inline', value: { kind: 'text', text: 'wrong slot' } };
    expect(() => freezeCanvas(canvas, configured)).toThrow('connections do not match');
  });

  it('refuses stale capability metadata and changed authenticated ownership', () => {
    const original = session(), canvas = createCanvas(original, { kind: 'new' }), task = addCanvasTask(canvas, original);
    updateInstructions(canvas, task, 'Research');
    const changed = session(); changed.authoring[0].generation++;
    expect(() => freezeCanvas(canvas, changed)).toThrow('changed');
    expect(() => freezeCanvas(canvas, { ...original, actor_id: 'other-actor' })).toThrow('another authenticated');
    chooseCapability(task, changed, capabilityKey(changed.capabilities[0]));
    expect(() => freezeCanvas(canvas, changed)).not.toThrow();
  });

  it('does not remove an ordering prerequisite needed by another input or explicit ordering', () => {
    const canvas = createCanvas(session(), { kind: 'new' }), task = addCanvasTask(canvas, session());
    const first = connectBinding(canvas, task, { kind: 'task_output', task: 'producer', output: 'model_response', optional: false }, session());
    const second = connectBinding(canvas, task, { kind: 'task_output', task: 'producer', output: 'final_text', optional: true }, session());
    removeConnection(task, first); expect(task.definition.dependencies).toEqual(['producer']);
    task.explicitDependencies = ['producer']; synchronizeDependencies(task);
    removeConnection(task, second); expect(task.definition.dependencies).toEqual(['producer']);
  });

  it('keeps source edits local to connected tasks and allocates distinct missing inputs', () => {
    const canvas = createCanvas(session(), { kind: 'new' }), first = addCanvasTask(canvas, session()), second = addCanvasTask(canvas, session());
    const a = addSource(canvas, 'A'), b = addSource(canvas, 'B', null);
    connectBinding(canvas, first, sourceBinding(a), session(), undefined, a.id);
    connectBinding(canvas, second, sourceBinding(b), session(), undefined, b.id);
    updateSource(canvas, a, 'Only A gets this', 16384);
    expect(Object.values(first.definition.inputs)).toEqual([{ kind: 'inline', value: { kind: 'text', text: 'Only A gets this' } }]);
    expect(Object.values(second.definition.inputs)).toEqual([{ kind: 'supplied', name: b.id, optional: false }]);
  });

  it('refuses the thirty-third change without splitting or losing the drawing', () => {
    const canvas = createCanvas(session(), { kind: 'existing', workflowId: 'a', title: 'A', generation: 1 });
    for (let index = 0; index < 32; index++) stageChange(canvas, { kind: 'wait', question: `Decision ${index}` });
    expect(() => stageChange(canvas, { kind: 'continue' })).toThrow('at most 32');
    expect(canvas.changes).toHaveLength(32);
    expect('changes' in freezeCanvas(canvas, session()).request).toBe(true);
  });

  it('reserves a single stop identity and refuses a ninth dirty drawing while allowing observation', () => {
    const canvas = createCanvas(session(), { kind: 'existing', workflowId: 'a', title: 'A', generation: 1 });
    cacheTask(canvas, acceptedTask()); stageChange(canvas, { kind: 'stop', task: 'accepted-task' });
    expect(freezeCanvas(canvas, session()).request.request_id).toBe('stop-accepted-task');
    const readonly = createCanvas(session(), { kind: 'existing', workflowId: 'b', title: 'B', generation: 1 }); readonly.capacityBlocked = true;
    expect(() => addCanvasTask(readonly, session())).toThrow('8 drafts');
    expect(() => cacheTask(readonly, acceptedTask())).not.toThrow();
  });

  it('keeps complete draft content when a text input exceeds its byte allowance', () => {
    const canvas = createCanvas(session(), { kind: 'new' }), source = addSource(canvas);
    updateSource(canvas, source, 'Kept', 8);
    expect(() => updateSource(canvas, source, '🦀🦀🦀', 8)).toThrow('byte limit');
    expect(source.text).toBe('Kept');
  });

  it('rolls back a large shared draft source after counting every connected copy', () => {
    const canvas = createCanvas(session(), { kind: 'new' }), source = addSource(canvas);
    for (let index = 0; index < 32; index++) { const task = addCanvasTask(canvas, session()); connectBinding(canvas, task, sourceBinding(source), session(), undefined, source.id); }
    expect(() => updateSource(canvas, source, 'x'.repeat(70000), 100000)).toThrow('2 MiB');
    expect(source.text).toBe('');
    expect(canvas.tasks.every((task) => Object.values(task.definition.inputs).every((input) => input.kind === 'inline' && input.value.kind === 'text' && input.value.text === ''))).toBe(true);
    const before = canvas.changes.length;
    expect(() => stageChange(createCanvas(session(), { kind: 'new' }), { kind: 'wait', question: 'x'.repeat(2097152) })).toThrow('2 MiB');
    expect(canvas.changes).toHaveLength(before);
  });

  it('preserves accepted prerequisites when a result connection is removed after reconnect', () => {
    const original = acceptedTask(); original.definition.dependencies = ['producer']; original.definition.inputs.answer = { kind: 'task_output', task: 'producer', output: 'answer', optional: false };
    const canvas = createCanvas(session(), { kind: 'existing', workflowId: 'workflow-a', title: 'A', generation: 1 });
    const draft = reviseTask(canvas, original, session()); removeConnection(draft, 'answer');
    expect(draft.definition.dependencies).toEqual(['producer']);
    expect(projectGraph(canvas, workflow([original])).edges).toContainEqual(expect.objectContaining({ source: 'producer', label: 'Run after' }));
  });

  it('refuses an output port left over from an incompatible executor selection', () => {
    const canvas = createCanvas(session(), { kind: 'new' }), task = addCanvasTask(canvas, session()); updateInstructions(canvas, task, 'Changed producer');
    task.definition.outputs.report = { required: true, media_type: 'text/plain', max_bytes: 1024 };
    expect(() => freezeCanvas(canvas, session())).toThrow('result is not published');
    expect(task.definition.outputs.report).toBeTruthy();
  });
});

describe('authoritative canvas projections', () => {
  it('shows current shared input separately from an older frozen start', () => {
    const task = acceptedTask(); task.definition.inputs.document = { kind: 'supplied', name: 'shared-document', optional: false };
    task.frozen = { capability_generation: 3, actor_id: 'actor-a', grant_id: 'grant', grant_revision: 1, entered: true, inputs: { document: { restricted: false, kind: 'text', name: null, text: 'Older frozen content', text_truncated: false, digest: 'old', artifact_id: null } } };
    const snapshot = workflow([task]); snapshot.supplied_inputs['shared-document'] = { availability: 'available', value: { restricted: false, kind: 'text', name: null, text: 'Current shared content', text_truncated: false, digest: 'new', artifact_id: null } };
    const canvas = createCanvas(session(), { kind: 'existing', workflowId: snapshot.id, title: snapshot.title, generation: 5 }); cacheTask(canvas, task);
    const source = projectGraph(canvas, snapshot).nodes.find((node) => node.id === 'supplied:shared-document');
    expect(source?.detail).toBe('Current shared content');
    expect(canvas.details[0].frozen?.inputs.document.text).toBe('Older frozen content');
  });

  it('reconstructs captured filenames before a queued task starts', () => {
    const task = acceptedTask(); task.definition.inputs.manifest = { kind: 'inline', value: { kind: 'uploaded', input_id: 'upload-a' } };
    task.inline_inputs.manifest = { availability: 'available', value: { restricted: false, kind: 'text_file', name: 'Cargo.toml', text: '[package]', text_truncated: false, digest: 'file', artifact_id: null } };
    const canvas = createCanvas(session(), { kind: 'existing', workflowId: 'a', title: 'A', generation: 1 }); cacheTask(canvas, task);
    expect(projectGraph(canvas, workflow([task])).nodes.find((node) => node.kind === 'source')).toMatchObject({ label: 'Cargo.toml', detail: '[package]' });
  });

  it('keeps off-page producers distinct from absent inputs and preserves duplicate task identities', () => {
    const first = acceptedTask('a'), second = acceptedTask('b'); first.definition.dependencies = ['outside']; first.definition.inputs.result = { kind: 'task_output', task: 'outside', output: 'answer', optional: false };
    const canvas = createCanvas(session(), { kind: 'existing', workflowId: 'a', title: 'A', generation: 1 }); cacheTask(canvas, first); cacheTask(canvas, second);
    const projection = projectGraph(canvas, workflow([first, second]));
    expect(projection.nodes.filter((node) => node.kind === 'task')).toHaveLength(2);
    expect(projection.nodes.find((node) => node.id === 'outside')).toMatchObject({ kind: 'boundary', label: 'Task outside this page' });
    expect(projection.edges).toHaveLength(1);
  });

  it('labels restricted, expired and missing sources without replacing them with empty text', () => {
    const task = acceptedTask();
    for (const name of ['restricted', 'expired', 'missing']) task.definition.inputs[name] = { kind: 'supplied', name, optional: false };
    const snapshot = workflow([task]); snapshot.supplied_inputs = { restricted: { availability: 'restricted', value: null }, expired: { availability: 'expired', value: null } };
    const canvas = createCanvas(session(), { kind: 'existing', workflowId: 'a', title: 'A', generation: 1 }); cacheTask(canvas, task);
    expect(projectGraph(canvas, snapshot).nodes.filter((node) => node.kind === 'source').map((node) => node.detail)).toEqual(['Access restricted', 'Input expired', 'Missing required input']);
  });
});


it('refuses oversized definition additions while preserving the prior draft', () => {
  const configured = session(), canvas = createCanvas(configured, { kind: 'new' }), task = addCanvasTask(canvas, configured);
  updateInstructions(canvas, task, 'Preserve this draft');
  const previous = task.definition;
  expect(() => updateDefinition(canvas, task, { ...task.definition, paths: [{ path: `/${'x'.repeat(2 * 1024 * 1024)}`, write: false }] })).toThrow('2 MiB');
  expect(task.definition).toBe(previous);
  expect(task.definition.instructions).toBe('Preserve this draft');
});


it('keeps source content unchanged while an exact submission is unconfirmed', () => {
  const configured = session(), canvas = createCanvas(configured, { kind: 'new' }), task = addCanvasTask(canvas, configured);
  updateInstructions(canvas, task, 'Keep this context');
  const source = addSource(canvas, 'Evidence', { kind: 'text', text: 'Original' });
  connectBinding(canvas, task, sourceBinding(source), configured, undefined, source.id);
  canvas.pending = freezeCanvas(canvas, configured);
  expect(() => updateSource(canvas, source, 'Changed', configured.input_limit_bytes)).toThrow('Recover');
  expect(source.text).toBe('Original');
  expect(task.definition.inputs[source.id]).toEqual({ kind: 'inline', value: { kind: 'text', text: 'Original' } });
});


it('pins the observed executor generation when revising accepted work', () => {
  const configured = session(), canvas = createCanvas(configured, { kind: 'existing', workflowId: 'workflow', title: 'Work', generation: 1 });
  const draft = reviseTask(canvas, acceptedTask(), configured);
  expect(draft.contractGeneration).toBe(configured.authoring[0].generation);
  const changed = session(); changed.authoring[0].generation++;
  expect(() => freezeCanvas(canvas, changed)).toThrow('changed. Reselect it');
});
