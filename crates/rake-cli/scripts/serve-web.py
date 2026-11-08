#!/usr/bin/env python3
"""Serve a built example on loopback, with app-shell rewrites for path routing."""

import argparse
import os
import io
import json
import secrets
import re
import signal
import threading
import urllib.parse
import urllib.error

from dev_bridge import BridgeError, DevBridge, MAX_BYTES
from runtime_broker import NoRedirect, RuntimeBroker
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from socketserver import TCPServer
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--example", choices=["foundation", "routing", "routing-hash", "notes", "view", "files"], default="routing")
parser.add_argument("--port", type=int, default=8081)
parser.add_argument("--control", action="store_true", help="Enable authenticated browser-to-CLI development control")
parser.add_argument("--directory", type=Path, help="Serve another built application directory")
parser.add_argument("--development-state", type=Path, help="Private supervisor status, enabled only with control")
parser.add_argument("--build-root", type=Path, help="Immutable published Web build directories")
parser.add_argument("--control-only", "--inspector-only", dest="control_only", action="store_true", help="Serve native development control without application assets")
parser.add_argument("--process-output", type=Path, help="Private supervisor output snapshot")
parser.add_argument("--cdp-url", help="Private loopback CDP discovery endpoint")
parser.add_argument("--preview-helper", type=Path, help="Compiled macOS capture helper, used only on the native preview origin")
args = parser.parse_args()
if args.preview_helper and not (args.control_only and args.control and args.development_state):
    parser.error("--preview-helper requires native inspection and supervisor state")
if bool(args.development_state) != bool(args.build_root) or (args.development_state and not args.control):
    parser.error("development state/build root require each other and --control")
if args.cdp_url and not args.control:
    parser.error("--cdp-url requires --control")
if args.control_only and not args.control:
    parser.error("--inspector-only requires --control")
# During phase 4 only preview transport stays here; the native origin owns all broker state.
class NativeBroker:
    def __init__(self, url, token):
        self.url, self.token, self.bridge = url, token, self

    def handle(self, action, body):
        request = urllib.request.Request(self.url + "/__rake/" + action, json.dumps(body).encode(),
            {"Content-Type": "application/json", "X-Rake-Token": self.token})
        try:
            with urllib.request.build_opener(NoRedirect).open(request, timeout=12) as response:
                data = response.read(MAX_BYTES + 1)
            if len(data) > MAX_BYTES:
                raise BridgeError(413, "development response too large")
            return json.loads(data)
        except urllib.error.HTTPError as error:
            raise BridgeError(error.code, str(error)) from error

    def development(self):
        return self.handle("development", {})

    def instances(self):
        return self.handle("inspect_instances", {})

    def close(self):
        pass


if os.environ.get("RAKE_NATIVE_BROKER_URL"):
    bridge = inspector = NativeBroker(os.environ["RAKE_NATIVE_BROKER_URL"], os.environ["RAKE_NATIVE_BROKER_TOKEN"])
else:
    bridge = DevBridge(f"http://127.0.0.1:{args.port}", development_state=args.development_state) if args.control else None
    inspector = RuntimeBroker(bridge, args.process_output, allow_native=args.control_only, cdp_url=args.cdp_url) if bridge else None
preview = None
if args.preview_helper:
    from preview import Preview
    preview = Preview(inspector, args.preview_helper)
directory = args.directory
if directory is None and not args.control_only:
    directory = Path(__file__).resolve().parents[3] / "dist" / args.example

if not args.control_only and not (directory / "index.html").is_file():
    build_args = "routing --hash" if args.example == "routing-hash" else args.example
    raise SystemExit(f"Build the example first: python3 scripts/build-web.py --example {build_args}")


