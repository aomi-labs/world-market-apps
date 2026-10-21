#!/usr/bin/env python3
"""Build one app's immutable release bundle inside a platform candidate.

deploy_project.py imports this in its build job only; nothing here handles
credentials.
"""
from __future__ import annotations

import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
from dataclasses import dataclass
from typing import Any, Literal, NoReturn, NotRequired, TypedDict, overload


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
APP_RE = re.compile(r"^[a-z0-9][a-z0-9_-]*$")
# Backend deploys are repo-keyed: apps/<installation>/<repo-key>/<app>.
REPO_KEY_RE = re.compile(r"^r[0-9a-f]{10}$")
COMMIT_RE = re.compile(r"^[0-9a-f]{7,40}$")
SHA256_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
SUPPORTED_INTERFACES = frozenset(
    {"async_tools_v1", "routes_v1", "app_skills_v1", "secret_slots_v1", "evm_sponsored_4337_v1"}
)
# SDKs whose compiled manifest predates ffi_version; both expose FFI 1.
LEGACY_FFI = {"5.1.1": 1, "5.1.2": 1}


def fail(message: str) -> NoReturn:
    raise SystemExit(f"error: {message}")


def run_capture(cmd: list[str], *, cwd: pathlib.Path | None = None) -> subprocess.CompletedProcess[str]:
    """Run a command and return its result; the caller classifies a non-zero exit."""
    return subprocess.run(cmd, cwd=cwd or REPO_ROOT, text=True, capture_output=True)


def run(cmd: list[str], *, cwd: pathlib.Path | None = None, capture: bool = True) -> str:
    """Run a command that must succeed; `capture=False` echoes its output."""
    result = run_capture(cmd, cwd=cwd)
    if not capture:
        print(result.stdout or "", end="")
        print(result.stderr or "", end="", file=sys.stderr)
    if result.returncode != 0:
        command = " ".join(cmd)
        detail = (result.stderr or "")[-6000:].strip()
        fail(f"command failed ({command}): {detail}")
    return result.stdout.strip() if capture and result.stdout else ""


def load_json(path: pathlib.Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        fail(f"missing JSON file: {path}")
    except json.JSONDecodeError as err:
        fail(f"invalid JSON in {path}: {err}")
    if not isinstance(value, dict):
        fail(f"{path} must contain a JSON object")
    return value


def write_json(path: pathlib.Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=False) + "\n", encoding="utf-8")


