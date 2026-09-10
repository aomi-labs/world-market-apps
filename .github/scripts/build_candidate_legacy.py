#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
from typing import Any


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
BRANCH_RE = re.compile(
    r"^(?P<owner>[A-Za-z0-9_.-]+)/(?P<repo>[A-Za-z0-9_.-]+)/(?P<installation_id>[0-9]+)/(?P<short_commit>[0-9a-fA-F]{7,40})$"
)
APP_RE = re.compile(r"^[a-z0-9][a-z0-9_-]*$")
# Backend deploys are repo-keyed: apps/<installation>/<repo-key>/<app>.
REPO_KEY_RE = re.compile(r"^r[0-9a-f]{10}$")
COMMIT_RE = re.compile(r"^[0-9a-f]{7,40}$")
SHA256_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
_PLATFORM_CONFIG: dict[str, Any] | None = None


def fail(message: str) -> None:
    raise SystemExit(f"error: {message}")


def run(
    cmd: list[str],
    *,
    cwd: pathlib.Path | None = None,
    capture: bool = True,
    env: dict[str, str] | None = None,
) -> str:
    merged_env = os.environ.copy()
    if env:
        merged_env.update(env)
    result = subprocess.run(
        cmd,
        cwd=cwd or REPO_ROOT,
        env=merged_env,
        text=True,
        stdout=subprocess.PIPE if capture else None,
        stderr=subprocess.PIPE if capture else None,
    )
    if result.returncode != 0:
        command = " ".join(cmd)
        detail = result.stderr.strip() if capture and result.stderr else ""
        fail(f"command failed ({command}): {detail}")
    return result.stdout.strip() if capture and result.stdout else ""


def run_capture(
    cmd: list[str],
    *,
    cwd: pathlib.Path | None = None,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    """Like run(), but returns the CompletedProcess instead of raising on a
    non-zero exit code.

    Use this (instead of run()) when a caller needs to classify *why* a
    command failed -- e.g. read_plugin_secrets distinguishing an older SDK
    whose `aomi-build` lacks the `manifest` subcommand (a legitimate, expected
    failure) from a transient/real failure (which must fail the build).
    """
    merged_env = os.environ.copy()
    if env:
        merged_env.update(env)
    return subprocess.run(
        cmd,
        cwd=cwd or REPO_ROOT,
        env=merged_env,
        text=True,
        capture_output=True,
    )


def git(args: list[str], *, capture: bool = True) -> str:
    return run(["git", *args], capture=capture)


def gh(args: list[str], *, capture: bool = True) -> str:
    return run(["gh", *args], capture=capture)


def current_branch() -> str:
    ref_name = os.environ.get("GITHUB_REF_NAME", "").strip()
    if ref_name:
        return ref_name
    return git(["branch", "--show-current"])


def current_commit() -> str:
    return os.environ.get("GITHUB_SHA", "").strip() or git(["rev-parse", "HEAD"])


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


def platform_config() -> dict[str, Any]:
    global _PLATFORM_CONFIG
    if _PLATFORM_CONFIG is None:
        _PLATFORM_CONFIG = load_json(REPO_ROOT / "platform.json")
    return _PLATFORM_CONFIG


def platform_name() -> str:
    value = platform_config().get("name")
    if not isinstance(value, str) or not value.strip():
        fail("platform.json must define a non-empty name")
    return value.strip()


def required_sdk_version() -> str:
    value = platform_config().get("required_sdk_version")
    if not isinstance(value, str) or not value.strip():
        fail("platform.json must define a non-empty required_sdk_version")
    return value.strip()


def write_json(path: pathlib.Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=False) + "\n", encoding="utf-8")


