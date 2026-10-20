"""Validation helpers for the built-in searchable tool catalog."""
import re
import unicodedata


def normalized_search_term(term: str) -> str:
    return re.sub(r"\s+", " ", unicodedata.normalize("NFKC", term)).strip().lower()


def validate_exact_search_terms(items: list[dict], routed_ids: set[str]) -> dict[str, str]:
    """Reject exact ID, display-name, or alias collisions between routed tools."""
    owners: dict[str, str] = {}
    for tool in items:
        if tool["id"] not in routed_ids:
            continue
        discovery = tool["discovery"]
        for term in (tool["id"], discovery["label"], *discovery["aliases"]):
            normalized = normalized_search_term(term)
            owner = owners.setdefault(normalized, tool["id"])
            assert owner == tool["id"], (
                f"Exact search term is shared by routed tools: "
                f"{term} ({owner}, {tool['id']})"
            )
    return owners
