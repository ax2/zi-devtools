import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from package_studio_catalog_revision import package_receipt_path, receipt_for_scope


class PackageReceiptMappingTests(unittest.TestCase):
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
