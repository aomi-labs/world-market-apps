"""Local regressions; no credentials, GitHub writes or database are used."""
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import build_candidate as candidate
import deploy_project as deploy

class SourcePackaging(unittest.TestCase):
    def write(self, root, files):
        for name, text in files.items():
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)

    def cargo(self, root, *args):
        return subprocess.run(["cargo", *args, "--offline"], cwd=root, capture_output=True, text=True)

    def fixture(self, root, workspace):
        if workspace:
            self.write(root, {
                "Cargo.toml": '[workspace]\nmembers=["apps/demo","libs/shared"]\nresolver="2"\n[workspace.package]\nversion="0.1.0"\n[workspace.dependencies]\nshared={path="libs/shared"}\n',
                "apps/demo/Cargo.toml": '[package]\nname="demo"\nversion.workspace=true\nedition="2021"\n[lib]\ncrate-type=["cdylib"]\n[dependencies]\nshared.workspace=true\n',
                "apps/demo/src/lib.rs": 'pub fn value() -> u32 { shared::value() }',
                "apps/demo/build.rs": 'fn main() { assert!(std::process::Command::new("../../scripts/check-build.sh").status().unwrap().success()); }',
                "scripts/check-build.sh": '#!/bin/sh\nexit 0\n',
                "apps/demo/aomi.toml": 'name="demo"',
                "libs/shared/Cargo.toml": '[package]\nname="shared"\nversion="0.1.0"\nedition="2021"',
                "libs/shared/src/lib.rs": 'pub fn value() -> u32 { 42 }',
                ".cargo/config.toml": '[build]\nrustflags=["--cfg", "snapshot_test"]',
            })
            (root / "scripts/check-build.sh").chmod(0o755)
            app = "apps/demo/aomi.toml"
        else:
            self.write(root, {"Cargo.toml": '[package]\nname="demo"\nversion="0.1.0"\nedition="2021"\n[lib]\ncrate-type=["cdylib"]', "aomi.toml": 'name="demo"', "src/lib.rs": 'pub fn value() -> u32 { 42 }'})
            app = "aomi.toml"
        generated = self.cargo(root, "generate-lockfile")
        self.assertEqual(generated.returncode, 0, generated.stderr)
        return app

    def test_workspace_and_standalone_build_outside_original_repository(self):
        for workspace in (True, False):
            with self.subTest(workspace=workspace), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp); original = root / "original"; original.mkdir()
                app = self.fixture(original, workspace)
                archive = root / "source.tar.gz"
                with tarfile.open(archive, "w:gz") as tar: tar.add(original, arcname="github-root")
                isolated = root / "isolated"
                deploy.extract_source(archive, isolated)
                app_dir = deploy.cargo_context(isolated, app)
                before = (isolated / "Cargo.lock").read_bytes()
                # Remove the original: parent workspace leakage cannot make this pass.
                import shutil; shutil.rmtree(original)
                checked = self.cargo(app_dir, "check", "--locked", "--lib")
                self.assertEqual(checked.returncode, 0, checked.stderr)
                self.assertEqual((isolated / "Cargo.lock").read_bytes(), before)

    def test_missing_and_stale_lockfiles_fail_without_regenerating(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); app = self.fixture(root, True)
            lock = (root / "Cargo.lock").read_bytes()
            manifest = root / "libs/shared/Cargo.toml"
            manifest.write_text(manifest.read_text().replace('0.1.0', '0.2.0'))
            checked = self.cargo(deploy.cargo_context(root, app), "check", "--locked", "--lib")
            self.assertNotEqual(checked.returncode, 0)
            self.assertEqual((root / "Cargo.lock").read_bytes(), lock)
            (root / "Cargo.lock").unlink()
            with self.assertRaisesRegex(RuntimeError, 'commit Cargo.lock'): deploy.cargo_context(root, app)

    def test_source_archive_rejects_links_and_escapes(self):
        for name, kind in [("root/../outside", tarfile.REGTYPE), ("root/link", tarfile.SYMTYPE)]:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as tmp:
                archive = Path(tmp) / "source.tar.gz"
                with tarfile.open(archive, "w:gz") as tar:
                    item = tarfile.TarInfo(name); item.type = kind; item.linkname = "/tmp/outside"
                    tar.addfile(item, io.BytesIO())
                with self.assertRaisesRegex(RuntimeError, "unsafe"): deploy.extract_source(archive, Path(tmp) / "out")

