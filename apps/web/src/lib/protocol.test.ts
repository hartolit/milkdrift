import { expect, it } from 'vitest';
import fixture from '../../../../crates/control-protocol/tests/fixtures/task-session.json';
import { decodeResponse } from './protocol.generated';
import type { TaskSessionDto } from './protocol.generated';

it('decodes the same golden session envelope exercised by Rust', () => {
  const session = decodeResponse<TaskSessionDto>(fixture);
  expect(session.daemon_id).toBe('fixture-daemon-a');
  expect(session.actor_id).toBe('fixture-actor');
  expect(session.input_limit_bytes).toBe(16384);
  expect(session.capabilities[0]).toEqual({ id: 'fixture-report', operation: 'process.run' });
  expect(session.limits.tasks).toBe(256);
});

it('refuses an incompatible daemon envelope before displaying its data', () => {
  expect(() => decodeResponse({ ...fixture, protocol: { major: 2, minor: 20 } })).toThrow('Incompatible daemon protocol');
  expect(() => decodeResponse({ protocol: fixture.protocol })).toThrow('Incompatible daemon protocol');
});
