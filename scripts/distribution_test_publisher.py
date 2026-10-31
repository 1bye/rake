"""Disposable HTTPS object/promotion service for publisher protocol tests."""
import hashlib
import http.server
import json
import pathlib
import sys
from distribution_test_feed import TestFeed


class Publisher(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def log_message(self, *_args):
        pass

    def reply(self, status, data=b'', **headers):
        self.send_response(status)
        self.send_header('Content-Length', str(len(data)))
        for key, value in headers.items():
            self.send_header(key, value)
        self.end_headers()
        self.wfile.write(data)

    def authenticated(self):
        if self.headers.get('Authorization') != 'Bearer disposable-test-token':
            self.reply(401)
            return False
        return True

    def do_GET(self):
        if not self.authenticated():
            return
        if self.path == '/channels/stable':
            if not current.exists():
                self.reply(404)
            else:
                data = current.read_bytes()
                self.reply(200, data, ETag='"' + hashlib.sha256(data).hexdigest() + '"')
        elif self.path.startswith('/objects/'):
            path = objects / self.path.removeprefix('/objects/')
            if path.parent != objects or not path.is_file():
                self.reply(404)
            else:
                self.reply(200, b'corrupt readback' if (root / 'corrupt').exists() else path.read_bytes())
        else:
            self.reply(404)

    def do_PUT(self):
        if not self.authenticated():
            return
        length = int(self.headers.get('Content-Length', '0'))
        if not 0 < length <= 64 * 1024 * 1024:
            self.reply(413)
            return
        data = self.rfile.read(length)
        if self.path.startswith('/objects/'):
            path = objects / self.path.removeprefix('/objects/')
            if path.parent != objects:
                self.reply(400)
            elif (root / 'reject-upload').exists():
                self.reply(500)
            elif path.exists():
                self.reply(412)
            elif path.name.split('-', 1)[0] != hashlib.sha256(data).hexdigest():
                self.reply(400)
            else:
                path.write_bytes(data)
                self.reply(201)
        elif self.path == '/channels/stable':
            expected = '"' + hashlib.sha256(current.read_bytes()).hexdigest() + '"' if current.exists() else None
            if (root / 'conflict').exists() or (expected is not None and self.headers.get('If-Match') != expected) or (expected is None and self.headers.get('If-None-Match') != '*'):
                self.reply(412)
                return
            state = json.loads(data)
            for record in state['files'].values():
                path = objects / record['url'].rsplit('/', 1)[1]
                if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != record['sha256']:
                    self.reply(409)
                    return
            pending = root / 'pending.json'
            pending.write_bytes(data)
            pending.replace(current)
            self.reply(200)
        else:
            self.reply(404)


if __name__ == '__main__':
    root = pathlib.Path(sys.argv[1])
    objects = root / 'objects'
    objects.mkdir()
    current = root / 'current.json'
    with TestFeed(root) as feed:
        feed.server.RequestHandlerClass = Publisher
        print(json.dumps({'url': feed.base_url, 'ca': str(feed.ca), 'public_key': feed.public_key.hex(), 'signer': feed.signer}), flush=True)
        sys.stdin.buffer.read()
