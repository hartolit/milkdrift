import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import ProjectSettingsTestHarness from './ProjectSettingsTestHarness.svelte';
import { newProjectDraft, type ProjectDraft } from './ProjectSettings.svelte';
import { project, session } from '../lib/canvas.test-fixtures';
import { DaemonClient, DaemonError } from '../lib/transport';
import type { ProjectCommandRequest, ProjectDto, ProjectReceiptDto } from '../lib/protocol.generated';

afterEach(() => { cleanup(); vi.restoreAllMocks(); });
const receipt = (request_id: string) => ({ request_id, project, status: 'accepted', operation_id: null, operation_folder: null, operation_name: null, workflow_id: null });
const environments = { items: [{ environment: { kind: 'host' }, label: 'Host file tools', available: true, reason: null }] };

it('creates one child on the named daemon from bounded permitted choices with no initial write tools', async () => {
  const client = new DaemonClient('http://127.0.0.1:10001', 'test-token');
  const request = vi.spyOn(client, 'request').mockImplementation(async <T>(path: string, body?: unknown): Promise<T> => {
    if (path === '/v1/project-environments') return environments as T;
    if (path.startsWith('/v1/project-directories')) return { parent: null, items: [{ path: '/permitted', name: 'Projects', can_create: true }], next_cursor: null } as T;
    return receipt((body as ProjectCommandRequest).request_id) as T;
  });
  render(ProjectSettingsTestHarness, { client, session: session() });
  await fireEvent.click(screen.getByRole('button', { name: 'Create folder' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Browse permitted folders' }));
  await fireEvent.click(await screen.findByRole('button', { name: 'Use Projects' }));
  await fireEvent.input(screen.getByLabelText('New folder name'), { target: { value: 'animation' } });
  expect(screen.getByText('Studio A', { selector: 'strong' })).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: 'Create project folder' }));
  await waitFor(() => expect(request.mock.calls.some(([path]) => path === '/v1/project-commands')).toBe(true));
  const body = request.mock.calls.find(([path]) => path === '/v1/project-commands')![1] as ProjectCommandRequest;
  expect(body.command).toEqual({ kind: 'create', name: 'animation', parent: '/permitted', folder_name: 'animation', settings: { environment: { kind: 'host' }, allow_changes: false } });
});

it('retains an exact uncertain request and recovers it without making another folder', async () => {
  const client = new DaemonClient('http://127.0.0.1:10001', 'test-token');
  let submitted!: ProjectCommandRequest;
  const request = vi.spyOn(client, 'request').mockImplementation(async <T>(path: string, body?: unknown): Promise<T> => {
    if (path === '/v1/project-environments') return environments as T;
    if (path === '/v1/project-commands') { submitted = body as ProjectCommandRequest; throw new TypeError('Connection interrupted'); }
    return receipt(submitted.request_id) as T;
  });
  const initial = { ...newProjectDraft(), name: 'Code', folder: '/projects/code' };
  render(ProjectSettingsTestHarness, { client, session: session(), initial });
  await fireEvent.click(screen.getByRole('button', { name: 'Attach project' }));
  await screen.findByRole('button', { name: 'Find project receipt' });
  expect((screen.getByLabelText('Project name') as HTMLInputElement).disabled).toBe(true);
  await fireEvent.click(screen.getByRole('button', { name: 'Find project receipt' }));
  await screen.findByText('Saved on this daemon.');
  expect(request.mock.calls.filter(([, body]) => !!body)).toHaveLength(1);
  expect(request.mock.calls.at(-1)?.[0]).toBe(`/v1/project-receipts/${submitted.request_id}`);
});

it('keeps a stale settings draft for an explicit decision and never silently enables writes', async () => {
  const client = new DaemonClient('http://127.0.0.1:10001', 'test-token');
  const refresh = vi.fn(async () => {});
  let state!: ProjectDraft;
  vi.spyOn(client, 'request').mockImplementation(async <T>(path: string): Promise<T> => {
    if (path === '/v1/project-environments') return environments as T;
    throw new DaemonError('conflict', 'Project settings changed.', 409);
  });
  render(ProjectSettingsTestHarness, { client, session: session(), project, onstate: (draft) => state = draft, onrefresh: refresh });
  expect((screen.getByLabelText('Allow project changes') as HTMLInputElement).checked).toBe(false);
  await fireEvent.click(screen.getByLabelText('Allow project changes'));
  await fireEvent.click(screen.getByRole('button', { name: 'Save project settings' }));
  await waitFor(() => expect(refresh).toHaveBeenCalled());
  expect(state.settings.allow_changes).toBe(true);
  expect(state.pending).toBeNull();
  expect(screen.getByRole('alert').textContent).toContain('Your choices are retained');
});

