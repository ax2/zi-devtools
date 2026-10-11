"""Fixed proposal.3 functional expectations, independent of runtime verifiers."""
from fields_vectors import rows as previous_rows, raw

OLD = "1.1.0-proposal.2"
VERSION = "1.1.0-proposal.3"

def success(text):
    return raw(dict(contractVersion=VERSION, ok=True, data=dict(text=text), error=None))


def failure(code, message):
    return raw(dict(contractVersion=VERSION, ok=False, data=None, error=dict(code=code, message=message)))


def request(left, right):
    return raw(dict(contractVersion=VERSION, pluginId='com.zicode.devtools.compare', sceneId='coding',
                    capabilityId='devtools.compare.text.diff', commandId=None, input=dict(left=left, right=right)))


cases = []
for name, body, expected in previous_rows:
    body = body.replace(OLD.encode(), VERSION.encode(), 1)
    expected = expected.replace(OLD.encode(), VERSION.encode(), 1)
    if name == 'unsupported-text-diff':
        expected = success('两份文本完全相同')
    cases.append((name, body, expected))
cases += [
    ('text-empty', request('', ''), success('两份文本完全相同')),
    ('text-replace', request('a\n', 'b\n'), success('--- 原文\n+++ 修改后\n@@ -1 +1 @@\n-a\n+b\n')),
    ('text-exact-byte', request('x' * 8192, 'x' * 8192), success('两份文本完全相同')),
    ('text-diff-byte-over', request('x' * 8193, ''), failure('INPUT_TOO_LARGE', '差异字段超过 UTF-8 字节限制')),
    ('text-unicode-byte-over', request('中' * 2731, ''), failure('INPUT_TOO_LARGE', '差异字段超过 UTF-8 字节限制')),
    ('text-exact-lines', request('\n' * 5000, '\n' * 5000), success('两份文本完全相同')),
    ('text-lines-over', request('\n' * 5001, '\n' * 5001), failure('INVALID_INPUT', '输入无法按该操作处理')),
    ('proposal2-rejected', request('a', 'b').replace(VERSION.encode(), OLD.encode(), 1),
     failure('INVALID_INPUT', '协议、插件或场景标识无效')),
]
for field in ['left', 'right']:
    valid_input = dict(left='a', right='b')
    for label, value in [('null', None), ('number', 1), ('bool', True), ('array', []), ('object', {})]:
        values = dict(valid_input, **{field: value})
        body = request('a', 'b').replace(raw(valid_input), raw(values))
        cases.append((f'text-{field}-{label}', body, failure('INVALID_INPUT', '差异输入结构无效')))
    body = request('a', 'b').replace(f'"{field}":"{valid_input[field]}"'.encode(),
                                    f'"{field}":"a","{field}":"b"'.encode())
    cases.append((f'text-duplicate-{field}', body, failure('INVALID_INPUT', '差异输入结构无效')))
# Report bytes fit the core limit, but JSON escaping pushes the envelope over it.
left = ''.join(chr(128 + i) + '\x01\x02\n' for i in range(1450))
right = ''.join(chr(128 + i) + '\x03\x04\n' for i in range(1450))
body = request(left, right)
assert len(left.encode('utf-8')) <= 8192 and len(right.encode('utf-8')) <= 8192 and len(body) <= 49152
cases.append(('text-final-escaped-envelope-over', body, failure('INPUT_TOO_LARGE', '结果超过 48 KiB')))
left = ''.join(chr(128 + i) + '\x01\x02\n' for i in range(1400))
right = ''.join(chr(128 + i) + '\x03\x04\n' for i in range(1400))
# No old/new line is equal, so the original unified diff deletes then inserts all.
# Split only on LF: Python splitlines also splits characters that Rust preserves.
report = '--- 原文\n+++ 修改后\n@@ -1,1400 +1,1400 @@\n'
report += ''.join('-' + line + '\n' for line in left.split('\n')[:-1])
report += ''.join('+' + line + '\n' for line in right.split('\n')[:-1])
assert len(success(report)) <= 49152
cases.append(('text-near-final-envelope-budget', request(left, right), success(report)))
for count, expected in [
    (1530, failure('INPUT_TOO_LARGE', '结果超过 48 KiB')),
    (1531, failure('INPUT_TOO_LARGE', '请求超过 48 KiB')),
]:
    left = ''.join(chr(128 + i) + '\x01\x02\n' for i in range(count))
    right = ''.join(chr(128 + i) + '\x03\x04\n' for i in range(count))
    body = request(left, right)
    assert (len(body) <= 49152) == (count == 1530)
    cases.append((f'text-escaped-request-edge-{count}', body, expected))
assert len({case[0] for case in cases}) == len(cases), 'Duplicate functional case ID'
