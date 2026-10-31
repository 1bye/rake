"""Static Web deployment artifacts; native packaging belongs to the Rust coordinator."""

import hashlib
import html
import json
from pathlib import Path
import re
import shutil
import stat
import zipfile
import web_offline


def base_path(value):
    if not isinstance(value, str) or not re.fullmatch(r"/(?:[A-Za-z0-9._~-]+/)*", value) or any(part in {".", ".."} for part in value.split("/")):
        raise ValueError("Web base_path must be / or an ASCII path with leading/trailing slashes and no dot segments")
    return value


def copy_tree(source, destination):
    """Copy explicit static inputs, rejecting links, special files and collisions."""
    destination.mkdir(parents=True, exist_ok=True)
    destination.chmod(0o755)
    for item in sorted(source.iterdir()):
        target = destination / item.name
        mode = item.lstat().st_mode
        if stat.S_ISLNK(mode) or not (stat.S_ISDIR(mode) or stat.S_ISREG(mode)):
            raise ValueError(f"asset input must be a regular file/directory, not a link or special file: {item}")
        if target.exists():
            raise ValueError(f"asset/output collision: {target.name}")
        if item.is_dir():
            copy_tree(item, target)
        else:
            shutil.copyfile(item, target)
            target.chmod(0o644)


class BundleConfig:
    def __init__(self, package, directory, native=False):
        self.directory = directory.resolve()
        config = (package.get("metadata") or {}).get("rake", {}).get("bundle", {})
        # The same Cargo application can declare native installers and a Web build.
        # Native sections remain owned/validated by the Rust distribution pipeline;
        # they do not become browser assets, helper processes or update providers.
        native_fields = {"schema_version", "package_version", "target_overrides", "format_overrides",
            "desktop", "macos", "windows", "linux", "dependencies", "components", "resources",
            "sidecars", "runtimes", "updates", "release", "custom_formats", "component_recipes"}
        if not isinstance(config, dict) or (not native and config.keys() - {"name", "identifier", "assets", "icon", "build_number", "minimum_system_version", "web"} - native_fields):
            raise ValueError("unknown or invalid package.metadata.rake.bundle configuration")
        self.name = config.get("name", package["name"])
        self.identifier = config.get("identifier", "dev.rake." + package["name"].replace("_", "-"))
        if not isinstance(self.name, str) or not self.name.strip() or self.name in {".", ".."} or any(character in '/\\\0' or ord(character) < 32 for character in self.name):
            raise ValueError("bundle name must be a nonempty filename without separators or control characters")
        if not isinstance(self.identifier, str) or not re.fullmatch(r"[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+", self.identifier):
            raise ValueError("bundle identifier must contain dot-separated ASCII letters, digits or hyphens")
        self.version = package["version"].split("-", 1)[0].split("+", 1)[0]
        self.build_number = config.get("build_number", self.version)
        self.minimum_system_version = config.get("minimum_system_version")
        for value in [self.build_number, self.minimum_system_version]:
            if value is not None and (not isinstance(value, str) or not re.fullmatch(r"[0-9]+(?:\.[0-9]+){0,2}", value)):
                raise ValueError("build_number/minimum_system_version must contain one to three numeric components")
        self.assets = self.input(config.get("assets"), directory=True)
        self.icon = self.input(config.get("icon"))
        web = config.get("web", {})
        if not isinstance(web, dict) or web.keys() - {"base_path", "routing", "offline"}:
            raise ValueError("unknown bundle.web configuration")
        self.web_base = base_path(web.get("base_path", "/"))
        self.routing = web.get("routing", "path")
        self.offline = web.get("offline", False)
        if not isinstance(self.offline, bool):
            raise ValueError("bundle.web.offline must be a boolean")
        if self.routing not in {"path", "hash"}:
            raise ValueError("bundle.web.routing must be path or hash")

    def input(self, value, directory=False):
        if value is None:
            return None
        if not isinstance(value, str) or not value or Path(value).is_absolute():
            raise ValueError("bundle inputs must be relative to the package directory")
        path = self.directory / value
        if path.is_symlink():
            raise ValueError(f"bundle inputs cannot be symlinks: {value}")
        path = path.resolve(strict=True)
        if path == self.directory or not path.is_relative_to(self.directory) or (not path.is_dir() if directory else not path.is_file()):
            raise ValueError(f"invalid bundle input: {value}")
        return path


