"""Native binary video with bounded latency and a separate non-retrying input channel."""
from collections import deque
import json
import math
import secrets
import socket
import struct
import subprocess
import threading
import time

from dev_bridge import BridgeError
from preview_socket import Socket


class Capture:
    """One codec/window encoder. Slow viewers recover at an IDR, never a partial GOP."""
    def __init__(self, helper, key, window, codec, epoch):
        self.condition = threading.Condition()
        self.packets = deque()
        self.bytes = 0
        self.sequence = 0
        self.config = None
        self.error = None
        self.epoch = epoch
        self.viewers = 0
        self.last_key = 0
        self.closed = False
        self.process = subprocess.Popen([str(helper), str(key[1]), str(window['native_id']),
            str(window['frame']['width']), str(window['frame']['height']), codec],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        threading.Thread(target=self._receive, daemon=True).start()

    @staticmethod
    def read(stream, count):
        result = bytearray()
        while len(result) < count:
            chunk = stream.read(count - len(result))
            if not chunk:
                raise EOFError()
            result.extend(chunk)
        return bytes(result)

    def _receive(self):
        try:
            while True:
                header_size, size = struct.unpack('>II', self.read(self.process.stdout, 8))
                if not 1 <= header_size <= 4096 or not 1 <= size <= 2 * 1024 * 1024:
                    raise ValueError('Capture exceeded its bounded packet size')
                metadata = json.loads(self.read(self.process.stdout, header_size))
                payload = self.read(self.process.stdout, size)
                if metadata.get('kind') not in ('config', 'frame', 'jpeg') or not all(type(metadata.get(k)) is int and 0 < metadata[k] <= 4096 for k in ('width', 'height')):
                    raise ValueError('Invalid capture packet')
                metadata['epoch'] = self.epoch
                header = json.dumps(metadata, separators=(',', ':')).encode()
                packet = struct.pack('>II', len(header), size) + header + payload
                with self.condition:
                    self.sequence += 1
                    if metadata['kind'] == 'config':
                        self.config = packet
                    self.packets.append((self.sequence, time.monotonic(), metadata, packet))
                    self.bytes += len(packet)
                    while len(self.packets) > 120 or self.bytes > 8 * 1024 * 1024:
                        self.bytes -= len(self.packets.popleft()[3])
                    self.condition.notify_all()
        except (EOFError, OSError, ValueError) as error:
            self.process.terminate()
            try:
                self.process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=2)
            detail = self.process.stderr.read(4096).decode(errors='replace').strip()
            with self.condition:
                self.error = detail or str(error) or 'Native capture ended'
                self.condition.notify_all()
        finally:
            self.process.stdout.close()
            self.process.stderr.close()

    def keyframe(self):
        now = time.monotonic()
        with self.condition:
            if now - self.last_key < .25 or self.closed:
                return
            self.last_key = now
            try:
                self.process.stdin.write(b'K')
                self.process.stdin.flush()
            except (OSError, ValueError):
                pass

    def close(self):
        with self.condition:
            if self.closed:
                return
            self.closed = True
            self.process.terminate()
            self.condition.notify_all()
        try:
            self.process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=2)
        self.process.stdin.close()

    def stream(self, handler):
        with self.condition:
            cursor = self.sequence
            self.viewers += 1
        self.keyframe()
        waiting = True
        try:
            while True:
                with self.condition:
                    self.condition.wait_for(lambda: self.closed or self.error or self.sequence > cursor, timeout=1)
                    if self.closed or self.error:
                        return
                    pending = [p for p in self.packets if p[0] > cursor]
                    config = self.config
                if not pending:
                    header = json.dumps({'kind': 'ping'}).encode()
                    handler.wfile.write(struct.pack('>II', len(header), 1) + header + b'\0')
                    continue
                # Never let stale TCP work turn into seconds of visible input latency.
                if pending[0][0] != cursor + 1 or time.monotonic() - pending[0][1] > .15:
                    waiting = True
                cursor = pending[-1][0]
                if waiting:
                    keys = [i for i, p in enumerate(pending) if p[2].get('key') or p[2]['kind'] == 'jpeg']
                    if not keys:
                        self.keyframe()
                        continue
                    pending = pending[keys[-1]:]
                    if config:
                        handler.wfile.write(config)
                    waiting = False
                for _, _, _, packet in pending:
                    handler.wfile.write(packet)
        finally:
            with self.condition:
                self.viewers -= 1


