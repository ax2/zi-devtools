#!/usr/bin/env python3
"""Strictly verify the Spring Actuator provider schemas and cross-field vectors."""
from __future__ import annotations

import json
from datetime import datetime
from pathlib import Path
import subprocess
from typing import Any

from importlib.metadata import version as package_version
from jsonschema import Draft202012Validator, FormatChecker
from referencing import Registry, Resource

ROOT = Path(__file__).resolve().parents[1]
SCHEMA_REGISTRY = Registry()


def unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON object key: {key}")
        result[key] = value
    return result


def read_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=unique_object)


def check_refs(path: Path, schema: Any) -> int:
    refs = 0
    if isinstance(schema, dict):
        ref = schema.get("$ref")
        if isinstance(ref, str) and not ref.startswith("#"):
            refs += 1
            target_name, _, pointer = ref.partition("#")
            target = (path.parent / target_name).resolve()
            if not target.is_relative_to(ROOT.resolve()) or not target.is_file():
                raise AssertionError(f"unresolved/out-of-tree ref {ref} in {path.relative_to(ROOT)}")
            doc = read_json(target)
            if pointer:
                for segment in pointer.lstrip("/").split("/"):
                    doc = doc[segment.replace("~1", "/").replace("~0", "~")]
        refs += sum(check_refs(path, value) for value in schema.values())
    elif isinstance(schema, list):
        refs += sum(check_refs(path, value) for value in schema)
    return refs


def validate_json(value: Any, schema: Any, schema_path: Path, location: str = "$", depth: int = 0) -> list[str]:
    validator = Draft202012Validator(schema, registry=SCHEMA_REGISTRY, format_checker=FormatChecker())
    return [f"{location}: {error.message}" for error in validator.iter_errors(value)]


def validate_facts(payload: dict[str, Any]) -> bool:
    if set(payload) != {"schemaVersion", "operation", "checks"}:
        return False
    if payload["schemaVersion"] != 1 or payload["operation"] != "resource.create.prepare":
        return False
    checks = payload["checks"]
    if not isinstance(checks, list) or not 1 <= len(checks) <= 3:
        return False
    seen: set[str] = set()
    for row in checks:
        if not isinstance(row, dict):
            return False
        if not set(row) <= {"endpoint", "httpStatus", "responseBytes", "topLevelFieldCount", "healthStatus"}:
            return False
        endpoint, status = row.get("endpoint"), row.get("httpStatus")
        if endpoint not in {"health", "info", "metrics"} or endpoint in seen:
            return False
        seen.add(endpoint)
        if not isinstance(status, int) or isinstance(status, bool):
            return False
        if endpoint == "health":
            if not (200 <= status < 300 or status == 503):
                return False
            if "healthStatus" in row and row["healthStatus"] not in {"UP", "DOWN", "OUT_OF_SERVICE", "UNKNOWN"}:
                return False
        elif not 200 <= status < 300 or "healthStatus" in row:
            return False
        if not isinstance(row.get("responseBytes"), int) or isinstance(row["responseBytes"], bool) or not 0 <= row["responseBytes"] <= 1_048_576:
            return False
        if not isinstance(row.get("topLevelFieldCount"), int) or isinstance(row["topLevelFieldCount"], bool) or not 0 <= row["topLevelFieldCount"] <= 20_000:
            return False
    return True


def host_report_valid(report: dict[str, Any], expected_health: str | None) -> bool:
    checks = report.get("checks")
    summary = report.get("summary")
    provenance = report.get("provenance")
    if not isinstance(checks, list) or not isinstance(summary, dict) or not isinstance(provenance, dict):
        return False
    endpoints = [row.get("endpoint") for row in checks if isinstance(row, dict)]
    if len(endpoints) != len(checks) or len(endpoints) != len(set(endpoints)):
        return False
    if summary.get("endpointCount") != len(checks):
        return False
    if summary.get("healthStatus") != expected_health:
        return False
    for row in checks:
        endpoint, status = row.get("endpoint"), row.get("httpStatus")
        if endpoint == "health":
            if not (200 <= status < 300 or status == 503):
                return False
        elif not (endpoint in {"info", "metrics"} and 200 <= status < 300):
            return False
    return (
        report.get("sourceToolId") == "spring-actuator"
        and provenance.get("pluginId") == "com.zicode.devtools.spring-actuator"
        and provenance.get("pluginVersion") == "0.1.1"
        and provenance.get("toolVersion") == "1.1.1"
        and isinstance(provenance.get("applicationVersion"), str)
        and len(provenance.get("runtimeFingerprint", "")) == 64
        and all(c in "0123456789abcdef" for c in provenance.get("runtimeFingerprint", ""))
        and isinstance(report.get("generatedAt"), str)
    )


