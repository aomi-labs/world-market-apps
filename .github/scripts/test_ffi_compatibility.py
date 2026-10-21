from dataclasses import replace
import hashlib
import io
import json
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).parent))
import build_candidate as build
import deploy_project as deploy


class FfiCompatibility(unittest.TestCase):
    def test_compiled_metadata_is_independent_of_sdk_release(self):
        def parse(manifest, sdk_version):
            return build.PluginManifest.parse(manifest, "x.so", sdk_version).ffi_version

        self.assertEqual(parse({"sdk_version": "99.0.0", "ffi_version": 1}, "99.0.0"), 1)
        self.assertEqual(parse({"sdk_version": "5.1.1"}, "5.1.1"), 1)
        self.assertIsNone(parse({"sdk_version": "3.0.1"}, "3.0.1"))
        for value in (True, "1", 0, -1):
            with self.subTest(value=value), self.assertRaises(SystemExit):
                parse({"sdk_version": "99.0.0", "ffi_version": value}, "99.0.0")
        with self.assertRaises(SystemExit):
            parse({"sdk_version": "other", "ffi_version": 1}, "99.0.0")
        with self.assertRaises(SystemExit):
            parse({"sdk_version": "99.0.0", "ffi_version": 1, "required_interfaces": ["future_permission_v2"]}, "99.0.0")

    def test_publisher_accepts_cross_sdk_bundle_but_preserves_integrity_gates(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = b"compiled fixture"
            app = deploy.App("demo", "apps-fixture", "apps/12/r0123456789/demo", "aomi.toml")
            deployment = deploy.Deployment(
                id="dep_12_r0123456789_" + "a" * 12, project_id=42, source_commit="a" * 40,
                repository_link="https://github.com/partner/apps", installation_id=12, platform="partner",
                repository="partner/apps", branch="partner/apps/12/" + "a" * 12, candidate_commit="b" * 40,
                sdk_version="6.0.0", ffi_version=1, apps=(app,))
            manifest = {"sdk_version": "5.1.1", "ffi_version": 1, "target": deploy.TARGET,
                        "plugins": {"demo": {"file": "demo.so", "sha256": hashlib.sha256(binary).hexdigest()}}}
            metadata = {"source": {"commit": "a" * 40}, "candidate": {"commit": "b" * 40},
                        "release": {"tag": "apps-fixture"}, "app": {"name": "demo"}}
            (root / "aomi-release.json").write_text(json.dumps(metadata))
            with tarfile.open(root / f"aomi-plugins-apps-fixture-{deploy.TARGET}.tar.gz", "w:gz") as archive:
                for name, data in (("plugins/manifest.json", json.dumps(manifest).encode()), ("plugins/demo.so", binary)):
                    member = tarfile.TarInfo(name)
                    member.size = len(data)
                    archive.addfile(member, io.BytesIO(data))
            bundle = deploy.BundleManifest.parse(manifest)
            deployment.verify_bundle(root, app, bundle)
            with self.assertRaisesRegex(RuntimeError, "FFI compatibility"):
                replace(deployment, ffi_version=2).verify_bundle(root, app, bundle)
            with self.assertRaisesRegex(RuntimeError, "FFI compatibility"):
                replace(deployment, ffi_version=None).verify_bundle(root, app, bundle)
            with self.assertRaisesRegex(RuntimeError, "FFI compatibility"):
                deployment.verify_bundle(root, app, deploy.BundleManifest.parse({**manifest, "ffi_version": True}))
            corrupted = json.loads(json.dumps(manifest))
            corrupted["plugins"]["demo"]["sha256"] = "0" * 64
            with self.assertRaisesRegex(RuntimeError, "manifests do not agree|checksum"):
                deployment.verify_bundle(root, app, deploy.BundleManifest.parse(corrupted))


if __name__ == "__main__":
    unittest.main()
