"""Bind cold-start evidence to each exact expected case before packaging."""
import math


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