def validate_cursor(response: dict[str, Any]) -> bool:
    required = {"schemaVersion", "taskRef", "events", "requestedAfterSequence", "nextSequence", "hasMore", "earliestRetainedSequence", "latestSequence", "snapshotRequired", "terminal"}
    if not required <= set(response) or response.get("schemaVersion") != 1:
        return False
    if not isinstance(response.get("taskRef"), str) or not response["taskRef"] or len(response["taskRef"]) > 256:
        return False
    events = response["events"]
    after = response["requestedAfterSequence"]
    next_sequence = response["nextSequence"]
    earliest = response["earliestRetainedSequence"]
    latest = response["latestSequence"]
    error = response.get("cursorError")
    if not isinstance(events, list) or not all(isinstance(e, dict) for e in events):
        return False
    if error:
        if events or response["hasMore"] or next_sequence != after:
            return False
        if error == "cursor_expired":
            if not response["snapshotRequired"] or after >= earliest - 1:
                return False
        elif error == "cursor_invalid":
            if not response["snapshotRequired"] or after <= latest:
                return False
        elif error == "snapshot_required":
            if not response["snapshotRequired"]:
                return False
        else:
            return False
    else:
        if response["snapshotRequired"] or after > latest or after < earliest - 1:
            return False
        sequences = [event.get("sequence") for event in events]
        if any(not isinstance(s, int) or s <= after or s > latest for s in sequences):
            return False
        if sequences != sorted(set(sequences)):
            return False
        expected_next = sequences[-1] if sequences else after
        if next_sequence != expected_next or response["hasMore"] != (expected_next < latest):
            return False
        for event in events:
            terminal = event.get("terminal")
            if event.get("eventType") == "terminal":
                if not isinstance(terminal, dict) or terminal.get("taskRef") != response["taskRef"] or terminal.get("sequence") != event.get("sequence") or terminal.get("executionFinished") is not True:
                    return False
    terminal = response.get("terminal")
    if terminal is not None and (terminal.get("taskRef") != response["taskRef"] or terminal.get("sequence", latest + 1) > latest or terminal.get("executionFinished") is not True):
        return False
    return True


def validate_event_chain(events: list[dict[str, Any]]) -> bool:
    if not events:
        return False
    task_ref = events[0].get("taskRef")
    sequences = [event.get("sequence") for event in events]
    if not isinstance(task_ref, str) or not task_ref or sequences != list(range(1, len(events) + 1)):
        return False
    if any(event.get("taskRef") != task_ref for event in events):
        return False
    terminal_indexes = [index for index, event in enumerate(events) if event.get("eventType") == "terminal"]
    if len(terminal_indexes) > 1 or terminal_indexes and terminal_indexes[0] != len(events) - 1:
        return False
    if terminal_indexes:
        event = events[-1]
        terminal = event.get("terminal")
        if not isinstance(terminal, dict):
            return False
        if terminal.get("taskRef") != task_ref or terminal.get("sequence") != event.get("sequence"):
            return False
        if terminal.get("state") != event.get("state") or terminal.get("executionFinished") is not True:
            return False
    return True


def validate_confirmation(vector: dict[str, Any], schema: Any, schema_path: Path) -> tuple[bool, int]:
    request = vector["request"]
    if validate_json(request, schema, schema_path):
        return False, 0
    binding = vector["binding"]
    try:
        now = datetime.fromisoformat(vector["now"].replace("Z", "+00:00"))
        expires = datetime.fromisoformat(binding["expiresAt"].replace("Z", "+00:00"))
    except (KeyError, TypeError, ValueError):
        return False, 0
    accepted = (
        not vector["alreadyUsed"]
        and now.tzinfo is not None
        and expires.tzinfo is not None
        and now < expires
        and request.get("confirmationId") == binding.get("confirmationId")
        and request.get("taskRef") == binding.get("taskRef")
        and request.get("operationStepId") == binding.get("operationStepId")
        and vector.get("currentTarget") == binding.get("target")
        and vector.get("currentPermissionGeneration") == binding.get("permissionGeneration")
    )
    requests = 1 if accepted and request.get("decision") == "approve" else 0
    return accepted, requests


