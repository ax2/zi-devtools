"""Minimal standard-library Chromium DevTools WebSocket client for local QA."""
import base64
import json
import os
import socket
import struct
import urllib.parse


class DevTools:
    def __init__(self, url):
        parsed = urllib.parse.urlparse(url)
        self.socket = socket.create_connection((parsed.hostname, parsed.port), timeout=10)
        key = base64.b64encode(os.urandom(16)).decode()
        self.socket.sendall((f'GET {parsed.path} HTTP/1.1\r\nHost: {parsed.hostname}:{parsed.port}\r\n'
                             f'Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\n'
                             'Sec-WebSocket-Version: 13\r\n\r\n').encode())
        response = b''
        while b'\r\n\r\n' not in response:
            response += self.socket.recv(4096)
        assert response.startswith(b'HTTP/1.1 101 '), response[:200]
        self.id = 0
        self.events = []

    def command(self, method, params=None):
        self.id += 1
        message = json.dumps({'id': self.id, 'method': method, 'params': params or {}}).encode()
        mask = os.urandom(4)
        length = len(message)
        if length < 126:
            header = bytes([0x81, 0x80 | length])
        elif length < 65536:
            header = bytes([0x81, 0x80 | 126]) + struct.pack('!H', length)
        else:
            header = bytes([0x81, 0x80 | 127]) + struct.pack('!Q', length)
        self.socket.sendall(header + mask + bytes(byte ^ mask[i % 4] for i, byte in enumerate(message)))
        while True:
            first = self._read(2)
            opcode, length = first[0] & 0x0f, first[1] & 0x7f
            if length == 126:
                length = struct.unpack('!H', self._read(2))[0]
            elif length == 127:
                length = struct.unpack('!Q', self._read(8))[0]
            payload = self._read(length)
            if opcode == 8:
                raise RuntimeError('DevTools websocket closed')
            if opcode == 9:
                self.socket.sendall(bytes([0x8a, len(payload)]) + payload)
                continue
            reply = json.loads(payload)
            if reply.get('id') == self.id:
                if 'error' in reply:
                    raise RuntimeError(reply['error'])
                return reply.get('result', {})
            self.events.append(reply)

    def _read(self, length):
        chunks = bytearray()
        while len(chunks) < length:
            part = self.socket.recv(length - len(chunks))
            if not part:
                raise RuntimeError('DevTools websocket ended')
            chunks.extend(part)
        return bytes(chunks)

    def evaluate(self, expression):
        result = self.command('Runtime.evaluate', {'expression': expression, 'returnByValue': True})
        if result.get('exceptionDetails'):
            raise RuntimeError(result['exceptionDetails'])
        return result.get('result', {}).get('value')

    def close(self):
        self.socket.close()
