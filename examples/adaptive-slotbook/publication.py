"""Publication helpers for the finite binary qualification; no alternate execution path."""
import copy
import json
import secrets
import tomllib


def toml_document(document):
    """Write the concrete bootstrap configuration's strings, numbers, lists and tables."""
    lines = []

    def table(value, path=(), array=False):
        if path:
            name = '.'.join(json.dumps(part) for part in path)
            lines.append(('[' if not array else '[[') + name + (']' if not array else ']]'))
        for key, item in value.items():
            if not isinstance(item, dict) and not (isinstance(item, list) and item and isinstance(item[0], dict)):
                lines.append(json.dumps(key) + ' = ' + json.dumps(item))
        lines.append('')
        for key, item in value.items():
            if isinstance(item, dict):
                table(item, (*path, key))
            elif isinstance(item, list) and item and isinstance(item[0], dict):
                for entry in item:
                    table(entry, (*path, key), True)
    table(document)
    result = '\n'.join(lines)
    assert tomllib.loads(result) == document
    return result


def configure(text, root):
    config = tomllib.loads(text)
    config['runtime'].update(controller_activation='enabled', effect_threads=1, effect_queue=1,
                             global_concurrency=1, per_run_concurrency=1, per_capability_concurrency=1,
                             per_branch_concurrency=1, maximum_effect_claim=1,
                             publication_services={'method:slotbook': 'grant:operator'})
    config['serving'].update(worker_threads=1, observation_hot_retention_ms=100)
    config['serving']['clients']['execution_limits']['nested_invocations'] = {'process': 8, 'model': 0}
    operator = config['actors'][0]
    operator['authority']['resources']['capability']['identities']['values'].append('method:slotbook')
    operator['authority']['resources']['capability']['operations']['values'].extend(
        ['method.publish', 'method.inspect', 'method.retire', 'method.invoke'])
    consumer = copy.deepcopy(operator)
    consumer.update(actor='human:consumer', credential_ref='credential:consumer',
                    grant_id='grant:consumer', preset='invoker')
    resources = consumer['authority']['resources']
    resources['capability']['identities']['values'] = ['method:slotbook']
    resources['capability']['operations']['values'] = ['method.invoke']
    resources['artifacts'] = {'type': 'deny_all'}
    resources['filesystem'] = []
    resources['workspace'] = {'allow_any_in_run': False, 'scopes': []}
    resources['secrets'] = []
    config['actors'].append(consumer)
    credential = root / 'consumer.token'
    credential.write_text(secrets.token_hex(32))
    credential.chmod(0o600)
    config['secret_sources']['credential:consumer'] = {'type': 'file', 'path': str(credential)}
    return toml_document(config)


def method(descriptor, revision, authority):
    descriptor = copy.deepcopy(descriptor)
    descriptor.update(identity='method:slotbook', descriptor_revision=1, category={'type': 'tool'})
    descriptor['operations'] = {'method.invoke': descriptor['operations']['workspace.execute']}
    return {
        'schema_version': 1, 'descriptor': descriptor,
        'documentation': 'Verify and deploy the governed Slotbook candidate to the fixed test installation. '
                         'A successful invocation confirms the internal agreement; application service state remains separately inspectable.',
        'revision': revision['id'], 'agreement': revision['semantic']['agreement']['digest'],
        'service': {'actor': authority['actor'], 'grant': authority['grant_id'],
                    'grant_revision': authority['grant_revision'], 'grant_digest': authority['grant_digest'],
                    'revocation_generation': 0},
        'inputs': {}, 'outputs': {},
        'workspace_budget': {'max_value_versions': 1024, 'max_inline_bytes_per_value': 65536,
                             'max_total_inline_bytes': 1048576, 'max_artifacts': 256,
                             'max_bytes_per_artifact': 16777216, 'max_total_artifact_bytes': 67108864},
        'allowance': {'cost_micros': 0, 'currency': None, 'input_units': 0, 'output_units': 0,
                      'artifact_bytes': 33554432, 'process_admissions': 8, 'model_admissions': 0},
        'maximum_outstanding': 1, 'maximum_depth': 4, 'maximum_duration_ms': 240000,
    }


