"""Disposable TLS OAuth server / SDK reverse proxy. Synthetic credentials only.
Requires cryptography; certificate/key live only in a newly created temp directory.
No system trust changes. No request logging. The SDK must run auth lifecycle mode.
"""
import argparse
import base64
from datetime import datetime, timedelta, timezone
import hashlib
import http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import ipaddress
import json
from pathlib import Path
import ssl
import tempfile
import threading
from urllib.parse import parse_qs, urlencode, urlsplit
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.x509.oid import NameOID

parser = argparse.ArgumentParser()
parser.add_argument('sdk_endpoint')
args = parser.parse_args()
sdk = urlsplit(args.sdk_endpoint)
assert sdk.scheme == 'http' and sdk.hostname == '127.0.0.1' and sdk.path == '/mcp'
temporary = tempfile.TemporaryDirectory(prefix='zi-oauth-tls-')
root = Path(temporary.name)
key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
ca_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, 'Zi synthetic TLS fixture')])
ca_name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, 'Zi disposable test CA')])
now = datetime.now(timezone.utc)
ca = (x509.CertificateBuilder().subject_name(ca_name).issuer_name(ca_name).public_key(ca_key.public_key())
      .serial_number(x509.random_serial_number()).not_valid_before(now-timedelta(minutes=1))
      .not_valid_after(now+timedelta(hours=1)).add_extension(x509.BasicConstraints(ca=True, path_length=0), True).sign(ca_key, hashes.SHA256()))
cert = (x509.CertificateBuilder().subject_name(name).issuer_name(ca_name).public_key(key.public_key())
        .serial_number(x509.random_serial_number()).not_valid_before(now-timedelta(minutes=1))
        .not_valid_after(now+timedelta(hours=1)).add_extension(x509.BasicConstraints(ca=False, path_length=None), True)
        .add_extension(x509.SubjectAlternativeName([x509.IPAddress(ipaddress.ip_address('127.0.0.1'))]), False)
        .sign(ca_key, hashes.SHA256()))
(root/'cert.pem').write_bytes(ca.public_bytes(serialization.Encoding.PEM))
(root/'server.pem').write_bytes(cert.public_bytes(serialization.Encoding.PEM)+ca.public_bytes(serialization.Encoding.PEM))
(root/'key.pem').write_bytes(key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()))
state = {'redirect': None, 'challenge': None, 'code_used': False, 'refresh_used': False, 'refresh_revoked': False}

