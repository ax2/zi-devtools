import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from verify_spring_actuator_candidate import BASE, NAME, digest, verify_candidate


class FrozenActuatorTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.candidate = self.root / "candidate"
        shutil.copytree(BASE / NAME, self.candidate)
        self.receipt = self.root / "receipt.json"
        self.data = json.loads((BASE / f"{NAME}-verification.json").read_text(encoding="utf-8"))
        self.data["candidate"] = "candidate"
        (self.root / "docs").mkdir()
        for name in ("tools.json", "studio-plugin-migration.json"):
            (self.root / "docs" / name).write_bytes(b"original")
        self.data["sourceCatalog"]["currentSha256"] = digest(b"original")
        self.data["migrationMap"]["sha256"] = digest(b"original")
        self.save_receipt()

    def save_receipt(self):
        self.receipt.write_text(json.dumps(self.data), encoding="utf-8")

    def verify(self):
        return verify_candidate(self.candidate, self.receipt, self.root)

    def test_frozen_bytes_survive_current_catalog_evolution_without_writes(self):
        before = {p: p.stat().st_mtime_ns for p in self.candidate.rglob("*") if p.is_file()}
        self.assertTrue(self.verify()["sourceCatalogMatchesCurrent"])
        (self.root / "docs/tools.json").write_bytes(b"new catalog")
        result = self.verify()
        self.assertTrue(result["verified"])
        self.assertFalse(result["sourceCatalogMatchesCurrent"])
        self.assertEqual(before, {p: p.stat().st_mtime_ns for p in before})

    def test_missing_current_source_is_unknown_not_frozen_failure(self):
        (self.root / "docs/tools.json").unlink()
        self.assertIsNone(self.verify()["sourceCatalogMatchesCurrent"])

    def test_changed_candidate_fails(self):
        with (self.candidate / "views/main.html").open("ab") as stream:
            stream.write(b"tampered")
        with self.assertRaisesRegex(ValueError, "inventory"):
            self.verify()

    def test_duplicate_and_traversal_receipt_paths_fail(self):
        original = self.data["files"][0]["path"]
        for invalid in (self.data["files"][1]["path"], "../outside"):
            self.data["files"][0]["path"] = invalid
            self.save_receipt()
            with self.assertRaises(RuntimeError):
                self.verify()
        self.data["files"][0]["path"] = original

    def test_index_and_identity_claims_fail(self):
        for field, value in (("directoryIndexSha256", "0" * 64), ("pluginId", "other"), ("pluginVersion", "9.9.9")):
            with self.subTest(field=field):
                original = self.data[field]
                self.data[field] = value
                self.save_receipt()
                with self.assertRaises(ValueError):
                    self.verify()
                self.data[field] = original

    def test_duplicate_json_keys_fail(self):
        self.receipt.write_text('{"candidate":"candidate","candidate":"candidate"}', encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "Duplicate JSON"):
            self.verify()

    def test_receipt_cannot_live_inside_candidate(self):
        nested = self.candidate / "receipt.json"
        shutil.copyfile(self.receipt, nested)
        with self.assertRaisesRegex(ValueError, "external"):
            verify_candidate(self.candidate, nested, self.root)


if __name__ == "__main__":
    unittest.main()
