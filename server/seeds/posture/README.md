# Cloud posture scanning seeds

`aws-scan-session-policy.json` is the inline session policy the server attaches when it obtains credentials for an
AWS posture scan (with the AWS managed policies `SecurityAudit` and `job-function/ViewOnlyAccess`): whatever the
customer's role allows, the scanner's credentials can do no more than these read-only actions.

It is Prowler's `permissions/prowler-additions-policy.json` at release 5.47.0 (commit `555a4cc4`,
https://github.com/prowler-cloud/prowler, Apache-2.0, Copyright Toni de la Fuente and the Prowler contributors), with
the one write action (`securityhub:BatchImportFindings`, only used to send findings to Security Hub) removed and
statement ids added. Update it with the pinned scanner image (`scanner.image` in the Helm chart's `values.yaml` and the `scanner`
service in `deploy/docker-compose.yml`).