def upstream(path, body=b'', headers=None, method='POST'):
    conn = http.client.HTTPConnection('127.0.0.1', sdk.port, timeout=5)
    conn.request(method, path, body, headers or {})
    response = conn.getresponse()
    payload = response.read(4*1024*1024+1)
    assert len(payload) <= 4*1024*1024
    result = response.status, response.getheaders(), payload
    conn.close()
    return result

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_): pass
    def respond(self, status, payload=None, headers=None):
        body = json.dumps(payload).encode() if payload is not None else b''
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.send_header('Cache-Control', 'no-store')
        for key, value in (headers or {}).items(): self.send_header(key, value)
        self.end_headers()
        self.wfile.write(body)
    def do_DELETE(self): self.handle_request()
    def do_GET(self): self.handle_request()
    def do_POST(self): self.handle_request()
    def handle_request(self):
        try:
            length = int(self.headers.get('Content-Length', 0))
            assert 0 <= length <= 64*1024 and not self.headers.get('Transfer-Encoding')
            body = self.rfile.read(length)
            path = urlsplit(self.path).path
            if path == '/mcp' or path == '/fixture/finish':
                headers = {k: v for k, v in self.headers.items() if k.lower() not in ('host','connection','content-length')}
                status, reply_headers, payload = upstream(self.path, body, headers, self.command)
                self.send_response(status)
                for key, value in reply_headers:
                    if key.lower() not in ('content-length','transfer-encoding','connection','www-authenticate'):
                        self.send_header(key, value)
                if status == 401:
                    self.send_header('WWW-Authenticate', f'Bearer resource_metadata="{origin}/.well-known/oauth-protected-resource/mcp"')
                self.send_header('Content-Length', str(len(payload)))
                self.end_headers(); self.wfile.write(payload)
                if path == '/fixture/finish': threading.Thread(target=server.shutdown, daemon=True).start()
                return
            assert not self.headers.get('Authorization') and not self.headers.get('Cookie')
            if path == '/.well-known/oauth-protected-resource/mcp':
                self.respond(200, {'resource':origin+'/mcp','authorization_servers':[origin],'scopes_supported':['fixture:read'],'bearer_methods_supported':['header']}); return
            if path == '/.well-known/oauth-authorization-server':
                self.respond(200, {'issuer':origin,'authorization_endpoint':origin+'/authorize','token_endpoint':origin+'/token',
                    'registration_endpoint':origin+'/register','revocation_endpoint':origin+'/revoke','revocation_endpoint_auth_methods_supported':['none'],
                    'token_endpoint_auth_methods_supported':['none'],'response_types_supported':['code'],'code_challenge_methods_supported':['S256'],
                    'authorization_response_iss_parameter_supported':True}); return
            if path == '/register':
                data = json.loads(body)
                assert data['scope'] == 'fixture:read' and data['token_endpoint_auth_method'] == 'none'
                assert len(data['redirect_uris']) == 1
                callback = urlsplit(data['redirect_uris'][0])
                assert callback.scheme == 'http' and callback.hostname == '127.0.0.1' and callback.path == '/oauth/callback/zi-devtools'
                state['redirect'] = data['redirect_uris'][0]
                data['client_id'] = 'synthetic-client'
                self.respond(201, data); return
            fields = parse_qs(urlsplit(self.path).query if self.command == 'GET' else body.decode(), strict_parsing=True)
            assert all(len(values) == 1 for values in fields.values())
            fields = {k:v[0] for k,v in fields.items()}
            assert fields['client_id'] == 'synthetic-client'
            if path == '/authorize':
                assert fields['resource'] == origin+'/mcp' and fields['redirect_uri'] == state['redirect']
                assert fields['code_challenge_method'] == 'S256' and fields['scope'] == 'fixture:read'
                state['challenge'] = fields['code_challenge']
                location = state['redirect']+'?'+urlencode({'state':fields['state'],'iss':origin,'code':'synthetic-tls-code'})
                self.respond(302, headers={'Location':location}); return
            if path == '/token':
                assert fields['resource'] == origin+'/mcp'
                if fields['grant_type'] == 'authorization_code':
                    assert not state['code_used'] and fields['code'] == 'synthetic-tls-code'
                    assert fields['redirect_uri'] == state['redirect']
                    challenge = base64.urlsafe_b64encode(hashlib.sha256(fields['code_verifier'].encode()).digest()).decode().rstrip('=')
                    assert challenge == state['challenge']
                    state['code_used'] = True
                    access, refresh = 'zi-sdk-synthetic-old', 'synthetic-refresh-old'
                else:
                    if state['refresh_revoked']:
                        self.respond(400, {'error':'invalid_grant'}); return
                    assert not state['refresh_used'] and fields['refresh_token'] == 'synthetic-refresh-old'
                    state['refresh_used'] = True
                    access, refresh = 'zi-sdk-synthetic-new', 'synthetic-refresh-new'
                self.respond(200, {'access_token':access,'refresh_token':refresh,'token_type':'Bearer','expires_in':120,'scope':'fixture:read'}); return
            if path == '/revoke':
                assert len(fields) == 3
                if fields['token_type_hint'] == 'refresh_token':
                    assert fields['token'] == 'synthetic-refresh-new'
                    state['refresh_revoked'] = True
                else: assert fields['token'] == 'zi-sdk-synthetic-new'
                status, _, _ = upstream('/fixture/revoke', json.dumps({'token':fields['token']}).encode(), {'Content-Type':'application/json'})
                assert status == 200
                self.respond(200); return
            self.respond(404)
        except Exception:
            self.respond(400, {'error':'synthetic_fixture_rejected'})

server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
origin = f'https://127.0.0.1:{server.server_port}'
context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.load_cert_chain(root/'server.pem', root/'key.pem')
server.socket = context.wrap_socket(server.socket, server_side=True)
print(json.dumps({'endpoint':origin+'/mcp','certificate':str(root/'cert.pem')}), flush=True)
timer = threading.Timer(90, server.shutdown); timer.daemon = True; timer.start()
try: server.serve_forever()
finally: timer.cancel(); server.server_close(); temporary.cleanup()