BOOTSTRAP_JS = rb"""(() => {
  const parameters = new URL(document.currentScript.src).searchParams;
  const build = Number(parameters.get('build') || 0);
  const key = 'rake-dev-control-token';
  const prefix = '#__rake_control=';
  function authenticate() {
    if (!location.hash.startsWith(prefix)) return false;
    const token = location.hash.slice(prefix.length);
    if (/^[A-Za-z0-9_-]{32,128}$/.test(token)) sessionStorage.setItem(key, token);
    history.replaceState(history.state, '', location.pathname + location.search);
    return true;
  }
  authenticate();
  const authenticateNavigation = event => {
    if (authenticate()) {
      event.stopImmediatePropagation();
      location.reload();
    }
  };
  window.addEventListener('hashchange', authenticateNavigation, {capture: true});
  window.addEventListener('popstate', authenticateNavigation, {capture: true});
  const token = sessionStorage.getItem(key);
  if (token) {
    const meta = document.createElement('meta');
    meta.name = 'rake-dev-bridge';
    meta.content = JSON.stringify({url: location.origin, token, build_id: build});
    document.head.append(meta);
    let seenRuntime = false;
    async function development() {
      try {
        const response = await fetch('/__rake/development', {method: 'POST', headers: {'Content-Type': 'application/json', 'X-Rake-Token': token}, body: '{}'});
        if (!response.ok) return;
        const state = await response.json();
        const runtime = document.documentElement.getAttribute('data-rake-runtime');
        seenRuntime ||= !!runtime;
        if (state.published > build && !seenRuntime) location.reload();
        const failure = runtime && state.runtimes[runtime] && state.runtimes[runtime].error;
        let status = document.getElementById('__rake_development');
        if (failure || state.error) {
          if (!status && document.body) {
            status = document.createElement('div'); status.id = '__rake_development'; status.setAttribute('role', 'alert');
            status.style.cssText = 'position:fixed;bottom:8px;left:8px;max-width:70vw;max-height:35vh;overflow:auto;white-space:pre-wrap;background:#18232b;color:#f0f4e8;padding:8px;font:12px monospace;z-index:2147483647';
            document.body.append(status);
          }
          if (status) status.textContent = 'Rake: ' + (failure || state.error);
        } else if (status) status.remove();
        if (state.devtools_url && window.parent === window && !document.getElementById('__rake_devtools')) {
          const button = document.createElement('a'); button.id = '__rake_devtools'; button.href = state.devtools_url + '#devtools';
          button.title = 'Open Chrome DevTools'; button.setAttribute('aria-label', button.title); button.textContent = '\u2039\u203a';
          button.style.cssText = 'position:fixed;bottom:12px;right:12px;width:28px;height:28px;border-radius:50%;background:#242424;color:#ddd;display:grid;place-items:center;text-decoration:none;font:14px monospace;z-index:2147483647';
          document.body.append(button);
        }
      } catch (_) { /* The running application remains usable while disconnected. */ }
    }
    // Recursive scheduling bounds concurrent requests if the server is slow.
    async function poll() { await development(); setTimeout(poll, 1000); }
    if (parameters.has('build')) setTimeout(poll, 500);
  }
})();"""

