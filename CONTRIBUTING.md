# Contributing

Application source belongs in its linked source repository, not in this
platform repository.

1. Pin `aomi-sdk` to `=3.0.4`.
2. Set `platform = "world-market-apps"` in `aomi.toml`.
3. Install the Aomi GitHub App on the source repository.
4. Deploy through Aomi Build or `POST /api/platforms/world-market-apps/deploy`.
5. Let the backend create the candidate branch and platform pull request.

The candidate release workflow accepts only backend-generated, repo-keyed app
paths and deployment manifests. It publishes Linux x86-64 release bundles for
backend activation.
