# World Market Apps

This private repository builds hosted Aomi app releases for the
`world-market-apps` platform.

The Aomi backend owns source access, deployment records, candidate branches,
release tags, and activation. Do not hand-edit app directories. Backend deploys
stage source under:

```text
apps/<installation-id>/<repo-key>/<app>/
```

Candidate branches use:

```text
<source-owner>/<source-repo>/<installation-id>/<short-source-commit>
```

Pushes from the Aomi Build GitHub Apps trigger
`.github/workflows/build-candidate.yml`. The workflow validates the
backend-generated deployment manifest, builds the Rust `cdylib`, and publishes
the release bundle and provenance metadata.

Apps must declare:

```toml
platform = "world-market-apps"
```

and pin the SDK version declared in [`platform.json`](./platform.json).

## Repository Layout

```text
world-market-apps/
|-- platform.json
|-- apps/
|   `-- .gitkeep
`-- .github/
    |-- workflows/build-candidate.yml
    `-- scripts/
        |-- build_candidate.py
        `-- test_build_candidate.py
```

See [`CONTRIBUTING.md`](./CONTRIBUTING.md) for the deployment contract.
