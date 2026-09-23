#!/usr/bin/env python3
"""Run the finite fixture qualification on this Linux host using actual product binaries.

Requires the prepared directory, rootless Podman, a user systemd session, and the pinned image.
Creates only the two named approved installations; removes them through the product on success.
On failure retains installations, configuration and evidence for explicit inspection/recovery.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time
import urllib.request

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--root', type=Path, required=True)
parser.add_argument('--cli', type=Path, default=Path('target/debug/milkdrift'))
parser.add_argument('--daemon', type=Path, default=Path('target/debug/milkdrift-daemon'))
parser.add_argument('--port', type=int, default=19748)
args = parser.parse_args()
root, cli, daemon = args.root.resolve(), str(args.cli.resolve()), str(args.daemon.resolve())
logs = root / 'evidence'
logs.mkdir(mode=0o700)
host = root / 'host'
quadlet = Path.home() / '.config/containers/systemd' / ('milkdrift-' + root.name)
bootstrap = [daemon, 'managed-bootstrap', '--root', str(host), '--quadlet-directory', str(quadlet),
             '--systemd-directory', str(Path.home() / '.config/systemd/user'), '--recipe', str(root / 'protected-recipe.json')]
result = subprocess.run(bootstrap, capture_output=True, text=True, check=True)
(logs / 'bootstrap.json').write_text(result.stdout)
protected_ref = json.loads(result.stdout)['recipe']
preview = bootstrap[:-1] + [str(root / 'worker-recipe.json'), '--preview']
result = subprocess.run(preview, capture_output=True, text=True, check=True)
(logs / 'worker-preview.json').write_text(result.stdout)
worker_ref = json.loads(result.stdout)['recipe']
config = host / 'daemon.toml'
text = config.read_text().replace('execution_only', 'workflow_enabled').replace('human:operator', 'agent:repair')
text = text.replace('127.0.0.1:9734', f'127.0.0.1:{args.port}')
text = text.replace('recipes = [' + json.dumps(str(host / 'recipe.json')) + ']',
                    'recipes = [' + json.dumps(str(host / 'recipe.json')) + ', ' + json.dumps(str(root / 'worker-recipe.json')) + ']')
text = text.replace('"managed.slotbook-test.worker",', '"managed.slotbook-test.worker", "managed.slotbook-build", "managed.slotbook-build.worker",')
config.write_text(text)
subprocess.run([daemon, '--config', str(config), '--check-config'], capture_output=True, check=True)
env = dict(os.environ, MILKDRIFT_ENDPOINT=f'http://127.0.0.1:{args.port}', MILKDRIFT_TOKEN_FILE=str(host / 'operator.token'))
process = None


def call(label, command, refused=False, allow_stale_catalog=False):
    result = subprocess.run([cli, '--json', '--yes', '--timeout-secs', '240', '--command-id', label, *map(str, command)],
                            env=env, capture_output=True, text=True)
    (logs / (label + '.json')).write_text(result.stdout)
    (logs / (label + '.stderr')).write_text(result.stderr)
    pages = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
    final = pages[-1]
    stale = (allow_stale_catalog and final['status'] == 'failure' and
             isinstance(final.get('value'), dict) and final['value'].get('type') == 'rejected' and
             final['value'].get('code') == 'catalog_stale' and
             final['value'].get('known_execution') is None)
    assert (result.returncode != 0) if refused else (result.returncode == 0 or stale), (label, result.stdout, result.stderr)
    print(label, flush=True)
    if final['type'] == 'invocation.wait':
        # Output artifacts can arrive in a progress page before the terminal page.
        final['value']['observations'] = [observation for page in pages
                                          for observation in page['value']['observations']]
    return final


def invoke(label, capability, operation, inputs):
    # Discovery is not a reservation. Refresh only after the server explicitly proves a stale
    # catalog refusal with no accepted execution; response loss or any other failure stops here.
    for attempt in range(3):
        identity = label if attempt == 0 else f'{label}-{attempt}'
        request = root / (identity + '-request.json')
        call(identity + '-prepare', ['invocation', 'prepare', capability, operation, '--host', 'host:slotbook-test',
                                    '--request-id', identity, '--inputs', inputs, '--output', request])
        accepted = call(identity + '-submit', ['invocation', 'submit', request], allow_stale_catalog=True)
        if accepted['status'] == 'success':
            return accepted['value']['execution']
    raise RuntimeError('catalog kept changing across three explicit pre-acceptance refusals')


def resource(label, action, version=0, extra=(), target='slotbook-test', refused=False):
    return call(label, ['resource', '--installation', target, '--expected-version', version, action, *extra], refused)


def ref_args(ref):
    return ['--recipe', ref['name'], '--digest', ref['digest']]


def start():
    global process
    process = subprocess.Popen([daemon, '--config', str(config)], stdout=open(logs / ('daemon-' + str(time.time_ns()) + '.log'), 'w'), stderr=subprocess.STDOUT)
    for _ in range(200):
        assert process.poll() is None, 'daemon exited before readiness'
        if subprocess.run([cli, '--json', 'daemon', 'readiness'], env=env, capture_output=True).returncode == 0:
            return
        time.sleep(.1)
    raise RuntimeError('readiness deadline')


def stop():
    global process
    if process is not None:
        process.terminate()
        process.wait(timeout=30)
        process = None


def write(name, value):
    path = root / name
    path.write_text(json.dumps(value))
    return path


try:
    start()
    prepared = resource('protected-prepare', 'prepare', extra=ref_args(protected_ref))['value']
    assert prepared['state'] == 'prepared' and prepared['version'] == 0, prepared
    resource('protected-apply', 'apply', extra=ref_args(protected_ref))
    state = resource('protected-before', 'inspect')['value']
    assert state['pending'] is None and state['desired_running'] is False, state
    version = state['version']
    target_version = json.loads((root / 'governed.json').read_text())['revision']['semantic']['nodes']['verify-candidate']['data_inputs']['target']['binding']['value']['expected_version']
    assert version == target_version, ('re-author with --target-version', version)
    resource('raw-start-refused', 'start', version, refused=True)
    resource('worker-apply', 'apply', extra=ref_args(worker_ref), target='slotbook-build')
    worker = resource('worker-inspect', 'inspect', target='slotbook-build')['value']
    assert worker['pending'] is None and 'managed.slotbook-build.worker' in worker['capabilities'], worker
    seed = (Path(__file__).resolve().parent / 'seeded.py').read_text()
    script = "from pathlib import Path; p=Path('/workspace/source'); p.mkdir(exist_ok=True); (p/'app.py').write_text(" + repr(seed) + "); print((p/'app.py').read_text(),end='')"
    inputs = write('seed-inputs.json', [{'name': 'command', 'value': {'type': 'inline', 'value': {'argv': ['/usr/local/bin/python3', '-I', '-c', script], 'stdout_artifact': True}}}])
    execution = invoke('seed', 'managed.slotbook-build.worker', 'workspace.execute', inputs)
    observation = call('seed-wait', ['invocation', 'wait', execution])['value']
    artifacts = [o['event']['kind']['reference'] for o in observation['observations'] if o['category'] == 'artifact']
    candidate = next(a for a in artifacts if a['media_type'] == 'application/octet-stream')
    evaluation = resource('seed-evaluate', 'evaluate', version, ['--artifact', candidate['identity'], '--digest', candidate['digest'], '--media-type', candidate['media_type'], '--size-bytes', candidate['size_bytes']])['value']['evaluation']['identity']
    failure = resource('seed-evidence', 'evidence', extra=['--evaluation', evaluation])['value']['evaluation']
    assert failure['complete'] and [c['name'] for c in failure['checks'] if c['passed'] is False] == ['authenticated-mutation', 'durable-bookings'], failure
    resource('failed-publish-refused', 'publish', version, ['--evaluation', evaluation], refused=True)
    resource('forged-publish-refused', 'publish', version, ['--evaluation', 'b3_' + 'a' * 64], refused=True)
    report = resource('failed-report', 'evidence', extra=['--evaluation', evaluation])['value']
    for check in report['evaluation']['checks']:
        check['passed'], check['diagnostic'] = True, 'untrusted fabricated pass'
    uploaded = call('forged-upload', ['artifact', 'upload', write('forged-report.json', report), '--host', 'host:slotbook-test',
                                     '--upload-id', 'forged-positive', '--media-type', 'application/vnd.milkdrift.managed+json'])['value']
    forged = {'identity': uploaded['artifact_id'], 'digest': uploaded['digest'], 'media_type': uploaded['content_type'], 'size_bytes': uploaded['size']}
    inputs = write('forged-inputs.json', [
        {'name': 'target', 'value': {'type': 'inline', 'value': {'schema_version': 2, 'command': 'forged-serving', 'installation': 'slotbook-test', 'expected_version': version}}},
        {'name': 'evaluation', 'value': {'type': 'artifact', 'reference': forged}}])
    execution = invoke('forged', 'milkdrift.resources', 'resource.publish_candidate', inputs)
    observation = call('forged-wait', ['invocation', 'wait', execution], refused=True)['value']
    terminal = [o['event']['kind']['terminal'] for o in observation['observations'] if o['category'] in ('terminal', 'uncertainty')]
    assert len(terminal) == 1 and terminal[0]['status'] != 'success', terminal
    assert 'verification is failed' in json.dumps(terminal), terminal
    unchanged = resource('after-forged-report', 'inspect')['value']
    assert unchanged['generation'] == 1 and unchanged['observed_running'] is False and unchanged['pending'] is None, unchanged
    call('base-import', ['blueprint', 'import', root / 'base.json'])
    call('governed-import', ['blueprint', 'import', root / 'governed.json'])
    revision = json.loads((root / 'governed.json').read_text())['revision']
    call('run-start', ['run', 'start', 'slotbook-run', 'slotbook', revision['id']])
    run = call('run-before', ['run', 'show', 'slotbook-run'])['value']
    draft = {'schema_version': 1, 'draft': {'identity': 'repair-slotbook', 'proposer': 'agent:repair', 'provenance': {'type': 'direct'},
             'workflow': 'slotbook', 'run': 'slotbook-run', 'base_revision': revision['id'], 'base_digest': revision['content_digest'],
             'observed_run_sequence': run['sequence'], 'mutation': json.loads((root / 'repair-mutations.json').read_text()),
             'rationale': 'Retain observed authentication and restart failures, repair the candidate and obtain fresh trusted evidence.',
             'rationale_artifact': None, 'risk_notes': [], 'assumptions': ['Finite seeded fixture, not model-generated repair'],
             'evidence': [{'kind': 'worker_observation', 'id': evaluation}], 'artifacts': [candidate],
             'application_policy': 'auto_apply_low_risk', 'requested_action': None, 'claimed_stop': 'complete'}}
    path = write('repair-proposal.json', draft)
    call('proposal-submit', ['--expected-sequence', run['sequence'], '--expected-revision', revision['id'], 'proposal', 'submit', path])
    completed = call('run-wait', ['run', 'wait', 'slotbook-run'])['value']
    assert completed['terminal'] == 'succeeded' and completed['agreement_adoptions'] == 1, completed
    after = resource('published-inspect', 'inspect')['value']
    assert after['pending'] is None and after['observed_running'] is True, after
    accepted_id = after['accepted_evaluation']
    accepted = resource('accepted-evidence', 'evidence', extra=['--evaluation', accepted_id])['value']['evaluation']
    assert accepted['complete'] and all(c['passed'] is True for c in accepted['checks']), accepted
    verified = accepted['subject']['artifact']
    exported = logs / 'deployed-candidate.py'
    call('candidate-download', ['artifact', 'get', verified['artifact'], '--output', exported])
    assert exported.read_bytes() == (Path(__file__).resolve().parent / 'repaired.py').read_bytes()
    service = next(r['identity'] for r in after['resources'] if r['kind'] == 'service')
    container = json.loads(subprocess.run(['/usr/bin/podman', 'inspect', service], capture_output=True, text=True, check=True).stdout)[0]
    mounted = next(m for m in container['Mounts'] if m['Destination'] == '/candidate/app.py')
    assert mounted['RW'] is False and Path(mounted['Source']).read_bytes() == exported.read_bytes()
    (logs / 'served-container.json').write_text(json.dumps(container))
    resource('wrong-generation-refused', 'publish', after['version'], ['--evaluation', accepted_id], refused=True)
    # The immutable report connects run output, private acceptance and served candidate.
    call('timeline', ['run', 'timeline', 'slotbook-run', '--limit', '256'])
    health = json.load(urllib.request.urlopen('http://127.0.0.1:19848/health', timeout=3))
    assert health['status'] == 'ready', health
    stop()
    start()
    reopened = resource('reopened-inspect', 'inspect')['value']
    assert reopened['generation'] == after['generation'] and reopened['observed_running'], reopened
    # A workflow request carries different authority provenance. A direct caller cannot replay it.
    resource('publish-candidate', 'publish', version, ['--evaluation', accepted_id], refused=True)
    direct_version = reopened['version']
    direct = resource('direct-evaluate', 'evaluate', direct_version, ['--artifact', verified['artifact'], '--digest', verified['digest'], '--media-type', verified['media_type'], '--size-bytes', verified['size_bytes']])['value']['evaluation']['identity']
    direct_checks = resource('direct-evidence', 'evidence', extra=['--evaluation', direct])['value']['evaluation']
    assert all(c['passed'] is True for c in direct_checks['checks']), direct_checks
    # A distinct authorized request must be able to verify the same bytes again at the same
    # generation, including when an earlier observation has expired. Exact replay stays inert.
    renewed = resource('renew-evaluate', 'evaluate', direct_version, ['--artifact', verified['artifact'], '--digest', verified['digest'], '--media-type', verified['media_type'], '--size-bytes', verified['size_bytes']])['value']['evaluation']['identity']
    renewed_checks = resource('renew-evidence', 'evidence', extra=['--evaluation', renewed])['value']['evaluation']
    assert renewed != direct and renewed_checks['subject'] == direct_checks['subject']
    assert all(c['passed'] is True for c in renewed_checks['checks']), renewed_checks
    assert resource('prior-evidence-unchanged', 'evidence', extra=['--evaluation', direct])['value']['evaluation'] == direct_checks
    resource('direct-publish', 'publish', direct_version, ['--evaluation', renewed])
    published = resource('direct-published', 'inspect')['value']
    assert published['pending'] is None and published['generation'] == after['generation'] + 1 and published['observed_running'], published
    service = next(r['identity'] for r in published['resources'] if r['kind'] == 'service')
    container = json.loads(subprocess.run(['/usr/bin/podman', 'inspect', service], capture_output=True, text=True, check=True).stdout)[0]
    stop()
    start()
    resource('direct-publish', 'publish', direct_version, ['--evaluation', renewed])
    replayed = resource('after-exact-replay', 'inspect')['value']
    assert replayed['generation'] == published['generation'] and replayed['version'] == published['version']
    again = json.loads(subprocess.run(['/usr/bin/podman', 'inspect', service], capture_output=True, text=True, check=True).stdout)[0]
    assert again['Id'] == container['Id'] and again['State']['StartedAt'] == container['State']['StartedAt']
    retained = resource('failure-retained', 'evidence', extra=['--evaluation', evaluation])['value']['evaluation']
    assert retained == failure
    call('run-reopened', ['run', 'show', 'slotbook-run'])
    for name in ('slotbook-test', 'slotbook-build'):
        state = resource('cleanup-inspect-' + name, 'inspect', target=name)['value']
        resource('cleanup-' + name, 'remove', state['version'], target=name)
        state = resource('removed-' + name, 'inspect', target=name)['value']
        assert state['state'] == 'removed', state
    write('qualification.json', {'completed_run': completed, 'protected_target': after, 'failure_evidence': evaluation,
                               'lane': 'seeded deterministic repair; rootless Linux; finite six-check HTTP verifier'})
    print('finite binary qualification passed', flush=True)
finally:
    stop()
