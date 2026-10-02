"""Run all disposable official-SDK/OAuth interoperability cases on Windows.
Dependencies are development-only: Node, SDK 1.31.0 and Python cryptography.
No existing services, user credentials or system certificate stores are used.
"""
import argparse
from contextlib import contextmanager
import json
import os
from pathlib import Path
import queue
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]
FLAGS = subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0
parser = argparse.ArgumentParser()
parser.add_argument('--sdk-directory', type=Path, required=True)
parser.add_argument('--output', type=Path)
args = parser.parse_args()
sdk_directory = args.sdk_directory.resolve(strict=True)
package = json.loads((sdk_directory/'node_modules/@modelcontextprotocol/sdk/package.json').read_text(encoding='utf-8'))
if package['version'] != '1.31.0':
    raise SystemExit('Expected the pinned official SDK 1.31.0')
if not shutil.which('node') or not shutil.which('cargo'):
    raise SystemExit('Node and Cargo must be available')
import cryptography  # Fail before starting any service if the optional dependency is absent.

def run(command, env=None, timeout=600):
    subprocess.run(command, cwd=ROOT, env=env, stdin=subprocess.DEVNULL,
                   check=True, timeout=timeout, creationflags=FLAGS)

@contextmanager
def service(command):
    # Only this invocation's child processes may be terminated during cleanup.
    with tempfile.TemporaryFile(mode='w+t', encoding='utf-8') as errors:
        process = subprocess.Popen(command, cwd=ROOT, stdin=subprocess.DEVNULL,
                                   stdout=subprocess.PIPE, stderr=errors, text=True,
                                   encoding='utf-8', creationflags=FLAGS)
        lines = queue.Queue()
        reader = threading.Thread(target=lambda: lines.put(process.stdout.readline()), daemon=True)
        reader.start()
        try:
            try:
                line = lines.get(timeout=20).strip()
            except queue.Empty:
                raise RuntimeError('Disposable service did not announce readiness') from None
            if not line:
                raise RuntimeError('Disposable service exited before readiness')
            yield line
            if process.wait(timeout=10) != 0:
                raise RuntimeError('Disposable service exited unsuccessfully')
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
            process.stdout.close()

def loopback(endpoint, scheme):
    url = urlsplit(endpoint)
    if url.scheme != scheme or url.hostname != '127.0.0.1' or url.path != '/mcp' or not url.port or url.username or url.password or url.query or url.fragment:
        raise RuntimeError('Fixture announced an unexpected endpoint')
    return endpoint

def test(target, name, env):
    run(['cargo', 'test', '--locked', *target, name, '--', '--ignored', '--exact'], env, timeout=30)

start = time.monotonic()
cases = []
# Compile first: a fixture's short lifetime must not be consumed by compilation.
run(['cargo', 'test', '--locked', '--lib', '--no-run'])
run(['cargo', 'test', '--locked', '--test', 'mcp_http_sdk', '--no-run'])
with tempfile.TemporaryDirectory(prefix='zi-sdk-verify-', dir=sdk_directory) as temporary, tempfile.TemporaryDirectory(prefix='zi-oauth-run-') as tls_temporary:
    fixture = Path(temporary)/'fixture.mjs'
    shutil.copyfile(ROOT/'tests/fixtures/mcp_sdk_http.mjs', fixture)
    for mode in ('json', 'sse'):
        for phase in ('transport', 'authentication', 'oauth', 'tls'):
            command = ['node', str(fixture), mode]
            if phase != 'transport': command += ['auth']
            if phase in ('oauth', 'tls'): command += ['lifecycle']
            if phase == 'tls': command += ['tls']
            case_start = time.monotonic()
            with service(command) as endpoint:
                endpoint = loopback(endpoint, 'http')
                env = os.environ.copy()
                # Do not inherit earlier fixture endpoints or scoped certificate paths.
                for key in ('ZIDEVTOOLS_MCP_SDK_ENDPOINT', 'ZIDEVTOOLS_MCP_TLS_ENDPOINT', 'ZIDEVTOOLS_MCP_TLS_CERTIFICATE'):
                    env.pop(key, None)
                if phase == 'tls':
                    # Parent owns this directory too: Windows process termination
                    # need not run the child's finally/TemporaryDirectory cleanup.
                    with service([sys.executable, str(ROOT/'tests/fixtures/mcp_oauth_tls.py'), endpoint, '--temporary-parent', tls_temporary]) as ready:
                        info = json.loads(ready)
                        env['ZIDEVTOOLS_MCP_TLS_ENDPOINT'] = loopback(info['endpoint'], 'https')
                        certificate = Path(info['certificate']).resolve(strict=True)
                        # A fixture-created temp directory is the only accepted trust source.
                        temp_root = Path(tls_temporary).resolve()
                        if not certificate.is_relative_to(temp_root) or not certificate.parent.name.startswith('zi-oauth-tls-') or certificate.name != 'cert.pem':
                            raise RuntimeError('Fixture certificate is outside its disposable directory')
                        env['ZIDEVTOOLS_MCP_TLS_CERTIFICATE'] = str(certificate)
                        test(['--lib'], 'mcp_oauth_tls_tests::https_discovery_registration_login_refresh_and_revocation', env)
                    if certificate.parent.exists():
                        raise RuntimeError('Disposable TLS certificate directory was not cleaned')
                else:
                    env['ZIDEVTOOLS_MCP_SDK_ENDPOINT'] = endpoint
                    if phase == 'transport':
                        test(['--test', 'mcp_http_sdk'], 'official_sdk_lists_calls_reads_and_gets_prompts', env)
                    elif phase == 'authentication':
                        test(['--test', 'mcp_http_sdk'], 'official_sdk_authentication_and_credential_rotation', env)
                    else:
                        test(['--lib'], 'mcp_oauth_flow_tests::registration_callback_exchange_refresh_sdk_rotation_and_revoke', env)
            cases.append({'mode':mode, 'phase':phase, 'result':'passed', 'seconds':round(time.monotonic()-case_start, 3)})
            print(f'{mode}/{phase}: passed; disposable services exited successfully', flush=True)
result = {'sdk_version':package['version'], 'cryptography_version':cryptography.__version__,
          'cases':cases, 'elapsed_seconds':round(time.monotonic()-start, 3)}
if args.output:
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2)+'\n', encoding='utf-8')
print(json.dumps(result, indent=2))