class AppHandler(SimpleHTTPRequestHandler):
    def log_message(self, format, *values):
        # Requests carry credentials in headers only; bound routine dev-server noise.
        if values and str(values[1] if len(values) > 1 else "").startswith(("4", "5")):
            super().log_message(format, *values)

    def translate_path(self, path):
        decoded = urllib.parse.unquote(urllib.parse.urlsplit(path).path)
        prefix = "/__rake/build/"
        if args.build_root and decoded.startswith(prefix):
            parts = decoded.removeprefix(prefix).split("/")
            if not parts[0].isdigit() or any(part in {"..", "."} for part in parts):
                return str(directory / "__invalid_build_path__")
            candidate = args.build_root.joinpath(*parts)
            if not candidate.resolve().is_relative_to(args.build_root.resolve()):
                return str(directory / "__invalid_build_path__")
            return str(candidate)
        active = bridge.development().get("directory") if args.development_state else None
        # Absolute imports in custom loaders also stay on the referring immutable build.
        referer = urllib.parse.urlsplit(self.headers.get("Referer", ""))
        referred_path = urllib.parse.unquote(referer.path)
        if args.build_root and referer.netloc == self.headers.get("Host") and referred_path.startswith(prefix):
            build = referred_path.removeprefix(prefix).split("/", 1)[0]
            if build.isdigit():
                active = str(args.build_root / build)
        original = self.directory
        try:
            self.directory = active or str(directory)
            return super().translate_path(path)
        finally:
            self.directory = original

    def setup(self):
        super().setup()
        self.connection.settimeout(15)

    def _valid_host(self):
        return self.headers.get("Host") in {f"127.0.0.1:{args.port}", f"localhost:{args.port}"}

    def do_POST(self):
        origin = self.headers.get("Origin")
        expected_origin = f"http://{self.headers.get('Host')}"
        token = self.headers.get("X-Rake-Token", "")
        if not bridge or not self._valid_host() or (origin and origin != expected_origin) or not secrets.compare_digest(token.encode(), bridge.token.encode()):
            self.send_error(403, "development bridge authentication failed")
            return
        try:
            length = int(self.headers.get("Content-Length", "-1"))
            if not 0 <= length <= MAX_BYTES - 1024 or self.headers.get("Content-Type") != "application/json":
                raise BridgeError(413, "invalid or oversized JSON frame")
            body = json.loads(self.rfile.read(length))
            if not isinstance(body, dict):
                raise BridgeError(400, "expected JSON object")
            if not self.path.startswith("/__rake/"):
                raise BridgeError(404, "unknown development endpoint")
            action = self.path.removeprefix("/__rake/")
            if action == "preview_video" and preview:
                preview.video(self, body)
                return
            if action.startswith("preview") and preview:
                result = preview.handle(action, body)
            else:
                result = inspector.handle(action, body) if action.startswith("inspect") else bridge.handle(action, body)
            encoded = json.dumps(result).encode()
            if len(encoded) > MAX_BYTES:
                raise BridgeError(413, "development response too large")
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Cache-Control", "no-store")
            self.send_header("Content-Length", str(len(encoded)))
            self.end_headers()
            self.wfile.write(encoded)
        except (ValueError, TypeError) as error:
            self.send_error(400, str(error))
        except (OSError, urllib.error.URLError) as error:
            self.send_error(502, str(error))
        except BridgeError as error:
            self.send_error(error.status, str(error))

    def do_GET(self):
        if preview and self.path == "/__rake/preview_control":
            if not self._valid_host():
                self.send_error(403, "invalid development host")
                return
            try:
                preview.control(self, bridge.token)
            except BridgeError as error:
                self.send_error(error.status, str(error))
            return
        super().do_GET()

    def send_head(self):
        if bridge and not self._valid_host():
            self.send_error(403, "invalid development host")
            return None
        route = self.path.split("?", 1)[0]
        if preview and route in {"/__rake/preview", "/__rake/preview.js", "/__rake/preview-stream.js", "/__rake/preview-annotations.js", "/__rake/preview.css"}:
            filename = {"/__rake/preview": "preview.html", "/__rake/preview.js": "preview.js", "/__rake/preview-stream.js": "preview-stream.js", "/__rake/preview-annotations.js": "preview-annotations.js", "/__rake/preview.css": "preview.css"}[route]
            encoded = Path(__file__).with_name(filename).read_bytes()
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8" if filename.endswith(".html") else "text/css; charset=utf-8" if filename.endswith(".css") else "text/javascript; charset=utf-8")
            self.send_header("Cache-Control", "no-store")
            self.send_header("X-Content-Type-Options", "nosniff")
            self.send_header("Content-Security-Policy", "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src data:; frame-ancestors 'none'; base-uri 'none'")
            self.send_header("Content-Length", str(len(encoded)))
            self.end_headers()
            return io.BytesIO(encoded)
        if route.startswith(("/__rake/inspector", "/__rake/elements")):
            self.send_error(410, "Rake Inspector is deprecated; use Chrome DevTools (--cdp)")
            return None
        if args.control_only:
            self.send_error(404, "native control serves no application assets")
            return None
        if bridge and self.path.split("?", 1)[0] == "/__rake/bootstrap.js":
            self.send_response(200)
            self.send_header("Content-Type", "text/javascript; charset=utf-8")
            self.send_header("Cache-Control", "no-store")
            self.send_header("Content-Length", str(len(BOOTSTRAP_JS)))
            self.end_headers()
            return io.BytesIO(BOOTSTRAP_JS)

        if "text/html" in self.headers.get("Accept", "") and not self.path.startswith("/__rake/") and not Path(self.translate_path(self.path)).is_file():
            self.path = "/index.html"

        if bridge and self.path.split("?", 1)[0] in {"/", "/index.html"}:
            development = bridge.development()
            active = Path(development.get("directory", directory))
            build = development.get("published", 0)
            document = (active / "index.html").read_text()
            if build:
                document = re.sub(r"(['\"])(?:/|\./)?pkg/", lambda match: match[1] + f"/__rake/build/{build}/pkg/", document)
            base = f'<base href="/__rake/build/{build}/">' if build else ""
            if build:
                document = re.sub(r'(src|href)=([\"\'])/(?!__rake/)', lambda match: match[1] + "=" + match[2] + f"/__rake/build/{build}/", document)
            loader = base + f'<script src="/__rake/bootstrap.js{f"?build={build}" if args.development_state else ""}"></script>'
            head = re.search(r"<head\b[^>]*>", document, re.IGNORECASE)
            container = head or re.search(r"<html\b[^>]*>", document, re.IGNORECASE)
            if container:
                document = document[:container.end()] + loader + document[container.end():]
            else:
                raise ValueError("development HTML requires an html or head element")
            encoded = document.encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Cache-Control", "no-store")
            self.send_header("Content-Length", str(len(encoded)))
            self.end_headers()
            return io.BytesIO(encoded)

        return super().send_head()


class BoundedServer(ThreadingHTTPServer):
    slots = threading.BoundedSemaphore(64)

    def server_bind(self):
        # The listener is numeric loopback; startup must not depend on reverse DNS.
        TCPServer.server_bind(self)
        self.server_name, self.server_port = self.server_address[:2]

    def process_request(self, request, address):
        if not self.slots.acquire(blocking=False):
            request.close()
            return
        try:
            super().process_request(request, address)
        except BaseException:
            self.slots.release()
            raise

    def process_request_thread(self, request, address):
        try:
            super().process_request_thread(request, address)
        finally:
            self.slots.release()


server = BoundedServer(("127.0.0.1", args.port), partial(AppHandler, directory=str(directory)))
args.port = server.server_port
if bridge:
    if not isinstance(bridge, NativeBroker):
        bridge.url = os.environ.get("RAKE_PUBLIC_BRIDGE_URL") or f"http://127.0.0.1:{args.port}"
launch_url = f"http://127.0.0.1:{args.port}"
if bridge:
    launch_url += f"{'/__rake/preview' if preview else '/'}#__rake_control={bridge.token}"
print(f"Serving {args.example} at {launch_url}", flush=True)

stopping = False


def stop(*_args):
    global stopping
    # Do not raise while HTTPServer is starting a handler or holding an internal lock.
    stopping = True


signal.signal(signal.SIGINT, stop)
signal.signal(signal.SIGTERM, stop)
server.timeout = .25

try:
    while not stopping:
        server.handle_request()
finally:
    server.server_close()
    if preview:
        preview.close()
    if bridge:
        bridge.close()
