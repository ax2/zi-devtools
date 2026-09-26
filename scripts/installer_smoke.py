"""Exercise the generated MSI in a disposable directory."""
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib

root = Path(__file__).resolve().parents[1]
version = tomllib.loads((root / 'Cargo.toml').read_text(encoding='utf-8'))['package']['version']
setup = root / 'release' / f'ZiDevTools-{version}-windows-x64.msi'
assert setup.is_file(), f'Current version installer missing: {setup.name}'
folder = Path(tempfile.mkdtemp(prefix='zi-installer-'))
target = folder / 'app'
msiexec = str(Path(os.environ['SystemRoot']) / 'System32/msiexec.exe')

def execute(operation):
    log = folder / ('install.log' if operation == '/i' else 'uninstall.log')
    args = [msiexec, operation, str(setup), '/qn', '/norestart', '/l*v', str(log)]
    if operation == '/i':
        args.append(f'INSTALLFOLDER={target}')
    result = subprocess.run(args, timeout=180, creationflags=subprocess.CREATE_NO_WINDOW)
    if result.returncode not in (0, 3010):
        if log.exists():
            print(log.read_text(encoding='utf-16', errors='replace')[-16000:])
        raise RuntimeError(f'MSI process exited {result.returncode}')

execute('/i')
installed = target / 'ZiDevTools.exe'
try:
    assert installed.is_file(), 'Installed executable missing'
    assert hashlib.sha256(installed.read_bytes()).digest() == hashlib.sha256((root / 'target/release/ZiDevTools.exe').read_bytes()).digest(), 'Installed file differs'
finally:
    execute('/x')
assert not installed.exists(), 'Executable remains after uninstall'
print('PASS: silent installation, executable SHA-256, silent uninstallation')