def sha256_file(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def relpath(path: pathlib.Path) -> str:
    return path.relative_to(REPO_ROOT).as_posix()


@overload
def get_str(value: dict[str, Any], path: tuple[str, ...]) -> str: ...
@overload
def get_str(value: dict[str, Any], path: tuple[str, ...], *, required: Literal[False]) -> str | None: ...
def get_str(value: dict[str, Any], path: tuple[str, ...], *, required: bool = True) -> str | None:
    cursor: Any = value
    for key in path:
        if not isinstance(cursor, dict) or key not in cursor:
            if required:
                fail(f"deployment manifest missing {'.'.join(path)}")
            return None
        cursor = cursor[key]
    if not isinstance(cursor, str) or not cursor.strip():
        if required:
            fail(f"deployment manifest field {'.'.join(path)} must be a non-empty string")
        return None
    return cursor.strip()


def normalize_repo(value: str) -> str:
    repo = value.strip().removesuffix(".git").strip("/")
    for prefix in [
        "git@github.com:",
        "ssh://git@github.com/",
        "https://github.com/",
        "http://github.com/",
        "github.com/",
    ]:
        if repo.startswith(prefix):
            repo = repo.removeprefix(prefix)
            break
    return repo.lower()


@dataclass(frozen=True)
class Candidate:
    """The platform candidate being built: its source identity and the deployment's pins."""

    owner_repo: str
    installation_id: str
    short_commit: str
    branch: str
    commit: str
    platform: str
    sdk_version: str | None
    ffi_version: int | None


@dataclass(frozen=True)
class Release:
    """One app's release identity, validated from its staged .aomi/deployment.json."""

    app_name: str
    installation_id: str
    source_commit: str
    tag: str
    app_path: str
    manifest: str

    @classmethod
    def load(cls, app_dir: pathlib.Path, candidate: Candidate, target: str, source_dir: pathlib.Path) -> Release:
        path = app_dir / ".aomi" / "deployment.json"
        manifest = load_json(path)
        parts = app_dir.relative_to(REPO_ROOT).parts
        if len(parts) != 4 or parts[0] != "apps" or not REPO_KEY_RE.match(parts[2]):
            fail(f"candidate app dir must be apps/<installation-id>/<repo-key>/<app>, got {relpath(app_dir)}")
        installation_id, repo_key, app_name = parts[1], parts[2], parts[3]
        if installation_id != candidate.installation_id:
            fail(f"{relpath(app_dir)} installation id does not match branch")
        if not APP_RE.match(app_name):
            fail(f"invalid app directory name: {app_name}")

        app_path = f"apps/{installation_id}/{repo_key}/{app_name}"
        manifest_installation_id = manifest.get("source", {}).get("installation_id")
        if str(manifest_installation_id) != installation_id:
            fail(f"deployment manifest source.installation_id must be {installation_id}")

        platform = manifest.get("platform")
        apps = platform.get("apps") if isinstance(platform, dict) else None
        if not isinstance(apps, list):
            fail("deployment manifest platform.apps must be an array")
        matches = [
            app
            for app in apps
            if isinstance(app, dict)
            and str(app.get("name", "")).lower() == app_name
            and app.get("path") == app_path
        ]
        if len(matches) != 1:
            fail(f"deployment manifest must contain one platform.apps entry for {app_path}")
        app_record: dict[str, Any] = matches[0]
        source_commit = get_str(manifest, ("source", "commit_hash")).lower()
        source_repo = get_str(manifest, ("source", "owner_repo_name"), required=False)
        if source_repo is None:
            source_repo = get_str(manifest, ("source", "repository_link"), required=False)
        platform_name = get_str(manifest, ("platform", "platform"))
        manifest_app_path = get_str(app_record, ("path",))
        release_tag = get_str(app_record, ("release_tag",))
        manifest_target = get_str(app_record, ("target",), required=False)
        files = app_record.get("files")

        if not COMMIT_RE.match(source_commit) or not source_commit.startswith(candidate.short_commit):
            fail("deployment manifest source commit does not match branch short commit")

        if source_repo and normalize_repo(source_repo) != candidate.owner_repo:
            fail("deployment manifest source repo does not match branch owner/repo")

        if platform_name != candidate.platform:
            fail("deployment manifest platform does not match the project")

        if manifest_app_path != app_path:
            fail(f"deployment manifest app path must be {app_path}")

        expected_tag = f"apps-{installation_id}-{repo_key}-{app_name}-{candidate.short_commit}"
        if release_tag != expected_tag:
            fail(f"deployment manifest release_tag must be {expected_tag}")

        if manifest_target and manifest_target != target:
            fail(f"deployment manifest target must be {target}")

        if not isinstance(files, list) or not files:
            fail("deployment manifest files must be a non-empty array")
        seen: set[str] = set()
        for entry in files:
            if not isinstance(entry, dict):
                fail("deployment manifest files entries must be objects")
            file = entry.get("path")
            digest = entry.get("sha256")
            byte_count = entry.get("bytes")
            if not isinstance(file, str) or file.startswith("/"):
                fail(f"invalid staged file path: {file!r}")
            rel = pathlib.PurePosixPath(file)
            if any(part in {"", ".", ".."} for part in rel.parts):
                fail(f"invalid staged file path: {file!r}")
            if file in seen:
                fail(f"duplicate staged file path: {file}")
            seen.add(file)
            file_path = source_dir / file
            if not file_path.is_file():
                fail(f"staged file missing from app directory: {file}")
            if not isinstance(digest, str) or not SHA256_RE.match(digest):
                fail(f"invalid sha256 for staged file {file}")
            if digest != f"sha256:{sha256_file(file_path)}":
                fail(f"staged file sha256 mismatch for {file}")
            if byte_count != file_path.stat().st_size:
                fail(f"staged file byte count mismatch for {file}")
        return cls(
            app_name=app_name,
            installation_id=installation_id,
            source_commit=source_commit,
            tag=release_tag,
            app_path=app_path,
            manifest=relpath(path),
        )


# NOTE: this is a stderr-text heuristic, not a structural check -- clap does
# not give us a machine-readable "unknown subcommand" signal via exit code
# alone, and we don't yet know the exact aomi-sdk version in which the
# `manifest` subcommand shipped. Once that version is known/pinned, prefer
# gating on `sdk_version` directly (e.g. `sdk_version < MANIFEST_MIN_VERSION`)
# instead of sniffing clap's stderr wording, which is inherently fragile
# across clap releases.
# TODO(manifest-min-version): replace this stderr heuristic with a version
# comparison once the introducing aomi-sdk version is known/released.
def is_unsupported_manifest_error(stderr: str) -> bool:
    """True iff `stderr` looks like clap rejecting the `manifest` SUBCOMMAND
    itself as unknown, i.e. the installed aomi-sdk predates the `manifest`
    subcommand.

    This must be narrow: it is only "unsupported" when the rejected/unknown
    token clap complains about is `manifest` itself. A flag-level rejection
    on an SDK that DOES have the `manifest` subcommand -- e.g.
    `error: unexpected argument '--lib' found` -- names `--lib`, not
    `manifest`, and is a real manifest-contract bug that must fail the build,
    not be silently treated as "old SDK, fall back to []".

    Any other non-zero exit (network blip, panic, timeout, unrelated flag
    rejection, etc.) is a real failure and must NOT be classified as
    "unsupported".
    """
    lowered = stderr.lower()

    # Modern clap (v4): subcommand plainly not registered.
    #   error: unrecognized subcommand 'manifest'
    if "unrecognized subcommand" in lowered and "manifest" in lowered:
        return True

    # Defensive: alternate wording for the same "no such subcommand" case.
    if "no such subcommand" in lowered and "manifest" in lowered:
        return True

    # clap can also report an unregistered subcommand as an "unexpected
    # argument" that names the subcommand itself, e.g.:
    #   error: unexpected argument 'manifest' found
    # This still names `manifest` as the rejected token -- unlike a
    # flag-level rejection such as `error: unexpected argument '--lib' found`,
    # which names `--lib` and must NOT match here.
    if "unexpected argument" in lowered and "'manifest'" in lowered:
        return True

    # Older clap (v2/v3) phrasing that names the subcommand directly:
    #   error: Found argument 'manifest' which wasn't expected, or isn't
    #   valid in this context
    # Requiring both the phrase AND the quoted `'manifest'` token (rather than
    # a bare "wasn't expected" substring match) keeps this from matching a
    # flag-level rejection phrased the same way for some other argument.
    if "wasn't expected" in lowered and "'manifest'" in lowered:
        return True

    return False


@dataclass(frozen=True)
class PluginManifest:
    """`aomi-build manifest` output for a compiled plugin: FFI requirements and secret slots."""

    ffi_version: int | None
    required_interfaces: list[str]
    secrets: list[object]

    @classmethod
    def parse(cls, raw: object, plugin: str, sdk_version: str) -> PluginManifest:
        """Validate the compiled manifest; semver ranges do not establish compatibility."""
        if not isinstance(raw, dict):
            fail(
                f"aomi-build manifest for {plugin} (sdk {sdk_version}) returned "
                f"{type(raw).__name__}, expected a JSON object"
            )
        secrets = raw.get("secrets")
        if secrets is not None and not isinstance(secrets, list):
            fail(
                f"aomi-build manifest for {plugin} (sdk {sdk_version}): \"secrets\" "
                f"field is {type(secrets).__name__}, expected a list"
            )
        if raw.get("sdk_version") != sdk_version:
            fail("Compiled manifest SDK does not match the resolved dependency")
        version = raw.get("ffi_version", LEGACY_FFI.get(sdk_version))
        if version is not None and (type(version) is not int or version <= 0):
            fail("Compiled manifest ffi_version must be a positive integer")
        interfaces = raw.get("required_interfaces", [])
        if not isinstance(interfaces, list) or any(
            not isinstance(item, str) or item not in SUPPORTED_INTERFACES for item in interfaces
        ):
            fail("Compiled manifest requires an unsupported FFI interface")
        return cls(ffi_version=version, required_interfaces=interfaces, secrets=secrets or [])

    @classmethod
    def read(cls, plugin_path: pathlib.Path, sdk_version: str) -> PluginManifest | None:
        """Read the compiled manifest via `aomi-build manifest`.

        Installs the app's exact pinned aomi-sdk so the manifest reader matches the
        ABI the plugin was built against. Returns None ONLY when the installed SDK
        genuinely predates the `manifest` subcommand -- that is the sole
        legitimate reason a build may ship without secret metadata. Any other
        failure (a flaky `cargo install`, a transient `aomi-build manifest`
        error, or malformed output from an SDK that DOES support `manifest`) must
        fail the build rather than silently publish a release with no secret
        metadata, which would permanently ungate an app that declares required
        secrets.
        """
        bin_root = pathlib.Path(tempfile.mkdtemp(prefix="aomi-build-cli-"))
        # A failure here (network hiccup, flaky cargo install, etc.) is always a
        # real failure -- an older SDK still installs fine, it just lacks the
        # `manifest` subcommand (handled below). Let run() raise via fail().
        run([
            "cargo", "install", "aomi-sdk",
            "--version", f"={sdk_version}",
            "--features", "cli", "--bin", "aomi-build",
            "--locked", "--root", str(bin_root),
        ])

        result = run_capture([str(bin_root / "bin" / "aomi-build"), "manifest", "--lib", str(plugin_path)])
        if result.returncode != 0:
            stderr = (result.stderr or "").strip()
            if is_unsupported_manifest_error(stderr):
                print(
                    f"::notice::{plugin_path.name}: installed aomi-sdk {sdk_version} has no "
                    "`manifest` subcommand (predates secret-declaration support); "
                    "this app cannot be secret-gated"
                )
                return None
            fail(
                f"aomi-build manifest failed for {plugin_path.name} (sdk {sdk_version}): "
                f"{stderr or 'no stderr captured'}"
            )

        try:
            raw = json.loads((result.stdout or "").strip())
        except json.JSONDecodeError as err:
            fail(
                f"aomi-build manifest for {plugin_path.name} (sdk {sdk_version}) produced "
                f"invalid JSON: {err}"
            )
        return cls.parse(raw, plugin_path.name, sdk_version)


class Plugin(TypedDict):
    """One plugins/* entry of the bundle manifest."""

    file: str
    sha256: str
    secrets: NotRequired[list[object]]


class BundleManifest(TypedDict):
    """plugins/manifest.json: what the runtime loads and deploy_project.py verifies."""

    app_release_tag: str
    sdk_version: str
    target: str
    commit: str
    plugins: dict[str, Plugin]
    ffi_version: NotRequired[int]
    required_interfaces: NotRequired[list[str]]


def build_release(
    app_dir: pathlib.Path, candidate: Candidate, target: str, dist_root: pathlib.Path, source_dir: pathlib.Path
) -> None:
    release = Release.load(app_dir, candidate, target, source_dir)
    app_name = release.app_name

    cargo_toml = source_dir / "Cargo.toml"
    if not cargo_toml.is_file():
        fail(f"{relpath(source_dir)} must contain Cargo.toml")
    cargo = tomllib.loads(cargo_toml.read_text(encoding="utf-8"))
    package = cargo.get("package")
    if not isinstance(package, dict) or not isinstance(package.get("name"), str):
        fail(f"{relpath(cargo_toml)} must define [package].name")
    package_name = package["name"].strip()
    lib = cargo.get("lib")
    if not isinstance(lib, dict):
        lib = {}
    if "cdylib" not in lib.get("crate-type", []):
        fail(f"{relpath(cargo_toml)} must set [lib].crate-type = [\"cdylib\"]")
    lib_name = str(lib.get("name") or package_name)

    manifest_path = cargo_toml.resolve()
    output = run(
        ["cargo", "metadata", "--locked", "--format-version", "1", "--manifest-path", str(manifest_path)],
        cwd=source_dir,
    )
    metadata = json.loads(output)
    packages = {pkg["id"]: pkg for pkg in metadata.get("packages", [])}
    root_pkg = next(
        (pkg for pkg in packages.values() if pathlib.Path(pkg["manifest_path"]).resolve() == manifest_path), None
    )
    if root_pkg is None:
        fail(f"cargo metadata did not include root package for {manifest_path}")
    nodes = {node["id"]: node for node in metadata.get("resolve", {}).get("nodes", [])}
    root_node = nodes.get(root_pkg["id"])
    if root_node is None:
        fail("cargo metadata did not include the app dependency graph")
    sdk_version: str | None = None
    for dep in root_node.get("deps", []):
        dep_pkg = packages.get(dep.get("pkg"))
        if dep_pkg and dep_pkg.get("name") == "aomi-sdk":
            sdk_version = dep_pkg["version"]
            break
    if sdk_version is None:
        fail("app must depend directly on aomi-sdk")
    if candidate.ffi_version is None and candidate.sdk_version and sdk_version != candidate.sdk_version:
        fail(f"{app_name}: aomi-sdk {sdk_version} does not match required {candidate.sdk_version}")

    target_dir = REPO_ROOT / ".aomi-ci-target"
    run(
        [
            "cargo",
            "build",
            "--locked",
            "--lib",
            "--release",
            "--target",
            target,
            "--target-dir",
            str(target_dir),
            "--manifest-path",
            str(source_dir / "Cargo.toml"),
        ],
        capture=False,
        cwd=source_dir,
    )

    if "linux" in target:
        ext = "so"
    elif "apple" in target or "darwin" in target:
        ext = "dylib"
    elif "windows" in target:
        ext = "dll"
    else:
        fail(f"unsupported target triple: {target}")
    normalized = lib_name.replace("-", "_")
    built_lib = target_dir / target / "release" / (f"{normalized}.dll" if ext == "dll" else f"lib{normalized}.{ext}")
    if not built_lib.is_file():
        fail(f"expected built library not found: {built_lib}")

    dist_dir = dist_root / release.tag
    if dist_dir.exists():
        shutil.rmtree(dist_dir)
    plugins_dir = dist_dir / "plugins"
    plugins_dir.mkdir(parents=True)

    final_plugin = plugins_dir / f"{app_name.replace('-', '_')}.{ext}"
    shutil.copy2(built_lib, final_plugin)
    entry: Plugin = {"file": final_plugin.name, "sha256": sha256_file(final_plugin)}
    plugin_manifest = PluginManifest.read(final_plugin, sdk_version)
    ffi_version = None if plugin_manifest is None else plugin_manifest.ffi_version
    if candidate.ffi_version is not None and ffi_version != candidate.ffi_version:
        fail(f"{app_name}: compiled ffi_version {ffi_version} does not match required {candidate.ffi_version}")
    if plugin_manifest and plugin_manifest.secrets:
        entry["secrets"] = plugin_manifest.secrets
    bundle: BundleManifest = {
        "app_release_tag": release.tag,
        "sdk_version": sdk_version,
        "target": target,
        "commit": release.source_commit,
        "plugins": {app_name: entry},
    }
    if ffi_version is not None:
        bundle["ffi_version"] = ffi_version
    if plugin_manifest and plugin_manifest.required_interfaces:
        bundle["required_interfaces"] = plugin_manifest.required_interfaces
    bundle_path = plugins_dir / "manifest.json"
    write_json(bundle_path, bundle)

    tarball = dist_dir / f"aomi-plugins-{release.tag}-{target}.tar.gz"
    with tarfile.open(tarball, "w:gz") as archive:
        archive.add(bundle_path, arcname="plugins/manifest.json")
        archive.add(final_plugin, arcname=f"plugins/{final_plugin.name}")
    with tempfile.TemporaryDirectory(prefix="aomi-candidate-") as tmp:
        tmp_path = pathlib.Path(tmp)
        with tarfile.open(tarball, "r:gz") as archive:
            archive.extractall(tmp_path, filter="data")
        if load_json(tmp_path / "plugins" / "manifest.json") != bundle:
            fail("tarball manifest does not match generated manifest")
        for name, plugin in bundle["plugins"].items():
            extracted = tmp_path / "plugins" / plugin["file"]
            if not extracted.is_file():
                fail(f"tarball plugin missing for {name}")
            if sha256_file(extracted) != plugin["sha256"]:
                fail(f"tarball plugin checksum mismatch for {name}")

    shutil.copy2(bundle_path, dist_dir / "manifest.json")
    write_json(
        dist_dir / "aomi-release.json",
        {
            "schema_version": 1,
            "platform": candidate.platform,
            "app": {
                "name": app_name,
                "path": release.app_path,
                "package": package_name,
            },
            "source": {
                "repository": candidate.owner_repo,
                "installation_id": int(release.installation_id),
                "commit": release.source_commit,
            },
            "candidate": {
                "branch": candidate.branch,
                "commit": candidate.commit,
                "deployment_manifest": release.manifest,
            },
            "release": {
                "tag": release.tag,
                "target": target,
                "sdk_version": sdk_version,
            },
            "assets": {
                "tarball": tarball.name,
                "manifest": "manifest.json",
                "metadata": "aomi-release.json",
            },
        },
    )
    (dist_dir / "release-notes.md").write_text(
        "\n".join(
            [
                f"Aomi {candidate.platform} candidate release.",
                "",
                f"- App: {app_name}",
                f"- Release: {release.tag}",
                f"- Source: {candidate.owner_repo}@{release.source_commit}",
                f"- Candidate: {candidate.branch}@{candidate.commit}",
                f"- SDK: {sdk_version}",
                f"- Target: {target}",
                "",
            ]
        ),
        encoding="utf-8",
    )
