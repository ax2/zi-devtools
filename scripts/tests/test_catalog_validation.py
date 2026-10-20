import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from catalog_validation import normalized_search_term, validate_exact_search_terms


class CatalogValidationTests(unittest.TestCase):
    def test_rejects_nfkc_whitespace_and_case_collisions(self):
        tools = [
            {
                "id": "json",
                "discovery": {"label": "JSON", "aliases": ["json format", "format JSON"]},
            },
            {
                "id": "plugin-tool",
                "discovery": {
                    "label": "Formatter",
                    "aliases": ["  ＪＳＯＮ   ", "format data"],
                },
            },
        ]
        with self.assertRaisesRegex(AssertionError, "Exact search term is shared"):
            validate_exact_search_terms(tools, {"json", "plugin-tool"})

    def test_lowercase_matches_runtime_without_casefold_expansion(self):
        tools = [
            {
                "id": "first",
                "discovery": {"label": "First", "aliases": ["Straße", "word one"]},
            },
            {
                "id": "second",
                "discovery": {"label": "Second", "aliases": ["STRASSE", "word two"]},
            },
        ]
        owners = validate_exact_search_terms(tools, {"first", "second"})
        self.assertIn("straße", owners)
        self.assertIn("strasse", owners)
        self.assertNotEqual(normalized_search_term("Straße"), normalized_search_term("STRASSE"))

    def test_allows_same_tool_to_repeat_its_own_name_as_an_alias(self):
        tools = [
            {
                "id": "json",
                "discovery": {"label": "JSON", "aliases": ["json", "json format"]},
            }
        ]
        owners = validate_exact_search_terms(tools, {"json"})
        self.assertEqual(owners["json"], "json")

    def test_ignores_unrouted_planned_entries(self):
        tools = [
            {"id": "json", "discovery": {"label": "JSON", "aliases": ["json", "format"]}},
            {"id": "planned", "name": "JSON", "status": "planned"},
        ]
        owners = validate_exact_search_terms(tools, {"json"})
        self.assertEqual(set(owners.values()), {"json"})


if __name__ == "__main__":
    unittest.main()
