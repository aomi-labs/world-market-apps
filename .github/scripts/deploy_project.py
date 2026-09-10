#!/usr/bin/env python3
"""Platform-owned deployment lifecycle. Only `build` executes app code.

The platform candidate contains the complete source snapshot. The platform's
GitHub token publishes releases; its existing Aomi token activates them.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import tarfile
import time
import tomllib
import urllib.error
import urllib.parse
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
WORK = ROOT / ".deployment"
TARGET = "x86_64-unknown-linux-gnu"


def diagnostic(message: str) -> str:
    lines = []
    for line in message.splitlines():
        if re.search(r"token|secret|password|authorization|private.?key|https?://|-----BEGIN", line, re.I):
            line = "[sensitive diagnostic line omitted]"
        line = re.sub(r"\S{81,}", "[redacted]", line)
        lines.append(line[:300])
    return "\n".join(lines[-20:])[:3000]


def annotation(message: str) -> None:
    text = diagnostic(message).replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
    print(f"::error title=Deployment failed::{text}")


def request(method: str, url: str, token: str, body=None):
    headers = {"Authorization": f"Bearer {token}", "User-Agent": "aomi-project-deployment", "Accept": "application/json"}
    if body is not None:
        headers["Content-Type"] = "application/json"
    req = urllib.request.Request(url, data=None if body is None else json.dumps(body).encode(), headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=90) as response:
            payload = response.read(64 * 1024 * 1024 + 1)
            if len(payload) > 64 * 1024 * 1024:
                raise RuntimeError("Response exceeds the deployment size limit")
    except urllib.error.HTTPError as error:
        # Neither upstream response bodies nor authenticated URLs belong in logs.
        if error.code == 409 and "/apps/activate" in url:
            raise RuntimeError("Activation needs attention. Review required configuration in the Environment tab, then retry deployment.") from None
        raise RuntimeError(f"Deployment service returned HTTP {error.code}") from None
    except urllib.error.URLError:
        raise RuntimeError("Deployment service is unreachable") from None
    return json.loads(payload) if payload else {}


def backend() -> str:
    return "https://api.aomi.dev" if os.environ["DEPLOY_ENVIRONMENT"] == "production" else "https://api-staging.aomi.dev"


def api(method: str, path: str, body=None):
    token = os.environ.get("AOMI_PLATFORM_TOKEN", "").strip()
    if not token:
        raise RuntimeError("Configure AOMI_PLATFORM_TOKEN in this platform's GitHub environment before activation")
    return request(method, backend() + path, token, body)


def output(name: str, value: str) -> None:
    if "\n" in value or "\r" in value:
        raise RuntimeError("Invalid workflow output")
    with open(os.environ["GITHUB_OUTPUT"], "a") as file:
        file.write(f"{name}={value}\n")


def read_deployment():
    return json.loads((WORK / "deployment.json").read_text())


def validate_inputs():
    if not re.fullmatch(r"[1-9][0-9]*", os.environ["PROJECT_ID"]):
        raise RuntimeError("Invalid project id")
    if not re.fullmatch(r"[0-9a-f]{40}", os.environ["SOURCE_REF"]):
        raise RuntimeError("Source must be an immutable commit")
    if os.environ["DEPLOY_ENVIRONMENT"] not in ("production", "staging"):
        raise RuntimeError("Invalid deployment environment")
    if not re.fullmatch(r"[0-9a-f]{40}", os.environ["CANDIDATE_REF"]):
        raise RuntimeError("Candidate must be an immutable commit")
    if not re.fullmatch(r"dep_[0-9]+_r[0-9a-f]{10}_[0-9a-f]{7,40}", os.environ["DEPLOYMENT_ID"]):
        raise RuntimeError("Invalid deployment identity")
    if not re.fullmatch(r"apps/[0-9]+/r[0-9a-f]{10}/[a-z0-9_-]+/\.aomi/deployment\.json", os.environ["DEPLOYMENT_PATH"]):
        raise RuntimeError("Invalid deployment manifest path")


def prepare():
    validate_inputs()
    WORK.mkdir(exist_ok=True)
    project = os.environ["PROJECT_ID"]
    # A duplicate which arrived while the preceding attempt was active must not
    # deploy later merely because Actions held it in its concurrency queue.
    repo = os.environ["GITHUB_REPOSITORY"]
    runs = request("GET", f"https://api.github.com/repos/{repo}/actions/workflows/deploy-project.yml/runs?event=workflow_dispatch&per_page=100", os.environ["GH_TOKEN"])["workflow_runs"]
    current = next((run for run in runs if str(run["id"]) == os.environ["GITHUB_RUN_ID"]), None)
    # Only a same-commit request is a duplicate; an older run for another SHA is a distinct deployment.
    prefix = f"aomi-deploy|{os.environ['DEPLOY_ENVIRONMENT']}|{project}|{os.environ['SOURCE_REF']}|"
    duplicate = current is not None and any(run["id"] < current["id"] and run.get("display_title", "").startswith(prefix)
                                            and run["updated_at"] > current["created_at"] for run in runs)
    output("skip", "true" if duplicate else "false")
    if duplicate:
        print("Another attempt already owns this deployment request.")
        return
    deployment = json.loads((WORK / "candidate" / os.environ["DEPLOYMENT_PATH"]).read_text())
    if (deployment["source"]["commit_hash"] != os.environ["SOURCE_REF"]
        or deployment["source"]["project_id"] != int(project)
        or deployment["id"] != os.environ["DEPLOYMENT_ID"]
        or deployment["platform"]["repository"].lower() != repo.lower()):
        raise RuntimeError("Deployment source does not match the authorized request")
    deployment["platform"]["commit_hash"] = os.environ["CANDIDATE_REF"]
    (WORK / "deployment.json").write_text(json.dumps(deployment))
    if not deployment["platform"]["apps"]:
        raise RuntimeError("No apps are configured. Add an app to .aomi/config.json before deploying.")
    plan = release_plan(deployment)
    if any(not app["reuse"] for app in plan):
        snapshot = WORK / "candidate" / ".aomi/candidates" / deployment["id"] / "source.tar.gz"
        if not snapshot.is_file():
            raise RuntimeError("This older candidate has published releases but no complete source snapshot. Push a new source commit and retry to rebuild the remaining apps.")
        shutil.copyfile(snapshot, WORK / "source.tar.gz")
    (WORK / "plan.json").write_text(json.dumps(plan))
    output("matrix", json.dumps({"include": plan}))


def release_plan(deployment):
    """Reuse only complete, immutable releases for this exact candidate."""
    repo = deployment["platform"]["repository"]
    token = os.environ["GH_TOKEN"]
    result = []
    for app in deployment["platform"]["apps"]:
        tag = app["release_tag"]
        reused = False
        try:
            release = request("GET", f"https://api.github.com/repos/{repo}/releases/tags/{tag}", token)
            ref = request("GET", f"https://api.github.com/repos/{repo}/git/ref/tags/{tag}", token)["object"]
            for _ in range(4):
                if ref["type"] != "tag": break
                ref = request("GET", f"https://api.github.com/repos/{repo}/git/tags/{ref['sha']}", token)["object"]
            expected = {"manifest.json", "aomi-release.json", f"aomi-plugins-{tag}-{TARGET}.tar.gz"}
            reused = (ref["type"] == "commit" and ref["sha"] == deployment["platform"]["commit_hash"]
                      and expected <= {asset["name"] for asset in release["assets"]} and not release.get("draft"))
            if not reused:
                raise RuntimeError(f"{app['name']}: an incomplete or conflicting immutable release exists; repair the release before retrying")
        except RuntimeError as error:
            if str(error) != "Deployment service returned HTTP 404": raise
        result.append({"name": app["name"], "reuse": reused})
    return result


def extract_source(archive_path: Path, destination: Path):
    destination.mkdir(parents=True, exist_ok=True)
    total = 0
    with tarfile.open(archive_path, "r:gz") as archive:
        for index, member in enumerate(archive):
            if index >= 10000:
                raise RuntimeError("Source archive has too many entries")
            path = PurePosixPath(member.name)
            if path.is_absolute() or ".." in path.parts or member.issym() or member.islnk():
                raise RuntimeError("Source archive contains unsafe paths or links")
            if member.isdir():
                continue
            if not member.isfile() or len(path.parts) < 2:
                raise RuntimeError("Source archive contains unsupported entries")
            total += member.size
            if member.size > 10 * 1024 * 1024 or total > 256 * 1024 * 1024:
                raise RuntimeError("Source archive exceeds the build size limit")
            target = destination.joinpath(*path.parts[1:])
            target.parent.mkdir(parents=True, exist_ok=True)
            if target.exists():
                raise RuntimeError("Source archive contains duplicate paths")
            with archive.extractfile(member) as source, target.open("wb") as output_file:
                shutil.copyfileobj(source, output_file)
            target.chmod(0o755 if member.mode & 0o111 else 0o644)


def cargo_context(source: Path, app_manifest: str) -> Path:
    app = (source / app_manifest).parent.resolve()
    if not app.is_relative_to(source.resolve()):
        raise RuntimeError("Application path escapes the source snapshot")
    manifest = tomllib.loads((app / "Cargo.toml").read_text())
    explicit = manifest.get("package", {}).get("workspace")
    candidates = [app / explicit] if isinstance(explicit, str) else [app, *app.parents]
    workspace = app
    for parent in candidates:
        parent = parent.resolve()
        if not parent.is_relative_to(source.resolve()):
            if explicit:
                raise RuntimeError("Workspace escapes the source snapshot")
            break
        root_manifest = parent / "Cargo.toml"
        if root_manifest.exists() and "workspace" in tomllib.loads(root_manifest.read_text()):
            workspace = parent
            break
    if not (workspace / "Cargo.lock").is_file():
        raise RuntimeError(f"{app_manifest}: commit Cargo.lock at the Cargo workspace root before deploying")
    return app


def build():
    # Only the build job executes app code; activation credentials stay in later jobs.
    import build_candidate as candidate
    deployment = read_deployment()
    source = WORK / "source"
    extract_source(WORK / "source.tar.gz", source)
    ctx = {"owner_repo": candidate.normalize_repo(deployment["source"]["repository_link"]), "installation_id": str(deployment["source"]["installation_id"]),
           "short_commit": deployment["source"]["commit_hash"][:12], "branch": deployment["platform"]["platform_branch"],
           "platform": deployment["platform"]["platform"], "sdk_version": deployment["sdk_version"]}
    # The deployment's branch is the canonical authority for short SHA length.
    ctx["short_commit"] = ctx["branch"].rsplit("/", 1)[1]
    candidate.current_commit = lambda: deployment["platform"]["commit_hash"]
    failures = []
    for app in deployment["platform"]["apps"]:
        if app["name"] != os.environ["BUILD_APP"]: continue
        try:
            source_dir = cargo_context(source, app["aomi_toml_path"])
            app_dir = ROOT / app["path"]
            app_dir.mkdir(parents=True, exist_ok=True)
            (app_dir / ".aomi").mkdir(exist_ok=True)
            (app_dir / ".aomi/deployment.json").write_text(json.dumps(deployment))
            print(f"Building app: {app['name']}", flush=True)
            candidate.build_release(app_dir, ctx, TARGET, WORK / "dist", source_dir)
        except (Exception, SystemExit) as error:
            failures.append(app["name"])
            annotation(f"{app['name']}: {error}")
    if failures:
        raise RuntimeError("Build failed for: " + ", ".join(failures))


def publish():
    deployment = read_deployment()
    repo = deployment["platform"]["repository"]
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo):
        raise RuntimeError("Invalid platform repository")
    token = os.environ["GH_TOKEN"]
    env = {**os.environ, "GH_TOKEN": token}
    # Recheck at publication as another run or operator may have created a
    # release since validation. Never accept an incomplete/conflicting tag.
    reused = {app["name"] for app in release_plan(deployment) if app["reuse"]}
    for app in deployment["platform"]["apps"]:
        if app["name"] in reused: continue
        tag = app["release_tag"]
        if not re.fullmatch(r"apps-[0-9]+-r[0-9a-f]{10}-[a-z0-9_-]+-[0-9a-f]{7,40}", tag):
            raise RuntimeError("Invalid release tag")
        directory = WORK / "dist" / tag
        manifest_path = directory / "manifest.json"
        manifest = json.loads(manifest_path.read_text())
        if manifest.get("app_release_tag") != tag or manifest.get("commit") != deployment["source"]["commit_hash"] or set(manifest.get("plugins", {})) != {app["name"]}:
            raise RuntimeError("Build output does not match the authorized deployment")
        verify_bundle(directory, deployment, app, manifest)
        # Upload only fixed asset names, never a path or command from customer output.
        assets = [directory / f"aomi-plugins-{tag}-{TARGET}.tar.gz", manifest_path, directory / "aomi-release.json"]
        result = subprocess.run(["gh", "release", "create", tag, "--repo", repo, "--target", deployment["platform"]["commit_hash"], "--title", tag, *map(str, assets)], env=env, capture_output=True, text=True)
        if result.returncode:
            raise RuntimeError(f"Could not publish release for {app['name']}\n{diagnostic(result.stderr[-2000:])}")


def verify_bundle(directory, deployment, app, manifest):
    # Customer compilation can change any build output. Parse data only here;
    # never extract or execute it on this credential-bearing runner.
    if manifest.get("sdk_version") != deployment["sdk_version"] or manifest.get("target") != TARGET:
        raise RuntimeError("Release SDK or target does not match the deployment")
    entry = manifest["plugins"][app["name"]]
    # The established release format permits the generated plugin basename.
    name = entry.get("file", "")
    if not re.fullmatch(r"[A-Za-z0-9_-]+\.so", name):
        raise RuntimeError("Invalid plugin filename")
    expected = {"plugins/manifest.json", f"plugins/{name}"}
    archive_path = directory / f"aomi-plugins-{app['release_tag']}-{TARGET}.tar.gz"
    with tarfile.open(archive_path, "r:gz") as archive:
        members = list(archive)
        if len(members) != 2 or {item.name for item in members} != expected:
            raise RuntimeError("Unexpected release archive entries")
        for item in members:
            if not item.isfile() or item.size > 256 * 1024 * 1024:
                raise RuntimeError("Unsafe release archive entry")
            with archive.extractfile(item) as file:
                if item.name.endswith("/manifest.json"):
                    if item.size > 1024 * 1024 or json.load(file) != manifest:
                        raise RuntimeError("Release manifests do not agree")
                elif hashlib.file_digest(file, "sha256").hexdigest() != entry.get("sha256"):
                    raise RuntimeError("Plugin checksum mismatch")
    metadata = json.loads((directory / "aomi-release.json").read_text())
    if (metadata.get("source", {}).get("commit") != deployment["source"]["commit_hash"]
        or metadata.get("candidate", {}).get("commit") != deployment["platform"]["commit_hash"]
        or metadata.get("release", {}).get("tag") != app["release_tag"]
        or metadata.get("app", {}).get("name") != app["name"]):
        raise RuntimeError("Release provenance does not match the deployment")


def activate():
    deployment = read_deployment()
    # Recheck cancellation immediately before the irreversible activation call.
    run = request("GET", f"https://api.github.com/repos/{os.environ['GITHUB_REPOSITORY']}/actions/runs/{os.environ['GITHUB_RUN_ID']}", os.environ["GH_TOKEN"])
    if run.get("conclusion") == "cancelled" or run["status"] != "in_progress":
        raise RuntimeError("Deployment is no longer active")
    apps = deployment["platform"]["apps"]
    platform = urllib.parse.quote(deployment["platform"]["platform"], safe="")
    result = api("POST", f"/api/platforms/{platform}/apps/activate", {"target": {"kind": "deployment", "value": deployment["id"]}, "apps": [app["name"] for app in apps], "actor": f"github-run:{os.environ['GITHUB_RUN_ID']}"})
    if not result.get("ok") or any(app.get("error") for app in result.get("activation", {}).get("apps", [])):
        raise RuntimeError("Activation was not accepted for every app; inspect required configuration and retry")


def verify():
    deployment = read_deployment()
    expected = {app["name"]: app["release_tag"] for app in deployment["platform"]["apps"] if app["name"] == os.environ["VERIFY_APP"]}
    if not expected:
        raise RuntimeError("VERIFY_APP is not part of this deployment")
    platform = urllib.parse.quote(deployment["platform"]["platform"], safe="")
    deadline = time.monotonic() + 8 * 60
    failures = 0
    ready = set()
    while time.monotonic() < deadline:
        try:
            result = {"apps": [api("GET", f"/api/platforms/{platform}/apps/{urllib.parse.quote(name, safe='')}?release_tag={urllib.parse.quote(tag, safe='')}")["app"] for name, tag in expected.items()]}
            failures = 0
        except RuntimeError as error:
            if "HTTP 4" in str(error) or failures >= 4: raise
            failures += 1
            print(f"Runtime status temporarily unavailable; reconnecting ({failures}/4)", flush=True)
            time.sleep(min(4 * 2 ** (failures - 1), 30))
            continue
        ready = {app["name"] for app in result["apps"] if app.get("project_id") == deployment["source"]["project_id"]
                 and app.get("is_active") and app.get("artifact_ready") is True and expected.get(app["name"]) == app.get("app_release_tag")}
        if ready == set(expected):
            print("All targeted apps report the expected release ready.")
            return
        print("Waiting for runtime: " + ", ".join(sorted(set(expected) - ready)), flush=True)
        time.sleep(5)
    raise RuntimeError("Runtime verification timed out for " + ", ".join(sorted(set(expected) - ready)) + ". Retry or roll back these apps.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("stage", choices=["prepare", "build", "publish", "activate", "verify"])
    args = parser.parse_args()
    try:
        globals()[args.stage]()
    except (Exception, SystemExit) as error:
        annotation(str(error))
        raise SystemExit(1)
