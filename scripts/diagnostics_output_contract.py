"""Provider verification only; jsonschema is not a desktop or plugin dependency."""
import argparse
import hashlib
import json
from pathlib import Path

from jsonschema import Draft202012Validator
from referencing import Registry, Resource

ROOT = Path(__file__).resolve().parents[1]
RESULT_BYTES = 49152


def load_report_contracts(root=ROOT):
    resources = []
    operation_schemas = {}
    for path in (root / 'contracts/diagnostics/v2').rglob('*.schema.json'):
        schema = json.loads(path.read_text(encoding='utf-8'))
        Draft202012Validator.check_schema(schema)
        resources.append((schema['$id'], Resource.from_contents(schema)))
        if path.parent.name == 'operations' and path.name.endswith('.report.schema.json'):
            operation_schemas[path.name.removesuffix('.report.schema.json')] = schema
    registry = Registry().with_resources(resources)
    return {action: Draft202012Validator(schema, registry=registry)
            for action, schema in operation_schemas.items()}


def load_contracts(manifest_path=None, schema_path=None):
    manifest = json.loads((manifest_path or ROOT / 'plugins/diagnostics/plugin.json').read_text(encoding='utf-8'))
    schema = json.loads((schema_path or ROOT / 'contracts/diagnostics/v2/result.schema.json').read_text(encoding='utf-8'))
    if manifest.get('schemaVersion') != 2 or manifest.get('requiredProfiles', {}).get('output.schema.v1') != '=1.0.0-rc.1':
        raise ValueError('Output profile is not explicitly negotiated')
    Draft202012Validator.check_schema(schema)
    validator = Draft202012Validator(schema)
    contracts = {}
    for capability in manifest['contributes']['capabilities']:
        if capability.get('outputSchema') != schema or capability.get('maxResultBytes') != RESULT_BYTES:
            raise ValueError('Every diagnostic capability must declare the exact output schema and byte budget')
        if capability['id'] in contracts:
            raise ValueError('Duplicate diagnostic capability')
        contracts[capability['id']] = validator
    if len(contracts) != 11:
        raise ValueError('Expected all eleven diagnostic capabilities')
    return contracts


def validate_result(validator, encoded):
    if len(encoded) > RESULT_BYTES:
        raise ValueError('Serialized result exceeds 48 KiB')
    value = json.loads(encoded)
    validator.validate(value)
    return value


def validate_runtime_evidence(functional, fuel, cold):
    """Require the same complete runtime matrix and output bytes in all runs."""
    def indexed(rows):
        result = {row['id']: row for row in rows}
        if not result or len(result) != len(rows):
            raise ValueError('Empty or duplicate runtime case IDs')
        return result

    expected = indexed(functional['cases'])
    budget = indexed(fuel['cases'])
    startup = indexed(cold['rows'])
    if expected.keys() != budget.keys() or expected.keys() != startup.keys():
        raise ValueError('Runtime matrices must cover the same cases')
    if cold.get('diagnosticOnly') is not False or cold.get('remaining') != 0:
        raise ValueError('Incomplete or diagnostic-only cold run')
    if fuel.get('failures') or cold.get('failures'):
        raise ValueError('Runtime evidence contains failures')
    for case_id, row in expected.items():
        measured, started = budget[case_id], startup[case_id]
        if not measured.get('passBudget') or not started.get('passed') or not started.get('closed'):
            raise ValueError('Runtime case did not pass and close')
        if (row['requestSha256'] != measured['requestSha256']
                or row['resultSha256'] != measured['expectedSha256']
                or row['resultSha256'] != started['resultSha256']):
            raise ValueError('Runtime request/result digest mismatch')


def verify_packaged_fixtures(package):
    """Check saved byte evidence; this does not execute or accept a Host runtime."""
    contracts = load_contracts(package / 'plugin.json', package / 'contracts/diagnostics/v2/result.schema.json')
    reports = load_report_contracts(package)
    counts = {}
    validator = next(iter(contracts.values()))
    for kind in ('budget', 'serializer'):
        folder = package / 'fixtures' / kind
        proof = json.loads((folder / 'fuel-proof.json').read_text(encoding='utf-8'))
        for case in proof['cases']:
            for key in ('requestFile', 'expectedFile'):
                if Path(case[key]).name != case[key]:
                    raise ValueError('Fixture names must be local basenames')
            request = (folder / case['requestFile']).read_bytes()
            expected = (folder / case['expectedFile']).read_bytes()
            if hashlib.sha256(request).hexdigest() != case['requestSha256'] or hashlib.sha256(expected).hexdigest() != case['expectedSha256']:
                raise ValueError('Fixture digest mismatch')
            value = validate_result(validator, expected)
            if kind == 'budget' and value['ok']:
                action = json.loads(request)['capabilityId'].removeprefix('devtools.diagnostics.')
                reports[action].validate(json.loads(value['data']['text']))
        counts[kind] = len(proof['cases'])
    return dict(capabilityCount=len(contracts), fixtureCounts=counts, runtimeExecuted=False, hostAccepted=False)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--package', type=Path, required=True)
    arguments = parser.parse_args()
    print(json.dumps(verify_packaged_fixtures(arguments.package), indent=2))
