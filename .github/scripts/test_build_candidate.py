import json, pathlib, subprocess, sys, tempfile, unittest
sys.path.insert(0, str(pathlib.Path(__file__).parent))
import build_candidate as bc


class PlatformConfigTests(unittest.TestCase):
    def setUp(self):
        self.temp_dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp_dir.cleanup)
        self.addCleanup(setattr, bc, "REPO_ROOT", bc.REPO_ROOT)
        self.addCleanup(setattr, bc, "_PLATFORM_CONFIG", bc._PLATFORM_CONFIG)
        bc.REPO_ROOT = pathlib.Path(self.temp_dir.name)
        bc._PLATFORM_CONFIG = None

    def write_config(self, **overrides):
        config = {
            "name": "world-market-apps",
            "required_sdk_version": "3.1.0",
            **overrides,
        }
        (bc.REPO_ROOT / "platform.json").write_text(json.dumps(config))

    def test_reads_platform_contract(self):
        self.write_config()
        self.assertEqual(bc.platform_name(), "world-market-apps")
        self.assertEqual(bc.required_sdk_version(), "3.1.0")

    def test_rejects_missing_platform_name(self):
        self.write_config(name="")
        with self.assertRaises(SystemExit):
            bc.platform_name()

    def test_rejects_missing_required_sdk_version(self):
        self.write_config(required_sdk_version="")
        with self.assertRaises(SystemExit):
            bc.required_sdk_version()


def _completed(*, returncode: int = 0, stdout: str = "", stderr: str = "") -> subprocess.CompletedProcess[str]:
    return subprocess.CompletedProcess(args=["aomi-build", "manifest"], returncode=returncode, stdout=stdout, stderr=stderr)


class ReadPluginSecretsTests(unittest.TestCase):
    def setUp(self):
        # Every test overrides these before calling read_plugin_secrets; restore
        # the real functions afterwards so tests can't leak into each other.
        self.addCleanup(setattr, bc, "run", bc.run)
        self.addCleanup(setattr, bc, "run_capture", bc.run_capture)
        # Default: cargo install "succeeds" (most tests only care about the
        # `aomi-build manifest` step). Tests that need to simulate a cargo
        # install failure override this explicitly.
        bc.run = lambda cmd, **kw: ""

    def test_returns_slots_from_the_manifest_command(self):
        bc.run_capture = lambda cmd, **kw: _completed(
            stdout=json.dumps(
                {"name": "binance",
                 "secrets": [{"name": "BINANCE_API_KEY", "description": "d", "required": True}]}
            )
        )
        slots = bc.read_plugin_secrets(pathlib.Path("/tmp/libbinance.so"), "3.0.2")
        self.assertEqual(slots[0]["name"], "BINANCE_API_KEY")

    def test_returns_empty_when_the_sdk_lacks_the_subcommand(self):
        # Real-world failure path for an older SDK: clap rejects `manifest` as
        # an unrecognized subcommand. This is the ONE legitimate reason to
        # fall back to [] -- the app simply can't be secret-gated with this
        # SDK version.
        bc.run_capture = lambda cmd, **kw: _completed(
            returncode=2, stderr="error: unrecognized subcommand 'manifest'"
        )
        self.assertEqual(bc.read_plugin_secrets(pathlib.Path("/tmp/x.so"), "3.0.1"), [])

    def test_returns_empty_when_the_manifest_has_no_secrets(self):
        bc.run_capture = lambda cmd, **kw: _completed(stdout=json.dumps({"name": "hello"}))
        self.assertEqual(bc.read_plugin_secrets(pathlib.Path("/tmp/x.so"), "3.0.2"), [])

    def test_raises_when_cargo_install_fails(self):
        # A flaky/transient `cargo install` (network hiccup, crates.io blip,
        # etc.) must fail the build, not silently return []. run() raises
        # SystemExit via fail() on a non-zero exit.
        def boom(cmd, **kw):
            raise SystemExit("error: command failed (cargo install ...): connection reset")
        bc.run = boom
        with self.assertRaises(SystemExit):
            bc.read_plugin_secrets(pathlib.Path("/tmp/x.so"), "3.0.2")

    def test_raises_on_transient_manifest_failure(self):
        # A non-zero exit from `aomi-build manifest` that is NOT the
        # unrecognized-subcommand case (e.g. a panic, OOM, transient IO
        # error) must fail the build rather than silently return [].
        bc.run_capture = lambda cmd, **kw: _completed(
            returncode=1, stderr="thread 'main' panicked at 'unexpected plugin ABI'"
        )
        with self.assertRaises(SystemExit):
            bc.read_plugin_secrets(pathlib.Path("/tmp/x.so"), "3.0.2")

    def test_raises_on_flag_level_rejection_even_though_it_says_unexpected_argument(self):
        # Regression for the P1 finding: an SDK whose `manifest` subcommand
        # DOES exist but rejects our `--lib` flag is a real manifest-contract
        # bug -- clap's message contains the substring "unexpected argument",
        # but it names `--lib`, not `manifest`. This must fail the build, not
        # be misclassified as "old SDK, fall back to []" (which would
        # silently publish a release with no secret metadata).
        bc.run_capture = lambda cmd, **kw: _completed(
            returncode=2, stderr="error: unexpected argument '--lib' found"
        )
        with self.assertRaises(SystemExit):
            bc.read_plugin_secrets(pathlib.Path("/tmp/x.so"), "3.0.2")

    def test_raises_when_manifest_json_is_malformed(self):
        # Not valid JSON at all from an SDK that DOES support `manifest`.
        bc.run_capture = lambda cmd, **kw: _completed(stdout="{not json")
        with self.assertRaises(SystemExit):
            bc.read_plugin_secrets(pathlib.Path("/tmp/x.so"), "3.0.2")

    def test_raises_when_manifest_json_is_null(self):
        bc.run_capture = lambda cmd, **kw: _completed(stdout="null")
        with self.assertRaises(SystemExit):
            bc.read_plugin_secrets(pathlib.Path("/tmp/x.so"), "3.0.2")

    def test_raises_when_manifest_json_is_a_string(self):
        bc.run_capture = lambda cmd, **kw: _completed(stdout='"a string"')
        with self.assertRaises(SystemExit):
            bc.read_plugin_secrets(pathlib.Path("/tmp/x.so"), "3.0.2")

    def test_raises_when_manifest_json_is_a_bare_list(self):
        bc.run_capture = lambda cmd, **kw: _completed(stdout="[]")
        with self.assertRaises(SystemExit):
            bc.read_plugin_secrets(pathlib.Path("/tmp/x.so"), "3.0.2")

    def test_raises_when_secrets_field_is_not_a_list(self):
        bc.run_capture = lambda cmd, **kw: _completed(
            stdout=json.dumps({"name": "hello", "secrets": "oops"})
        )
        with self.assertRaises(SystemExit):
            bc.read_plugin_secrets(pathlib.Path("/tmp/x.so"), "3.0.2")