def sha256_file(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def sha256_prefixed_file(path: pathlib.Path) -> str:
    return f"sha256:{sha256_file(path)}"


def relpath(path: pathlib.Path) -> str:
    return path.relative_to(REPO_ROOT).as_posix()


def branch_context() -> dict[str, str]:
    branch = current_branch()
    match = BRANCH_RE.match(branch)
    if not match:
        fail(
            "candidate release workflow must run on "
            "`<owner>/<repo>/<installation-id>/<short-commit>` branches"
        )
    value = match.groupdict()
    value["owner_repo"] = f"{value['owner'].lower()}/{value['repo'].lower()}"
    value["short_commit"] = value["short_commit"].lower()
    value["branch"] = branch
    return value


def changed_paths(base: str, head: str) -> list[str]:
    base = base.strip()
    head = head.strip() or "HEAD"
    if base:
        try:
            return git(["diff", "--name-only", base, head]).splitlines()
        except SystemExit:
            pass
    return git(["diff-tree", "--root", "--no-commit-id", "--name-only", "-r", head]).splitlines()


def changed_app_dirs(base: str, head: str) -> list[str]:
    dirs: set[str] = set()
    for path in changed_paths(base, head):
        parts = pathlib.PurePosixPath(path).parts
        if (
            len(parts) >= 4
            and parts[0] == "apps"
            and parts[1].isdigit()
            and REPO_KEY_RE.match(parts[2])
            and not parts[3].startswith(".")
        ):
            dirs.add("/".join(parts[:4]))
    return sorted(dirs)


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


def parse_cargo_manifest(app_dir: pathlib.Path) -> tuple[str, str]:
    manifest_path = app_dir / "Cargo.toml"
    if not manifest_path.is_file():
        fail(f"{relpath(app_dir)} must contain Cargo.toml")
    data = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
    package = data.get("package")
    if not isinstance(package, dict) or not isinstance(package.get("name"), str):
        fail(f"{relpath(manifest_path)} must define [package].name")
    package_name = package["name"].strip()
    lib = data.get("lib") if isinstance(data.get("lib"), dict) else {}
    crate_types = lib.get("crate-type", [])
    if "cdylib" not in crate_types:
        fail(f"{relpath(manifest_path)} must set [lib].crate-type = [\"cdylib\"]")
    return package_name, str(lib.get("name") or package_name)


def resolve_sdk_version(app_dir: pathlib.Path) -> str:
    manifest_path = (app_dir / "Cargo.toml").resolve()
    output = run(
        [
            "cargo",
            "metadata",
            "--format-version",
            "1",
            "--manifest-path",
            str(manifest_path),
        ]
    )
    metadata = json.loads(output)
    packages = {pkg["id"]: pkg for pkg in metadata.get("packages", [])}
    root_pkg = None
    for pkg in packages.values():
        if pathlib.Path(pkg["manifest_path"]).resolve() == manifest_path:
            root_pkg = pkg
            break
    if root_pkg is None:
        fail(f"cargo metadata did not include root package for {manifest_path}")
    nodes = {node["id"]: node for node in metadata.get("resolve", {}).get("nodes", [])}
    root_node = nodes.get(root_pkg["id"])
    if root_node is None:
        fail("cargo metadata did not include the app dependency graph")
    for dep in root_node.get("deps", []):
        package = packages.get(dep.get("pkg"))
        if package and package.get("name") == "aomi-sdk":
            return package["version"]
    fail("app must depend directly on aomi-sdk")


def lib_ext(target: str) -> str:
    if "linux" in target:
        return "so"
    if "apple" in target or "darwin" in target:
        return "dylib"
    if "windows" in target:
        return "dll"
    fail(f"unsupported target triple: {target}")


def cargo_lib_name(lib_name: str, target: str) -> str:
    normalized = lib_name.replace("-", "_")
    ext = lib_ext(target)
    if ext == "dll":
        return f"{normalized}.dll"
    return f"lib{normalized}.{ext}"


def plugin_file_name(app_name: str, target: str) -> str:
    return f"{app_name.replace('-', '_')}.{lib_ext(target)}"


def validate_file_manifest(app_dir: pathlib.Path, files: Any) -> None:
    if not isinstance(files, list) or not files:
        fail("deployment manifest files must be a non-empty array")
    seen: set[str] = set()
    for entry in files:
        if not isinstance(entry, dict):
            fail("deployment manifest files entries must be objects")
        path = entry.get("path")
        digest = entry.get("sha256")
        byte_count = entry.get("bytes")
        if not isinstance(path, str) or path.startswith("/"):
            fail(f"invalid staged file path: {path!r}")
        rel = pathlib.PurePosixPath(path)
        if any(part in {"", ".", ".."} for part in rel.parts):
            fail(f"invalid staged file path: {path!r}")
        if path in seen:
            fail(f"duplicate staged file path: {path}")
        seen.add(path)
        file_path = app_dir / path
        if not file_path.is_file():
            fail(f"staged file missing from app directory: {path}")
        if not isinstance(digest, str) or not SHA256_RE.match(digest):
            fail(f"invalid sha256 for staged file {path}")
        if digest != sha256_prefixed_file(file_path):
            fail(f"staged file sha256 mismatch for {path}")
        if byte_count != file_path.stat().st_size:
            fail(f"staged file byte count mismatch for {path}")


def deployment_app_record(
    manifest: dict[str, Any], app_name: str, expected_app_path: str
) -> dict[str, Any] | None:
    platform = manifest.get("platform")
    apps = platform.get("apps") if isinstance(platform, dict) else None
    if apps is None:
        return None
    if not isinstance(apps, list):
        fail("deployment manifest platform.apps must be an array")
    matches = [
        app
        for app in apps
        if isinstance(app, dict)
        and str(app.get("name", "")).lower() == app_name
        and app.get("path") == expected_app_path
    ]
    if len(matches) != 1:
        fail(f"deployment manifest must contain one platform.apps entry for {expected_app_path}")
    return matches[0]


def load_deployment(app_dir: pathlib.Path, ctx: dict[str, str], target: str) -> dict[str, str]:
    path = app_dir / ".aomi" / "deployment.json"
    manifest = load_json(path)
    parts = app_dir.relative_to(REPO_ROOT).parts
    if len(parts) != 4 or parts[0] != "apps" or not REPO_KEY_RE.match(parts[2]):
        fail(f"candidate app dir must be apps/<installation-id>/<repo-key>/<app>, got {relpath(app_dir)}")
    installation_id, repo_key, app_name = parts[1], parts[2], parts[3]
    if installation_id != ctx["installation_id"]:
        fail(f"{relpath(app_dir)} installation id does not match branch")
    if not APP_RE.match(app_name):
        fail(f"invalid app directory name: {app_name}")

    expected_app_path = f"apps/{installation_id}/{repo_key}/{app_name}"
    manifest_installation_id = manifest.get("source", {}).get("installation_id")
    if str(manifest_installation_id) != installation_id:
        fail(f"deployment manifest source.installation_id must be {installation_id}")

    app_record = deployment_app_record(manifest, app_name, expected_app_path)
    if app_record is None:
        manifest_app = get_str(manifest, ("app", "name"))
        if manifest_app.lower() != app_name:
            fail(f"deployment manifest app.name must be {app_name}")
        source_commit = get_str(manifest, ("source", "commit")).lower()
        source_repo = get_str(manifest, ("source", "repository_link"), required=False)
        manifest_platform = get_str(manifest, ("platform", "name"))
        app_path = get_str(manifest, ("target", "app_path"))
        release_tag = get_str(manifest, ("target", "release_tag"))
        manifest_target = get_str(manifest, ("target", "target"), required=False)
        files = manifest.get("files")
    else:
        source_commit = get_str(manifest, ("source", "commit_hash")).lower()
        source_repo = get_str(manifest, ("source", "owner_repo_name"), required=False)
        if source_repo is None:
            source_repo = get_str(manifest, ("source", "repository_link"), required=False)
        manifest_platform = get_str(manifest, ("platform", "platform"))
        app_path = get_str(app_record, ("path",))
        release_tag = get_str(app_record, ("release_tag",))
        manifest_target = get_str(app_record, ("target",), required=False)
        files = app_record.get("files")

    if not COMMIT_RE.match(source_commit) or not source_commit.startswith(ctx["short_commit"]):
        fail("deployment manifest source commit does not match branch short commit")

    if source_repo and normalize_repo(source_repo) != ctx["owner_repo"]:
        fail("deployment manifest source repo does not match branch owner/repo")

    expected_platform = platform_name()
    if manifest_platform != expected_platform:
        fail(f"deployment manifest platform must be {expected_platform}")

    if app_path != expected_app_path:
        fail(f"deployment manifest app path must be {expected_app_path}")

    expected_tag = f"apps-{installation_id}-{repo_key}-{app_name}-{ctx['short_commit']}"
    if release_tag != expected_tag:
        fail(f"deployment manifest release_tag must be {expected_tag}")

    if manifest_target and manifest_target != target:
        fail(f"deployment manifest target must be {target}")

    validate_file_manifest(app_dir, files)
    return {
        "app_name": app_name,
        "installation_id": installation_id,
        "source_commit": source_commit,
        "release_tag": release_tag,
        "deployment_manifest": relpath(path),
        "app_path": expected_app_path,
        "platform": expected_platform,
    }


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


def read_plugin_secrets(plugin_path: pathlib.Path, sdk_version: str) -> list[dict[str, Any]]:
    """Declared secret slots for a built plugin, via `aomi-build manifest`.

    Installs the app's exact pinned aomi-sdk so the manifest reader matches the
    ABI the plugin was built against. Returns [] ONLY when the installed SDK
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
            return []
        fail(
            f"aomi-build manifest failed for {plugin_path.name} (sdk {sdk_version}): "
            f"{stderr or 'no stderr captured'}"
        )

    output = (result.stdout or "").strip()
    try:
        manifest = json.loads(output)
    except json.JSONDecodeError as err:
        fail(
            f"aomi-build manifest for {plugin_path.name} (sdk {sdk_version}) produced "
            f"invalid JSON: {err}"
        )

    if not isinstance(manifest, dict):
        fail(
            f"aomi-build manifest for {plugin_path.name} (sdk {sdk_version}) returned "
            f"{type(manifest).__name__}, expected a JSON object"
        )

    secrets = manifest.get("secrets")
    if secrets is None:
        return []
    if not isinstance(secrets, list):
        fail(
            f"aomi-build manifest for {plugin_path.name} (sdk {sdk_version}): \"secrets\" "
            f"field is {type(secrets).__name__}, expected a list"
        )
    return secrets


def build_release(app_dir: pathlib.Path, ctx: dict[str, str], target: str, dist_root: pathlib.Path) -> dict[str, str]:
    info = load_deployment(app_dir, ctx, target)
    app_name = info["app_name"]
    package_name, lib_name = parse_cargo_manifest(app_dir)
    sdk_version = resolve_sdk_version(app_dir)
    expected_sdk_version = required_sdk_version()
    if sdk_version != expected_sdk_version:
        fail(
            f"{relpath(app_dir)} must pin aomi-sdk {expected_sdk_version}, "
            f"got {sdk_version}"
        )

    target_dir = REPO_ROOT / ".aomi-ci-target"
    run(
        [
            "cargo",
            "build",
            "--lib",
            "--release",
            "--target",
            target,
            "--target-dir",
            str(target_dir),
            "--manifest-path",
            str(app_dir / "Cargo.toml"),
        ],
        capture=False,
    )

    built_lib = target_dir / target / "release" / cargo_lib_name(lib_name, target)
    if not built_lib.is_file():
        fail(f"expected built library not found: {built_lib}")

    release_tag = info["release_tag"]
    dist_dir = dist_root / release_tag
    if dist_dir.exists():
        shutil.rmtree(dist_dir)
    plugins_dir = dist_dir / "plugins"
    plugins_dir.mkdir(parents=True)

    final_plugin = plugins_dir / plugin_file_name(app_name, target)
    shutil.copy2(built_lib, final_plugin)
    digest = sha256_file(final_plugin)
    entry: dict[str, Any] = {"file": final_plugin.name, "sha256": digest}
    secrets = read_plugin_secrets(final_plugin, sdk_version)
    if secrets:
        entry["secrets"] = secrets
    bundle_manifest = {
        "app_release_tag": release_tag,
        "sdk_version": sdk_version,
        "target": target,
        "commit": info["source_commit"],
        "plugins": {app_name: entry},
    }
    manifest_path = plugins_dir / "manifest.json"
    write_json(manifest_path, bundle_manifest)

    tarball = dist_dir / f"aomi-plugins-{release_tag}-{target}.tar.gz"
    with tarfile.open(tarball, "w:gz") as archive:
        archive.add(manifest_path, arcname="plugins/manifest.json")
        archive.add(final_plugin, arcname=f"plugins/{final_plugin.name}")
    verify_tarball(tarball, bundle_manifest)

    standalone_manifest = dist_dir / "manifest.json"
    shutil.copy2(manifest_path, standalone_manifest)
    release_metadata = {
        "schema_version": 1,
        "platform": info["platform"],
        "app": {
            "name": app_name,
            "path": info["app_path"],
            "package": package_name,
        },
        "source": {
            "repository": ctx["owner_repo"],
            "installation_id": int(info["installation_id"]),
            "commit": info["source_commit"],
        },
        "candidate": {
            "branch": ctx["branch"],
            "commit": current_commit(),
            "deployment_manifest": info["deployment_manifest"],
        },
        "release": {
            "tag": release_tag,
            "target": target,
            "sdk_version": sdk_version,
        },
        "assets": {
            "tarball": tarball.name,
            "manifest": "manifest.json",
            "metadata": "aomi-release.json",
        },
    }
    metadata_path = dist_dir / "aomi-release.json"
    write_json(metadata_path, release_metadata)
    notes = dist_dir / "release-notes.md"
    notes.write_text(
        "\n".join(
            [
                f"Aomi {info['platform']} candidate release.",
                "",
                f"- App: {app_name}",
                f"- Release: {release_tag}",
                f"- Source: {ctx['owner_repo']}@{info['source_commit']}",
                f"- Candidate: {ctx['branch']}@{current_commit()}",
                f"- SDK: {sdk_version}",
                f"- Target: {target}",
                "",
            ]
        ),
        encoding="utf-8",
    )
    return {
        "release_tag": release_tag,
        "tarball": relpath(tarball),
        "manifest": relpath(standalone_manifest),
        "metadata": relpath(metadata_path),
        "notes": relpath(notes),
    }


def verify_tarball(tarball: pathlib.Path, expected_manifest: dict[str, Any]) -> None:
    with tempfile.TemporaryDirectory(prefix="aomi-candidate-") as tmp:
        tmp_path = pathlib.Path(tmp)
        with tarfile.open(tarball, "r:gz") as archive:
            archive.extractall(tmp_path, filter="data")
        manifest = load_json(tmp_path / "plugins" / "manifest.json")
        if manifest != expected_manifest:
            fail("tarball manifest does not match generated manifest")
        for name, entry in manifest["plugins"].items():
            plugin = tmp_path / "plugins" / entry["file"]
            if not plugin.is_file():
                fail(f"tarball plugin missing for {name}")
            if sha256_file(plugin) != entry["sha256"]:
                fail(f"tarball plugin checksum mismatch for {name}")


def publish_release(bundle: dict[str, str]) -> None:
    release_tag = bundle["release_tag"]
    if not os.environ.get("GH_TOKEN"):
        fail("GH_TOKEN is required to publish candidate releases")
    assets = [bundle["tarball"], bundle["manifest"], bundle["metadata"]]
    existing = subprocess.run(
        ["gh", "release", "view", release_tag],
        cwd=REPO_ROOT,
        env=os.environ.copy(),
        text=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    if existing.returncode == 0:
        gh(["release", "upload", release_tag, *assets, "--clobber"], capture=False)
    else:
        gh(
            [
                "release",
                "create",
                release_tag,
                "--target",
                current_commit(),
                "--title",
                release_tag,
                "--notes-file",
                bundle["notes"],
                *assets,
            ],
            capture=False,
        )


def command_detect(args: argparse.Namespace) -> None:
    branch_context()
    dirs = changed_app_dirs(args.base, args.head)
    value = json.dumps(dirs)
    print(value)
    if args.github_output:
        with pathlib.Path(args.github_output).open("a", encoding="utf-8") as fh:
            fh.write(f"apps={value}\n")


def command_release(args: argparse.Namespace) -> None:
    ctx = branch_context()
    dirs = changed_app_dirs(args.base, args.head)
    if not dirs:
        print("No candidate app directories changed.")
        return
    dist_root = (REPO_ROOT / args.dist_dir).resolve()
    if dist_root.exists():
        shutil.rmtree(dist_root)
    dist_root.mkdir(parents=True)
    for app_dir in dirs:
        bundle = build_release(REPO_ROOT / app_dir, ctx, args.target, dist_root)
        publish_release(bundle)
        print(f"published {bundle['release_tag']}")


def main() -> None:
    parser = argparse.ArgumentParser(description="Build backend-deployed Aomi candidate apps")
    subparsers = parser.add_subparsers(dest="command", required=True)

    detect = subparsers.add_parser("detect")
    detect.add_argument("--base", required=True)
    detect.add_argument("--head", default="HEAD")
    detect.add_argument("--github-output")
    detect.set_defaults(func=command_detect)

    release = subparsers.add_parser("release")
    release.add_argument("--base", required=True)
    release.add_argument("--head", default="HEAD")
    release.add_argument("--target", default="x86_64-unknown-linux-gnu")
    release.add_argument("--dist-dir", default="dist")
    release.set_defaults(func=command_release)

    args = parser.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
