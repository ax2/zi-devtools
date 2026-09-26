"""Exercise the generated installer using CreateProcess, without shell shims."""
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
setup = next((root / 'release').glob('*-setup.exe'))
folder = Path(tempfile.mkdtemp(prefix='zi-installer-'))
target = folder / 'app'
log = folder / 'install.log'
env = os.environ.copy()
env.pop('__COMPAT_LAYER', None)

def execute(args):
    result = subprocess.run(args, env=env, timeout=120, creationflags=subprocess.CREATE_NO_WINDOW)
    if result.returncode:
        if log.exists():
            print(log.read_text(encoding='utf-8-sig', errors='replace'))
        raise RuntimeError(f'Installer process exited {result.returncode}')

execute([str(setup), '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', f'/DIR={target}', f'/LOG={log}'])
installed = target / 'ZiDevTools.exe'
assert installed.is_file(), 'Installed executable missing'
assert hashlib.sha256(installed.read_bytes()).digest() == hashlib.sha256((root / 'target/release/ZiDevTools.exe').read_bytes()).digest(), 'Installed file differs'
execute([str(target / 'unins000.exe'), '/VERYSILENT', '/NORESTART'])
assert not installed.exists(), 'Executable remains after uninstall'
print('PASS: silent installation, executable SHA-256, silent uninstallation')
