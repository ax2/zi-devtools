"""Run explicitly inside a trusted project's Django shell; writes only URL metadata JSON."""
import json
from django.urls import URLPattern, URLResolver, get_resolver


def collect(patterns, prefix="", namespaces=(), parameters=(), depth=0):
    if depth > 40:
        raise ValueError("URL nesting exceeds 40 levels")
    rows = []
    for item in patterns:
        route = prefix + str(item.pattern)
        names = tuple(dict.fromkeys((*parameters, *item.pattern.regex.groupindex)))
        if isinstance(item, URLResolver):
            nested = namespaces + ((item.namespace,) if item.namespace else ())
            rows.extend(collect(item.url_patterns, route, nested, names, depth + 1))
        elif isinstance(item, URLPattern):
            rows.append({"route": route, "name": item.name, "namespace": ":".join(namespaces),
                         "parameters": list(names), "regexRoute": item.pattern.__class__.__name__ == "RegexPattern"})
        if len(rows) > 10000:
            raise ValueError("More than 10000 URL patterns; export a smaller subtree")
    return rows


print(json.dumps(collect(get_resolver().url_patterns), ensure_ascii=False, indent=2))
