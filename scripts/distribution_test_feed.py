"""Disposable loopback HTTPS + Ed25519 signing for native distribution tests.

Private test keys live outside the served tree. No production keychain/store or
system trust is changed. This module also implements the fixture's signing argv.
"""
import functools
import http.server
import pathlib
import re
import ssl
import subprocess
import sys
import tempfile
import threading


def execute(*args):
    return subprocess.check_output([str(value) for value in args], stderr=subprocess.PIPE)


def sign(key, message):
    with tempfile.NamedTemporaryFile(prefix='rake-test-message-', delete=False) as temporary:
        path = pathlib.Path(temporary.name)
        temporary.write(message)
    try:
        return execute('openssl', 'pkeyutl', '-sign', '-rawin', '-inkey', key, '-in', path)
    finally:
        path.unlink()


class FeedHandler(http.server.SimpleHTTPRequestHandler):
    # Keep explicitly framed responses alive. HTTP/1.0 connection-delimited
    # responses otherwise depend on Python/Schannel TLS shutdown behavior.
    protocol_version = 'HTTP/1.1'
    extensions_map = {
        **http.server.SimpleHTTPRequestHandler.extensions_map,
        '.appinstaller': 'application/appinstaller',
        '.msix': 'application/msix',
    }

    def end_headers(self):
        self.send_header('Accept-Ranges', 'bytes')
        super().end_headers()

    def send_head(self):
        self.byte_range = None
        value = self.headers.get('Range')
        path = pathlib.Path(self.translate_path(self.path))
        if not value or not path.is_file():
            return super().send_head()
        size = path.stat().st_size
        match = re.fullmatch(r'bytes=(\d*)-(\d*)', value)
        try:
            assert match and any(match.groups())
            first, last = match.groups()
            if first:
                start, end = int(first), min(int(last), size - 1) if last else size - 1
            else:
                start, end = max(0, size - int(last)), size - 1
            assert 0 <= start <= end < size
        except (AssertionError, ValueError):
            self.send_response(416)
            self.send_header('Content-Range', f'bytes */{size}')
            self.send_header('Content-Length', '0')
            self.end_headers()
            return None
        source = path.open('rb')
        source.seek(start)
        self.byte_range = end - start + 1
        self.send_response(206)
        self.send_header('Content-Type', self.guess_type(str(path)))
        self.send_header('Content-Range', f'bytes {start}-{end}/{size}')
        self.send_header('Content-Length', str(self.byte_range))
        self.send_header('Last-Modified', self.date_time_string(path.stat().st_mtime))
        self.end_headers()
        return source

    def copyfile(self, source, destination):
        if self.byte_range is None:
            return super().copyfile(source, destination)
        remaining = self.byte_range
        while remaining:
            data = source.read(min(remaining, 64 * 1024))
            if not data:
                break
            destination.write(data)
            remaining -= len(data)

    def do_GET(self):
        if self.path == getattr(self.server, 'truncated_path', None):
            path = pathlib.Path(self.translate_path(self.path))
            data = path.read_bytes()
            data = data[:len(data) // 2]
            self.send_response(200)
            self.send_header('Content-Length', str(len(data)))
            self.end_headers()
            self.wfile.write(data)
            return
        super().do_GET()


class TestFeed:
    def __init__(self, root):
        self.root = pathlib.Path(root)
        self.private = self.root / 'credentials'
        self.private.mkdir(mode=0o700)
        self.site = self.root / 'site'
        self.site.mkdir()
        self.ca = self.private / 'ca.pem'
        self.key = self.private / 'update-key.pem'
        execute('openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1', '-keyout', self.private / 'ca.key', '-out', self.ca, '-subj', '/CN=Rake disposable native test CA', '-addext', 'basicConstraints=critical,CA:TRUE', '-addext', 'keyUsage=critical,keyCertSign,cRLSign')
        execute('openssl', 'req', '-newkey', 'rsa:2048', '-nodes', '-keyout', self.private / 'server.key', '-out', self.private / 'server.csr', '-subj', '/CN=127.0.0.1')
        extensions = self.private / 'server.ext'
        extensions.write_text('basicConstraints=critical,CA:FALSE\nsubjectAltName=IP:127.0.0.1\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n')
        execute('openssl', 'x509', '-req', '-days', '1', '-in', self.private / 'server.csr', '-CA', self.ca, '-CAkey', self.private / 'ca.key', '-CAcreateserial', '-extfile', extensions, '-out', self.private / 'server.pem')
        execute('openssl', 'genpkey', '-algorithm', 'ED25519', '-out', self.key)
        self.key.chmod(0o600)
        public = execute('openssl', 'pkey', '-in', self.key, '-pubout', '-outform', 'DER')
        assert len(public) == 44 and public[:12] == bytes.fromhex('302a300506032b6570032100')
        self.public_key = public[-32:]
        handler = functools.partial(FeedHandler, directory=str(self.site))
        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), handler)
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(self.private / 'server.pem', self.private / 'server.key')
        self.server.socket = context.wrap_socket(self.server.socket, server_side=True)
        self.base_url = f'https://127.0.0.1:{self.server.server_port}'
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    @property
    def signer(self):
        return [sys.executable, str(pathlib.Path(__file__).resolve()), 'sign', str(self.key)]

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()


if __name__ == '__main__':
    if len(sys.argv) != 3 or sys.argv[1] != 'sign':
        raise SystemExit('fixture signer: sign <disposable-test-key-path>')
    message = sys.stdin.buffer.read(1024 * 1024 + 1)
    if len(message) > 1024 * 1024:
        raise SystemExit('fixture signing message exceeds bound')
    sys.stdout.buffer.write(sign(sys.argv[2], message))
