"""Small, bounded RFC 6455 text channel for authenticated local preview controls."""
import base64
import hashlib
import json
import secrets
import socket
import struct
import threading

from dev_bridge import BridgeError


class Socket:
    def __init__(self, handler):
        self.handler = handler
        self.lock = threading.Lock()

    @staticmethod
    def upgrade(handler, token):
        protocols = [p.strip() for p in handler.headers.get('Sec-WebSocket-Protocol', '').split(',')]
        expected = 'http://' + handler.headers.get('Host', '')
        if (handler.headers.get('Origin') != expected or 'rake-preview-v1' not in protocols
                or not any(secrets.compare_digest(p, 'rake-token.' + token) for p in protocols)):
            raise BridgeError(403, 'Preview socket authentication failed')
        key = handler.headers.get('Sec-WebSocket-Key', '')
        try:
            valid_key = len(base64.b64decode(key, validate=True)) == 16
        except ValueError:
            valid_key = False
        if (not valid_key or handler.headers.get('Sec-WebSocket-Version') != '13'
                or handler.headers.get('Upgrade', '').lower() != 'websocket'
                or 'upgrade' not in handler.headers.get('Connection', '').lower()):
            raise BridgeError(400, 'Invalid WebSocket upgrade')
        handler.send_response(101)
        handler.send_header('Upgrade', 'websocket')
        handler.send_header('Connection', 'Upgrade')
        handler.send_header('Sec-WebSocket-Protocol', 'rake-preview-v1')
        digest = hashlib.sha1((key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest()
        handler.send_header('Sec-WebSocket-Accept', base64.b64encode(digest).decode())
        handler.end_headers()
        handler.close_connection = True
        return Socket(handler)

    def send(self, value, opcode=1):
        data = json.dumps(value).encode() if opcode == 1 else value
        if len(data) > 256 * 1024:
            raise ValueError('Preview response too large')
        header = bytes([0x80 | opcode])
        header += bytes([len(data)]) if len(data) < 126 else b'\x7e' + struct.pack('>H', len(data)) if len(data) < 65536 else b'\x7f' + struct.pack('>Q', len(data))
        with self.lock:
            self.handler.wfile.write(header + data)

    def read(self):
        def exact(count):
            data = bytearray()
            while len(data) < count:
                chunk = self.handler.rfile.read(count - len(data))
                if not chunk:
                    raise EOFError()
                data.extend(chunk)
            return data
        first, second = exact(2)
        opcode = first & 15
        if first & 0xf0 != 0x80 or not second & 0x80 or opcode not in (1, 8, 9, 10):
            raise ValueError('Unsupported WebSocket frame')
        size = second & 127
        if size == 126:
            size = struct.unpack('>H', exact(2))[0]
        elif size == 127:
            size = struct.unpack('>Q', exact(8))[0]
        if size > 16384 or opcode >= 8 and size > 125:
            raise ValueError('Preview control frame too large')
        mask = exact(4)
        payload = bytes(v ^ mask[i % 4] for i, v in enumerate(exact(size)))
        if opcode == 8:
            self.send(b'', 8)
            raise EOFError()
        if opcode == 9:
            self.send(payload, 10)
        if opcode != 1:
            return None
        value = json.loads(payload)
        if not isinstance(value, dict):
            raise ValueError('Expected preview control object')
        return value

    def close(self):
        try:
            self.handler.connection.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