def main() -> None:
    json_files = sorted(ROOT.rglob("*.json"))
    parsed: dict[Path, Any] = {}
    for path in json_files:
        parsed[path] = read_json(path)

    schema_files = sorted(ROOT.rglob("*.schema.json"))
    global SCHEMA_REGISTRY
    resources = []
    schema_ids = set()
    for path in schema_files:
        schema = parsed[path]
        Draft202012Validator.check_schema(schema)
        schema_id = schema.get("$id")
        assert isinstance(schema_id, str) and schema_id not in schema_ids, f"missing/duplicate $id in {path.relative_to(ROOT)}"
        schema_ids.add(schema_id)
        resources.append((schema_id, Resource.from_contents(schema)))
    SCHEMA_REGISTRY = Registry().with_resources(resources)
    ref_count = sum(check_refs(path, parsed[path]) for path in schema_files)
    facts_schema_path = ROOT / "schemas/result-resource-proposal/prepare.schema.json"
    facts_schema = read_json(facts_schema_path)
    fact_item_schema = read_json(ROOT / "schemas/result-resource-proposal/report-facts.schema.json")
    report_schema = read_json(ROOT / "schemas/result-resource-proposal/report.schema.json")
    assert isinstance(fact_item_schema["properties"]["checks"]["items"], dict)
    assert "allOf" in fact_item_schema["properties"]["checks"]["items"]
    assert "allOf" in report_schema["allOf"][0]["properties"]["checks"]["items"]
    assert {"generatedAt", "sourceToolId", "summary", "checks", "provenance"} <= set(report_schema["properties"])

    resource_vectors = read_json(ROOT / "schemas/result-resource-proposal/conformance-vectors.json")["vectors"]
    for vector in resource_vectors:
        facts_schema_valid = not validate_json(vector["facts"], facts_schema, facts_schema_path)
        assert facts_schema_valid is vector["factsSchemaValid"], f"facts schema failed: {vector['id']}"
        accepted = validate_facts(vector["facts"])
        if accepted:
            captured = vector.get("captured", [])
            facts = vector["facts"]["checks"]
            accepted = len(captured) == len(facts) and all(a == b for a, b in zip(facts, captured, strict=True))
        if accepted and "hostReport" in vector:
            expected_health = next((row.get("healthStatus") for row in vector["captured"] if row["endpoint"] == "health"), None)
            accepted = host_report_valid(vector["hostReport"], expected_health)
        if accepted and "hostReport" not in vector:
            checks = vector["facts"]["checks"]
            accepted = len(checks) == len({row["endpoint"] for row in checks})
        assert accepted is vector["expectedAccepted"], f"resource vector failed: {vector['id']}"
        if "hostReport" in vector:
            report_schema_valid = not validate_json(vector["hostReport"], report_schema, ROOT / "schemas/result-resource-proposal/report.schema.json")
            assert report_schema_valid, f"host report schema failed: {vector['id']}"

    schema_vectors = read_json(ROOT / "schemas/result-resource-proposal/schema-vectors.json")["vectors"]
    schema_paths = {path.relative_to(ROOT).as_posix(): path for path in schema_files}
    schema_paths.update({
        path.relative_to(ROOT).as_posix(): path
        for path in (ROOT / "schemas/result-resource-proposal").glob("*.schema.json")
    })
    for vector in schema_vectors:
        schema_path = schema_paths[vector["schema"]]
        errors = validate_json(vector["instance"], read_json(schema_path), schema_path)
        assert (not errors) is vector["valid"], f"schema vector failed: {vector['id']}: {errors}"

    cursor_vectors = read_json(ROOT / "schemas/task-api-proposal/cursor-vectors.json")["vectors"]
    events_schema_path = ROOT / "schemas/task-api-proposal/events.response.schema.json"
    events_schema = read_json(events_schema_path)
    for vector in cursor_vectors:
        schema_errors = validate_json(vector["response"], events_schema, events_schema_path)
        assert (not schema_errors) is vector["schemaAccepted"], f"cursor schema failed: {vector['id']}: {schema_errors}"
        assert validate_cursor(vector["response"]) is vector["accepted"], f"cursor vector failed: {vector['id']}"

    event_schema_path = ROOT / "schemas/task-api-proposal/event.schema.json"
    event_schema = read_json(event_schema_path)
    event_vectors = read_json(ROOT / "schemas/task-api-proposal/event-vectors.json")["vectors"]
    for vector in event_vectors:
        schema_valid = all(not validate_json(event, event_schema, event_schema_path) for event in vector["events"])
        semantically_valid = validate_event_chain(vector["events"])
        assert (schema_valid and semantically_valid) is vector["accepted"], f"event vector failed: {vector['id']}"

    confirmation_schema_path = ROOT / "schemas/task-api-proposal/confirm.request.schema.json"
    confirmation_schema = read_json(confirmation_schema_path)
    confirmation_vectors = read_json(ROOT / "schemas/task-api-proposal/confirmation-vectors.json")["vectors"]
    for vector in confirmation_vectors:
        accepted, requests = validate_confirmation(vector, confirmation_schema, confirmation_schema_path)
        assert accepted is vector["accepted"], f"confirmation vector failed: {vector['id']}"
        assert requests == vector["networkRequests"], f"confirmation side-effect count failed: {vector['id']}"

    duplicate_fixture = ROOT / "fixtures/duplicate-json-key.input.txt"
    try:
        json.loads(duplicate_fixture.read_text(encoding="utf-8"), object_pairs_hook=unique_object)
    except ValueError as error:
        assert "duplicate JSON object key" in str(error)
    else:
        raise AssertionError("duplicate-key fixture was not rejected")

    print(f"PASS strict JSON {len(json_files)} files; Draft 2020-12 schemas {len(schema_files)}; local refs {ref_count}; schema vectors {len(schema_vectors)}; cursor vectors {len(cursor_vectors)} schema + semantic; event vectors {len(event_vectors)} schema + terminal semantics; confirmation vectors {len(confirmation_vectors)} binding/expiry/replay + side-effect semantics; resource vectors {len(resource_vectors)}; jsonschema {package_version('jsonschema')}; duplicate-key input rejected")


if __name__ == "__main__":
    main()
