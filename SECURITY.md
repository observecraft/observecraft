# Security policy

## Supported versions

ObserveCraft is before 1.0. Security fixes go into the latest release only; upgrade to it to get them.

## Reporting a vulnerability

Please **do not** open a public issue, discussion or pull request for a vulnerability.

Report it privately through GitHub:
[Security → Report a vulnerability](https://github.com/observecraft/observecraft/security/advisories/new).
Include what is affected (component, version, configuration), how to reproduce it, and the impact you expect.

What happens next:

- we acknowledge the report within 3 working days;
- we confirm the issue, agree a fix and a disclosure date with you, and keep you updated in the advisory;
- the fix ships in a release, and the advisory is published with credit to you unless you prefer otherwise.

## Scope

In scope: the server (API, web app), the intake, the Helm charts, the container image and the deployment examples in
this repository.

Out of scope: the third-party components we run unmodified (ClickHouse, Postgres, NATS, Vector, the OpenTelemetry
Collector, RustFS, the Datadog Agent); report those to their projects. The demo ingest key and passwords in
`deploy/docker-compose.yml` are for local development only and are not vulnerabilities.

## Verifying releases

Release images and Helm charts are signed with [Sigstore cosign](https://docs.sigstore.dev/) (keyless, from the
release workflow) and images carry an SBOM and build provenance:

```bash
cosign verify ghcr.io/observecraft/observecraft:<version> \
  --certificate-identity-regexp '^https://github\.com/observecraft/observecraft/\.github/workflows/release\.yml@' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

The same command with `ghcr.io/observecraft/charts/observecraft:<version>` verifies a chart. The build provenance and
SBOM attestations are checked with the GitHub CLI:

```bash
gh attestation verify oci://ghcr.io/observecraft/observecraft:<version> -R observecraft/observecraft
gh attestation verify oci://ghcr.io/observecraft/observecraft:<version> -R observecraft/observecraft \
  --predicate-type https://spdx.dev/Document/v2.3
```