class IsUnsupportedManifestErrorTests(unittest.TestCase):
    def test_recognizes_common_clap_phrasings(self):
        self.assertTrue(bc.is_unsupported_manifest_error("error: unrecognized subcommand 'manifest'"))
        self.assertTrue(bc.is_unsupported_manifest_error("error: no such subcommand: `manifest`"))
        self.assertTrue(bc.is_unsupported_manifest_error("error: unexpected argument 'manifest' found"))

    def test_recognizes_older_clap_phrasing_that_names_the_subcommand(self):
        # Older clap (v2/v3) phrasing that still names `manifest` as the
        # rejected token -- this is the subcommand-missing case, not a flag
        # rejection, so it must still classify as "unsupported".
        self.assertTrue(
            bc.is_unsupported_manifest_error(
                "error: Found argument 'manifest' which wasn't expected, or isn't valid in this context"
            )
        )

    def test_does_not_recognize_unrelated_failures(self):
        self.assertFalse(bc.is_unsupported_manifest_error("thread 'main' panicked at 'index out of bounds'"))
        self.assertFalse(bc.is_unsupported_manifest_error("error: connection reset by peer"))
        self.assertFalse(bc.is_unsupported_manifest_error(""))

    def test_does_not_recognize_flag_level_rejection(self):
        # P1 regression: `manifest` subcommand exists but rejects our `--lib`
        # flag. clap's message contains "unexpected argument", but the named
        # token is `--lib`, not `manifest` -- this is a real manifest-contract
        # bug and must NOT be classified as "unsupported" (old SDK).
        self.assertFalse(bc.is_unsupported_manifest_error("error: unexpected argument '--lib' found"))


if __name__ == "__main__":
    unittest.main()
