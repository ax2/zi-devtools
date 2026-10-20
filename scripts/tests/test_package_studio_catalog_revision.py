import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from package_studio_catalog_revision import explicit_scope_receipt, package_receipt_path, receipt_for_scope, snapshot_changes, validate_package_operations, capability_provider_conflicts


class PackageReceiptMappingTests(unittest.TestCase):
    def test_declared_navigation_matches_mapping_and_exact_profile(self):
        manifest = {"requiredProfiles":{"view.operation.directory.v1":"=1.0.0-preview.1"},
                    "contributes":{"views":[{"id":"main"}],"capabilities":[],
                                   "viewOperations":[{"id":"health","viewId":"main"}]}}
        scopes = [{"operationId":"health","viewId":"main","executionSurface":"view"}]
        self.assertEqual(validate_package_operations(manifest,set(),scopes),["health"])
        with self.assertRaisesRegex(RuntimeError,"differs"):
            validate_package_operations(manifest,set(),[])
        manifest['requiredProfiles']={}
        with self.assertRaisesRegex(RuntimeError,"exact profile"):
            validate_package_operations(manifest,set(),scopes)

    def test_alternative_resource_provider_is_explicitly_reported(self):
        packages = [{"packageId": "text", "packageVersion": "1", "path": "text", "capabilities": ["url.encode", "url.decode"]},
                    {"packageId": "resource", "packageVersion": "2", "path": "resource", "capabilities": ["url.encode"]}]
        conflicts = capability_provider_conflicts(packages)
        self.assertEqual([row["capabilityId"] for row in conflicts], ["url.encode"])
        self.assertEqual([row["packageId"] for row in conflicts[0]["providers"]], ["resource", "text"])
        self.assertIn("concurrent_activation_not_verified", conflicts[0]["resolution"])
        self.assertEqual(capability_provider_conflicts(packages[:1]), [])

    def test_view_operations_do_not_inflate_agent_capabilities(self):
        manifest = {"contributes": {"capabilities": [], "views": [{"id": "actuator.main"}]}}
        scopes = [{"operationId": "actuator.health", "executionSurface": "view", "viewId": "actuator.main"}]
        self.assertEqual(validate_package_operations(manifest, set(), scopes), ["actuator.health"])
        with self.assertRaisesRegex(RuntimeError, "Mapped capabilities"):
            validate_package_operations(manifest, {"actuator.health"}, scopes)
        with self.assertRaisesRegex(RuntimeError, "declared View"):
            validate_package_operations(manifest, set(), [{**scopes[0], "viewId": "missing"}])
        with self.assertRaisesRegex(RuntimeError, "Duplicate View"):
            validate_package_operations(manifest, set(), scopes * 2)

    def test_every_declared_capability_must_be_mapped(self):
        manifest = {"contributes": {"capabilities": [{"id": "json.format"}]}}
        self.assertEqual(validate_package_operations(manifest, {"json.format"}, []), [])
        with self.assertRaisesRegex(RuntimeError, "Mapped capabilities"):
            validate_package_operations(manifest, set(), [])

    def test_comparison_preserves_changed_counts_and_rejects_missing_baseline(self):
        changes = {"versionStatusChanged": [{"id": "recorder"}], "scopeChanged": [], "discoveryChanged": []}
        self.assertEqual(snapshot_changes({"snapshotComparison": changes}), changes)
        for invalid in ({}, {"snapshotComparison": {}}, {"snapshotComparison": {**changes, "scopeChanged": None}}):
            with self.assertRaisesRegex(RuntimeError, "explicit frozen-baseline"):
                snapshot_changes(invalid)

    def test_frozen_operation_rejects_inherited_receipt_until_source_is_explicit(self):
        scope = {"operationId": "devtools.text.json.format", "packageId": "text",
                 "packageVersion": "0.4.1", "candidateDirectory": "candidate"}
        package = {**scope, "verificationReceipt": "receipt.json"}
        for row in (package, {"candidatePackages": [package]}):
            with self.assertRaisesRegex(RuntimeError, "explicitly declare"):
                explicit_scope_receipt(row, scope)
            self.assertEqual(explicit_scope_receipt(row, {**scope, "verificationReceipt": "receipt.json"}),
                             "receipt.json")

    def test_resolves_operation_receipt_from_candidate_package_mapping(self):
        row = {
            "candidatePackages": [
                {
                    "packageId": "com.zicode.devtools.text",
                    "packageVersion": "0.4.1",
                    "candidateDirectory": "release/text-plugin-0.4.1-candidate-02",
                    "verificationReceipt": "release/plugin-candidates/text-plugin-revision-01.json",
                }
            ]
        }
        scope = {
            "packageId": "com.zicode.devtools.text",
            "packageVersion": "0.4.1",
            "candidateDirectory": "release/text-plugin-0.4.1-candidate-02",
        }
        self.assertEqual(receipt_for_scope(row, scope), "release/plugin-candidates/text-plugin-revision-01.json")

    def test_explicit_operation_receipt_takes_precedence(self):
        row = {"verificationReceipt": "fallback.json"}
        scope = {"verificationReceipt": "operation.json", "candidateDirectory": "candidate"}
        self.assertEqual(receipt_for_scope(row, scope), "operation.json")

    def test_uses_tool_level_receipt_for_single_package_tools(self):
        row = {
            "packageId": "com.zicode.devtools.text",
            "packageVersion": "0.4.1",
            "candidateDirectory": "release/text-plugin-0.4.1-candidate-02",
            "verificationReceipt": "release/plugin-candidates/text-revision-01.json",
        }
        scope = {
            "packageId": "com.zicode.devtools.text",
            "packageVersion": "0.4.1",
            "candidateDirectory": "release/text-plugin-0.4.1-candidate-02",
        }
        self.assertEqual(receipt_for_scope(row, scope), row["verificationReceipt"])

    def test_rejects_implicit_default_when_receipt_mapping_is_missing(self):
        rows = {
            "text": {
                "operationScopes": [
                    {
                        "candidateDirectory": "release/text-plugin-0.4.1-candidate-02",
                        "packageId": "com.zicode.devtools.text",
                        "packageVersion": "0.4.1",
                    }
                ]
            }
        }
        with self.assertRaisesRegex(RuntimeError, "exactly one explicit verification receipt"):
            package_receipt_path({"text"}, rows, "release/text-plugin-0.4.1-candidate-02")

    def test_rejects_conflicting_receipts_for_one_package(self):
        rows = {
            "text-a": {
                "operationScopes": [
                    {"candidateDirectory": "candidate", "verificationReceipt": "a.json"}
                ]
            },
            "text-b": {
                "operationScopes": [
                    {"candidateDirectory": "candidate", "verificationReceipt": "b.json"}
                ]
            },
        }
        with self.assertRaisesRegex(RuntimeError, "exactly one explicit verification receipt"):
            package_receipt_path({"text-a", "text-b"}, rows, "candidate")


if __name__ == "__main__":
    unittest.main()
