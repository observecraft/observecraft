# ADR-0006: Cloud accounts are scanned with Prowler, pinned, in a separate scanner

Status: proposed (2026-11-06).

## Context

Cloud Security's misconfigurations cover hosts and Kubernetes only (`cloud_security_rules.rs`, our own rules). The
cloud accounts themselves (public buckets, wide IAM policies, unencrypted databases, logging turned off) aren't
checked, and there are no compliance frameworks. Datadog evaluates its own rules on the resources its cloud
integrations collect; those rules are proprietary.

Writing hundreds of checks per cloud ourselves would take months. The providers' own services (AWS Security Hub with
Config, Microsoft Defender CSPM, Google Security Command Center Premium) are billed per resource or per evaluation, which
customers running a self-hosted platform often want to avoid. Open-source scanners cover the same ground:

- **Prowler** (Apache-2.0): AWS, Azure, GCP and Kubernetes checks with compliance mappings (CIS, NIST, PCI, SOC 2,
  MITRE ATT&CK, …), OCSF JSON output, read-only credentials.
- **Trivy** (Apache-2.0): images and filesystems. Its cloud account scanning (`trivy aws`) was removed in v0.53 and the
  plugin replacing it is unmaintained, so it is only for images.

Both are supply-chain targets: in March 2026 Trivy's release pipeline was compromised and malicious binaries and
images were published under new tags (GHSA-69fq-xp46-6x23); images referenced by digest were not affected.

Threat detection (GuardDuty-like detections on CloudTrail and activity logs) is a separate concern, handled by Cloud SIEM
rules, and out of scope here.

## Decision

1. **Scanners are third-party, unmodified and pinned.** Prowler runs from its official image referenced by digest
   (`prowlercloud/prowler:<version>@sha256:…`); the image isn't cosign-signed, so a version bump is a reviewed PR that
   changes the digest and checks the image's build provenance. Trivy, when added, is pinned by digest and its cosign
   signature verified. No floating tags (`latest`, `stable`, action tags).
2. **A separate `scanner` deployment** (the pinned Prowler image plus a small entrypoint) pulls work from the server:
   it claims a scan job, receives short-lived read-only credentials the server obtained (the AWS role assumed with
   its external id, the Azure app registration, the GCP impersonated service account), runs Prowler with OCSF JSON
   output and uploads the results. The server needs no access to Docker or Kubernetes; more replicas scan more.
3. **Scheduling** reuses the cloud crawlers' jobs (`cloud_crawl_jobs`, SKIP LOCKED): a daily posture job per account
   whose `cloud_security_posture_management_collection` (AWS `resources_config`, as in Datadog's API) is on.
4. **Findings** are stored in Postgres per org (resource, check, pass/fail, severity, framework mappings, first and last
   seen, mute) and served through Datadog's `/api/v2/security/findings` (list, search, mute; the legacy
   `/api/v2/posture_management/findings` is deprecated upstream). The rule catalog is generated from Prowler's check
   metadata at the pinned version. Security → Cloud Security → Findings shows them next to our host and Kubernetes rules.
5. **Permissions:** AWS adds `SecurityAudit`, `ViewOnlyAccess` and Prowler's small additions policy to the role customers
   already create. Azure asks for `Reader` and Graph read permissions only; the checks that need Prowler's custom role
   (which can read app keys) are disabled and shown as not evaluated. GCP asks for `roles/viewer` and the IAM read
   permissions Prowler lists.

## Consequences

- Hundreds of maintained checks and framework mappings from day one; upgrades are a digest bump plus a regenerated
  catalog.
- One more image to run (Python). Scans take minutes to an hour on large accounts because of API rate limits; they run
  daily, not continuously.
- Customers grant broader read access than the metrics crawler needs; the AWS page and the IAM permissions endpoint say
  what each policy is for, and scanning stays off until enabled per account.
- Customers already paying for Security Hub, GuardDuty or Defender may later import those findings into the same model;
  it is not required.

## Accounts that already run the providers' services

Customers paying for AWS Security Hub, GuardDuty or Inspector (or Defender, Security Command Center) keep them; their
findings are imported with the same read-only role, tagged with their origin (`origin: aws_guardduty`, …), never run a
second rule engine:

- **GuardDuty** findings become Cloud SIEM signals: they are threat detection, which the scans don't cover.
- **Inspector** findings join the vulnerabilities from the Agent's SBOMs, deduplicated by CVE and resource.
- **Security Hub** overlaps the scans: per account, either ObserveCraft scanning or the Security Hub import, not both.

## Phases

1. AWS: scanner, jobs, findings storage, `/api/v2/security/findings`, the Findings page, permissions on the AWS page.
2. Import of the AWS services customers already run: GuardDuty (signals) first, then Inspector and Security Hub.
3. Azure and GCP.
4. Compliance frameworks (`datadog_compliance_custom_framework`, a compliance page).
5. Trivy for registry images (ECR, ACR, Artifact Registry).