class RetryAndPublication(unittest.TestCase):
    def test_reuse_requires_complete_release_at_the_original_candidate(self):
        deployment = {"platform": {"repository": "aomi-labs/test", "commit_hash": "candidate", "apps": [{"name": "demo", "release_tag": "tag"}]}}
        release = {"assets": [{"name": name} for name in ["manifest.json", "aomi-release.json", f"aomi-plugins-tag-{deploy.TARGET}.tar.gz"]]}
        with patch.dict(os.environ, {"GH_TOKEN": "test"}), patch.object(deploy, "request", side_effect=[release, {"object": {"type": "commit", "sha": "candidate"}}]):
            self.assertEqual(deploy.release_plan(deployment), [{"name": "demo", "reuse": True}])
        with patch.dict(os.environ, {"GH_TOKEN": "test"}), patch.object(deploy, "request", side_effect=[release, {"object": {"type": "commit", "sha": "different"}}]):
            with self.assertRaisesRegex(RuntimeError, "conflicting immutable release"): deploy.release_plan(deployment)
        with patch.dict(os.environ, {"GH_TOKEN": "test"}), patch.object(deploy, "request", side_effect=RuntimeError("Deployment service returned HTTP 404")):
            self.assertEqual(deploy.release_plan(deployment), [{"name": "demo", "reuse": False}])

    def test_diagnostics_redact_credentials_but_keep_compilation_location(self):
        result = deploy.diagnostic("error[E0432] at src/lib.rs:4\nAuthorization: Bearer example\nhttps://user:password@host/path")
        self.assertIn("src/lib.rs:4", result)
        self.assertNotIn("example", result)
        self.assertNotIn("password", result)

    def test_trusted_jobs_never_execute_customer_build_outputs(self):
        workflow = (deploy.ROOT / ".github/workflows/deploy-project.yml").read_text()
        compile_job = workflow.split("  build:\n", 1)[1].split("  publish:\n", 1)[0]
        self.assertNotIn("secrets.", compile_job)
        self.assertNotIn("actions/checkout", compile_job)
        self.assertIn("permissions: {}", compile_job)
        self.assertIn("needs.prepare.outputs.metadata_id", workflow.split("  publish:\n", 1)[1])
        self.assertNotIn("ADMIN_PRIVATE_KEY", workflow)
        self.assertNotIn("AOMI_GITHUB_APP", workflow)