def web_deployment(project, site):
    config = project.bundle
    base = base_path(project.args.base_path if project.args.base_path is not None else config.web_base)
    routing = project.args.routing or config.routing
    index = site / "index.html"
    document = index.read_text()
    if (site / "rake-deployment.json").exists():
        raise ValueError("rake-deployment.json is reserved for generated deployment metadata")
    if (site / "__rake").exists():
        raise ValueError("__rake is reserved for development tooling")
    if re.search(r"rake-dev-bridge|/__rake/|rake-web-config", document):
        raise ValueError("Web input contains development or existing deployment metadata")
    if re.search(r"<base\b", document, re.IGNORECASE):
        raise ValueError("remove the custom base element; rake bundle owns the deployment base")
    # Only Rake's managed namespaces change; application/API URLs remain author-owned.
    for path in sorted([*site.rglob("*.html"), *site.rglob("*.js"), *site.rglob("*.css")]):
        if (site / "pkg") in path.parents:
            continue  # wasm-bindgen resolves its Wasm relative to import.meta.url.
        text = path.read_text()
        text = re.sub(r"(['\"])/(pkg|assets)/", lambda match: match[1] + base + match[2] + "/", text)
        path.write_text(text)
    document = index.read_text()
    metadata = html.escape(json.dumps({"base_path": base, "routing": routing}), quote=True)
    head = re.search(r"<head\b[^>]*>", document, re.IGNORECASE) or re.search(r"<html\b[^>]*>", document, re.IGNORECASE)
    if not head:
        raise ValueError("Web document requires an html or head element")
    insertion = f'<base href="{html.escape(base, quote=True)}"><meta name="rake-web-config" content="{metadata}">'
    index.write_text(document[:head.end()] + insertion + document[head.end():])
    metadata = {"protocol": 1, "application_id": config.identifier, "version": project.package["version"],
        "base_path": base, "routing": routing, "assets": base + "assets/", "wasm_mime": "application/wasm",
        "development_control": False, "cache": "revalidate; filenames are not content hashed"}
    web_offline.prepare(site, metadata, config.offline)
    (site / "rake-deployment.json").write_text(json.dumps(metadata, indent=2) + "\n")
    return metadata


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as file:
        while chunk := file.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def distribution(project, output, processes, invoke):
    if project.args.target != "web":
        raise ValueError("native packaging requires the Rust CLI")
    payload = output / "site"
    artifact = project.build(processes, payload)
    signing = "not_applicable"
    name = f"{project.package['name']}-{project.package['version']}-web"
    deployment = web_deployment(project, payload)
    archive = output / (name + ".zip")
    with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as file:
        for path in sorted(payload.rglob("*")):
            if path.is_file():
                file.write(path, path.relative_to(payload))
    base = deployment["base_path"]
    fallback = f"try_files $uri $uri/ {base}index.html;" if deployment["routing"] == "path" else "try_files $uri $uri/ =404;"
    redirect = f"location = {base[:-1]} {{ return 308 {base}; }}\n" if base != "/" else ""
    (output / "nginx.conf.example").write_text(
        f"# Install site/ contents below /srv/www{base}; include the server's standard MIME types.\n{redirect}"
        f"# Retain previous rake-builds/ directories while old tabs may still request their assets.\n"
        f"location {base}rake-builds/ {{ root /srv/www; try_files $uri =404; add_header Cache-Control 'public, max-age=31536000, immutable'; }}\n"
        f"location {base}pkg/ {{ root /srv/www; try_files $uri =404; add_header Cache-Control 'no-cache'; }}\n"
        f"location {base}assets/ {{ root /srv/www; try_files $uri =404; add_header Cache-Control 'no-cache'; }}\n"
        f"location {base} {{ root /srv/www; index index.html; {fallback} add_header Cache-Control 'no-cache'; }}\n")
    files = [{"path": str(path.relative_to(output)), "bytes": path.stat().st_size, "sha256": sha256(path)}
        for path in sorted(payload.rglob("*")) if path.is_file()]
    manifest = {"protocol": 1, "package": project.package["name"], "application_id": project.bundle.identifier,
        "name": project.bundle.name, "version": project.package["version"], "target": project.args.target,
        "platform": "web", "architecture": "wasm32",
        "profile": "release" if project.args.release else "debug", "signing": signing, "notarized": False,
        "artifact": str(artifact.relative_to(output)), "deployment": deployment,
        "archive": {"path": archive.name, "bytes": archive.stat().st_size, "sha256": sha256(archive)}, "files": files}
    (output / "bundle.json").write_text(json.dumps(manifest, indent=2) + "\n")
    (output / "SHA256SUMS").write_text(f"{manifest['archive']['sha256']}  {archive.name}\n")
    return output / "bundle.json"
