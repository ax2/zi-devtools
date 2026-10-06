"""Read one earlier official stable release; never installs or executes its files."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile


def gh(*args):
    result = subprocess.run(['gh', *args, '--repo', 'ax2/zi-devtools'], capture_output=True, timeout=180)
    if result.returncode:
        raise RuntimeError('Cannot read previous official release; refusing to publish partial update')
    if len(result.stdout) > 2 * 1024 * 1024:
        raise RuntimeError('Release response too large')
    return result.stdout


def numeric(text):
    if not re.fullmatch(r'0|[1-9][0-9]*', text):
        raise ValueError('Invalid numeric version component')
    return int(text)


def version(text):
    parts = text.split('.')
    if len(parts) != 3:
        raise ValueError('Release baseline requires stable numeric SemVer')
    return tuple(map(numeric, parts))


def main():
    if os.environ.get('GITHUB_ACTIONS') != 'true' or os.environ.get('RUNNER_ENVIRONMENT') != 'github-hosted':
        raise RuntimeError('Baseline collection is limited to disposable release CI')
    target = sys.argv[1]
    current = version(target)
    releases = json.loads(gh('release', 'list', '--limit', '100', '--json', 'tagName,isDraft,isPrerelease'))
    candidates = []
    for release in releases:
        if release['isDraft'] or release['isPrerelease']:
            continue
        tag = release['tagName']
        try:
            parsed = version(tag.removeprefix('v'))
        except ValueError:
            continue
        if tag == 'v' + '.'.join(map(str, parsed)) and parsed < current:
            candidates.append((parsed, tag))
    if not candidates:
        print('No previous stable release; complete update only')
        return
    _, tag = max(candidates)
    source = tag[1:]
    base = Path(os.environ['RUNNER_TEMP']).resolve(strict=True)
    folder = Path(tempfile.mkdtemp(prefix='zi-delta-baseline-', dir=base))
    if folder.is_symlink() or not folder.resolve().is_relative_to(base):
        raise RuntimeError('Invalid baseline directory')
    names = [f'ZiDevTools-{source}-windows-x64.exe', f'ZiDevTools-{source}-windows-x64.msi', f'ZiDevToolsMcp-{source}-windows-x64.exe']
    metadata = json.loads(gh('release', 'view', tag, '--json', 'assets'))['assets']
    if len(metadata) > 32:
        raise RuntimeError('Too many previous-release assets')
    for name in ['SHA256SUMS.txt', names[0], names[2]]:
        matches = [a for a in metadata if a['name'] == name]
        maximum = 8192 if name == 'SHA256SUMS.txt' else 128 * 1024 * 1024
        if len(matches) != 1 or not 0 < matches[0]['size'] <= maximum:
            raise RuntimeError('Baseline asset missing or size invalid')
    for name in ['SHA256SUMS.txt', names[0], names[2]]:
        gh('release', 'download', tag, '--pattern', name, '--dir', str(folder))
    sums = (folder / 'SHA256SUMS.txt').read_bytes()
    if len(sums) > 8192:
        raise RuntimeError('Baseline checksums too large')
    entries = {}
    for line in sums.decode('utf-8-sig').splitlines():
        if not line.strip():
            continue
        match = re.fullmatch(r'([0-9a-fA-F]{64})[ \t]+\*?([^/\\]+)', line)
        if not match or match[2] not in names or match[2] in entries:
            raise RuntimeError('Invalid previous-release checksum list')
        entries[match[2]] = match[1].lower()
    if set(entries) != set(names):
        raise RuntimeError('Incomplete baseline checksum list')
    for name in [names[0], names[2]]:
        path = folder / name
        if path.is_symlink() or not 0 < path.stat().st_size <= 128 * 1024 * 1024:
            raise RuntimeError('Baseline file type/size invalid')
        with path.open('rb') as content:
            if hashlib.file_digest(content, 'sha256').hexdigest() != entries[name]:
                raise RuntimeError('Baseline checksum mismatch')
    with open(os.environ['GITHUB_OUTPUT'], 'a', encoding='utf-8') as output:
        output.write(f'directory={folder}\nversion={source}\n')
    print(f'Previous official release {tag}: two EXE checksums verified; no execution')


if __name__ == '__main__':
    main()