it('makes unresolved creation recoverable with a new explicit command while preserving its original identity', async () => {
  const client = new DaemonClient('http://127.0.0.1:10001', 'test-token');
  let submitted!: ProjectCommandRequest;
  vi.spyOn(client, 'request').mockImplementation(async <T>(path: string, body?: unknown): Promise<T> => {
    if (path === '/v1/project-environments') return environments as T;
    submitted = body as ProjectCommandRequest;
    return receipt(submitted.request_id) as T;
  });
  render(ProjectSettingsTestHarness, { client, session: session(), initial: { ...newProjectDraft(), recovery: { operationId: 'lost-create', folder: '/projects/recovered' } } });
  expect(screen.getByText(/Unresolved folder creation at \/projects\/recovered/)).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: 'Attach inspected folder' }));
  await waitFor(() => expect(submitted).toBeDefined());
  expect(submitted.command).toEqual({ kind: 'reconcile', operation_id: 'lost-create' });
  expect(submitted.request_id).not.toBe('lost-create');
});

it('requires review when an observation changes settings beneath a dirty form', async () => {
  const client = new DaemonClient('http://127.0.0.1:10001', 'test-token');
  const request = vi.spyOn(client, 'request').mockImplementation(async <T>(path: string, body?: unknown): Promise<T> => {
    if (path === '/v1/project-environments') return environments as T;
    return receipt((body as ProjectCommandRequest).request_id) as T;
  });
  const rendered = render(ProjectSettingsTestHarness, { client, session: session(), project });
  await fireEvent.click(screen.getByLabelText('Allow project changes'));
  await rendered.rerender({ project: { ...project, generation: project.generation + 1 } });
  expect((screen.getByRole('button', { name: 'Save project settings' }) as HTMLButtonElement).disabled).toBe(true);
  expect((screen.getByLabelText('Allow project changes') as HTMLInputElement).checked).toBe(true);
  await fireEvent.click(screen.getByRole('button', { name: 'Keep my choices after reviewing current settings' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Save project settings' }));
  await waitFor(() => expect(request.mock.calls.some(([, body]) => !!body)).toBe(true));
  expect((request.mock.calls.find(([, body]) => !!body)![1] as ProjectCommandRequest).command).toMatchObject({
    kind: 'settings', expected_generation: project.generation + 1, settings: { allow_changes: true },
  });
});

it.each([
  { first: 'Rename project', second: 'Save project settings', recover: false },
  { first: 'Save project settings', second: 'Rename project', recover: false },
  { first: 'Rename project', second: 'Save project settings', recover: true },
])('preserves the other form after $first (receipt recovery: $recover)', async ({ first, second, recover }) => {
  const client = new DaemonClient('http://127.0.0.1:10001', 'test-token');
  let saved = structuredClone(project), state!: ProjectDraft, accepted!: ProjectReceiptDto;
  const commands: ProjectCommandRequest[] = [];
  vi.spyOn(client, 'request').mockImplementation(async <T>(path: string, body?: unknown): Promise<T> => {
    if (path === '/v1/project-environments') return environments as T;
    if (path.startsWith('/v1/project-receipts/')) return accepted as T;
    const request = JSON.parse(JSON.stringify(body)) as ProjectCommandRequest;
    commands.push(request);
    expect(request.command).toMatchObject({ project_id: project.id, expected_generation: saved.generation });
    if (request.command.kind === 'rename') saved = { ...saved, name: request.command.name, generation: saved.generation + 1 };
    else if (request.command.kind === 'settings') saved = { ...saved, settings: request.command.settings, generation: saved.generation + 1 };
    else throw new Error('Unexpected project command');
    accepted = { ...receipt(request.request_id), project: structuredClone(saved) };
    if (recover && commands.length === 1) throw new TypeError('Accepted reply was interrupted');
    return accepted as T;
  });
  const rendered = render(ProjectSettingsTestHarness, {
    client, session: session(), project,
    onstate: (draft) => state = draft,
    onaccepted: async (result) => { await rendered.rerender({ project: result.project as ProjectDto }); },
  });
  await fireEvent.input(screen.getByLabelText('Project name'), { target: { value: 'Unsaved project name' } });
  await fireEvent.click(screen.getByLabelText('Allow project changes'));
  await fireEvent.click(screen.getByRole('button', { name: first }));
  if (recover) await fireEvent.click(await screen.findByRole('button', { name: 'Find project receipt' }));
  await waitFor(() => expect(state.reviewedGeneration).toBe(project.generation + 1));
  expect(state.dirty).toBe(true);
  expect((screen.getByLabelText('Project name') as HTMLInputElement).value).toBe('Unsaved project name');
  expect((screen.getByLabelText('Allow project changes') as HTMLInputElement).checked).toBe(true);
  await waitFor(() => expect((screen.getByRole('button', { name: second }) as HTMLButtonElement).disabled).toBe(false));
  await fireEvent.click(screen.getByRole('button', { name: second }));
  await waitFor(() => expect(state.dirty).toBe(false));
  expect(commands).toHaveLength(2);
  expect(saved.name).toBe('Unsaved project name');
  expect(saved.settings.allow_changes).toBe(true);
  expect(state.pending).toBeNull();
});
