"""Bind cold-start evidence to each exact expected case before packaging."""
import math


def validate_compute_metadata(manifest, catalog, reference):
    """Check frozen Pi routing/inputs; this is not complete Host validation."""
    contributions = manifest['contributes']
    capabilities = contributions['capabilities']
    ids = {cap['id'] for cap in capabilities}
    assert len(ids) == len(capabilities), 'Duplicate capability IDs'
    tools = catalog['tools']
    assert len(tools) == len(ids) and {tool['capabilityId'] for tool in tools} == ids
    assert catalog['packageId'] == manifest['id'] and catalog['packageVersion'] == manifest['version']
    reference_pi = reference['contributes']['piTools'][0]
    pi_tools = contributions['piTools']
    assert len(pi_tools) == len(ids) and len({tool['name'] for tool in pi_tools}) == len(ids)
    for tool in pi_tools:
        assert set(tool) == set(reference_pi), 'Pi fields differ from frozen declaration; capabilityId is not capability'
        assert tool['capability'] in ids, 'Pi routes to an unregistered capability'
        assert tool['inputSchema'] == reference_pi['inputSchema'], 'Single-text compute input schema drifted'
    assert {tool['capability'] for tool in pi_tools} == ids
    assert all(cap['inputSchema'] == reference_pi['inputSchema'] for cap in capabilities)
    assert manifest['permissions'] == reference['permissions'], 'Compute permissions differ from frozen optional Pi grant'


def validate_budget(budget, parity, module_sha256, module_bytes):
    """Reject internally inconsistent budget evidence before directory writes."""
    assert budget['wasmSha256'] == parity['wasmSha256'] == module_sha256
    assert budget['wasmBytes'] == parity['wasmBytes'] == module_bytes
    assert budget['fixtureKind'] == 'plugin-runtime'
    assert budget['budget'] == dict(fuel=10000000, deadlineSeconds=5, memoryBytes=67108864, guestStackBytes=2097152)
    assert budget['engineProvenance']['nativeEngineVersion'] == '36.0.2'
    cases = {case['id']: case for case in parity['cases']}
    assert len(cases) == len(parity['cases'])
    rows = budget['cases']
    assert len(rows) == len(cases) and {row['id'] for row in rows} == set(cases), 'Budget cases differ from parity'
    assert budget['failures'] == []
    for row in rows:
        reference = cases[row['id']]
        assert row['passBudget'] is True and row['trap'] is None
        for key in ['requestSha256', 'expectedSha256', 'requestBytes', 'resultBytes', 'expectedError']:
            assert row[key] == reference[key], 'Budget case bytes differ from parity: ' + key
        elapsed = row['elapsedMs']
        assert type(elapsed) in (int, float) and math.isfinite(elapsed) and 0 <= elapsed < 5000
        used, remaining, memory = row['fuelUsed'], row['fuelRemaining'], row['memoryBytes']
        assert type(used) is int and type(remaining) is int and 0 <= used <= 10000000 and 0 <= remaining <= 10000000
        assert used + remaining == 10000000, 'Fuel counters disagree'
        assert type(memory) is int and 0 < memory <= 67108864
        assert type(row['resultBytes']) is int and 0 < row['resultBytes'] <= 49152


def validate_cold(cold, parity, module_sha256, module_bytes, engine, require_pass=True):
    assert cold['wasmSha256'] == module_sha256 and cold['wasmBytes'] == module_bytes
    assert cold['remaining'] == 0
    assert cold['engine']['nativeEngineVersion'] == engine['nativeEngineVersion'] == '36.0.2'
    assert cold['engine']['dllSha256'] == engine['dllSha256']
    cases = {case['id']: case['expectedSha256'] for case in parity['cases']}
    assert len(cases) == len(parity['cases']), 'Duplicate reference cases'
    rows = cold['rows']
    assert len(rows) == len(cases) and {row['id'] for row in rows} == set(cases), 'Cold cases differ from exact parity cases'
    failures = []
    for row in rows:
        assert row['closed'] is True and type(row['passed']) is bool
        elapsed = row['elapsedMs']
        assert type(elapsed) in (int, float) and math.isfinite(elapsed) and elapsed >= 0
        passed = type(row.get('exitCode')) is int and row['exitCode'] == 0 and row.get('resultSha256') == cases[row['id']] and elapsed < 5000
        assert row['passed'] == passed, 'Cold pass flag disagrees with measured bytes/deadline/exit'
        if not passed:
            failures.append(row['id'])
    assert cold['failures'] == failures, 'Cold failures omitted or changed'
    if require_pass:
        assert not failures, 'Cold deadline or result failures remain'
    return failures
