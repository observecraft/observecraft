# ADR-0008: Findings are fixed where they come from: provenance, tagging policies and Assist fixes

Status: proposed (2026-11-14).

## Context

Security findings tell people what is wrong, not where to change it. Code Security findings (CI uploads, the GitLab
import) carry a repository, a file and a line; everything else does not:

- Cloud Security misconfigurations name a cloud resource (an ARN, an Azure resource id, a GCP self link) or a
  Kubernetes object, never the Terraform file or the Helm values that created it.
- Container image vulnerabilities name an image, not the Dockerfile.
- Runtime findings (runtime SCA, IAST, App & API Protection) name a service, and only sometimes its repository
  (`git.repository_url` on the spans, from `DD_GIT_*`).

Assist already reads repositories and opens merge requests and pull requests through the org's source code connections
(`assist/code.rs`, Developer Portal connectors): it proposes whole-file changes, a person reviews them and opens the
request. What it lacks is knowing which repository and file to open for a finding, and a way to start from a finding.

Where a resource comes from can be known, but only from conventions the customer's tooling follows:

- **Terraform:** tags on the resources naming the repository and the module path (the AWS provider's `default_tags`;
  `locals` merged into every resource on Azure, whose provider has no default tags; or Yor, Apache-2.0, adding
  `git_repo` / `git_file` / `yor_trace` in CI), or the state file mapping resource addresses to cloud ids.
- **Kubernetes through Argo CD:** the tracking annotation `argocd.argoproj.io/tracking-id` (or the
  `app.kubernetes.io/instance` label) names the Application, whose `spec.source` gives `repoURL`, `path` and
  `targetRevision`. The orchestrator manifests the Agent sends already include the objects; Application objects come
  with the CRD collection.
- **Images:** the OCI labels `org.opencontainers.image.source` and `org.opencontainers.image.revision`.
- **Services:** Datadog's unified service tagging (`service`, `env`, `version`) and git metadata (`git.repository_url`,
  `git.commit.sha`).

Datadog recommends the service conventions; there is no common convention for infrastructure provenance, and most
estates follow none consistently. Fixing infrastructure automatically without a reliable source would edit the wrong
file.

## Decision

1. **Provenance is a first-class property of findings and resources.** A resolver (`provenance.rs`) returns, for a
   cloud resource, a Kubernetes object, an image or a service: `{repo, path, ref, address?, kind: terraform | helm |
   kustomize | manifest | dockerfile | code, confidence: exact | inferred, via: tag | state | argocd | oci | git_metadata}`.
   Order: explicit tags and labels (exact), Argo CD Applications and OCI labels (exact), Terraform state when the org
   connected a read-only state source (exact), a repository search for the resource's name or address in the org's
   connected repositories (inferred). No provenance is a valid answer and is shown as such.
2. **Tagging policies** describe the conventions: per scope (cloud account, cluster, resource type), the required tags
   and their allowed values. We ship a recommended default set (unified service tagging + git metadata for services;
   `managed-by`, `tf_repo`, `tf_path` for Terraform-managed cloud resources; Argo CD tracking for Kubernetes objects;
   OCI source and revision for images), which an org can change. Stored in Postgres per org; evaluated against the
   resource catalog, the Kubernetes inventory, the image inventory and the service catalog.
3. **Coverage is visible where it hurts:** a Tag Coverage page (compliance per policy, account, cluster and team, over
   time, with the non-compliant resources and what each lacks); a "Source" column and a "no provenance" filter in the
   resource catalog and the Kubernetes explorer; a "Resource missing required tags" misconfiguration rule (low
   severity) so non-compliance reaches the Security Inbox, monitors and notifications; on a finding without
   provenance, the fix action explains which tag is missing and how to add it instead of failing.
4. **Fixes start from a finding and end in a reviewed request.** Three levels, chosen by what is known:
   - *Explain:* any finding, an explanation and general remediation steps (Assist, no repository access).
   - *Fix in code:* findings with a repository, file and line. Library vulnerabilities are fixed deterministically
     (bump the dependency to the first fixed version in the manifest and, where we can update it without running the
     package manager, the lockfile); static code, secrets and IaC findings are fixed by an Assist run limited to the
     finding's file and its neighbours.
   - *Fix in infrastructure:* findings whose resource has provenance. Assist opens the resolved file (the Terraform
     resource at its address, the Helm values or manifest of the Argo CD Application, the Dockerfile) and proposes the
     change; inferred provenance is shown as such and needs a confirmation before Assist edits.
   Every fix is a proposal: the person reviews the diff and opens the merge request; nothing is merged automatically;
   the description names the finding, its rule or CVE and the change; the repository's own CI validates it
   (`terraform plan`, `helm template`, tests). The finding links to the request and shows it until it merges; the next
   scan closes the finding.
5. **Helping people tag** is part of the feature: the policy and setup pages give the snippets for each convention, and
   Assist can open the merge request that adds them to a repository (Terraform `default_tags` or a Yor CI job, OCI
   labels in the image build, `DD_GIT_*` in the deployment).

## Consequences

- Code findings get fixes immediately (the data exists); infrastructure fixes grow with the estate's conventions, and
  the coverage page tells which part of the estate can be fixed and what to change to get the rest.
- The same provenance serves other features: cost allocation by team or repository, ownership in the Software Catalog,
  Scorecards (a "tagging policy" rule per service and team).
- Terraform state access is optional and read-only (an S3 / Azure Blob / GCS object or an HTTP backend); without it,
  Terraform provenance needs the tags.
- Repository writes go through the existing connections and their tokens' scopes; orgs that only grant read access get
  explanations and diffs to copy, not merge requests.
- An inferred match can be wrong; it is labelled, needs a confirmation, and a wrong fix stops at review and CI.
