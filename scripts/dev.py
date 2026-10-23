#!/usr/bin/env python3
"""Rake project builds and the development supervisor (Python 3.11+, standard library)."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import secrets
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import tomllib
import uuid
import webbrowser

MAX_BYTES = 1024 * 1024
EXCLUDED = {".git", "target", "dist", "node_modules", ".rake", "__pycache__", ".DS_Store"}


def private_json(path, value):
    temporary = path.with_name(f".{path.name}-{secrets.token_hex(6)}")
    try:
        with open(temporary, "x", encoding="utf-8", opener=lambda name, flags: os.open(name, flags, 0o600)) as file:
            json.dump(value, file, ensure_ascii=False, separators=(",", ":"))
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


class Processes:
    """Own every process group, including cargo's compiler children, until shutdown."""
    def __init__(self):
        self.lock = threading.Lock()
        self.children = set()
        self.stopping = False

    def start(self, command, **kwargs):
        with self.lock:
            if self.stopping:
                raise RuntimeError("supervisor is stopping")
            process = subprocess.Popen(command, start_new_session=True, **kwargs)
            self.children.add(process)
            return process

    def output(self, command):
        process = self.start(command, stdout=subprocess.PIPE)
        try:
            output = process.stdout.read(32 * MAX_BYTES + 1)
            if len(output) > 32 * MAX_BYTES:
                raise RuntimeError("tool metadata exceeds 32 MiB")
            if process.wait():
                raise RuntimeError(f"{command[0]} metadata command failed")
            return output.decode()
        finally:
            process.stdout.close()
            self.stop(process)

    def stop(self, process):
        # The group may still contain compiler/loader descendants after its leader exits.
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait(timeout=3)
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        with self.lock:
            self.children.discard(process)

    def close(self):
        with self.lock:
            self.stopping = True
            children = list(self.children)
        for process in children:
            self.stop(process)


def run_output(processes, command, *, cwd=None, env=None, artifact=None):
    process = processes.start(command, cwd=cwd, env=env, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, text=True, errors="replace")
    tail = []
    try:
        with process.stdout:
            while line := process.stdout.readline(MAX_BYTES):
                try:
                    message = json.loads(line)
                except ValueError:
                    message = None
                if artifact and isinstance(message, dict) and message.get("reason") == "compiler-artifact":
                    artifact(message)
                elif isinstance(message, dict) and message.get("reason") == "compiler-message":
                    line = message.get("message", {}).get("rendered", "")
                    if line:
                        print(line, file=sys.stderr, end="", flush=True)
                        tail.append(line[-4000:])
                elif not isinstance(message, dict):
                    print(line, file=sys.stderr, end="", flush=True)
                    tail.append(line[-4000:])
                tail = tail[-8:]
        code = process.wait()
        if code:
            raise RuntimeError(f"{command[0]} exited {code}\n{''.join(tail)[-8000:]}")
    finally:
        processes.stop(process)