class PlatformLifecycle(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.work = Path(directory.name)
        self.source = "a" * 40
        self.candidate = "b" * 40
        self.deployment_id = "dep_12_r0123456789_" + self.source[:12]
        self.manifest_path = "apps/12/r0123456789/demo/.aomi/deployment.json"
        self.deployment = {
            "id": self.deployment_id,
            "sdk_version": "4.0.0",
            "source": {"project_id": 42, "commit_hash": self.source},
            "platform": {"repository": "partner/apps", "platform": "partner", "commit_hash": None,
                         "apps": [{"name": "demo", "release_tag": "apps-12-r0123456789-demo-" + self.source[:12]}]},
        }
        self.run = {"id": 20, "created_at": "2026-09-08T00:00:01Z", "updated_at": "2026-09-08T00:00:01Z",
                    "display_title": "aomi-deploy|staging|42|" + self.source + "|main|", "status": "in_progress"}
        variables = {"GH_TOKEN": "fixture-github", "AOMI_PLATFORM_TOKEN": "fixture-platform",
                     "GITHUB_RUN_ID": "20", "GITHUB_REPOSITORY": "partner/apps", "PROJECT_ID": "42",
                     "SOURCE_REF": self.source, "CANDIDATE_REF": self.candidate, "DEPLOYMENT_ID": self.deployment_id,
                     "DEPLOYMENT_PATH": self.manifest_path, "DEPLOY_ENVIRONMENT": "staging",
                     "GITHUB_OUTPUT": str(self.work / "outputs")}
        for replacement in (patch.object(deploy, "WORK", self.work), patch.dict(os.environ, variables)):
            replacement.start()
            self.addCleanup(replacement.stop)
        manifest = self.work / "candidate" / self.manifest_path
        manifest.parent.mkdir(parents=True)
        manifest.write_text(json.dumps(self.deployment))
        self.snapshot = self.work / "candidate" / ".aomi/candidates" / self.deployment_id / "source.tar.gz"
        self.snapshot.parent.mkdir(parents=True)
        with tarfile.open(self.snapshot, "w:gz") as archive:
            content = b'[workspace]\nmembers=[]\n'
            member = tarfile.TarInfo("source/Cargo.toml"); member.size = len(content)
            archive.addfile(member, io.BytesIO(content))
        (self.work / "deployment.json").write_text(json.dumps(self.deployment))

    def test_prepare_uses_the_platform_snapshot_without_source_credentials_or_backend_writes(self):
        with patch.object(deploy, "request", side_effect=[{"workflow_runs": [self.run]}, RuntimeError("Deployment service returned HTTP 404")]) as request:
            deploy.prepare()
        self.assertEqual((self.work / "source.tar.gz").read_bytes(), self.snapshot.read_bytes())
        self.assertEqual(deploy.read_deployment()["platform"]["commit_hash"], self.candidate)
        self.assertEqual(json.loads((self.work / "plan.json").read_text()), [{"name": "demo", "reuse": False}])
        for call in request.call_args_list:
            self.assertEqual(call.args[0], "GET")
            self.assertTrue(call.args[1].startswith("https://api.github.com/repos/partner/apps/"))
            self.assertEqual(call.args[2], "fixture-github")

    def test_prepare_rejects_a_candidate_from_another_platform(self):
        manifest = self.work / "candidate" / self.manifest_path
        self.deployment["platform"]["repository"] = "other/platform"
        manifest.write_text(json.dumps(self.deployment))
        with patch.object(deploy, "request", return_value={"workflow_runs": [self.run]}):
            with self.assertRaisesRegex(RuntimeError, "authorized request"):
                deploy.prepare()

    def test_published_legacy_candidate_can_reuse_releases_without_a_snapshot(self):
        self.snapshot.unlink()
        with patch.object(deploy, "request", return_value={"workflow_runs": [self.run]}), patch.object(deploy, "release_plan", return_value=[{"name": "demo", "reuse": True}]):
            deploy.prepare()
        self.assertFalse((self.work / "source.tar.gz").exists())
        with patch.object(deploy, "request", return_value={"workflow_runs": [self.run]}), patch.object(deploy, "release_plan", return_value=[{"name": "demo", "reuse": False}]):
            with self.assertRaisesRegex(RuntimeError, "Push a new source commit"):
                deploy.prepare()

    def test_publication_rechecks_releases_and_never_overwrites_them(self):
        with patch.object(deploy, "release_plan", return_value=[{"name": "demo", "reuse": True}]) as releases, patch.object(deploy.subprocess, "run") as command:
            deploy.publish()
        releases.assert_called_once_with(self.deployment)
        command.assert_not_called()
        with patch.object(deploy, "release_plan", side_effect=RuntimeError("conflicting immutable release")), patch.object(deploy.subprocess, "run") as command:
            with self.assertRaisesRegex(RuntimeError, "conflicting immutable release"):
                deploy.publish()
        command.assert_not_called()

    def test_queued_duplicate_does_not_reactivate_after_earlier_run_finishes(self):
        prior = {**self.run, "id": 19, "updated_at": "2026-09-08T00:00:02Z", "status": "completed"}
        with patch.object(deploy, "request", return_value={"workflow_runs": [self.run, prior]}) as request:
            deploy.prepare()
        self.assertIn("skip=true", (self.work / "outputs").read_text())
        self.assertEqual(request.call_count, 1)
        self.assertFalse((self.work / "source.tar.gz").exists())

    def test_older_run_for_a_different_commit_is_not_a_duplicate(self):
        prior = {**self.run, "id": 19, "updated_at": "2026-09-08T00:00:02Z", "status": "completed",
                 "display_title": "aomi-deploy|staging|42|" + "c" * 40 + "|main|"}
        with patch.object(deploy, "request", side_effect=[{"workflow_runs": [self.run, prior]}, RuntimeError("Deployment service returned HTTP 404")]):
            deploy.prepare()
        self.assertIn("skip=false", (self.work / "outputs").read_text())
        self.assertTrue((self.work / "source.tar.gz").exists())

    def test_build_normalizes_the_source_repository_link_for_the_identity_check(self):
        self.deployment["source"].update({"repository_link": "github.com/Partner/Apps", "installation_id": 12})
        self.deployment["platform"].update({"platform_branch": "partner/apps/12/" + self.source[:12]})
        self.deployment["platform"]["apps"][0].update({"aomi_toml_path": "aomi.toml", "path": "apps/12/r0123456789/demo"})
        (self.work / "deployment.json").write_text(json.dumps(self.deployment))
        with tarfile.open(self.work / "source.tar.gz", "w:gz") as archive:
            for name, content in {"source/Cargo.toml": b'[workspace]\nmembers=[]\n', "source/Cargo.lock": b'', "source/aomi.toml": b'name="demo"'}.items():
                member = tarfile.TarInfo(name); member.size = len(content)
                archive.addfile(member, io.BytesIO(content))
        with patch.dict(os.environ, {"BUILD_APP": "demo"}), patch.object(deploy, "ROOT", self.work / "root"), patch.object(candidate, "build_release") as build_release:
            deploy.build()
        ctx = build_release.call_args.args[1]
        self.assertEqual(ctx["owner_repo"], "partner/apps")
        self.assertEqual(ctx["short_commit"], self.source[:12])

    def test_verify_rejects_an_app_outside_the_deployment(self):
        with patch.dict(os.environ, {"VERIFY_APP": "other"}), patch.object(deploy, "api") as api:
            with self.assertRaisesRegex(RuntimeError, "not part of this deployment"):
                deploy.verify()
        api.assert_not_called()

    def test_activation_uses_existing_platform_token_and_cancellation_prevents_write(self):
        with patch.object(deploy, "request", side_effect=[self.run, {"ok": True, "activation": {"apps": [{}]}}]) as request:
            deploy.activate()
        activation = request.call_args_list[1]
        self.assertEqual(activation.args[0], "POST")
        self.assertEqual(activation.args[1], "https://api-staging.aomi.dev/api/platforms/partner/apps/activate")
        self.assertEqual(activation.args[2], "fixture-platform")
        self.assertEqual(activation.args[3]["target"]["value"], self.deployment_id)
        with patch.object(deploy, "request", return_value={"status": "completed", "conclusion": "cancelled"}) as request:
            with self.assertRaisesRegex(RuntimeError, "no longer active"):
                deploy.activate()
        self.assertEqual(request.call_count, 1)

    def test_missing_platform_token_is_actionable(self):
        with patch.dict(os.environ, {"AOMI_PLATFORM_TOKEN": ""}):
            with self.assertRaisesRegex(RuntimeError, "Configure AOMI_PLATFORM_TOKEN"):
                deploy.api("GET", "/api/platforms/partner/apps/demo")

    def test_verify_retries_transient_reads_and_requires_exact_artifact_readiness(self):
        app = {"name": "demo", "project_id": 42, "is_active": True, "artifact_ready": True,
               "app_release_tag": self.deployment["platform"]["apps"][0]["release_tag"]}
        with patch.dict(os.environ, {"VERIFY_APP": "demo"}), patch.object(deploy, "api", side_effect=[
            RuntimeError("Deployment service is unreachable"), {"app": {**app, "artifact_ready": False}},
            {"app": {**app, "app_release_tag": "older-release"}}, {"app": app},
        ]) as api, patch.object(deploy.time, "sleep") as sleep:
            deploy.verify()
        self.assertEqual(api.call_count, 4)
        self.assertEqual([call.args[0] for call in sleep.call_args_list], [4, 5, 5])

    def test_verify_stops_after_bounded_reconnection_failures(self):
        with patch.dict(os.environ, {"VERIFY_APP": "demo"}), patch.object(deploy, "api", side_effect=RuntimeError("Deployment service is unreachable")) as api, patch.object(deploy.time, "sleep") as sleep:
            with self.assertRaisesRegex(RuntimeError, "unreachable"):
                deploy.verify()
        self.assertEqual(api.call_count, 5)
        self.assertEqual([call.args[0] for call in sleep.call_args_list], [4, 8, 16, 30])

if __name__ == '__main__': unittest.main()
