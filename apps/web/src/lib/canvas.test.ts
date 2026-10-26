import { describe, expect, it } from 'vitest';
import { addCanvasTask, addSource, addConnection, removeUnusedCanvasReferences, adoptAuthoring, canvasDocument, cacheTask, capabilityKey, chooseCapability, copy, projectGraph, resolveConnections, reviseTask, updateDefinition, updateInstructions, updateSource, useUsualModel } from './canvas';
import { acceptedTask, session, workflow, projectCanvas as createCanvas, topology } from './canvas.test-fixtures';

describe('saved canvas authoring', () => {
  it('places blank cards without choosing a model, changing selection, opening details or moving the camera', () => {
    const configured = session(), canvas = createCanvas(configured, { kind: 'existing', workflowId: 'w', title: 'Ideas', generation: 1 });
    canvas.camera = { x: 32, y: -50, zoom: .6 }; canvas.selected = 'other';
    const a = addCanvasTask(canvas, configured), b = addCanvasTask(canvas, configured);
    expect(a.definition.instructions).toBe(''); expect(a.definition.capability.id).toBe(''); expect(a.definition.dependencies).toEqual({});
    expect(canvas.positions[a.id]).not.toEqual(canvas.positions[b.id]); expect(canvas.inspector).toBe(false); expect(canvas.selected).toBe('other'); expect(canvas.camera).toEqual({ x: 32, y: -50, zoom: .6 });
    expect(canvasDocument(canvas).tasks).toHaveLength(2);
  });
  it('applies a deliberate usual model to just unassigned new cards and future cards', () => {
    const configured = session(), canvas = createCanvas(configured, { kind: 'new' });
    const a = addCanvasTask(canvas, configured), b = addCanvasTask(canvas, configured);
    chooseCapability(b, configured, capabilityKey(configured.capabilities[0])); b.contractGeneration = 2;
    const accepted = acceptedTask(); canvas.topology = topology([accepted]); reviseTask(canvas, accepted, configured);
    useUsualModel(canvas, configured, capabilityKey(configured.capabilities[0]), true);
    expect(canvas.highlighted).toEqual([a.id]); expect(a.contractGeneration).toBe(3); expect(b.contractGeneration).toBe(2);
    expect(canvas.tasks.find((task) => task.version !== null)?.definition).toMatchObject({ capability: accepted.definition.capability });
    expect(addCanvasTask(canvas, configured).definition.capability).toEqual(configured.capabilities[0]);
  });
  it('keeps default-only choice off existing unassigned tasks and preserves an unavailable reviewed generation', () => {
    const configured = session(), canvas = createCanvas(configured, { kind: 'new' });
    const first = addCanvasTask(canvas, configured);
    useUsualModel(canvas, configured, capabilityKey(configured.capabilities[0]), false);
    configured.authoring[0].generation = 4;
    const next = addCanvasTask(canvas, configured);
    expect(first.definition.capability.id).toBe(''); expect(next.contractGeneration).toBe(3); expect(canvas.usualModel?.generation).toBe(3);
  });
  it('preserves an unfinished physical wire and resolves only an advertised default after model choice', () => {
    const configured = session(), canvas = createCanvas(configured, { kind: 'new' });
    const a = addCanvasTask(canvas, configured), b = addCanvasTask(canvas, configured);
    const wire = addConnection(canvas, configured, a.id, b.id);
    expect(wire.input).toBeNull(); expect(wire.output).toBeNull(); expect(projectGraph(canvas, null).edges[0].unfinished).toBe(true);
    useUsualModel(canvas, configured, capabilityKey(configured.capabilities[0]), true);
    expect(wire.output).toBe('model_response'); expect(wire.input).toBe(`input_${wire.id}`);
    expect(a.definition.outputs).toEqual({}); expect(b.definition.inputs).toEqual({}); expect(b.definition.dependencies).toEqual({});
  });
  it('does not choose between equally meaningful output or input ports', () => {
    const configured = session(); configured.authoring[0].default_output = null;
    configured.authoring[0].allows_additional_inputs = false;
    configured.authoring[0].inputs = ['question', 'context'].map((name) => ({ name, label: name, required: false, media_type: 'text/plain', max_bytes: 1024, accepts_artifacts: true }));
    const canvas = createCanvas(configured, { kind: 'new' }); useUsualModel(canvas, configured, capabilityKey(configured.capabilities[0]), false);
    const a = addCanvasTask(canvas, configured), b = addCanvasTask(canvas, configured), wire = addConnection(canvas, configured, a.id, b.id);
    expect(wire.input).toBeNull(); expect(wire.output).toBeNull();
    wire.output = 'final_text'; wire.input = 'context'; resolveConnections(canvas, configured);
    expect(wire.output).toBe('final_text'); expect(wire.input).toBe('context');
    configured.authoring[0].outputs = []; resolveConnections(canvas, configured); expect(wire.output).toBe('final_text');
  });
  it('round-trips incomplete cards, sources and wires without storing viewport or duplicating admitted definitions', () => {
    const configured = session(), original = createCanvas(configured, { kind: 'new' });
    const task = addCanvasTask(original, configured), source = addSource(original, 'Notes'); updateSource(original, source, 'A useful question', 1024);
    addConnection(original, configured, source.id, task.id); original.accepted = { former_card: 'accepted-task' };
    const document = canvasDocument(original), restored = createCanvas(configured, { kind: 'new' }); adoptAuthoring(restored, document);
    expect(canvasDocument(restored)).toEqual(document); expect(document).not.toHaveProperty('camera'); expect(document).not.toHaveProperty('positions'); expect(restored.tasks).toHaveLength(1);
  });
  it('edits a source once while all wires retain the same source identity', () => {
    const configured = session(), canvas = createCanvas(configured, { kind: 'new' });
    const source = addSource(canvas), a = addCanvasTask(canvas, configured), b = addCanvasTask(canvas, configured);
    addConnection(canvas, configured, source.id, a.id); addConnection(canvas, configured, source.id, b.id);
    updateSource(canvas, source, 'Updated shared context', 1024);
    expect(canvasDocument(canvas).sources[0].value).toEqual({ kind: 'text', text: 'Updated shared context' }); expect(canvas.connections.map((wire) => wire.source)).toEqual([source.id, source.id]);
    expect(a.definition.inputs).toEqual({}); expect(b.definition.inputs).toEqual({});
  });
  it('materializes existing candidate connections using daemon-projected legacy predicates', () => {
    const configured = session(), canvas = createCanvas(configured, { kind: 'new' }), original = acceptedTask();
    original.definition.dependencies = ['producer']; original.definition.inputs = { result: { kind: 'task_output', task: 'producer', output: 'report', optional: true } };
    canvas.topology = topology([original]); canvas.topology.tasks[0].dependencies = { producer: 'settled' }; canvas.topology.tasks[0].dependencies_legacy = true;
    const draft = reviseTask(canvas, original, configured);
    expect(draft.definition.inputs).toEqual({}); expect(draft.definition.dependencies).toEqual({});
    expect(canvas.connections.map((wire) => wire.kind)).toEqual(['data', 'after_completion']);
    canvas.connections = canvas.connections.filter((wire) => wire.kind !== 'data');
    expect(canvas.connections[0].kind).toBe('after_completion'); expect(original.definition.inputs.result.kind).toBe('task_output');
  });
  it('retains admitted source placement only while its exact authoritative binding matches', () => {
    const configured = session(), canvas = createCanvas(configured, { kind: 'new' }), task = acceptedTask();
    const source = addSource(canvas, 'Notes', { kind: 'text', text: 'Original context' });
    task.definition.inputs = { context: { kind: 'inline', value: { kind: 'text', text: 'Original context' } } };
    canvas.accepted.old = task.id; canvas.topology = topology([task]); canvas.topology.tasks[0].inputs.context = { kind: 'inline', source: `inline:${task.id}:context`, output: null, optional: false, preview: null }; cacheTask(canvas, task);
    canvas.connections = [{ id: 'retained', source: source.id, target: 'old', input: 'context', output: null, kind: 'data', optional: false, outcome: false }];
    let graph = projectGraph(canvas, workflow([task]));
    expect(graph.edges).toHaveLength(1); expect(graph.edges[0]).toMatchObject({ source: source.id, target: task.id, draft: false });
    expect(graph.nodes.filter((node) => node.kind === 'source')).toHaveLength(1);
    source.value = { kind: 'text', text: 'Prospective replacement' };
    graph = projectGraph(canvas, workflow([task])); expect(graph.edges[0].source).toBe(`inline:${task.id}:context`);
    source.value = { kind: 'text', text: 'Original context' };
    reviseTask(canvas, task, configured);
    expect(canvas.connections).toHaveLength(1); expect(canvas.connections[0].source).toBe(source.id); expect(canvas.connections[0].target).toBe(task.id);
  });
  it('matches an uploaded source by authorized artifact identity without requiring its content', () => {
    const canvas = createCanvas(session(), { kind: 'new' }), task = acceptedTask();
    const source = addSource(canvas, 'Captured file', { input_id: 'captured-1', kind: 'uploaded' });
    canvas.topology = topology([task]); canvas.topology.tasks[0].inputs.context = { kind: 'inline', source: `inline:${task.id}:context`, output: null, optional: false, preview: { availability: 'available', value: { restricted: false, kind: 'artifact', name: 'Input.txt', text: 'truncated', text_truncated: true, digest: 'digest', artifact_id: 'captured-1' } } };
    canvas.connections = [{ id: 'retained', source: source.id, target: task.id, input: 'context', output: null, kind: 'data', optional: false, outcome: false }];
    expect(projectGraph(canvas, workflow([task])).edges[0].source).toBe(source.id);
    canvas.topology.tasks[0].inputs.context.preview = null;
    expect(projectGraph(canvas, workflow([task])).edges[0].source).toBe(`inline:${task.id}:context`);
  });
  it('refuses editing frozen work and an unreviewed topology version without losing current drafts', () => {
    const canvas = createCanvas(session(), { kind: 'new' }), original = acceptedTask(); addCanvasTask(canvas, session());
    expect(() => reviseTask(canvas, original, session())).toThrow('Refresh');
    original.started_at = 10; expect(() => reviseTask(canvas, original, session())).toThrow('already started'); expect(canvas.tasks).toHaveLength(1);
  });
  it('retains daemon-advertised draft bounds independently of the smaller Run action limit', () => {
    const configured = session(); configured.authoring_bounds.pending_tasks = 35;
    const canvas = createCanvas(configured, { kind: 'new' });
    for (let index = 0; index < 35; index++) addCanvasTask(canvas, configured);
    expect(canvas.tasks).toHaveLength(35); expect(() => addCanvasTask(canvas, configured)).toThrow('35 pending');
    expect(canvas.tasks).toHaveLength(35);
  });
  it('retains content after an oversized edit and derives a readable bounded title', () => {
    const configured = session(), canvas = createCanvas(configured, { kind: 'new' }), task = addCanvasTask(canvas, configured);
    updateInstructions(canvas, task, 'A useful question\nDetails'); expect(task.definition.title).toBe('A useful question');
    const before = copy(task.definition); canvas.bounds.document_bytes = 1024;
    expect(() => updateDefinition(canvas, task, { ...task.definition, instructions: 'x'.repeat(3000) })).toThrow('draft limit'); expect(task.definition).toEqual(before);
  });
  it('projects current pause and off-page sources without requiring task result reads', () => {
    const task = acceptedTask(), snapshot = workflow([task]), canvas = createCanvas(session(), { kind: 'new' });
    task.reason = 'Project is in use'; snapshot.tasks[0].reason = task.reason; snapshot.wait_question = 'Wait for review';
    canvas.topology = topology([task]); canvas.topology.tasks[0].inputs = { context: { kind: 'task_output', source: 'outside', output: 'report', optional: false, preview: null } };
    const graph = projectGraph(canvas, snapshot); expect(canvas.details).toEqual([]); expect(graph.nodes.find((node) => node.id === task.id)?.detail).toBe('Workflow paused'); expect(graph.nodes.find((node) => node.id === 'outside')?.kind).toBe('boundary');
    cacheTask(canvas, task); expect(canvas.details[0].reason).toBe('Project is in use');
  });
  it('projects only the visible consumer page and its actual off-page references', () => {
    const consumer = acceptedTask('consumer'), producer = acceptedTask('producer'), unrelated = acceptedTask('unrelated');
    const canvas = createCanvas(session(), { kind: 'new' }); canvas.topology = topology([consumer, producer, unrelated]);
    canvas.topology.tasks[0].inputs.context = { kind: 'task_output', source: producer.id, output: 'report', optional: false, preview: null };
    canvas.topology.tasks[2].inputs.secret = { kind: 'supplied', source: 'supplied:unrelated', output: null, optional: false, preview: null };
    const graph = projectGraph(canvas, workflow([consumer]));
    expect(graph.nodes.map((node) => node.id)).toEqual(['consumer', 'producer']); expect(graph.edges).toHaveLength(1);
  });
  it('retains restricted and expired shared-input status instead of replacing it with a generic label', () => {
    const task = acceptedTask(), canvas = createCanvas(session(), { kind: 'new' }); canvas.topology = topology([task]);
    for (const availability of ['restricted', 'expired'] as const) {
      canvas.topology.tasks[0].inputs.context = { kind: 'supplied', source: 'supplied:private', output: null, optional: false, preview: { availability, value: null } };
      expect(projectGraph(canvas, workflow([task])).nodes.find((node) => node.id === 'supplied:private')).toMatchObject({ label: 'Task input', detail: availability === 'restricted' ? 'Access restricted' : 'Input expired' });
    }
  });
  it('frees full alias and wire envelopes while retaining pending endpoints, changes and standalone ideas', () => {
    const visible = acceptedTask('visible'), candidate = acceptedTask('old_2'), snapshot = workflow([visible]);
    const canvas = createCanvas(session(), { kind: 'new' }); canvas.topology = topology([visible, candidate]); cacheTask(canvas, candidate);
    reviseTask(canvas, candidate, session());
    const pending = addCanvasTask(canvas, session()), shared = addSource(canvas, 'Still useful'), obsolete = addSource(canvas, 'Earlier input'), idea = addSource(canvas, 'Unconnected idea');
    pending.definition.corrective_of = 'old_30';
    for (let index = 0; index < 600; index++) canvas.accepted[`alias_${index}`] = `old_${index}`;
    const wire = (id: string, source: string, target: string) => ({ id, source, target, input: 'context', output: 'report', kind: 'data' as const, optional: false, outcome: false });
    canvas.connections = [wire('pending', 'alias_1', pending.id), wire('candidate', shared.id, 'alias_2'), wire('shared-old', shared.id, 'alias_3'), wire('visible', shared.id, visible.id)];
    for (let index = 4; index < 1024; index++) canvas.connections.push(wire(`oldwire_${index}`, obsolete.id, `alias_${5 + index % 595}`));
    canvas.changes = [{ kind: 'adopt_correction', task: 'old_0', correction: 'old_10', redirects: [{ consumer: 'old_20', expected_version: 1, input: 'context' }] }];
    canvas.positions[candidate.id] = { x: 333, y: 444 }; canvas.positions[obsolete.id] = { x: 7, y: 8 }; canvas.positions.old_50 = { x: 50, y: 50 }; canvas.positions.old_20 = { x: 20, y: 20 };
    const accepted = copy({ snapshot, topology: canvas.topology, details: canvas.details }), tasks = copy(canvas.tasks), changes = copy(canvas.changes), keptPositions = copy(canvas.positions);
    expect(Object.keys(canvas.accepted)).toHaveLength(600); expect(canvas.connections).toHaveLength(1024);
    removeUnusedCanvasReferences(canvas, snapshot);
    expect(canvas.connections.map((entry) => entry.id)).toEqual(['pending', 'candidate', 'visible']);
    expect(canvas.sources.map((source) => source.id)).toEqual([shared.id, idea.id]);
    expect(Object.keys(canvas.accepted).sort()).toEqual(['alias_0', 'alias_1', 'alias_10', 'alias_2', 'alias_20', 'alias_30']);
    expect(canvas.tasks).toEqual(tasks); expect(canvas.changes).toEqual(changes); expect({ snapshot, topology: canvas.topology, details: canvas.details }).toEqual(accepted);
    expect(canvas.positions[obsolete.id]).toBeUndefined(); expect(canvas.positions.old_50).toBeUndefined(); expect(canvas.positions.old_20).toEqual(keptPositions.old_20);
    for (const id of [pending.id, candidate.id, shared.id, idea.id]) expect(canvas.positions[id]).toEqual(keptPositions[id]);
    expect(() => addConnection(canvas, session(), idea.id, pending.id)).not.toThrow();
  });

});