class Project:
    def __init__(self, args, processes):
        metadata = json.loads(processes.output(
            ["cargo", "metadata", "--format-version", "1", "--locked", "--manifest-path", str(args.manifest_path)]))
        packages = [item for item in metadata["packages"] if item["id"] in metadata["workspace_members"]]
        if args.package:
            packages = [item for item in packages if item["name"] == args.package]
        else:
            manifest = args.manifest_path.resolve()
            exact = [item for item in packages if Path(item["manifest_path"]) == manifest]
            packages = exact or packages
        if len(packages) != 1:
            raise ValueError("select one application with --package: " + ", ".join(item["name"] for item in packages))
        self.package = packages[0]
        self.root = Path(metadata["workspace_root"])
        self.directory = Path(self.package["manifest_path"]).parent
        self.args = args
        self.watch_roots = sorted({self.root, *(Path(item["manifest_path"]).parent for item in metadata["packages"] if item["source"] is None)})
        targets = [target for target in self.package["targets"] if
                   ("bin" in target["kind"] if args.target == "desktop" else "cdylib" in target["crate_types"])]
        if args.bin:
            targets = [target for target in targets if target["name"] == args.bin]
        if len(targets) != 1:
            raise ValueError("select one binary with --bin, or a single cdylib for Web")
        self.target = targets[0]
        self.web_dir = (args.web_dir or self.directory / "web").resolve()
        if args.target == "web" and self.web_dir.exists() and self.web_dir not in self.watch_roots:
            self.watch_roots.append(self.web_dir)
        self.bindgen = None
        if args.target == "web":
            lock = tomllib.loads((self.root / "Cargo.lock").read_text())
            version = next(item["version"] for item in lock["package"] if item["name"] == "wasm-bindgen")
            self.bindgen = shutil.which("wasm-bindgen")
            if not self.bindgen or subprocess.check_output([self.bindgen, "--version"], text=True).strip() != f"wasm-bindgen {version}":
                raise ValueError(f"install matching loader tooling: cargo install wasm-bindgen-cli --version {version} --locked")

    def fingerprint(self):
        """Hash contents, not timestamps; ignore generated outputs and symlinked trees."""
        digest = hashlib.sha256()
        visited = set()
        for root in self.watch_roots:
            for directory, names, files in os.walk(root, followlinks=False):
                names[:] = sorted(name for name in names if name not in EXCLUDED
                                  and not (Path(directory) / name).is_symlink()
                                  and (not self.args.output or Path(directory) / name != self.args.output))
                for name in sorted(files):
                    path = Path(directory) / name
                    if name in EXCLUDED or path.is_symlink() or path in visited:
                        continue
                    if self.args.output and (path == self.args.output or self.args.output in path.parents):
                        continue
                    visited.add(path)
                    digest.update(os.fsencode(path))
                    try:
                        with path.open("rb") as file:
                            while chunk := file.read(65536):
                                digest.update(chunk)
                    except FileNotFoundError:
                        digest.update(b"deleted-during-scan")
        return digest.digest()

    def build(self, processes, output):
        output.mkdir(parents=True)
        artifacts = []
        command = ["cargo", "build", "--locked", "--message-format=json-render-diagnostics",
                   "--manifest-path", self.package["manifest_path"], "--package", self.package["name"]]
        if self.args.features:
            command += ["--features", self.args.features]
        if self.args.release:
            command += ["--release"]
        command += ["--bin", self.target["name"]] if self.args.target == "desktop" else ["--lib", "--target", "wasm32-unknown-unknown"]
        run_output(processes, command, cwd=self.root, artifact=lambda item: artifacts.append(item)
                   if item["package_id"] == self.package["id"] and item["target"]["name"] == self.target["name"] else None)
        if not artifacts:
            raise RuntimeError("cargo did not report an application artifact")
        if self.args.target == "desktop":
            source = Path(artifacts[-1]["executable"])
            if sys.platform == "darwin":
                contents = output / f"{self.package['name']}.app" / "Contents"
                binary = contents / "MacOS" / self.target["name"]
                binary.parent.mkdir(parents=True)
                import plistlib
                with (contents / "Info.plist").open("wb") as file:
                    plistlib.dump({"CFBundleExecutable": binary.name, "CFBundleIdentifier": "dev.rake.application",
                                  "CFBundleName": self.package["name"], "CFBundlePackageType": "APPL"}, file)
            else:
                binary = output / self.target["name"]
            shutil.copy2(source, binary)
            return binary
        source = next((Path(name) for artifact in reversed(artifacts) for name in artifact["filenames"] if name.endswith(".wasm")), None)
        if source is None:
            raise RuntimeError("cargo did not report a Wasm artifact")
        if self.web_dir.is_dir():
            shutil.copytree(self.web_dir, output, dirs_exist_ok=True)
        run_output(processes, [self.bindgen, "--target", "web", "--out-dir", str(output / "pkg"), str(source)])
        if not (output / "index.html").is_file():
            (output / "index.html").write_text("<!doctype html><html><head><meta charset='utf-8'><title>Rake</title>"
                "<style>html,body,canvas{margin:0;width:100%;height:100%;display:block}</style></head><body>"
                f"<script type='module'>import init from '/pkg/{self.target['name']}.js';await init();</script></body></html>")
        return output


