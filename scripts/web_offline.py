"""Production Web identities and opt-in offline payloads. No deployment authority."""

import hashlib
import html
import json
from pathlib import Path
import re
import shutil
from urllib.parse import quote


RESERVED = ("rake-version.json", "rake-service-worker.js", "rake-offline.js", "rake-builds")


def prepare(site, metadata, offline):
    for name in RESERVED:
        if (site / name).exists():
            raise ValueError(f"{name} is reserved for generated deployment assets")
    identity = {key: metadata[key] for key in ("protocol", "application_id", "version", "base_path")}
    digest = hashlib.sha256(json.dumps(identity, sort_keys=True).encode())
    digest.update(Path(__file__).read_bytes())
    template = Path(__file__).with_name("web-service-worker.js").read_text()
    if offline:
        digest.update(template.encode())
    for path in sorted(site.rglob("*")):
        if path.is_file():
            digest.update(path.relative_to(site).as_posix().encode() + b"\0")
            digest.update(hashlib.sha256(path.read_bytes()).digest())
    identity["build"] = digest.hexdigest()
    metadata["build"] = identity["build"]
    metadata["offline"] = offline
    base = metadata["base_path"]
    index = site / "index.html"
    document = index.read_text()
    version_meta = '<meta name="rake-deployment" content="' + html.escape(json.dumps(identity), quote=True) + '">'
    head = re.search(r"<head\b[^>]*>", document, re.IGNORECASE) or re.search(r"<html\b[^>]*>", document, re.IGNORECASE)
    document = document[:head.end()] + version_meta + document[head.end():]
    index.write_text(document)
    if offline:
        entries = list(site.iterdir())
        relative = "rake-builds/" + identity["build"] + "/"
        assets_base = base + relative
        metadata["assets"] = assets_base + "assets/"
        metadata["cache"] = "immutable build assets; network-first navigation; network-only version checks"
        for path in sorted([*site.rglob("*.html"), *site.rglob("*.js"), *site.rglob("*.css")]):
            if (site / "pkg") in path.parents:
                continue
            text = path.read_text()
            text = re.sub(r"(['\"])" + re.escape(base) + r"(pkg|assets)/", lambda match: match[1] + assets_base + match[2] + "/", text)
            text = text.replace('<base href="' + html.escape(base, quote=True) + '">', '<base href="' + html.escape(assets_base, quote=True) + '">')
            path.write_text(text)
        destination = site / relative
        destination.mkdir(parents=True)
        for entry in entries:
            shutil.move(str(entry), destination / entry.name)
        document = (destination / "index.html").read_text()
        registration = '<script src="' + assets_base + 'rake-offline.js" defer></script>'
        head = re.search(r"<head\b[^>]*>", document, re.IGNORECASE) or re.search(r"<html\b[^>]*>", document, re.IGNORECASE)
        index.write_text(document[:head.end()] + registration + document[head.end():])
        (destination / "index.html").write_text(index.read_text())
        (destination / "rake-offline.js").write_text(
            "(() => { const root = " + json.dumps(base) + ";\n"
            "if (!('serviceWorker' in navigator)) { document.documentElement.dataset.rakeOffline = 'unavailable'; return; }\n"
            "navigator.serviceWorker.register(root + 'rake-service-worker.js', {scope: root, updateViaCache: 'none'})\n"
            ".then(() => navigator.serviceWorker.ready).then(() => { document.documentElement.dataset.rakeOffline = 'ready'; })\n"
            ".catch(error => { document.documentElement.dataset.rakeOffline = 'failed'; console.warn('Offline setup failed', error); }); })();\n"
        )
        assets = [base + quote(path.relative_to(site).as_posix(), safe="/") for path in sorted(destination.rglob("*")) if path.is_file()]
        prefix = "rake-offline:" + hashlib.sha256((metadata["application_id"] + base).encode()).hexdigest() + ":"
        config = "const CACHE=" + json.dumps(prefix + identity["build"]) + ", PREFIX=" + json.dumps(prefix) + ", ROOT=" + json.dumps(base) + ", INDEX=" + json.dumps(assets_base + "index.html") + ", ASSETS=" + json.dumps(assets) + ";\n"
        (site / "rake-service-worker.js").write_text(config + template)
    (site / "rake-version.json").write_text(json.dumps(identity, indent=2) + "\n")
    return identity