class Preview:
    def __init__(self, inspector, helper):
        self.inspector, self.helper = inspector, helper
        self.lock = threading.RLock()
        self.key = None
        self.epoch = None
        self.captures = {}
        self.references = deque(maxlen=64)
        self.closed = False

    def request(self, runtime, value, command=False):
        result = self.inspector.handle('inspect', {'request': {
            'protocol': 1, 'runtime_id': runtime, 'key': None,
            'message': {'type': 'command' if command else 'query', 'value': value}}})
        if 'Err' in result:
            raise BridgeError(409, 'Preview changed or input unavailable: ' + json.dumps(result['Err'])[:512])
        return result['Ok'].get('value')

    def _stop(self):
        for capture in self.captures.values():
            capture.close()
        self.captures.clear()
        self.key = self.epoch = None
        self.references.clear()

    def close(self):
        with self.lock:
            self.closed = True
            self._stop()

    def state(self):
        development = self.inspector.bridge.development()
        pid = development.get('runtime_pid') if development.get('app_running') else None
        instance = next((i for i in self.inspector.instances() if i['pid'] == pid and i['transport'] == 'local'), None)
        if not instance:
            with self.lock:
                self._stop()
            return {'error': 'Waiting for the native application and its inspection host…'}
        runtime = instance['info']['runtime_id']
        windows = self.request(runtime, {'type': 'preview'})
        if not windows:
            with self.lock:
                self._stop()
            return {'error': 'Restore the inspected native window to start preview capture.'}
        window = windows[0]  # shortcut: one inspected window; multiwindow chooser is future work.
        key = (runtime, pid, window['native_id'], json.dumps([window['frame'], window['content']], sort_keys=True))
        with self.lock:
            if self.closed:
                raise BridgeError(503, 'Preview stopped')
            if self.key != key:
                self._stop()
                self.key, self.epoch = key, secrets.token_urlsafe(18)
            token = secrets.token_urlsafe(18)
            self.references.append((token, runtime, window, time.monotonic()))
            errors = {codec: c.error for codec, c in self.captures.items() if c.error}
            return {'runtime_id': runtime, 'window': window, 'epoch': self.epoch, 'reference': token, 'capture_errors': errors}

    def video(self, handler, body):
        codec = body.get('codec')
        if codec not in ('h264', 'jpeg'):
            raise BridgeError(400, 'Expected h264 or jpeg codec')
        state = self.state()
        if state.get('error'):
            raise BridgeError(409, state['error'])
        with self.lock:
            capture = self.captures.get(codec)
            if not capture or capture.error:
                if capture:
                    capture.close()
                capture = Capture(self.helper, self.key, state['window'], codec, self.epoch)
                self.captures[codec] = capture
            # Reserve the encoder before sending headers; even an early disconnect
            # must release it, and concurrent viewers must not close each other's helper.
            with capture.condition:
                capture.viewers += 1
        try:
            handler.connection.setsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF, 64 * 1024)
            handler.connection.settimeout(.5)
            handler.send_response(200)
            handler.send_header('Content-Type', 'application/octet-stream')
            handler.send_header('Cache-Control', 'no-store')
            handler.send_header('Connection', 'close')
            handler.end_headers()
            handler.close_connection = True
            capture.stream(handler)
        except OSError:
            pass
        finally:
            # No encoder runs with no viewers; reconnect starts at a fresh IDR.
            with self.lock:
                with capture.condition:
                    capture.viewers -= 1
                if not capture.viewers and self.captures.get(codec) is capture:
                    self.captures.pop(codec)
                    capture.close()

    def control(self, handler, token):
        channel = Socket.upgrade(handler, token)
        stopped = threading.Event()
        def metadata():
            try:
                while not stopped.is_set():
                    try:
                        state = self.state()
                    except (BridgeError, OSError) as error:
                        state = {'error': str(error)}
                    channel.send({'type': 'state', 'value': state})
                    stopped.wait(.25)
            except (OSError, ValueError):
                channel.close()
        worker = threading.Thread(target=metadata, daemon=True)
        worker.start()
        try:
            while True:
                body = channel.read()
                if body is None:
                    continue
                identifier = body.get('id')
                if type(identifier) is not int or not 0 <= identifier <= 2**53 - 1:
                    raise ValueError('Expected control request ID')
                try:
                    result = self.handle(body.get('action'), body.get('body', {}))
                    channel.send({'type': 'reply', 'id': identifier, 'value': result})
                except (BridgeError, ValueError, TypeError) as error:
                    channel.send({'type': 'reply', 'id': identifier, 'error': str(error)})
        except (EOFError, OSError, ValueError):
            pass
        finally:
            stopped.set()
            channel.close()
            worker.join(timeout=2)

    def handle(self, action, body):
        if not isinstance(body, dict):
            raise BridgeError(400, 'Expected preview object')
        if action == 'preview_state':
            return self.state()
        if action == 'preview_restart':
            with self.lock:
                self._stop()
            return None
        if action == 'preview_resolve':
            return self.resolve(body)
        if action not in {'preview_input', 'preview_select'}:
            raise BridgeError(404, 'Unknown preview endpoint')
        x, y = body.get('x'), body.get('y')
        if any(type(v) not in (int, float) or not math.isfinite(v) for v in (x, y)):
            raise BridgeError(400, 'Finite content coordinates required')
        with self.lock:
            saved = next((r for r in self.references if r[0] == body.get('reference')), None)
        if not saved or time.monotonic() - saved[3] > 5:
            raise BridgeError(409, 'Preview reference expired')
        _, runtime, window, _ = saved
        if not 0 <= x < window['content']['width'] or not 0 <= y < window['content']['height']:
            raise BridgeError(400, 'Position is outside GPUI content')
        point = {k: window[k] for k in ('window', 'native_id', 'generation')}
        point.update(x=math.floor(x), y=math.floor(y))
        if action == 'preview_select':
            return self.request(runtime, {'type': 'preview_element', 'point': point})
        value = body.get('input')
        if not isinstance(value, dict) or value.get('type') not in {'move', 'click', 'scroll', 'wheel'}:
            raise BridgeError(400, 'Unsupported preview input')
        if value['type'] in ('scroll', 'wheel'):
            limit = 128000 if value['type'] == 'wheel' else 2000
            if any(type(value.get(k)) is not int or abs(value[k]) > limit for k in ('dx', 'dy')):
                raise BridgeError(400, 'Invalid or oversized scroll delta')
            if value['type'] == 'wheel' and value.get('phase') not in ('started', 'moved', 'ended'):
                raise BridgeError(400, 'Invalid scroll phase')
        # Live input follows a viewport, selection follows an exact inspection tree.
        point['geometry'] = [round(window[r][k] * 64) for r, k in
            [('frame', 'width'), ('frame', 'height'), ('content', 'x'), ('content', 'y'), ('content', 'width'), ('content', 'height')]]
        return self.request(runtime, {'type': 'preview_input', 'point': point, 'input': value}, command=True)

    def resolve(self, body):
        """Read current geometry for bounded annotation targets, without native mutation."""
        targets = body.get('targets')
        if not isinstance(targets, list) or not 1 <= len(targets) <= 33:
            raise BridgeError(400, 'Expected up to 33 annotation targets')
        for target in targets:
            if not isinstance(target, dict):
                raise BridgeError(400, 'Invalid annotation target')
            fields = ('stable_id',) if target.get('stable_id') else ('id', 'generation')
            if set(target) != set(fields) or any(not isinstance(target.get(k), str) or not 1 <= len(target[k]) <= 128 for k in fields):
                raise BridgeError(400, 'Invalid annotation identity')
        with self.lock:
            saved = next((r for r in self.references if r[0] == body.get('reference')), None)
        if not saved or time.monotonic() - saved[3] > 5:
            raise BridgeError(409, 'Preview reference expired')
        _, runtime, window, _ = saved
        elements = [None] * len(targets)
        page = self.request(runtime, {'type':'inspect_tree_page', 'generation':None, 'offset':0})
        generation = page['snapshot']['capture']
        # Verify the paged index belongs to this exact native host, not another window.
        windows = self.request(runtime, {'type':'preview'})
        if not any(w['window'] == window['window'] and w['native_id'] == window['native_id'] and w['generation'] == generation for w in windows):
            raise BridgeError(409, 'Annotation frame changed')
        offset = 0
        deadline = time.monotonic() + 1.5
        for _ in range(32):
            if page['snapshot']['capture'] != generation:
                raise BridgeError(409, 'Annotation capture changed')
            boundaries = page['snapshot']['elements']
            stable = {e['stable_id']:e for e in boundaries if e.get('stable_id')}
            exact = {e['id']:e for e in boundaries}
            for index, target in enumerate(targets):
                element = stable.get(target['stable_id']) if 'stable_id' in target else exact.get(target['id']) if generation == target['generation'] else None
                if element:
                    elements[index] = {key:element.get(key) for key in ('id', 'stable_id', 'layout', 'content_mask')}
                    if (element.get('accessibility') or {}).get('hidden'):
                        elements[index]['layout'] = None
            following = page.get('next')
            if all(element is not None for element in elements) or following is None:
                return {'generation':generation, 'elements':elements}
            if time.monotonic() > deadline:
                raise BridgeError(409, 'Annotation lookup timed out')
            if type(following) is not int or not offset < following < 8192:
                raise BridgeError(409, 'Invalid annotation page')
            offset = following
            page = self.request(runtime, {'type':'inspect_tree_page', 'generation':generation, 'offset':offset})
        raise BridgeError(409, 'Annotation tree exceeded its page bound')