def exchange(instance, kind, value, timeout=2, key=None):
    request = {"protocol": 1, "runtime_id": instance["info"]["runtime_id"], "key": (key or str(uuid.uuid4())) if kind == "command" else None,
               "message": {"type": kind, "value": value}}
    encoded = json.dumps(request).encode() + b"\n"
    if len(encoded) > MAX_BYTES:
        raise ValueError("request exceeds the control frame limit")
    # A lost response retries the exact payload and key, never a second mutation.
    for attempt in range(2):
        try:
            with socket.socket(socket.AF_UNIX) as connection:
                connection.settimeout(timeout)
                connection.connect(instance["endpoint"]["path"])
                connection.sendall(encoded)
                response = bytearray()
                while b"\n" not in response:
                    chunk = connection.recv(min(65536, MAX_BYTES + 1 - len(response)))
                    if not chunk:
                        raise ConnectionError("runtime disconnected during reload")
                    response.extend(chunk)
                    if len(response) > MAX_BYTES:
                        raise ValueError("runtime response exceeds the control frame limit")
            break
        except OSError:
            if attempt:
                raise
    result = json.loads(response)
    if "Err" in result:
        raise RuntimeError(json.dumps(result["Err"]))
    return result["Ok"]["value"]


class Supervisor:
    def __init__(self, project, session, processes):
        self.project, self.session, self.processes = project, session, processes
        self.control = session / "control"
        self.control.mkdir(mode=0o700)
        self.builds = session / "builds"
        self.builds.mkdir()
        self.status_path = session / "status.json"
        self.status = {"protocol": 1, "pid": os.getpid(), "target": project.args.target, "package": project.package["name"],
                       "phase": "starting", "published": 0, "build": 0, "error": None,
                       "mechanism": "web_page_reload" if project.args.target == "web" else
                           ("native_code_replacement" if project.args.native_reloader else "native_restart")}
        self.app = None
        self.server = None
        self.checkpoint = session / "checkpoint.json"
        self.record("starting")

    def record(self, phase, **fields):
        self.status.update(phase=phase, **fields)
        private_json(self.status_path, self.status)
        print(json.dumps({"event": phase, **fields}) if self.project.args.json else f"rake: {phase}" + (f" — {fields['error']}" if fields.get("error") else ""), flush=True)

    def start_server(self):
        waiting = self.session / "waiting"
        waiting.mkdir()
        (waiting / "index.html").write_text("<!doctype html><html><head><title>Rake development</title></head><body>Waiting for the first successful build…</body></html>")
        env = {**os.environ, "RAKE_CONTROL_DIR": str(self.control)}
        self.server = self.processes.start([sys.executable, str(Path(__file__).with_name("serve-web.py")),
            "--control", "--port", str(self.project.args.port), "--directory", str(waiting),
            "--development-state", str(self.status_path), "--build-root", str(self.builds)],
            env=env, stdout=subprocess.PIPE, text=True)
        # Server writes its private launch URL after binding, before serving requests.
        line = self.server.stdout.readline()
        if not line.startswith("Serving "):
            raise RuntimeError("development server could not start")
        url = line.split(" at ", 1)[1].strip()
        print(json.dumps({"event": "web_listening", "url": url, "session": str(self.session)}) if self.project.args.json else
              f"{line.strip()}\nSession: {self.session}", flush=True)
        self.server.stdout.close()
        if not self.project.args.no_open:
            webbrowser.open(url)

    def instance(self):
        if not self.app:
            return None
        for path in self.control.glob("*.json"):
            try:
                value = json.loads(path.read_bytes()[:MAX_BYTES])
                if value["pid"] == self.app.pid:
                    return value
            except (OSError, ValueError, KeyError):
                continue
        return None

    def launch(self, artifact):
        env = {**os.environ, "RAKE_DEV_CONTROL": "1", "RAKE_CONTROL_DIR": str(self.control)}
        if self.checkpoint.exists():
            env["RAKE_RELOAD_CHECKPOINT"] = str(self.checkpoint)
        self.app = self.processes.start([str(artifact)], cwd=self.project.root, env=env, stdout=sys.stderr)
        self.record("running", runtime_pid=self.app.pid, app_running=True, mechanism="native_restart", error=None)

    def publish(self, number, artifact):
        if self.project.args.target == "web":
            self.record("watching", published=number, directory=str(artifact), error=None)
            return
        if self.app and self.app.poll() is None:
            instance = self.instance()
            if not instance:
                if not self.project.args.restart_without_state:
                    raise RuntimeError("live app has no development control endpoint; enable ControlConfig::FromEnvironment, or explicitly use --restart-without-state")
                self.record("restarting_without_state")
            else:
                status = exchange(instance, "query", {"type": "reload"})
                if status["phase"] != "restore_failed":
                    operations = exchange(instance, "query", {"type": "operations"})
                    if any(state in ("Queued", "Running") if isinstance(state, str) else "AwaitingDecision" in state for _, state in operations):
                        raise RuntimeError('"Busy": waiting for pending operations and decisions')
                    boundary = str(uuid.uuid4())
                    prepared = False
                    try:
                        mechanism = "native_code_replacement" if self.project.args.native_reloader else "native_restart"
                        self.record("preparing_reload", mechanism=mechanism)
                        deadline = time.monotonic() + 15
                        prepared = True
                        exchange(instance, "command", {"type": "prepare_reload", "request": {"id": boundary, "mechanism": mechanism}}, key=boundary + "/prepare")
                        while exchange(instance, "query", {"type": "reload"})["phase"] != "ready":
                            if time.monotonic() >= deadline:
                                raise RuntimeError("reload quiescence timed out")
                            time.sleep(.05)
                        checkpoint = exchange(instance, "query", {"type": "checkpoint", "application_id": instance["application_id"]})
                        private_json(self.checkpoint, checkpoint)
                        if self.project.args.native_reloader:
                            self.record("replacing_native_code")
                            env = {**os.environ, "RAKE_RELOAD_ARTIFACT": str(artifact), "RAKE_RELOAD_CHECKPOINT": str(self.checkpoint),
                                   "RAKE_RELOAD_RUNTIME": instance["info"]["runtime_id"], "RAKE_RELOAD_GENERATION": str(checkpoint["generation"] + 1),
                                   "RAKE_RELOAD_PROJECT": str(self.project.root)}
                            run_output(self.processes, [str(self.project.args.native_reloader)], cwd=self.project.root, env=env)
                            exchange(instance, "command", {"type": "finish_reload", "id": boundary, "applied": True}, key=boundary + "/apply")
                            self.checkpoint.unlink(missing_ok=True)
                            self.record("watching", published=number, error=None)
                            return
                    except BaseException:
                        if prepared:
                            try:
                                exchange(instance, "command", {"type": "finish_reload", "id": boundary, "applied": False}, key=boundary + "/abort")
                            except (OSError, RuntimeError):
                                pass
                        raise
            old_pid = self.app.pid
            self.processes.stop(self.app)
            for path in self.control.glob("*.json"):
                try:
                    advertisement = json.loads(path.read_bytes()[:MAX_BYTES])
                    if advertisement["pid"] == old_pid:
                        path.unlink(missing_ok=True)
                        # This session directory is private; only remove its owned socket names.
                        self.control.joinpath(path.stem + ".sock").unlink(missing_ok=True)
                except (OSError, ValueError, KeyError):
                    pass
        # A crashed app cannot provide a new checkpoint. Retain the latest captured one.
        self.launch(artifact)
        self.record("watching", published=number, error=None)

    def run(self):
        if self.project.args.target == "web":
            self.start_server()
        else:
            print(json.dumps({"event": "session", "session": str(self.session), "mechanism": self.status["mechanism"]}) if self.project.args.json else f"Session: {self.session}; reload mechanism: {self.status['mechanism']}", flush=True)
        results = queue.Queue(maxsize=1)
        worker = None
        fingerprint = None
        changed_at = time.monotonic()
        built_fingerprint = None
        number = 0
        pending = None
        retry_after = 0
        while True:
            if self.server and self.server.poll() is not None:
                raise RuntimeError("development Web server exited")
            current = self.project.fingerprint()
            if current != fingerprint:
                fingerprint, changed_at = current, time.monotonic()
                pending = None
            if worker and not results.empty():
                artifact, error, started = results.get_nowait()
                worker.join()
                worker = None
                if current != started:
                    shutil.rmtree(self.builds / str(number), ignore_errors=True)
                    self.record("build_superseded")
                elif error:
                    self.record("build_failed", error=error)
                    shutil.rmtree(self.builds / str(number), ignore_errors=True)
                else:
                    try:
                        self.publish(number, artifact)
                    except (RuntimeError, OSError, ValueError) as failure:
                        if '"Busy"' in str(failure):
                            pending, retry_after = (number, artifact), time.monotonic() + 1
                            self.record("reload_waiting", error=str(failure))
                        else:
                            self.record("reload_blocked", error=str(failure))
            if pending and not worker and time.monotonic() >= retry_after:
                try:
                    self.publish(*pending)
                    pending = None
                except (RuntimeError, OSError, ValueError) as failure:
                    retry_after = time.monotonic() + 1
                    if '"Busy"' not in str(failure):
                        pending = None
                        self.record("reload_blocked", error=str(failure))
            if not worker and fingerprint != built_fingerprint and time.monotonic() - changed_at >= .25:
                number += 1
                built_fingerprint = fingerprint
                self.record("building", build=number, error=None)
                def build(started=fingerprint, output=self.builds / str(number)):
                    try:
                        updated = Project(self.project.args, self.processes)
                        self.project = updated
                        results.put((updated.build(self.processes, output), None, started))
                    except Exception as error:
                        results.put((None, str(error), started))
                worker = threading.Thread(target=build, daemon=True)
                worker.start()
            if self.app and self.app.poll() is not None and self.status.get("app_running"):
                self.record("app_exited", app_running=False, exit_code=self.app.returncode)
            # Surface replacement compatibility outcomes while keeping the supervisor alive.
            if self.app and self.app.poll() is None and self.status["phase"] in {"watching", "running", "restore_failed"}:
                instance = self.instance()
                if instance:
                    try:
                        status = exchange(instance, "query", {"type": "reload"}, timeout=.5)
                        if status["phase"] == "restore_failed":
                            self.record("restore_failed", error=status["error"])
                        elif self.status.get("runtime_id") != instance["info"]["runtime_id"] or self.status["phase"] == "restore_failed":
                            self.record("watching", runtime_id=instance["info"]["runtime_id"], reload=status, error=None)
                    except (OSError, RuntimeError):
                        pass
            time.sleep(.1)


