#!/usr/bin/env python3
"""Author a fresh finite Slotbook example through the production CLI readers/writers.

This prepares local files only. The operator starts the daemon and applies the approved recipes
through the ordinary API. Fixture candidates are explicitly separate from model-generated work.
"""
import argparse
import copy
import json
from pathlib import Path
import secrets
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--root', type=Path, required=True)
parser.add_argument('--cli', type=Path, default=Path('target/debug/milkdrift'))
parser.add_argument('--image', default='sha256:915835de46a63f9598442159437904042f1c69a9b41c4965fba1db9ceab66578')
parser.add_argument('--target-version', type=int, default=6, help='Expected version after initial protected installation; check it before starting the run.')
args = parser.parse_args()
root = args.root.resolve()
root.mkdir(mode=0o700)
example = Path(__file__).resolve().parent
cli = str(args.cli.resolve())


def write(name, value):
    path = root / name
    path.write_text(json.dumps(value, sort_keys=True, separators=(',', ':')))
    path.chmod(0o600)
    return path


def local(*arguments):
    result = subprocess.run([cli, '--json', *map(str, arguments)], capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError(result.stdout + result.stderr)
    return json.loads(result.stdout)['value']


def digest(path):
    return 'b3_' + local('artifact', 'digest', path)['digest']


(root / 'service.token').write_text(secrets.token_hex(24))
(root / 'service.token').chmod(0o600)
(root / 'clock').write_text('2027-04-10T09:00:00Z')
(root / 'clock').chmod(0o444)
source_digest = digest(example / 'verifier.py')
runtime_digest = digest('/usr/bin/python3')
verifier_identity = write('verifier-identity.json', {'source': source_digest, 'runtime': runtime_digest})
policy = {'schema_version': 1, 'required_checks': ['public-availability', 'authenticated-mutation', 'capacity-and-intervals',
          'durable-bookings', 'cancellation-policy', 'exact-deployment'], 'verifier': digest(verifier_identity),
          'producer': 'trusted:slotbook-verifier', 'maximum_candidate_bytes': 65536, 'validity_ms': 3600000}
policy_path = write('policy.json', policy)
policy_digest = local('blueprint', 'effect-policy', policy_path)['digest']

# The editable work selects only this worker envelope. Verifier/publisher nodes are outside it.
requirement = {'cancellation_required': False, 'categories': [{'type': 'process'}], 'exact_capability': 'managed.slotbook-build.worker',
               'maximum_side_effect': 'non_idempotent_write', 'operation': 'workspace.execute', 'provider_profile': None,
               'required_features': [], 'streaming': None, 'trust_zones': []}
context = {'ancestor_depth': None, 'artifact_selector': None, 'budget': {'max_artifact_bytes': 16777216, 'max_bytes': 262144, 'max_items': 64, 'max_model_input_units': None},
           'exclude_categories': ['raw_progress', 'tool_trace', 'verbose_command_output', 'prior_prompt'], 'fail_closed': True,
           'include_categories': ['direct_input'], 'include_direct_inputs': True, 'ordering': 'causal_kind_source',
           'selected_nodes': [], 'selected_roles': [], 'session': 'fresh', 'truncation': 'omit_oversized'}
artifact_schema = {'id': 'milkdrift.artifact-reference', 'version': 1}


def port(direction, schema, binding=None, required=False):
    return {'direction': direction, 'schema': schema, 'binding': binding, 'required': required}


def node(identity, kind, incoming=True, outgoing=True):
    return {'id': identity, 'kind': kind, 'control_inputs': ['in'] if incoming else [],
            'control_outputs': ['out'] if outgoing else [], 'data_inputs': {}, 'data_outputs': {}}


def task(identity, req, literal_name, literal, schema, outputs):
    result = node(identity, {'type': 'task', 'config': {'requirement': req, 'context_policy': context}})
    result['data_inputs'][literal_name] = port('Input', schema, {'type': 'literal', 'value': literal}, True)
    result['data_outputs'] = {name: port('Output', artifact_schema) for name in outputs}
    return result


def worker(identity, argv, capture=False):
    return task(identity, requirement, 'command', {'argv': argv, 'stdout_artifact': capture},
                {'id': 'milkdrift.managed.worker', 'version': 2}, ['worker_result', 'stdout'])


def effect(identity, operation, selected):
    req = dict(requirement, categories=[{'type': 'tool'}], exact_capability='milkdrift.resources', maximum_side_effect='idempotent_write', operation=operation)
    target = {'schema_version': 3, 'command': identity, 'installation': 'slotbook-test', 'expected_version': args.target_version}
    result = task(identity, req, 'target', target, {'id': 'milkdrift.managed.command', 'version': 3}, ['resource_result'])
    result['data_inputs'][selected] = port('Input', artifact_schema, required=True)
    return result


def edge(identity, source, target, kind='control', source_port='out', target_port='in'):
    return {'id': identity, 'kind': kind, 'source_node': source, 'source_port': source_port, 'target_node': target, 'target_port': target_port}


nodes = [node('repair-window', {'type': 'wait', 'duration_ms': 45000}, incoming=False),
         worker('repair.begin', ['/bin/sh', '-c', 'test -r /workspace/source/app.py']),
         worker('repair.end', ['/bin/cat', '/workspace/source/app.py'], True),
         effect('verify-candidate', 'resource.evaluate_candidate', 'candidate'),
         effect('publish-candidate', 'resource.publish_candidate', 'evaluation'),
         node('done', {'type': 'terminal', 'outcome': 'success'}, outgoing=False)]
edges = [edge('control-' + str(i), a['id'], b['id']) for i, (a, b) in enumerate(zip(nodes, nodes[1:]))]
edges += [edge('candidate-input', 'repair.end', 'verify-candidate', 'data', 'stdout', 'candidate'),
          edge('evaluation-input', 'verify-candidate', 'publish-candidate', 'data', 'resource_result', 'evaluation')]
mutations = [{'type': 'add_node', 'node': n} for n in nodes] + [{'type': 'add_edge', 'edge': e} for e in edges]
write('method-mutations.json', mutations)
write('adaptation-scope.json', {'node_prefix': 'repair.', 'maximum_nodes': 8, 'maximum_revisions': 4, 'requirements': [requirement]})
local('blueprint', 'create', root / 'method-mutations.json', '--workflow', 'slotbook', '--author', 'human:operator', '--output', root / 'base.json')
agreement = local('blueprint', 'govern', root / 'base.json', '--scope', root / 'adaptation-scope.json', '--name', 'slotbook-agreement',
                  '--effect-policy', policy_digest, '--author', 'human:operator', '--output', root / 'governed.json')['agreement']
limits = {'memory_bytes': 536870912, 'cpu_percent': 100, 'pids': 64, 'temporary_bytes': 16777216}
write('protected-recipe.json', {'schema_version': 1, 'kind': 'protected_service', 'name': 'slotbook-test', 'image': args.image,
      'executable': '/usr/local/bin/python3', 'limits': limits, 'startup_ms': 30000, 'shutdown_ms': 10000,
      'port': 19848, 'application': {'resource': 'pottery', 'capacity': 2}, 'token_file': str(root / 'service.token'),
      'token_digest': digest(root / 'service.token'), 'clock_file': str(root / 'clock'), 'agreement': agreement, 'policy': policy,
      'verifier_file': str(example / 'verifier.py'), 'verifier_source_digest': source_digest, 'verifier_runtime_digest': runtime_digest,
      'verification_timeout_ms': 180000, 'data_disposition': 'preserve'})
write('worker-recipe.json', {'schema_version': 2, 'name': 'slotbook-build', 'worker_image': args.image, 'worker_network': 'none',
      'worker_limits': limits, 'task_timeout_ms': 30000, 'output_bytes': 65536, 'minimum_free_bytes': 16777216,
      'data_disposition': 'preserve', 'model_service': {'type': 'disabled'}})
# A source-selected deterministic repair. Qualification retains the failed evidence separately.
code = (example / 'repaired.py').read_text()
repair = worker('repair.begin', ['/usr/local/bin/python3', '-I', '-c', "from pathlib import Path; Path('/workspace/source/app.py').write_text(" + repr(code) + ")"])
investigate = worker('repair.investigate', ['/bin/sh', '-c', 'test -s /workspace/source/app.py; printf "Retain initial authentication and restart failures; verify the new immutable candidate.\\n"'])
write('repair-mutations.json', [{'type': 'replace_node', 'node': repair}, {'type': 'add_node', 'node': investigate},
      {'type': 'replace_edge', 'edge': edge('control-1', 'repair.begin', 'repair.investigate')},
      {'type': 'add_edge', 'edge': edge('investigated', 'repair.investigate', 'repair.end')}])
print(json.dumps({'root': str(root), 'agreement': agreement, 'policy': policy_digest, 'candidate_lane': 'seeded fixtures, not model output'}))