def outer_method(root, cli, capability, peer=None):
    """Author a caller workflow through the supported CLI with an explicit provider selection."""
    import subprocess
    internal = json.loads((root / 'governed.json').read_text())['revision']
    task = copy.deepcopy(internal['semantic']['nodes']['repair.begin'])
    task.update(id='invoke-method', control_inputs=[], data_inputs={}, data_outputs={})
    requirement = task['kind']['config']['requirement']
    requirement.update(categories=[{'type': 'tool'}], exact_capability=capability, operation='method.invoke')
    if peer:
        requirement['placement'] = {'localities': ['peer'], 'peers': [peer]}
    done = copy.deepcopy(internal['semantic']['nodes']['done'])
    mutations = [{'type': 'add_node', 'node': task}, {'type': 'add_node', 'node': done},
                 {'type': 'add_edge', 'edge': {'id': 'finish', 'kind': 'control',
                  'source_node': 'invoke-method', 'source_port': 'out', 'target_node': 'done', 'target_port': 'in'}}]
    path = root / 'outer-mutations.json'
    path.write_text(json.dumps(mutations))
    output = root / 'outer.json'
    subprocess.run([cli, '--json', 'blueprint', 'create', str(path), '--workflow', 'slotbook-caller',
                    '--author', 'agent:repair', '--output', str(output)], capture_output=True, text=True, check=True)
    return json.loads(output.read_text())['revision']


def peer_configuration(text, root, provider_port):
    """Two explicit loopback hosts, separate stores, fixed peer credentials and one worker each."""
    config = tomllib.loads(text)
    origin_port = provider_port + 1
    credential = root / 'peer.token'
    credential.write_text(secrets.token_hex(32))
    credential.chmod(0o600)
    config['secret_sources']['credential:peer'] = {'type': 'file', 'path': str(credential)}

    def relationship(peer, port):
        return {'peer_id': peer, 'endpoint': f'http://127.0.0.1:{port}/',
                'credential_ref': 'credential:peer', 'insecure_loopback_development': True,
                'minimum_minor': 5, 'maximum_minor': 5,
                'actions': ['read_catalog', 'invoke', 'cancel', 'artifact_upload', 'artifact_download'],
                'capability_allow': ['method:slotbook'], 'capability_deny': [],
                'operation_allow': ['method.invoke'], 'maximum_side_effect': 'non_idempotent_write',
                'maximum_concurrent': 1, 'maximum_requests_per_minute': 6000,
                'maximum_artifact_bytes': 67108864, 'artifact_sensitivities': ['internal', 'restricted'],
                'maximum_duration_ms': 240000, 'maximum_cost_micros': 0,
                'maximum_input_units': 0, 'maximum_output_units': 0,
                'nested_invocations': {'process': 8, 'model': 0},
                'maximum_observations': 256, 'catalog_ttl_ms': 300000,
                'trust_zone': 'slotbook-qualification', 'delegation_ref': 'delegation:slotbook',
                'expires_at_unix_ms': 4102444800000, 'enabled': True}

    config['peers'] = {'mode': 'enabled', 'relationships': [relationship('host:slotbook-caller', origin_port)]}
    origin = copy.deepcopy(config)
    origin.update(host_id='host:slotbook-caller', data_root=str(root / 'origin-data'),
                  bind=f'127.0.0.1:{origin_port}')
    origin['runtime']['publication_services'] = {}
    origin['adapters'] = {'process_profiles': [], 'model_profiles': []}
    origin['peers']['relationships'] = [relationship('host:slotbook-test', provider_port)]
    origin['actors'] = [origin['actors'][0]]
    resources = origin['actors'][0]['authority']['resources']
    resources['capability']['identities'] = {'type': 'any'}
    resources['capability']['operations'] = {'type': 'only', 'values': ['method.invoke']}
    resources['filesystem'] = []
    resources['network'] = {'profiles': ['peer:host:slotbook-test'], 'destinations': [f'127.0.0.1:{provider_port}']}
    resources['peers'] = {'allow_any': False, 'identities': ['host:slotbook-test']}
    origin_path = root / 'origin.toml'
    origin_path.write_text(toml_document(origin))
    return toml_document(config), origin_path, origin_port