def doctor(args):
    checks = {}
    for name, command in {"rust": ["rustc", "--version"], "cargo": ["cargo", "--version"],
                          "wasm_targets": ["rustup", "target", "list", "--installed"],
                          "wasm_bindgen": ["wasm-bindgen", "--version"]}.items():
        try:
            checks[name] = subprocess.check_output(command, text=True, stderr=subprocess.STDOUT).strip()
        except (OSError, subprocess.CalledProcessError) as error:
            checks[name] = {"error": str(error)}
    checks["python"] = sys.version.split()[0]
    print(json.dumps(checks, indent=None if args.json else 2))
    ready = not isinstance(checks["rust"], dict) and not isinstance(checks["cargo"], dict)
    if args.target == "web":
        ready = ready and not isinstance(checks["wasm_bindgen"], dict) and "wasm32-unknown-unknown" in str(checks["wasm_targets"])
    return 0 if ready else 1


class Parser(argparse.ArgumentParser):
    def error(self, message):
        if "--json" in sys.argv:
            print(json.dumps({"error": {"message": message}}), file=sys.stderr)
            raise SystemExit(2)
        super().error(message)


def main():
    parser = Parser(description=__doc__)
    parser.add_argument("command", choices=["dev", "build", "doctor"])
    parser.add_argument("--target", choices=["desktop", "web"], default="desktop")
    parser.add_argument("--manifest-path", type=Path, default=Path("Cargo.toml"))
    parser.add_argument("--package", "-p")
    parser.add_argument("--bin")
    parser.add_argument("--features")
    parser.add_argument("--release", action="store_true")
    parser.add_argument("--web-dir", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--native-reloader", type=Path, help="External native backend; owns ABI/code retirement and exits zero only after successful replacement")
    parser.add_argument("--restart-without-state", action="store_true", help="Explicitly allow restarting apps without a control endpoint")
    parser.add_argument("--port", type=int, default=8081)
    parser.add_argument("--no-open", action="store_true")
    parser.add_argument("--json", action="store_true")
    parser.add_argument("status", nargs="?", choices=["status"])
    parser.add_argument("--session", type=Path)
    args = parser.parse_args()
    if args.status:
        if args.command != "dev" or not args.session:
            parser.error("use rake dev status --session PATH")
        status = json.loads((args.session / "status.json").read_bytes()[:MAX_BYTES])
        print(json.dumps(status, indent=None if args.json else 2))
        return 0
    if args.command == "doctor":
        return doctor(args)
    if not 0 <= args.port <= 65535:
        parser.error("port must be 0–65535")
    if args.native_reloader and args.target != "desktop":
        parser.error("--native-reloader applies to desktop only")
    if args.native_reloader:
        args.native_reloader = args.native_reloader.resolve(strict=True)
    if args.output:
        args.output = args.output.resolve()
    processes = Processes()
    supervisor = None
    signal.signal(signal.SIGTERM, lambda *_: (_ for _ in ()).throw(KeyboardInterrupt()))
    try:
        project = Project(args, processes)

        if args.command == "build":
            output = args.output or project.root / "dist" / project.package["name"] / args.target
            marker = {"package": project.package["name"], "target": args.target}
            if output.exists() and (not (output / ".rake-build.json").is_file() or json.loads((output / ".rake-build.json").read_text()) != marker):
                raise ValueError(f"output contains files not owned by this build: {output}; choose --output")
            output.parent.mkdir(parents=True, exist_ok=True)
            staging = Path(tempfile.mkdtemp(prefix=".rake-build-", dir=output.parent))
            backup = output.with_name(f".{output.name}-{uuid.uuid4()}.old")
            try:
                # Project.build owns creation of its destination.
                staging.rmdir()
                staged_artifact = project.build(processes, staging)
                private_json(staging / ".rake-build.json", marker)
                artifact = output / staged_artifact.relative_to(staging)
                if output.exists():
                    output.rename(backup)
                try:
                    staging.rename(output)
                except BaseException:
                    if backup.exists():
                        backup.rename(output)
                    raise
                shutil.rmtree(backup, ignore_errors=True)
            finally:
                shutil.rmtree(staging, ignore_errors=True)
            print(json.dumps({"artifact": str(artifact)}) if args.json else f"Built {artifact}")
        else:
            session = Path(tempfile.mkdtemp(prefix="rake-dev-", dir="/tmp"))
            supervisor = Supervisor(project, session, processes)
            supervisor.run()
    except KeyboardInterrupt:
        return 0
    finally:
        processes.close()
        if supervisor:
            supervisor.record("stopped", app_running=False)
            # Keep status/checkpoints for inspection and recovery, remove stale advertisements.
            for path in supervisor.control.glob("*"):
                path.unlink(missing_ok=True)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(json.dumps({"error": {"message": str(error)}}) if "--json" in sys.argv else f"rake: {error}", file=sys.stderr)
        raise SystemExit(1)
    finally:
        if os.environ.get("RAKE_EMBEDDED_HELPERS") == str(Path(__file__).parent):
            shutil.rmtree(Path(__file__).parent, ignore_errors=True)
